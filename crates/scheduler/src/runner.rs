// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 장치별 주기 스캔 루프.

use std::sync::Arc;
use std::time::Duration;

use nuriscada_driver_api::{Driver, DriverError};
use nuriscada_project::Project;
use tokio::sync::watch;
use tokio::time::{Instant, sleep_until};

use crate::engine::DeviceEngine;
use crate::hub::Hub;
use crate::plan::PlannedBlock;

/// 한 장치를 주기적으로 스캔한다. `shutdown`이 `true`가 되면 현재 스캔을 마치고 끝난다.
///
/// 스캔 시각은 시작 시각에서 `주기 × 횟수`로 계산한 절대 시각이라 지연이 누적되지 않는다.
/// 스캔이 주기보다 오래 걸려 시각을 놓치면 놓친 주기를 건너뛰고 `overruns`에 센다.
pub async fn run_device<D: Driver>(
    hub: Arc<Hub>,
    project: Arc<Project>,
    device: usize,
    mut driver: D,
    blocks: Vec<PlannedBlock>,
    mut shutdown: watch::Receiver<bool>,
) {
    let period = project.devices[device].scan;
    let period_ns = u64::try_from(period.as_nanos()).unwrap_or(u64::MAX).max(1);
    let names: Vec<String> = project.tags.iter().map(|t| t.name.clone()).collect();
    let mut engine = DeviceEngine::new(&project, device);
    let start = Instant::now();
    let mut n: u64 = 0;

    loop {
        let planned = start + Duration::from_nanos(period_ns.saturating_mul(n));
        tokio::select! {
            () = sleep_until(planned) => {}
            _ = shutdown.changed() => {}
        }
        if *shutdown.borrow() {
            return;
        }
        let jitter = Instant::now().saturating_duration_since(planned);

        let last_error = scan_once(&hub, &names, &mut engine, &mut driver, &blocks).await;
        hub.set_link(device, driver.status(), last_error);

        // 다음 계획 시각. 이미 지난 주기는 건너뛴다.
        let elapsed_ns = u64::try_from(Instant::now().saturating_duration_since(start).as_nanos())
            .unwrap_or(u64::MAX);
        let next = (elapsed_ns / period_ns).saturating_add(1).max(n + 1);
        hub.record_scan(device, jitter, next - (n + 1));
        n = next;
    }
}

/// 계획된 모든 블록을 한 번 읽는다. 마지막 오류의 설명을 돌려준다.
async fn scan_once<D: Driver>(
    hub: &Hub,
    names: &[String],
    engine: &mut DeviceEngine,
    driver: &mut D,
    blocks: &[PlannedBlock],
) -> Option<String> {
    let mut last_error = None;
    for (i, planned) in blocks.iter().enumerate() {
        match driver.read(&planned.block).await {
            Ok(data) => engine.apply(hub, names, planned, &data),
            Err(e) => {
                last_error = Some(e.to_string());
                DeviceEngine::mark_bad(hub, planned);
                // 연결·시간 초과·프로토콜 오류는 나머지 블록도 읽을 수 없다고 본다.
                // 장치가 오류를 응답한 경우는 이 블록만 실패한 것이다.
                if !matches!(
                    e,
                    DriverError::Device { .. } | DriverError::InvalidRequest(_)
                ) {
                    for rest in &blocks[i + 1..] {
                        DeviceEngine::mark_bad(hub, rest);
                    }
                    break;
                }
            }
        }
    }
    last_error
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use nuriscada_driver_api::{BlockData, LinkState, LinkStatus, ReadBlock};

    use crate::hub::{Quality, TagValue};
    use crate::plan::{PlanLimits, plan};

    /// 시험용 드라이버: 공유 상태에서 값을 읽고, 실패를 켜고 끌 수 있다.
    struct Fake {
        shared: Arc<Mutex<FakeState>>,
    }

    #[derive(Default)]
    struct FakeState {
        words: Vec<u16>,
        bits: Vec<bool>,
        fail: bool,
        reads: u64,
    }

    impl Driver for Fake {
        async fn connect(&mut self) -> Result<(), DriverError> {
            Ok(())
        }

        async fn read(&mut self, block: &ReadBlock) -> Result<BlockData, DriverError> {
            let mut s = self.shared.lock().unwrap();
            s.reads += 1;
            if s.fail {
                return Err(DriverError::Timeout);
            }
            let (a, n) = (usize::from(block.start), usize::from(block.count));
            Ok(match block.kind {
                nuriscada_driver_api::DataKind::Word => {
                    BlockData::Words(s.words[a..a + n].to_vec())
                }
                nuriscada_driver_api::DataKind::Bit => BlockData::Bits(s.bits[a..a + n].to_vec()),
            })
        }

        fn status(&self) -> LinkStatus {
            LinkStatus {
                state: LinkState::Connected,
                ..LinkStatus::initial()
            }
        }
    }

    const PROJECT: &str = r#"
[[device]]
name = "d"
driver = "modbus-tcp"
address = "127.0.0.1:1"
scan_ms = 100

[[tag]]
name = "LEVEL"
device = "d"
space = "holding"
address = 0
type = "analog"
scale = { plc_base = 0, plc_full = 100, base = 0, full = 100 }
alarm = { kind = 1, hihi = 90, gap = 5 }

[[tag]]
name = "RUN"
device = "d"
space = "coil"
address = 0
type = "digital"
alarm = { kind = 0 }
"#;

    struct Rig {
        hub: Arc<Hub>,
        shared: Arc<Mutex<FakeState>>,
        stop: watch::Sender<bool>,
        handle: tokio::task::JoinHandle<()>,
    }

    fn start() -> Rig {
        let project = Arc::new(Project::from_toml(PROJECT).unwrap());
        let hub = Hub::new(&project);
        let shared = Arc::new(Mutex::new(FakeState {
            words: vec![0; 16],
            bits: vec![false; 16],
            ..FakeState::default()
        }));
        let blocks = plan(&project, PlanLimits::MODBUS);
        let (stop, rx) = watch::channel(false);
        let handle = tokio::spawn(run_device(
            Arc::clone(&hub),
            project,
            0,
            Fake {
                shared: Arc::clone(&shared),
            },
            blocks,
            rx,
        ));
        Rig {
            hub,
            shared,
            stop,
            handle,
        }
    }

    fn level(hub: &Hub) -> crate::hub::TagSnapshot {
        hub.tags().into_iter().find(|t| t.name == "LEVEL").unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn scans_on_schedule_and_applies_values_and_alarms() {
        let rig = start();
        tokio::time::sleep(Duration::from_millis(50)).await;
        // 첫 스캔은 즉시 실행된다.
        assert_eq!(level(&rig.hub).quality, Quality::Good);
        assert_eq!(level(&rig.hub).value, Some(TagValue::Number(0.0)));

        rig.shared.lock().unwrap().words[0] = 95;
        rig.shared.lock().unwrap().bits[0] = true;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let l = level(&rig.hub);
        assert_eq!(l.value, Some(TagValue::Number(95.0)));
        assert_eq!(l.alarm_state, Some("HIHI"));
        let log = rig.hub.recent_alarms();
        let events: Vec<(&str, &str)> = log.iter().map(|r| (r.tag.as_str(), r.event)).collect();
        // 읽기 계획은 영역 번호 순(코일 → 보유 레지스터)이라 RUN이 먼저 평가된다.
        assert_eq!(events, [("RUN", "DI_ON"), ("LEVEL", "HIHI")]);
        assert_eq!(rig.hub.active_alarms().len(), 1);

        // 주기에 맞춰 스캔한 횟수: 약 150 ms 동안 2~3회.
        let scans = rig.hub.devices()[0].scans;
        assert!((2..=3).contains(&scans), "scans = {scans}");
        // 가상 시계에서는 지터가 거의 없다.
        assert!(rig.hub.devices()[0].jitter_max_us < 1000);

        rig.stop.send(true).unwrap();
        rig.handle.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn failure_marks_bad_keeps_last_value_and_recovers() {
        let rig = start();
        rig.shared.lock().unwrap().words[0] = 40;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(level(&rig.hub).value, Some(TagValue::Number(40.0)));

        rig.shared.lock().unwrap().fail = true;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let l = level(&rig.hub);
        assert_eq!(l.quality, Quality::Bad);
        assert_eq!(
            l.value,
            Some(TagValue::Number(40.0)),
            "마지막 값을 유지한다"
        );
        assert!(rig.hub.devices()[0].last_error.is_some());

        rig.shared.lock().unwrap().fail = false;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(level(&rig.hub).quality, Quality::Good);

        rig.stop.send(true).unwrap();
        rig.handle.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn unchanged_values_are_not_republished() {
        let rig = start();
        tokio::time::sleep(Duration::from_millis(50)).await;
        // 첫 스캔: 태그 두 개의 품질이 Bad → Good으로 바뀌어 한 묶음에 담긴다.
        let first = rig.hub.drain_changes().expect("첫 변경 묶음");
        assert_eq!(first.seq, 1);
        assert_eq!(first.items.len(), 2);
        // 값이 그대로면 이후 스캔은 변경을 만들지 않는다.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(rig.hub.drain_changes().is_none());
        assert!(rig.hub.devices()[0].scans >= 5);
        // 값이 바뀌면 다음 묶음에 한 번만 담기고 순번이 이어진다.
        rig.shared.lock().unwrap().words[0] = 7;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let next = rig.hub.drain_changes().expect("변경 묶음");
        assert_eq!(next.seq, 2);
        assert_eq!(next.items.len(), 1);
        rig.stop.send(true).unwrap();
        rig.handle.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn slow_scans_skip_missed_periods() {
        // 읽기 한 번에 250 ms가 걸리는 드라이버: 주기 100 ms를 놓친다.
        struct Slow;
        impl Driver for Slow {
            async fn connect(&mut self) -> Result<(), DriverError> {
                Ok(())
            }
            async fn read(&mut self, block: &ReadBlock) -> Result<BlockData, DriverError> {
                tokio::time::sleep(Duration::from_millis(250)).await;
                Ok(match block.kind {
                    nuriscada_driver_api::DataKind::Word => {
                        BlockData::Words(vec![0; usize::from(block.count)])
                    }
                    nuriscada_driver_api::DataKind::Bit => {
                        BlockData::Bits(vec![false; usize::from(block.count)])
                    }
                })
            }
            fn status(&self) -> LinkStatus {
                LinkStatus::initial()
            }
        }
        let project = Arc::new(Project::from_toml(PROJECT).unwrap());
        let hub = Hub::new(&project);
        let blocks = plan(&project, PlanLimits::MODBUS);
        let (stop, rx) = watch::channel(false);
        let handle = tokio::spawn(run_device(Arc::clone(&hub), project, 0, Slow, blocks, rx));
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let d = &hub.devices()[0];
        assert!(d.overruns >= 1, "overruns = {}", d.overruns);
        stop.send(true).unwrap();
        handle.await.unwrap();
    }
}
