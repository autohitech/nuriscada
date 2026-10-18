// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 기대값 데이터(`tag-name.json`, `comma-text-write.json`) 및 원본 형식 가져오기(SQ-M01~SQ-M03) 시험.

use std::fs;
use std::path::PathBuf;

use nuriscada_project::legacy::{
    CommaField, format_comma_text_line, import_legacy_text, parse_comma_text_line,
    resolve_tag_names,
};
use serde::Deserialize;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/expected")
        .join(name)
}

#[derive(Deserialize)]
struct TagNameFixture {
    cases: Vec<TagNameCase>,
}

#[derive(Deserialize)]
struct TagNameCase {
    case: String,
    siblings: Vec<String>,
    tries: Vec<String>,
    expect: Vec<String>,
}

#[derive(Deserialize)]
struct CommaTextWriteFixture {
    cases: Vec<CommaTextWriteCase>,
}

#[derive(Deserialize)]
struct CommaTextWriteCase {
    case: String,
    fields: Vec<CommaField>,
    expect: String,
}

#[test]
fn passes_tag_name_expected_fixtures() {
    let raw = fs::read_to_string(fixture_path("tag-name.json")).unwrap();
    let fixture: TagNameFixture = serde_json::from_str(&raw).unwrap();
    assert!(!fixture.cases.is_empty());
    for c in fixture.cases {
        let got = resolve_tag_names(&c.siblings, &c.tries);
        assert_eq!(got, c.expect, "case {} failed", c.case);
    }
}

#[test]
fn passes_comma_text_write_expected_fixtures_and_read_rules() {
    let raw = fs::read_to_string(fixture_path("comma-text-write.json")).unwrap();
    let fixture: CommaTextWriteFixture = serde_json::from_str(&raw).unwrap();
    assert!(!fixture.cases.is_empty());
    for c in fixture.cases {
        let got = format_comma_text_line(&c.fields);
        assert_eq!(got, c.expect, "case {} failed", c.case);
    }

    // SQ-M03: 인용 복원 및 따옴표 없는 항목의 앞 공백 손실 확인
    assert_eq!(parse_comma_text_line("Name,\"a,b\","), vec!["Name", "a,b"]);
    assert_eq!(
        parse_comma_text_line("Name,\"he said \"\"hi\"\"\","),
        vec!["Name", "he said \"hi\""]
    );
    assert_eq!(parse_comma_text_line("Name,  x,"), vec!["Name", "x"]);
    assert_eq!(parse_comma_text_line("Name,,"), vec!["Name", ""]);
}

#[test]
fn imports_legacy_tagx_and_scan_with_report() {
    let input = r#"
# SCAN 포트와 Local.tagx 혼합 예시
Type,Name,DriverOrDevice,Address
SCAN,plc1,MODBUS_TCP,127.0.0.1:5020,5,300,
SCAN,legacy_melsec,MELSEC,192.168.0.10:5000,100,500,
AI,TANK_LEVEL,plc1,40001,"%",0,1000,0,100,6,90,80,20,10,2,True,
AI,TANK_LEVEL,plc1,holding,1,"°C",0,500,0,50,
DO,PUMP_CMD,plc1,coil,0,0,
ST,IGNORED_STRING_TAG,plc1,0,
AI,BAD_LINE,plc1,invalid_space,
"#;

    let report = import_legacy_text(None, input, None).unwrap();
    assert_eq!(report.imported_devices, 1);
    assert_eq!(report.imported_tags, 3);
    assert_eq!(report.renamed_tags.len(), 1);
    assert_eq!(report.renamed_tags[0].original, "TANK_LEVEL");
    assert_eq!(report.renamed_tags[0].resolved, "TANK_LEVEL1");

    // 40001 -> holding 0 변환 확인
    let t0 = &report.project.tags[0];
    assert_eq!(t0.name, "TANK_LEVEL");
    assert_eq!(t0.space, "holding");
    assert_eq!(t0.address, 0);
    assert!(t0.writable);

    // 중복 이름 자동 해결 확인
    let t1 = &report.project.tags[1];
    assert_eq!(t1.name, "TANK_LEVEL1");
    assert_eq!(t1.address, 1);

    // DO 태그는 기본 writable=true
    let t2 = &report.project.tags[2];
    assert_eq!(t2.name, "PUMP_CMD");
    assert_eq!(t2.kind, "digital");
    assert!(t2.writable);

    // SQ-M02 보고서 항목 검증: 바뀐 항목, 무시한 항목, 읽지 못한 항목이 모두 기록되어야 함
    assert!(!report.modified_items.is_empty());
    assert!(report.ignored_items.len() >= 3); // 헤더 + MELSEC + ST
    assert_eq!(report.unreadable_items.len(), 1);
}
