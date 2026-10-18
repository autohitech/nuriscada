// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 설정 파서 시험.

use nuriscada_project::{ConfigError, Project, Space, TagBody};

const GOOD: &str = r#"
[server]
listen = "127.0.0.1:9000"
publish_ms = 50

[[device]]
name = "sim"
driver = "modbus-tcp"
address = "127.0.0.1:5020"
timeout_ms = 300
scan_ms = 100

[[tag]]
name = "TANK_LEVEL"
device = "sim"
space = "holding"
address = 0
type = "analog"
unit_text = "%"
scale = { plc_base = 0, plc_full = 100, base = 0, full = 100 }
alarm = { kind = 6, hihi = 90, high = 70, low = 30, lolo = 10, gap = 5 }

[[tag]]
name = "PUMP_RUN"
device = "sim"
space = "coil"
address = 0
type = "digital"
alarm = { kind = 0 }
"#;

fn err(text: &str) -> String {
    match Project::from_toml(text).unwrap_err() {
        ConfigError::Parse(m) | ConfigError::Invalid(m) => m,
    }
}

#[test]
fn parses_a_valid_project() {
    let p = Project::from_toml(GOOD).unwrap();
    assert_eq!(p.server.listen, "127.0.0.1:9000");
    assert_eq!(p.server.publish.as_millis(), 50);
    assert_eq!(p.devices.len(), 1);
    assert_eq!(p.devices[0].scan.as_millis(), 100);
    assert_eq!(p.tags.len(), 2);
    assert_eq!(p.tags[0].space, Space::Holding);
    assert!(matches!(
        p.tags[1].body,
        TagBody::Digital {
            alarm_kind: Some(0)
        }
    ));
}

#[test]
fn defaults_to_loopback() {
    let p = Project::from_toml("").unwrap();
    assert_eq!(p.server.listen, "127.0.0.1:8080");
    assert_eq!(p.server.publish.as_millis(), 100);
    assert!(p.devices.is_empty() && p.tags.is_empty());
}

#[test]
fn rejects_bad_projects() {
    let dup = format!(
        "{GOOD}\n[[tag]]\nname = \"PUMP_RUN\"\ndevice = \"sim\"\nspace = \"coil\"\naddress = 1\ntype = \"digital\"\n"
    );
    assert!(err(&dup).contains("중복"));

    let no_dev = GOOD.replace(
        "device = \"sim\"\nspace = \"coil\"",
        "device = \"none\"\nspace = \"coil\"",
    );
    assert!(err(&no_dev).contains("없는 장치"));

    let slow_pub = GOOD.replace("publish_ms = 50", "publish_ms = 5");
    assert!(err(&slow_pub).contains("방송 주기"));

    let fast = GOOD.replace("scan_ms = 100", "scan_ms = 5");
    assert!(err(&fast).contains("최소"));

    let drv = GOOD.replace("modbus-tcp", "other");
    assert!(err(&drv).contains("지원하지 않는 드라이버"));

    let wrong_space = GOOD.replace("space = \"holding\"", "space = \"coil\"");
    assert!(err(&wrong_space).contains("아날로그 태그는"));

    let missing_limit = GOOD.replace("lolo = 10, ", "");
    assert!(err(&missing_limit).contains("lolo"));

    let unknown_field = GOOD.replace("unit_text = \"%\"", "unit_txt = \"%\"");
    assert!(err(&unknown_field).contains("unit_txt"));

    let bad_kind = GOOD.replace("kind = 0 }", "kind = 7 }");
    assert!(err(&bad_kind).contains("알람"));

    assert!(matches!(
        Project::from_toml("[[tag"),
        Err(ConfigError::Parse(_))
    ));
}

#[test]
fn round_trips_spec_and_toml() {
    let original = Project::from_toml(GOOD).unwrap();
    let spec = original.to_spec();
    let from_spec = Project::from_spec(spec).unwrap();
    assert_eq!(original, from_spec);
    let toml_text = original.to_toml();
    let reparsed = Project::from_toml(&toml_text).unwrap();
    assert_eq!(original, reparsed);
}
