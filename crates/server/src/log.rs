// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 한 줄 JSON 진단 기록(SQ-O03). 구조화 로그 수집기가 그대로 읽는다.
//!
//! 형식: `{"ts":<UTC밀리초>,"level":"...","target":"...","msg":"..."}` 한 줄.
//! `target`은 발생 위치(예: `api`, `diagnostic`, `archive`), `msg`는 사람용 한 줄이다.

use serde_json::{Map, Value};

/// 진단 기록 한 줄을 만든다(순수 함수라 시험할 수 있다).
pub fn event_line(level: &str, target: &str, msg: &str, fields: &[(&str, &str)]) -> String {
    let mut obj = Map::with_capacity(3 + fields.len());
    obj.insert("ts".into(), Value::from(nuriscada_scheduler::now_ms()));
    obj.insert("level".into(), Value::from(level));
    obj.insert("target".into(), Value::from(target));
    obj.insert("msg".into(), Value::from(msg));
    for (k, v) in fields {
        obj.insert((*k).into(), Value::from(*v));
    }
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| String::from("{}"))
}

/// 진단 기록 한 줄을 표준 오류에 낸다.
pub fn emit(level: &str, target: &str, msg: &str) {
    eprintln!("{}", event_line(level, target, msg, &[]));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 한_줄_json에_개행이_없다() {
        let line = event_line("error", "api", "실패: \"따옴표\"\n개행", &[("tag", "T1")]);
        assert!(!line.contains('\n'), "{line}");
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["level"], "error");
        assert_eq!(v["target"], "api");
        assert_eq!(v["msg"], "실패: \"따옴표\"\n개행");
        assert_eq!(v["tag"], "T1");
        assert!(v["ts"].is_number());
    }
}
