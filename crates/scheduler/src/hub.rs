// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 태그 값, 알람 기록, 장치 상태의 허브. 서버가 읽고 웹소켓으로 밀어 보낸다.

use std::collections::VecDeque;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nuriscada_driver_api::{LinkState, LinkStatus};
use nuriscada_project::{Project, TagBody};
use serde::Serialize;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::jitter::JitterStats;

/// 보관하는 최근 알람 기록 수.
pub const ALARM_LOG_CAPACITY: usize = 1000;
/// 선(wire) 방송 채널의 용량(프레임 수). 프레임은 주기마다 한 개 정도라 작아도 충분하다.
/// 느린 수신자는 일부를 놓치고 전체 스냅샷을 다시 받는다.
const BROADCAST_CAPACITY: usize = 256;

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

/// 방송하는 프레임 하나. 이미 부호화되어 있어 모든 수신자가 같은 문자열을 공유한다.
#[derive(Debug, Clone)]
pub struct Frame {
    /// 태그 묶음이면 그 순번. 알람 기록이면 `None`.
    pub seq: Option<u64>,
    /// JSON 문자열.
    pub text: Arc<str>,
}

/// 마지막 방송 이후 바뀐 태그들의 묶음. 한 주기에 하나만 만든다.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    /// 방송 순번(1부터, 빈 묶음은 만들지 않으므로 건너뛰지 않는다).
    pub seq: u64,
    /// 묶음을 만든 시각(UTC 밀리초). 묶음 안 태그의 변경 시각으로도 쓴다.
    pub time_ms: u64,
    /// 바뀐 태그: (태그 목록에서의 위치, 값, 품질, 알람 상태). 값이 없으면 `None`이다.
    pub items: Vec<BatchItem>,
}

/// 묶음 안의 태그 하나.
pub type BatchItem = (usize, Option<TagValue>, Quality, Option<&'static str>);

/// 허브의 일관된 전체 상태. 새로 연결한 수신자가 맞추는 기준이다.
#[derive(Debug, Clone)]
pub struct HubSnapshot {
    /// 이 상태에 반영된 마지막 묶음의 순번. 이보다 작거나 같은 묶음은 이미 반영되어 있다.
    pub seq: u64,
    /// 모든 태그.
    pub tags: Vec<TagSnapshot>,
    /// 알람 상태가 정상이 아닌 태그.
    pub active: Vec<TagSnapshot>,
    /// 최근 알람 기록.
    pub recent: Vec<AlarmRecord>,
}

struct Inner {
    tags: Vec<TagSnapshot>,
    alarms: VecDeque<AlarmRecord>,
    next_seq: u64,
    devices: Vec<DeviceStatus>,
    jitter: Vec<JitterStats>,
    /// 마지막 방송 이후 바뀐 태그 위치.
    dirty: Vec<usize>,
    is_dirty: Vec<bool>,
    publish_seq: u64,
    /// 아직 방송하지 않은 가장 오래된 알람 기록의 순번.
    alarm_cursor: u64,
}

/// 값·알람·상태 허브.
pub struct Hub {
    inner: RwLock<Inner>,
    /// 이미 부호화한 프레임(JSON 문자열)의 방송. 한 번만 부호화해 모든 수신자가 공유한다.
    wire: broadcast::Sender<Frame>,
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
        let (wire, _) = broadcast::channel(BROADCAST_CAPACITY);
        let tag_count = project.tags.len();
        Arc::new(Self {
            inner: RwLock::new(Inner {
                tags,
                alarms: VecDeque::new(),
                next_seq: 1,
                devices,
                jitter,
                dirty: Vec::new(),
                is_dirty: vec![false; tag_count],
                publish_seq: 0,
                alarm_cursor: 1,
            }),
            wire,
            started: Instant::now(),
        })
    }

    /// 새 프로젝트 설정으로 태그·장치 목록을 재구성하고 전체 스냅샷을 방송한다.
    ///
    /// 기존 최근 알람 기록과 알람 순번은 유지한다.
    pub fn reconfigure(&self, project: &Project) {
        let tags: Vec<TagSnapshot> = project
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
        let tag_count = tags.len();
        let snap = {
            let mut inner = self.write();
            inner.tags = tags;
            inner.devices = devices;
            inner.jitter = jitter;
            inner.dirty.clear();
            inner.is_dirty = vec![false; tag_count];
            inner.publish_seq += 1;
            HubSnapshot {
                seq: inner.publish_seq,
                tags: inner.tags.clone(),
                active: Vec::new(),
                recent: inner.alarms.iter().cloned().collect(),
            }
        };
        let _ = self.wire.send(Frame {
            seq: None,
            text: crate::wire::encode_snapshot(&snap).into(),
        });
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner.read().expect("허브 잠금")
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner.write().expect("허브 잠금")
    }

    /// 부호화된 프레임(태그 묶음, 알람 기록)을 구독한다. 연결 상태를 맞추려면 **먼저 구독한 뒤**
    /// [`Hub::snapshot`]을 가져와야 하고, 순번이 스냅샷 이하인 묶음은 버려야 한다(서버가 한다).
    pub fn subscribe_wire(&self) -> broadcast::Receiver<Frame> {
        self.wire.subscribe()
    }

    /// 일관된 전체 상태(한 번의 잠금 안에서 읽는다).
    pub fn snapshot(&self) -> HubSnapshot {
        let inner = self.read();
        HubSnapshot {
            seq: inner.publish_seq,
            tags: inner.tags.clone(),
            active: inner
                .tags
                .iter()
                .filter(|t| matches!(t.alarm_state, Some(s) if s != "NORMAL"))
                .cloned()
                .collect(),
            recent: inner.alarms.iter().cloned().collect(),
        }
    }

    /// 마지막 방송 이후 바뀐 태그를 묶음으로 꺼낸다. 바뀐 것이 없으면 `None`이다.
    pub fn drain_changes(&self) -> Option<Batch> {
        let mut inner = self.write();
        if inner.dirty.is_empty() {
            return None;
        }
        let dirty = std::mem::take(&mut inner.dirty);
        let items = dirty
            .into_iter()
            .map(|i| {
                inner.is_dirty[i] = false;
                let t = &inner.tags[i];
                (i, t.value, t.quality, t.alarm_state)
            })
            .collect();
        inner.publish_seq += 1;
        Some(Batch {
            seq: inner.publish_seq,
            time_ms: now_ms(),
            items,
        })
    }

    /// 마지막 방송 이후 생긴 알람 기록(오래된 것부터). 최근 기록 한도를 넘어 밀려난 것은 포함하지 못한다.
    pub fn drain_alarms(&self) -> Vec<AlarmRecord> {
        let mut inner = self.write();
        let from = inner.alarm_cursor;
        let mut out: Vec<AlarmRecord> = inner
            .alarms
            .iter()
            .rev()
            .take_while(|r| r.seq >= from)
            .cloned()
            .collect();
        out.reverse();
        inner.alarm_cursor = inner.next_seq;
        out
    }

    /// 변경 묶음과 알람 기록을 주기마다 부호화해 방송하는 작업을 시작한다. `shutdown`이 `true`가 되면 끝난다.
    pub fn spawn_publisher(
        self: &Arc<Self>,
        interval: Duration,
        mut shutdown: watch::Receiver<bool>,
    ) -> JoinHandle<()> {
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        let alarms = hub.drain_alarms();
                        if !alarms.is_empty() {
                            let _ = hub.wire.send(Frame {
                                seq: None,
                                text: crate::wire::encode_alarms(&alarms).into(),
                            });
                        }
                        if let Some(batch) = hub.drain_changes() {
                            let _ = hub.wire.send(Frame {
                                seq: Some(batch.seq),
                                text: crate::wire::encode_batch(&batch).into(),
                            });
                        }
                    }
                    _ = shutdown.changed() => {}
                }
                if *shutdown.borrow() {
                    return;
                }
            }
        })
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

    /// 태그 상태를 갱신한다. 값·품질·알람 상태 중 바뀐 것이 있을 때만 변경으로 표시한다.
    /// 변경은 [`Hub::drain_changes`]가 묶어 방송한다.
    pub(crate) fn set_tag(
        &self,
        index: usize,
        value: Option<TagValue>,
        quality: Quality,
        alarm_state: Option<&'static str>,
    ) {
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
        if !inner.is_dirty[index] {
            inner.is_dirty[index] = true;
            inner.dirty.push(index);
        }
    }

    /// 태그를 읽을 수 없는 상태(품질 Bad)로 표시한다. 마지막 값은 유지한다.
    pub(crate) fn mark_bad(&self, index: usize) {
        self.set_tag(index, None, Quality::Bad, None);
    }

    /// 알람 기록을 추가한다. 방송은 주기마다 묶어서 한다([`Hub::drain_alarms`]).
    pub(crate) fn push_alarm(&self, tag: &str, event: &'static str, value: f64) {
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
        inner.alarms.push_back(record);
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
