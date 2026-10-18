// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 장치 하나의 값 엔진: 읽은 원시 값을 환산하고 알람을 평가해 허브에 반영한다.

use std::collections::HashMap;

use nuriscada_core::alarm::{AnalogAlarm, DigitalAlarm, DigitalKind};
use nuriscada_core::scaling::AiValue;
use nuriscada_driver_api::BlockData;
use nuriscada_project::{Project, TagBody};

use crate::hub::{Hub, Quality, TagValue};
use crate::plan::PlannedBlock;

enum Slot {
    Analog {
        value: AiValue,
        alarm: Option<AnalogAlarm>,
    },
    Digital {
        alarm: Option<DigitalAlarm>,
    },
}

/// 장치 하나가 가진 태그들의 실행 상태.
pub struct DeviceEngine {
    slots: HashMap<usize, Slot>,
}

impl DeviceEngine {
    /// 프로젝트에서 `device`에 속한 태그의 상태를 만든다. 검증을 마친 프로젝트를 가정한다.
    pub fn new(project: &Project, device: usize) -> Self {
        let mut slots = HashMap::new();
        for (i, t) in project
            .tags
            .iter()
            .enumerate()
            .filter(|(_, t)| t.device == device)
        {
            let slot = match &t.body {
                TagBody::Analog { scale, alarm, .. } => Slot::Analog {
                    value: AiValue::new(*scale),
                    alarm: alarm.map(|a| {
                        AnalogAlarm::new(a.kind, a.limits).expect("프로젝트 검증이 확인했다")
                    }),
                },
                TagBody::Digital { alarm_kind } => Slot::Digital {
                    alarm: alarm_kind.map(|k| {
                        DigitalAlarm::new(
                            DigitalKind::new(k).expect("프로젝트 검증이 확인했다"),
                            false,
                        )
                    }),
                },
            };
            slots.insert(i, slot);
        }
        Self { slots }
    }

    /// 블록 하나의 읽기 결과를 반영한다.
    pub fn apply(&mut self, hub: &Hub, names: &[String], planned: &PlannedBlock, data: &BlockData) {
        for &(index, offset) in &planned.tags {
            let Some(slot) = self.slots.get_mut(&index) else {
                continue;
            };
            let offset = usize::from(offset);
            let name = &names[index];
            match (slot, data) {
                (Slot::Analog { value, alarm }, BlockData::Words(words)) => {
                    let Some(&raw) = words.get(offset) else {
                        continue;
                    };
                    match value.update(f64::from(raw)) {
                        Some(v) => {
                            let mut state = None;
                            if let Some(a) = alarm {
                                for e in a.check(v) {
                                    hub.push_alarm(name, e.as_str(), v);
                                }
                                state = Some(a.status().as_str());
                            }
                            hub.set_tag(index, Some(TagValue::Number(v)), Quality::Good, state);
                        }
                        // 범위 밖이라 버려진 값: 이전 값을 유지하고 읽기는 성공으로 본다.
                        None => hub.set_tag(index, None, Quality::Good, None),
                    }
                }
                (Slot::Digital { alarm }, BlockData::Bits(bits)) => {
                    let Some(&bit) = bits.get(offset) else {
                        continue;
                    };
                    if let Some(a) = alarm
                        && let Some(e) = a.check(bit)
                    {
                        hub.push_alarm(name, e.as_str(), if bit { 1.0 } else { 0.0 });
                    }
                    hub.set_tag(index, Some(TagValue::Bool(bit)), Quality::Good, None);
                }
                // 데이터 종류가 맞지 않는 블록은 반영하지 않는다(계획 단계에서 막힌다).
                _ => {}
            }
        }
    }

    /// 블록의 태그를 읽을 수 없는 상태로 표시한다.
    pub fn mark_bad(hub: &Hub, planned: &PlannedBlock) {
        for &(index, _) in &planned.tags {
            hub.mark_bad(index);
        }
    }
}
