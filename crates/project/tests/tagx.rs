// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 태그 파일 읽기 시험. 줄은 분석 명세의 필드 순서(공통 19개 + 종류별 고유 필드)로 만든다.

use nuriscada_project::tagx::import_tagx;
use nuriscada_project::{DeviceSpec, ProjectSpec, ServerSpec};

fn base() -> ProjectSpec {
    ProjectSpec {
        server: ServerSpec::default(),
        devices: vec![DeviceSpec {
            name: "plc".into(),
            driver: "modbus-tcp".into(),
            address: "127.0.0.1:502".into(),
            timeout_ms: 500,
            scan_ms: 100,
        }],
        tags: Vec::new(),
        screens: Vec::new(),
    }
}

/// 공통 19개 필드(이름, 설명, act, ...). `name`과 `act`만 바꾼다.
fn common(name: &str, act: &str) -> Vec<String> {
    let mut f: Vec<String> = vec![name.into(), "설명".into(), act.into()];
    f.extend(std::iter::repeat_n("0".to_string(), 16));
    assert_eq!(f.len(), 19);
    f
}

fn quote(s: &str) -> String {
    if s.contains([',', '"']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn line(kind: &str, fields: &[String]) -> String {
    let mut parts = vec![kind.to_string()];
    parts.extend(fields.iter().map(|f| quote(f)));
    parts.join(",") + ","
}

/// AI 줄: 고유 51개 중 읽는 값만 채운다.
#[allow(clippy::too_many_arguments)]
fn ai(
    name: &str,
    station: &str,
    address: &str,
    filter: &str,
    alarm: &str,
    alarm_kind: &str,
    unit: &str,
) -> String {
    let mut f = common(name, "1");
    let mut s = vec!["0".to_string(); 51];
    s[1] = station.into(); // 2: 국번
    s[2] = address.into(); // 3: 주소
    s[10] = unit.into(); // 11: 단위
    s[11] = "0".into(); // 12: 표시 하한
    s[12] = "100".into(); // 13: 표시 상한
    s[13] = "0".into(); // 14: 장치 하한
    s[14] = "4000".into(); // 15: 장치 상한
    s[15] = "90".into(); // 16: 상상한
    s[16] = "70".into(); // 17: 상한
    s[17] = "30".into(); // 18: 하한
    s[18] = "10".into(); // 19: 하하한
    s[21] = alarm.into(); // 22: 알람 사용
    s[28] = alarm_kind.into(); // 29: 알람 종류
    s[31] = filter.into(); // 32: 환산 필터
    s[32] = "1".into(); // 33: 범위 처리
    s[33] = "2".into(); // 34: 지연
    s[37] = "5".into(); // 38: 복귀 여유
    f.extend(s);
    line("AI", &f)
}

fn di(name: &str, address: &str, alarm: &str, alarm_kind: &str) -> String {
    let mut f = common(name, "1");
    let mut s = vec!["0".to_string(); 32];
    s[1] = "1".into();
    s[2] = address.into();
    s[12] = alarm.into(); // 13: 알람 사용
    s[18] = alarm_kind.into(); // 19: 알람 종류
    f.extend(s);
    line("DI", &f)
}

fn ao(name: &str, address_hex: &str) -> String {
    let mut f = common(name, "1");
    let mut s = vec!["0".to_string(); 20];
    s[1] = "1".into();
    s[2] = address_hex.into();
    s[10] = "bar".into();
    s[12] = "100".into();
    s[14] = "100".into();
    f.extend(s);
    line("AO", &f)
}

fn doo(name: &str, address_hex: &str) -> String {
    let mut f = common(name, "1");
    let mut s = vec!["0".to_string(); 16];
    s[1] = "1".into();
    s[2] = address_hex.into();
    f.extend(s);
    line("DO", &f)
}

#[test]
fn reads_analog_input_with_alarm_and_scaling() {
    let text = ai("TANK.LEVEL", "3", "100", "1", "1", "6", "%");
    let r = import_tagx(&base(), &text, "plc").unwrap();
    assert_eq!(r.imported_tags, 1);
    let t = &r.project.tags[0];
    assert_eq!((t.name.as_str(), t.unit, t.address), ("TANK.LEVEL", 3, 100));
    assert_eq!(
        (t.space.as_str(), t.kind.as_str(), t.writable),
        ("holding", "analog", false)
    );
    assert_eq!(t.unit_text, "%");
    let s = t.scale.as_ref().unwrap();
    assert_eq!(
        (s.plc_base, s.plc_full, s.base, s.full),
        (0.0, 4000.0, 0.0, 100.0)
    );
    assert_eq!((s.profile, s.range, s.delay), (1, 1, 2));
    let a = t.alarm.as_ref().unwrap();
    assert_eq!(a.kind, 6);
    assert_eq!(
        (a.hihi, a.high, a.low, a.lolo),
        (Some(90.0), Some(70.0), Some(30.0), Some(10.0))
    );
    assert_eq!(a.gap, 5.0);
    // 통신 매핑 경고가 보고서에 있다.
    assert!(r.modified_items.iter().any(|m| m.item == "통신 매핑"));
}

#[test]
fn alarm_off_means_no_alarm_spec() {
    let r = import_tagx(&base(), &ai("A", "1", "1", "0", "0", "6", ""), "plc").unwrap();
    assert!(r.project.tags[0].alarm.is_none());
}

#[test]
fn unsupported_filter_is_reported() {
    let r = import_tagx(&base(), &ai("A", "1", "1", "5", "0", "0", ""), "plc").unwrap();
    assert!(
        r.modified_items
            .iter()
            .any(|m| m.detail.contains("환산 필터 5"))
    );
}

#[test]
fn reads_digital_input_address_as_coil_bit() {
    let text = [di("D1", "2.3", "1", "0"), di("D2", "1.Ah", "0", "0")].join("\n");
    let r = import_tagx(&base(), &text, "plc").unwrap();
    let t = &r.project.tags;
    assert_eq!(
        (t[0].space.as_str(), t[0].address, t[0].kind.as_str()),
        ("coil", 35, "digital")
    );
    assert_eq!(t[0].alarm.as_ref().unwrap().kind, 0);
    assert_eq!(t[1].address, 26); // 1*16 + 0xA
    assert!(t[1].alarm.is_none());
}

#[test]
fn outputs_are_writable_with_hex_addresses() {
    let text = [ao("SP", "1F"), doo("RUN", "0Ah")].join("\r\n");
    let r = import_tagx(&base(), &text, "plc").unwrap();
    let t = &r.project.tags;
    assert!(t[0].writable && t[0].address == 0x1F && t[0].kind == "analog");
    assert!(t[1].writable && t[1].address == 0x0A && t[1].kind == "digital");
}

#[test]
fn skips_comments_blank_lines_groups_and_unknown_types() {
    let text = format!(
        "\u{feff};TagType,Tag,Description\n\n{}\nGR,TANK,{}\nXX,foo,bar\nST,STR1,x\n",
        ai("TANK.A", "1", "1", "0", "0", "0", ""),
        "g,".repeat(18)
    );
    let r = import_tagx(&base(), &text, "plc").unwrap();
    assert_eq!(r.imported_tags, 1);
    assert!(r.ignored_items.iter().any(|i| i.item == "XX"));
    assert!(r.ignored_items.iter().any(|i| i.item == "STR1"));
    assert!(r.ignored_items.iter().any(|i| i.item == "GR"));
    assert!(r.unreadable_items.is_empty(), "{:?}", r.unreadable_items);
}

#[test]
fn resolves_name_collisions_case_insensitively() {
    let text = [
        ai("Tag01", "1", "1", "0", "0", "0", ""),
        ai("TAG01", "1", "2", "0", "0", "0", ""),
    ]
    .join("\n");
    let r = import_tagx(&base(), &text, "plc").unwrap();
    assert_eq!(r.project.tags[1].name, "TAG02");
    assert_eq!(r.renamed_tags.len(), 1);
    assert_eq!(r.renamed_tags[0].original, "TAG01");
}

#[test]
fn quoted_commas_and_short_lines() {
    let mut f = common("A,B", "1");
    f.extend(vec!["0".to_string(); 51]);
    let ok = line("AI", &f);
    let short = "AI,SHORT,desc,1,";
    let r = import_tagx(&base(), &format!("{ok}\n{short}"), "plc").unwrap();
    assert_eq!(r.project.tags[0].name, "A,B");
    assert_eq!(r.unreadable_items.len(), 1);
    assert_eq!(r.unreadable_items[0].line, 2);
}

#[test]
fn bad_numbers_and_ranges_are_unreadable_not_panics() {
    let text = [
        ai("N1", "1", "abc", "0", "0", "0", ""),
        ai("N2", "1", "70000", "0", "0", "0", ""),
        ai("N3", "1", "1", "0", "1", "42", ""),
        di("N4", "x.y", "0", "0"),
    ]
    .join("\n");
    let r = import_tagx(&base(), &text, "plc").unwrap();
    assert_eq!(r.imported_tags, 0);
    assert_eq!(r.unreadable_items.len(), 4, "{:?}", r.unreadable_items);
}

#[test]
fn station_out_of_range_falls_back_and_reports() {
    let r = import_tagx(&base(), &ai("S", "300", "1", "0", "0", "0", ""), "plc").unwrap();
    assert_eq!(r.project.tags[0].unit, 1);
    assert!(
        r.modified_items
            .iter()
            .any(|m| m.detail.contains("국번 300"))
    );
}

#[test]
fn unknown_target_device_is_an_error() {
    assert!(import_tagx(&base(), "", "nope").is_err());
}

#[test]
fn keeps_existing_tags_and_avoids_their_names() {
    let mut b = base();
    let first = import_tagx(&b, &ai("X1", "1", "1", "0", "0", "0", ""), "plc").unwrap();
    b = first.project;
    let again = import_tagx(&b, &ai("x1", "1", "2", "0", "0", "0", ""), "plc").unwrap();
    assert_eq!(again.project.tags.len(), 2);
    assert_eq!(again.project.tags[1].name, "x2");
}
