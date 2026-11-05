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
    let with_screen = format!(
        r#"{GOOD}
[[screen]]
name = "주 공정"
width = 800
height = 480

[[screen.widget]]
id = "w1"
kind = "tank"
label = "수위 탱크"
tag = "TANK_LEVEL"
x = 40.0
y = 50.0
width = 180.0
height = 200.0
min = 0.0
max = 100.0
"#
    );
    let original = Project::from_toml(&with_screen).unwrap();
    assert_eq!(original.screens.len(), 1);
    assert_eq!(original.screens[0].widgets.len(), 1);
    let spec = original.to_spec();
    let from_spec = Project::from_spec(spec).unwrap();
    assert_eq!(original, from_spec);
    let toml_text = original.to_toml();
    let reparsed = Project::from_toml(&toml_text).unwrap();
    assert_eq!(original, reparsed);

    let unknown_tag = with_screen.replace("tag = \"TANK_LEVEL\"", "tag = \"MISSING_TAG\"");
    assert!(err(&unknown_tag).contains("없는 태그"));
}

const MELSEC: &str = r#"
[[device]]
name = "plc1"
driver = "melsec-mc"
address = "127.0.0.1:5007"
timeout_ms = 300
scan_ms = 100

[[tag]]
name = "FURNACE_TEMP"
device = "plc1"
space = "d"
address = 0
type = "analog"
unit_text = "℃"
scale = { plc_base = 0, plc_full = 100, base = 0, full = 100 }

[[tag]]
name = "RUN_SIGNAL"
device = "plc1"
space = "m"
address = 0
type = "digital"
alarm = { kind = 0 }
"#;

#[test]
fn melsec_장치와_영역을_받아들인다() {
    let p = Project::from_toml(MELSEC).unwrap();
    assert_eq!(p.devices.len(), 1);
    assert_eq!(p.tags.len(), 2);
    assert_eq!(p.tags[0].space, Space::MelsecD);
    assert_eq!(p.tags[1].space, Space::MelsecM);
    // TOML 왕복.
    let reparsed = Project::from_toml(&p.to_toml()).unwrap();
    assert_eq!(p, reparsed);
}

#[test]
fn melsec_장치의_쓰기와_영역_혼용을_검사한다() {
    // d·w·r·m·x·y 쓰기 허용.
    let writable_d = format!(
        "{MELSEC}\n[[tag]]\nname = \"W\"\ndevice = \"plc1\"\nspace = \"d\"\naddress = 1\ntype = \"analog\"\nscale = {{ plc_base = 0, plc_full = 100, base = 0, full = 100 }}\nwritable = true\n"
    );
    let p = Project::from_toml(&writable_d).unwrap();
    assert!(p.tags.iter().any(|t| t.name == "W" && t.writable));

    let writable_w = r#"
[[device]]
name = "plc1"
driver = "melsec-mc"
address = "127.0.0.1:5007"

[[tag]]
name = "W"
writable = true
device = "plc1"
space = "w"
address = 0
type = "analog"
scale = { plc_base = 0, plc_full = 100, base = 0, full = 100 }

[[tag]]
name = "X1"
writable = true
device = "plc1"
space = "x"
address = 16
type = "digital"
"#;
    let p = Project::from_toml(writable_w).unwrap();
    assert!(p.tags.iter().any(|t| t.name == "W" && t.writable));
    assert!(p.tags.iter().any(|t| t.name == "X1" && t.writable));

    let mixed = MELSEC.replace("space = \"d\"", "space = \"holding\"");
    assert!(err(&mixed).contains("쓸 수 없다"));

    let mixed_modbus = GOOD.replace("space = \"coil\"", "space = \"m\"");
    assert!(err(&mixed_modbus).contains("쓸 수 없다"));
}

const XGT: &str = r#"
[[device]]
name = "xgt1"
driver = "xgt-fenet"
address = "127.0.0.1:2004"
timeout_ms = 300
scan_ms = 100

[[tag]]
name = "TEMP"
device = "xgt1"
space = "mw"
address = 100
type = "analog"
unit_text = "℃"
scale = { plc_base = 0, plc_full = 100, base = 0, full = 100 }

[[tag]]
name = "RUN"
device = "xgt1"
space = "mx"
address = 0
type = "digital"
alarm = { kind = 0 }
"#;

#[test]
fn xgt_장치와_영역을_받아들이고_mw_외_쓰기를_거부한다() {
    let p = Project::from_toml(XGT).unwrap();
    assert_eq!(p.devices.len(), 1);
    assert_eq!(p.tags.len(), 2);
    assert_eq!(p.tags[0].space, Space::XgtMw);
    assert_eq!(p.tags[1].space, Space::XgtMx);
    let reparsed = Project::from_toml(&p.to_toml()).unwrap();
    assert_eq!(p, reparsed);

    let writable = format!(
        "{XGT}\n[[tag]]\nname = \"W\"\ndevice = \"xgt1\"\nspace = \"mw\"\naddress = 1\ntype = \"analog\"\nscale = {{ plc_base = 0, plc_full = 100, base = 0, full = 100 }}\nwritable = true\n"
    );
    let p = Project::from_toml(&writable).unwrap();
    assert!(p.tags.iter().any(|t| t.name == "W" && t.writable));

    let mixed = XGT.replace("space = \"mw\"", "space = \"holding\"");
    assert!(err(&mixed).contains("쓸 수 없다"));
}

const FINS: &str = r#"
[[device]]
name = "omron1"
driver = "fins-tcp"
address = "127.0.0.1:9600"
timeout_ms = 300
scan_ms = 100

[[tag]]
name = "TEMP"
device = "omron1"
space = "dm"
address = 20
type = "analog"
unit_text = "℃"
scale = { plc_base = 0, plc_full = 100, base = 0, full = 100 }

[[tag]]
name = "RUN"
device = "omron1"
space = "cio"
address = 16
type = "digital"
alarm = { kind = 0 }
"#;

#[test]
fn fins_장치의_쓰기와_영역_혼용을_검사한다() {
    // dm·cio 쓰기 허용.
    let p = Project::from_toml(FINS).unwrap();
    assert_eq!(p.devices.len(), 1);
    assert_eq!(p.tags.len(), 2);
    assert_eq!(p.tags[0].space, Space::FinsDm);
    assert_eq!(p.tags[1].space, Space::FinsCio);
    let reparsed = Project::from_toml(&p.to_toml()).unwrap();
    assert_eq!(p, reparsed);

    let writable = format!(
        "{FINS}\n[[tag]]\nname = \"W\"\ndevice = \"omron1\"\nspace = \"dm\"\naddress = 1\ntype = \"analog\"\nscale = {{ plc_base = 0, plc_full = 100, base = 0, full = 100 }}\nwritable = true\n\n[[tag]]\nname = \"C1\"\ndevice = \"omron1\"\nspace = \"cio\"\naddress = 19\ntype = \"digital\"\nwritable = true\n"
    );
    let p = Project::from_toml(&writable).unwrap();
    assert!(p.tags.iter().any(|t| t.name == "W" && t.writable));
    assert!(p.tags.iter().any(|t| t.name == "C1" && t.writable));

    let mixed = FINS.replace("space = \"dm\"", "space = \"holding\"");
    assert!(err(&mixed).contains("쓸 수 없다"));
}

const S7: &str = r#"
[[device]]
name = "siemens1"
driver = "s7-tcp"
address = "127.0.0.1:102"
timeout_ms = 300
scan_ms = 100

[[tag]]
name = "TEMP"
device = "siemens1"
unit = 1
space = "s7db"
address = 0
type = "analog"
unit_text = "℃"
scale = { plc_base = 0, plc_full = 100, base = 0, full = 100 }

[[tag]]
name = "RUN"
device = "siemens1"
unit = 0
space = "s7m"
address = 0
type = "digital"
alarm = { kind = 0 }
"#;

#[test]
fn s7_장치와_영역을_받아들이고_쓰기를_거부한다() {
    let p = Project::from_toml(S7).unwrap();
    assert_eq!(p.devices.len(), 1);
    assert_eq!(p.tags.len(), 2);
    assert_eq!(p.tags[0].space, Space::S7Db);
    assert_eq!(p.tags[1].space, Space::S7M);
    let reparsed = Project::from_toml(&p.to_toml()).unwrap();
    assert_eq!(p, reparsed);

    let writable = format!(
        "{S7}\n[[tag]]\nname = \"W\"\nwritable = true\ndevice = \"siemens1\"\nunit = 1\nspace = \"s7db\"\naddress = 0\ntype = \"analog\"\nscale = {{ plc_base = 0, plc_full = 100, base = 0, full = 100 }}\n"
    );
    assert!(err(&writable).contains("읽기 전용"));

    let mixed = S7.replace("space = \"s7db\"", "space = \"holding\"");
    assert!(err(&mixed).contains("쓸 수 없다"));
}

#[test]
fn 문자열_태그를_받아들이고_왕복한다() {
    let text = r#"
[[device]]
name = "sim"
driver = "modbus-tcp"
address = "127.0.0.1:5020"

[[tag]]
name = "LINE_NAME"
device = "sim"
space = "holding"
address = 10
type = "string"
unit_text = "호"
value = "1호기 라인"
"#;
    let p = Project::from_toml(text).unwrap();
    assert_eq!(p.tags.len(), 1);
    assert!(matches!(
        &p.tags[0].body,
        nuriscada_project::TagBody::String { unit_text, initial }
        if unit_text == "호" && initial == "1호기 라인"
    ));
    let reparsed = Project::from_toml(&p.to_toml()).unwrap();
    assert_eq!(p, reparsed);

    // scale·alarm·writable은 거부한다.
    for extra in [
        "writable = true\n",
        "scale = { plc_base = 0, plc_full = 100, base = 0, full = 100 }\n",
        "alarm = { kind = 0 }\n",
    ] {
        let bad = text.replace("value = ", &format!("{extra}value = "));
        assert!(err(&bad).contains("문자열 태그"), "{extra}");
    }
}
