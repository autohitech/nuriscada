// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 쓰기 인터록(허용 조건) 설정 파서 시험.

use nuriscada_project::Project;

const BASE: &str = r#"
[[device]]
name = "sim"
driver = "modbus-tcp"
address = "127.0.0.1:5020"
timeout_ms = 300
scan_ms = 100

[[tag]]
name = "READY"
device = "sim"
space = "coil"
address = 0
type = "digital"
writable = true

[[tag]]
name = "MOTOR"
device = "sim"
space = "coil"
address = 1
type = "digital"
writable = true

[[tag.interlock]]
tag = "READY"
value = true
"#;

#[test]
fn 인터록이_없으면_빈_목록이다() {
    let text = BASE.replace("\n[[tag.interlock]]\ntag = \"READY\"\nvalue = true\n", "\n");
    let p = Project::from_toml(&text).unwrap();
    assert!(p.tags.iter().all(|t| t.interlocks.is_empty()));
}

#[test]
fn 인터록을_읽고_다시_쓰면_유지된다() {
    let p = Project::from_toml(BASE).unwrap();
    assert_eq!(p.tags.len(), 2);
    assert_eq!(p.tags[1].interlocks.len(), 1);
    assert_eq!(p.tags[1].interlocks[0].tag, 0);
    let out = p.to_toml();
    assert!(out.contains("[[tag.interlock]]"), "{out}");
    let q = Project::from_toml(&out).unwrap();
    assert_eq!(q.tags[1].interlocks, p.tags[1].interlocks);
}

#[test]
fn 없는_태그를_조건으로_두면_거부한다() {
    let text = BASE.replace("tag = \"READY\"", "tag = \"MISSING\"");
    assert!(Project::from_toml(&text).is_err());
}

#[test]
fn 자기_자신을_조건으로_두면_거부한다() {
    let text = BASE.replace("tag = \"READY\"", "tag = \"MOTOR\"");
    assert!(Project::from_toml(&text).is_err());
}

#[test]
fn 종류가_맞지_않는_조건값이면_거부한다() {
    let text = BASE.replace("value = true", "value = 1");
    assert!(Project::from_toml(&text).is_err());
}

#[test]
fn 쓰기_불가_태그의_인터록은_거부한다() {
    let text = BASE.replace(
        "name = \"MOTOR\"\ndevice = \"sim\"\nspace = \"coil\"\naddress = 1\ntype = \"digital\"\nwritable = true",
        "name = \"MOTOR\"\ndevice = \"sim\"\nspace = \"coil\"\naddress = 1\ntype = \"digital\"",
    );
    assert!(Project::from_toml(&text).is_err());
}

#[test]
fn 아날로그_태그에_정수_조건을_둘_수_있다() {
    let text = BASE
        .replace(
            "name = \"READY\"\ndevice = \"sim\"\nspace = \"coil\"\naddress = 0\ntype = \"digital\"\nwritable = true",
            "name = \"MODE\"\ndevice = \"sim\"\nspace = \"holding\"\naddress = 0\ntype = \"analog\"\nwritable = true\nscale = { plc_base = 0, plc_full = 10, base = 0, full = 10 }",
        )
        .replace("tag = \"READY\"\nvalue = true", "tag = \"MODE\"\nvalue = 2");
    let p = Project::from_toml(&text).unwrap();
    assert_eq!(p.tags[1].interlocks.len(), 1);
}

#[test]
fn 문자열_태그에_정확히_일치_조건을_둘_수_있다() {
    let text = BASE
        .replace(
            "name = \"READY\"\ndevice = \"sim\"\nspace = \"coil\"\naddress = 0\ntype = \"digital\"\nwritable = true",
            "name = \"MODE\"\ndevice = \"sim\"\nspace = \"holding\"\naddress = 5\ntype = \"string\"\nvalue = \"AUTO\"\nlength = 4",
        )
        .replace("tag = \"READY\"\nvalue = true", "tag = \"MODE\"\nvalue = \"AUTO\"");
    let p = Project::from_toml(&text).unwrap();
    assert_eq!(p.tags[1].interlocks.len(), 1);
    let out = p.to_toml();
    assert!(out.contains("value = \"AUTO\""), "{out}");
    let q = Project::from_toml(&out).unwrap();
    assert_eq!(q.tags[1].interlocks, p.tags[1].interlocks);
}

#[test]
fn 문자열_조건의_종류가_맞지_않으면_거부한다() {
    let text = BASE
        .replace(
            "name = \"READY\"\ndevice = \"sim\"\nspace = \"coil\"\naddress = 0\ntype = \"digital\"\nwritable = true",
            "name = \"MODE\"\ndevice = \"sim\"\nspace = \"holding\"\naddress = 5\ntype = \"string\"\nvalue = \"AUTO\"\nlength = 4",
        )
        .replace("tag = \"READY\"\nvalue = true", "tag = \"MODE\"\nvalue = 1");
    assert!(Project::from_toml(&text).is_err());
    let text2 = BASE.replace(
        "tag = \"READY\"\nvalue = true",
        "tag = \"READY\"\nvalue = \"x\"",
    );
    assert!(Project::from_toml(&text2).is_err());
}
