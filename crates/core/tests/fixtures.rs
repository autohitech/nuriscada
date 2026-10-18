// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 기대값 데이터(`tests/fixtures/expected/`)로 환산과 알람을 검증한다.
//!
//! 데이터는 정적 분석에서 도출한 **예측**이다. 통과는 "예측과 일치"를 뜻하며
//! 기존 제품과의 동일성을 증명하지 않는다 (결정 0010).

use std::path::PathBuf;

use nuriscada_core::alarm::{AnalogAlarm, DigitalAlarm, DigitalKind, Event, Limits};
use nuriscada_core::scaling::{AiValue, Profile, RangeMode, ScaleParams};
use serde::Deserialize;
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "..",
        "tests",
        "fixtures",
        "expected",
        name,
    ]
    .iter()
    .collect();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("JSON 형식")
}

fn cases(doc: &Value) -> &Vec<Value> {
    assert_eq!(doc["status"], "prediction", "예측 데이터여야 한다");
    doc["cases"].as_array().expect("cases 배열")
}

/// 기대값 데이터의 숫자 표기: 소수 6자리 반올림, 음의 영은 "-0", 버려짐은 "drop".
fn show(v: Option<f64>) -> Value {
    match v {
        None => Value::from("drop"),
        Some(x) => {
            let r = (x * 1e6).round() / 1e6;
            if r == 0.0 && r.is_sign_negative() {
                Value::from("-0")
            } else {
                Value::from(r)
            }
        }
    }
}

#[derive(Deserialize)]
struct AiParams {
    plc_base: f64,
    plc_full: f64,
    base: f64,
    full: f64,
    #[serde(default)]
    flt: i32,
    #[serde(default)]
    delay: u32,
    #[serde(default)]
    cut: i32,
}

#[test]
fn ai_scaling_matches_expected() {
    let doc = fixture("ai-scaling.json");
    let all = cases(&doc);
    assert!(!all.is_empty());
    for c in all {
        let name = c["case"].as_str().unwrap();
        let p: AiParams = serde_json::from_value(c["params"].clone()).unwrap();
        let mut value = AiValue::new(ScaleParams {
            plc_base: p.plc_base,
            plc_full: p.plc_full,
            base: p.base,
            full: p.full,
            profile: Profile::from_code(p.flt),
            delay: p.delay,
            range: RangeMode::from_code(p.cut),
        });
        let got: Vec<Value> = c["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| show(value.update(x.as_f64().unwrap())))
            .collect();
        let want = c["expect"].as_array().unwrap();
        assert_eq!(&got, want, "사례 {name}");
    }
}

#[derive(Deserialize, Default)]
struct AlarmParams {
    hihi: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    lolo: Option<f64>,
    #[serde(default)]
    gap: f64,
}

fn names(events: &[Event]) -> Vec<Value> {
    events.iter().map(|e| Value::from(e.as_str())).collect()
}

#[test]
fn alarms_match_expected() {
    let doc = fixture("alarm.json");
    let all = cases(&doc);
    assert!(!all.is_empty());
    let (mut analog, mut digital) = (0, 0);
    for c in all {
        let name = c["case"].as_str().unwrap();
        let input = c["input"].as_array().unwrap();
        let want = c["expect"].as_array().unwrap();
        let got: Vec<Vec<Value>> = if let Some(kind) = c["kind"].as_str() {
            // 디지털: "DI0".."DI3". 입력 직전 값은 0이다.
            let k: u8 = kind.trim_start_matches("DI").parse().unwrap();
            let mut alarm = DigitalAlarm::new(DigitalKind::new(k).unwrap(), false);
            digital += 1;
            input
                .iter()
                .map(|v| {
                    let e = alarm.check(v.as_i64().unwrap() != 0);
                    names(e.as_slice())
                })
                .collect()
        } else {
            let kind = c["kind"].as_u64().unwrap() as u8;
            let p: AlarmParams = serde_json::from_value(c["params"].clone()).unwrap();
            let mut alarm = AnalogAlarm::new(
                kind,
                Limits {
                    hihi: p.hihi,
                    high: p.high,
                    low: p.low,
                    lolo: p.lolo,
                    gap: p.gap,
                },
            )
            .unwrap();
            analog += 1;
            input
                .iter()
                .map(|v| names(&alarm.check(v.as_f64().unwrap())))
                .collect()
        };
        let want: Vec<Vec<Value>> = want.iter().map(|e| e.as_array().unwrap().clone()).collect();
        assert_eq!(got, want, "사례 {name}");
    }
    assert_eq!((analog, digital), (11, 4), "사례 수가 달라졌다");
}
