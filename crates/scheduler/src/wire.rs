// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 웹소켓 프레임의 부호화. 같은 프레임을 모든 수신자가 공유하도록 한 번만 만든다.
//!
//! 태그 묶음은 **작게** 만든다: 정적 정보(이름, 장치, 단위)는 스냅샷에만 있고 묶음은 위치(인덱스)로
//! 태그를 가리킨다. 형식은 설계 0003에 있다.

use serde_json::{Value, json};

use crate::hub::{AlarmRecord, Batch, HubSnapshot, Quality, TagValue};

/// 태그 묶음 프레임: `{"type":"tags","data":{"seq":N,"time_ms":T,"items":[[i,value,q,state],...]}}`.
///
/// `q`는 1(good) 또는 0(bad), `state`는 알람 상태 문자열 또는 `null`이다.
pub fn encode_batch(batch: &Batch) -> String {
    let items: Vec<Value> = batch
        .items
        .iter()
        .map(|&(index, value, quality, state)| {
            let v = match value {
                Some(TagValue::Number(n)) => json!(n),
                Some(TagValue::Bool(b)) => json!(b),
                None => Value::Null,
            };
            json!([index, v, u8::from(quality == Quality::Good), state])
        })
        .collect();
    json!({
        "type": "tags",
        "data": { "seq": batch.seq, "time_ms": batch.time_ms, "items": items },
    })
    .to_string()
}

/// 알람 기록 프레임(한 주기에 생긴 것을 묶는다): `{"type":"alarms","data":[{...},...]}`.
///
/// 스냅샷의 `recent`와 겹칠 수 있으므로 수신자는 `seq`가 이미 본 것보다 큰 기록만 반영해야 한다.
pub fn encode_alarms(records: &[AlarmRecord]) -> String {
    json!({ "type": "alarms", "data": records }).to_string()
}

/// 전체 스냅샷 프레임: `{"type":"snapshot","data":{"seq":N,"tags":[...],"active":[...],"recent":[...]}}`.
pub fn encode_snapshot(snapshot: &HubSnapshot) -> String {
    json!({
        "type": "snapshot",
        "data": {
            "seq": snapshot.seq,
            "tags": snapshot.tags,
            "active": snapshot.active,
            "recent": snapshot.recent,
        },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_is_compact_and_indexed() {
        let batch = Batch {
            seq: 7,
            time_ms: 1000,
            items: vec![
                (3, Some(TagValue::Number(12.5)), Quality::Good, Some("HIGH")),
                (9, Some(TagValue::Bool(true)), Quality::Good, None),
                (4, None, Quality::Bad, None),
            ],
        };
        let v: Value = serde_json::from_str(&encode_batch(&batch)).unwrap();
        assert_eq!(v["type"], "tags");
        assert_eq!(v["data"]["seq"], 7);
        assert_eq!(v["data"]["items"][0], json!([3, 12.5, 1, "HIGH"]));
        assert_eq!(v["data"]["items"][1], json!([9, true, 1, null]));
        assert_eq!(v["data"]["items"][2], json!([4, null, 0, null]));
    }

    #[test]
    fn alarms_are_grouped() {
        let r = |seq| AlarmRecord {
            seq,
            time_ms: 5,
            tag: "A".into(),
            event: "HIHI",
            value: 1.0,
        };
        let v: Value = serde_json::from_str(&encode_alarms(&[r(1), r(2)])).unwrap();
        assert_eq!(v["type"], "alarms");
        assert_eq!(v["data"].as_array().unwrap().len(), 2);
        assert_eq!(v["data"][1]["seq"], 2);
    }
}
