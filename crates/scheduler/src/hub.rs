// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 태그 값, 알람 기록, 장치 상태의 허브. 서버가 읽고 웹소켓으로 밀어 보낸다.

use std::collections::VecDeque;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use nuriscada_driver_api::{LinkState, LinkStatus};
use nuriscada_project::{Project, TagBody};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::jitter::JitterStats;

/// 보관하는 최근 알람 기록 수.
pub const ALARM_LOG_CAPACITY: usize = 1000;
/// 업데이트 방송 채널의 용량. 느린 수신자는 일부를 놓치고 전체 스냅샷을 다시 받는다.
const BROADCAST_CAPACITY: usize = 4096;

/// 지금 시각(UTC, 1970년부터의 밀리초).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// 값의 품질.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// 마지막 읽기가 성공했다.
    Good,
    /// 읽지 못했거나 아직 읽은 적이 없다.
    Bad,
}

/// 태그 값.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TagValue {
    /// 아날로그 값.
    Number(f64),
    /// 디지털 값.
    Bool(bool),
}

/// 태그의 현재 상태.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagSnapshot {
    /// 태그 이름.
    pub name: String,
    /// 장치 이름.
    pub device: String,
    /// `analog` 또는 `digital`.
    pub kind: &'static str,
    /// 현재 값. 아직 읽은 적이 없으면 없다.
    pub value: Option<TagValue>,
    /// 품질.
    pub quality: Quality,
    /// 마지막으로 값·품질·알람 상태가 바뀐 시각(UTC 밀리초). 읽은 적이 없으면 0.
    pub changed_ms: u64,
    /// 단위 표시.
    pub unit_text: String,
    /// 아날로그 알람 상태(`NORMAL`, `LOLO`, `LOW`, `HIGH`, `HIHI`, `MIDDLE`). 알람이 없으면 없다.
    pub alarm_state: Option<&'static str>,
}

/// 알람 기록 하나.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlarmRecord {
    /// 일련 번호(1부터, 순서 보장).
    pub seq: u64,
    /// 발생 시각(UTC 밀리초).
    pub time_ms: u64,
    /// 태그 이름.
    pub tag: String,
    /// 기록 종류(`HIHI`, `HIGH`, `LOW`, `LOLO`, `CNF`, `RETURN`, `DI_ON`, `DI_OFF`).
    pub event: &'static str,
    /// 발생 시점의 값(디지털은 0 또는 1).
    pub value: f64,
}

/// 장치 상태.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeviceStatus {
    /// 장치 이름.
    pub name: String,
    /// 연결 여부.
    pub connected: bool,
    /// 마지막 성공 요청의 왕복 시간(밀리초).
    pub round_trip_ms: Option<f64>,
    /// 성공한 요청 수.
    pub requests_ok: u64,
    /// 실패한 요청 수.
    pub requests_failed: u64,
    /// 설정한 스캔 주기(밀리초).
    pub scan_ms: u64,
    /// 완료한 스캔 수.
    pub scans: u64,
    /// 계획된 시각을 놓친(밀린) 스캔 수.
    pub overruns: u64,
    /// 지터의 99번째 백분위수(마이크로초, 최근 표본).
    pub jitter_p99_us: Option<u64>,
    /// 지터 최댓값(마이크로초, 전체 기간).
    pub jitter_max_us: u64,
    /// 마지막 오류 설명.
    pub last_error: Option<String>,
}

/// 방송하는 업데이트.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "lowercase")]
pub enum Update {
    /// 태그 상태 변화.
    Tag(TagSnapshot),
    /// 새 알람 기록.
    Alarm(AlarmRecord),
}

struct Inner {
    tags: Vec<TagSnapshot>,
    alarms: VecDeque<AlarmRecord>,
    next_seq: u64,
    devices: Vec<DeviceStatus>,
    jitter: Vec<JitterStats>,
}

/// 값·알람·상태 허브.
pub struct Hub {
    inner: RwLock<Inner>,
    tx: broadcast::Sender<Update>,
    started: Instant,
}

impl Hub {
    /// 프로젝트 정의로 허브를 만든다. 모든 태그는 품질 `Bad`로 시작한다.
    pub fn new(project: &Project) -> Arc<Self> {
        let tags = project
            .tags
            .iter()
            .map(|t| {
                let (kind, unit_text, has_alarm) = match &t.body {
                    TagBody::Analog {
                        unit_text, alarm, ..
                    } => ("analog", unit_text.clone(), alarm.is_some()),
                    TagBody::Digital { .. } => ("digital", String::new(), false),
                };
                TagSnapshot {
                    name: t.name.clone(),
                    device: project.devices[t.device].name.clone(),
                    kind,
                    value: None,
                    quality: Quality::Bad,
                    changed_ms: 0,
                    unit_text,
                    alarm_state: has_alarm.then_some("NORMAL"),
                }
            })
            .collect();
        let devices = project
            .devices
            .iter()
            .map(|d| DeviceStatus {
                name: d.name.clone(),
                connected: false,
                round_trip_ms: None,
                requests_ok: 0,
                requests_failed: 0,
                scan_ms: u64::try_from(d.scan.as_millis()).unwrap_or(u64::MAX),
                scans: 0,
                overruns: 0,
                jitter_p99_us: None,
                jitter_max_us: 0,
                last_error: None,
            })
            .collect();
        let jitter = project.devices.iter().map(|_| JitterStats::new()).collect();
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Arc::new(Self {
            inner: RwLock::new(Inner {
                tags,
                alarms: VecDeque::new(),
                next_seq: 1,
                devices,
                jitter,
            }),
            tx,
            started: Instant::now(),
        })
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner.read().expect("허브 잠금")
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner.write().expect("허브 잠금")
    }

    /// 업데이트를 구독한다.
    pub fn subscribe(&self) -> broadcast::Receiver<Update> {
        self.tx.subscribe()
    }

    /// 서버가 시작된 뒤 흐른 초.
    pub fn uptime_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// 모든 태그의 현재 상태.
    pub fn tags(&self) -> Vec<TagSnapshot> {
        self.read().tags.clone()
    }

    /// 알람 상태가 정상이 아닌 아날로그 태그.
    pub fn active_alarms(&self) -> Vec<TagSnapshot> {
        self.read()
            .tags
            .iter()
            .filter(|t| matches!(t.alarm_state, Some(s) if s != "NORMAL"))
            .cloned()
            .collect()
    }

    /// 최근 알람 기록(오래된 것부터).
    pub fn recent_alarms(&self) -> Vec<AlarmRecord> {
        self.read().alarms.iter().cloned().collect()
    }

    /// 장치 상태.
    pub fn devices(&self) -> Vec<DeviceStatus> {
        self.read().devices.clone()
    }

    /// 태그 상태를 갱신한다. 값·품질·알람 상태 중 바뀐 것이 있을 때만 방송한다.
    pub(crate) fn set_tag(
        &self,
        index: usize,
        value: Option<TagValue>,
        quality: Quality,
        alarm_state: Option<&'static str>,
    ) {
        let update = {
            let mut inner = self.write();
            let tag = &mut inner.tags[index];
            let new_value = value.or(tag.value);
            let new_state = alarm_state.or(tag.alarm_state);
            if tag.value == new_value && tag.quality == quality && tag.alarm_state == new_state {
                return;
            }
            tag.value = new_value;
            tag.quality = quality;
            tag.alarm_state = new_state;
            tag.changed_ms = now_ms();
            tag.clone()
        };
        let _ = self.tx.send(Update::Tag(update));
    }

    /// 태그를 읽을 수 없는 상태(품질 Bad)로 표시한다. 마지막 값은 유지한다.
    pub(crate) fn mark_bad(&self, index: usize) {
        self.set_tag(index, None, Quality::Bad, None);
    }

    /// 알람 기록을 추가한다.
    pub(crate) fn push_alarm(&self, tag: &str, event: &'static str, value: f64) {
        let record = {
            let mut inner = self.write();
            let record = AlarmRecord {
                seq: inner.next_seq,
                time_ms: now_ms(),
                tag: tag.to_string(),
                event,
                value,
            };
            inner.next_seq += 1;
            if inner.alarms.len() == ALARM_LOG_CAPACITY {
                inner.alarms.pop_front();
            }
            inner.alarms.push_back(record.clone());
            record
        };
        let _ = self.tx.send(Update::Alarm(record));
    }

    /// 스캔 한 번을 기록한다.
    pub(crate) fn record_scan(&self, device: usize, jitter: std::time::Duration, overruns: u64) {
        let mut inner = self.write();
        inner.jitter[device].record(jitter);
        let (p99, max) = (
            inner.jitter[device].percentile_us(0.99),
            inner.jitter[device].max_us(),
        );
        let d = &mut inner.devices[device];
        d.scans += 1;
        d.overruns += overruns;
        d.jitter_p99_us = p99;
        d.jitter_max_us = max;
    }

    /// 드라이버 상태와 마지막 오류를 반영한다.
    pub(crate) fn set_link(&self, device: usize, link: LinkStatus, last_error: Option<String>) {
        let mut inner = self.write();
        let d = &mut inner.devices[device];
        d.connected = link.state == LinkState::Connected;
        d.round_trip_ms = link.last_round_trip.map(|r| r.as_secs_f64() * 1000.0);
        d.requests_ok = link.requests_ok;
        d.requests_failed = link.requests_failed;
        if last_error.is_some() {
            d.last_error = last_error;
        }
    }
}
