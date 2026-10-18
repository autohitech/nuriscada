// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 쉼표 텍스트 읽기·쓰기 규칙, 태그 이름 충돌 해결, 프로젝트 가져오기 보고서(SQ-M02, SQ-M03).
//!
//! **주의**: [`import_legacy_text`]가 읽는 줄 형식(`DEVICE,...`, `AI,...`)은 NuriScada가 정한 단순한 가져오기
//! 형식이다. 기존 제품의 실제 프로젝트 파일(`Local.tagx`, `SCAN.NNN`)의 레코드 배치를 구현한 것이 **아니다**.
//! 실제 파일 형식 읽기(SQ-M01)는 분석 명세를 바탕으로 한 별도 모듈로 추가해야 하며 아직 없다.

use serde::{Deserialize, Serialize};

use crate::{
    AlarmSpec, ConfigError, DeviceSpec, MIN_SCAN_MS, Project, ProjectSpec, ScaleSpec, ServerSpec,
    TagSpec,
};

/// 쉼표 텍스트 한 칸의 값 형식(`tests/fixtures/expected/comma-text-write.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommaField {
    /// 불리언(`True` / `False`).
    Bool(bool),
    /// 정수.
    Int(i64),
    /// 부동소수.
    Float(f64),
    /// 문자열.
    Str(String),
}

impl From<&str> for CommaField {
    fn from(value: &str) -> Self {
        Self::Str(value.to_string())
    }
}

impl From<String> for CommaField {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<bool> for CommaField {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for CommaField {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<f64> for CommaField {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

/// 원본 쉼표 텍스트 쓰기 규칙(`comma-text-write.json`)에 따라 한 줄을 직렬화한다.
///
/// - 모든 항목 뒤에 쉼표(`,`)를 붙인다(줄 끝에도 쉼표가 붙는다).
/// - 문자열에 쉼표(`,`), 큰따옴표(`"`)가 있으면 큰따옴표로 감싸고 내부 큰따옴표는 `""`로 두 번 적는다.
///   줄바꿈(`\r`, `\n`)이 있는 문자열도 **의도적으로** 따옴표로 감싼다. 분석에서 예측한 기존 쓰기 규칙은 줄바꿈을
///   보호하지 않아 레코드가 깨지는데, 새 구현은 그 결함을 재현하지 않는다(결정 필요 시 되돌릴 수 있다).
/// - 앞 공백은 따옴표로 감싸지 않고 그대로 출력한다(`CT-W3`).
/// - 불리언은 `True` / `False`로 적는다(`CT-W5`).
pub fn format_comma_text_line(fields: &[CommaField]) -> String {
    let mut out = String::new();
    for field in fields {
        match field {
            CommaField::Bool(b) => out.push_str(if *b { "True" } else { "False" }),
            CommaField::Int(i) => out.push_str(&i.to_string()),
            CommaField::Float(v) => out.push_str(&v.to_string()),
            CommaField::Str(s) => {
                if s.contains([',', '"', '\r', '\n']) {
                    out.push('"');
                    for ch in s.chars() {
                        if ch == '"' {
                            out.push_str("\"\"");
                        } else {
                            out.push(ch);
                        }
                    }
                    out.push('"');
                } else {
                    out.push_str(s);
                }
            }
        }
        out.push(',');
    }
    out
}

/// 원본 쉼표 텍스트 읽기 규칙(SQ-M03)에 따라 한 줄을 분리한다.
///
/// - 따옴표 없는 항목의 앞 공백(스페이스·탭)은 손실된다(SQ-M03 앞 공백 손실 재현).
/// - 큰따옴표로 감싼 항목은 내부 공백·쉼표를 유지하고 `""`를 `"`로 복원한다.
/// - 줄 끝에 붙은 구분자 쉼표 하나는 빈 항목으로 추가하지 않는다.
pub fn parse_comma_text_line(line: &str) -> Vec<String> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() {
        return Vec::new();
    }
    let has_trailing_comma = trimmed.ends_with(',');
    let chars: Vec<char> = trimmed.chars().collect();
    let mut fields = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
            i += 1;
        }
        if i >= chars.len() {
            fields.push(String::new());
            break;
        }
        if chars[i] == '"' {
            i += 1;
            let mut val = String::new();
            while i < chars.len() {
                if chars[i] == '"' {
                    if i + 1 < chars.len() && chars[i + 1] == '"' {
                        val.push('"');
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                } else {
                    val.push(chars[i]);
                    i += 1;
                }
            }
            while i < chars.len() && chars[i] != ',' {
                i += 1;
            }
            fields.push(val);
            if i < chars.len() && chars[i] == ',' {
                i += 1;
                if i == chars.len() && !has_trailing_comma {
                    fields.push(String::new());
                }
            }
        } else {
            let start = i;
            while i < chars.len() && chars[i] != ',' {
                i += 1;
            }
            let val: String = chars[start..i].iter().collect();
            fields.push(val);
            if i < chars.len() && chars[i] == ',' {
                i += 1;
                if i == chars.len() && !has_trailing_comma {
                    fields.push(String::new());
                }
            }
        }
    }

    fields
}

/// 태그 이름 충돌 시 새 이름을 결정한다(`tests/fixtures/expected/tag-name.json`).
///
/// - 충돌 검사는 영문 대소문자를 구분하지 않는다(`S_TN08`).
/// - 후보 이름에 연속된 아라비아 숫자 구간이 있으면 마지막 숫자 구간을 자릿수(0 채움 폭)를 유지해 1씩 증가시킨다(`S_TN02`, `S_TN03`, `S_TN05`, `S_TN06`, `S_TN07`).
/// - 숫자 구간이 없으면 뒤에 `1`, `2`, ...를 붙인다(`S_TN01`, `S_TN08`).
pub fn resolve_tag_name(siblings: &[String], candidate: &str) -> String {
    let collides = |name: &str| siblings.iter().any(|s| s.eq_ignore_ascii_case(name));
    if !collides(candidate) {
        return candidate.to_string();
    }

    let bytes = candidate.as_bytes();
    let mut digit_end = None;
    let mut digit_start = 0;
    for idx in (0..bytes.len()).rev() {
        if bytes[idx].is_ascii_digit() {
            if digit_end.is_none() {
                digit_end = Some(idx + 1);
            }
            digit_start = idx;
        } else if digit_end.is_some() {
            break;
        }
    }

    if let Some(end) = digit_end {
        let prefix = &candidate[..digit_start];
        let digits = &candidate[digit_start..end];
        let suffix = &candidate[end..];
        let width = digits.len();
        let mut num = digits.parse::<u64>().unwrap_or(0);
        loop {
            num = num.saturating_add(1);
            let next = format!("{prefix}{num:0width$}{suffix}");
            if !collides(&next) {
                return next;
            }
        }
    } else {
        let mut num = 1u64;
        loop {
            let next = format!("{candidate}{num}");
            if !collides(&next) {
                return next;
            }
            num = num.saturating_add(1);
        }
    }
}

/// 여러 후보 태그 이름을 순서대로 배정하며, 앞서 배정된 이름도 형제 목록에 누적한다(`S_TN07`).
pub fn resolve_tag_names(siblings: &[String], tries: &[String]) -> Vec<String> {
    let mut known: Vec<String> = siblings.to_vec();
    let mut out = Vec::with_capacity(tries.len());
    for candidate in tries {
        let resolved = resolve_tag_name(&known, candidate);
        known.push(resolved.clone());
        out.push(resolved);
    }
    out
}

/// 태그 이름 변경 내역.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRenameItem {
    /// 줄 번호(1부터 시작).
    pub line: usize,
    /// 원본 태그 이름.
    pub original: String,
    /// 충돌 해결 후 배정된 태그 이름.
    pub resolved: String,
}

/// 가져오기 보고서 개별 항목(SQ-M02: 읽지 못한 항목, 무시한 항목, 값이 바뀐 항목).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportNotice {
    /// 줄 번호(1부터 시작).
    pub line: usize,
    /// 대상 이름 또는 레코드 종류.
    pub item: String,
    /// 상세 설명.
    pub detail: String,
}

/// 원본·쉼표 텍스트 가져오기 결과 보고서(SQ-M02).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportReport {
    /// 가져온 장치 수.
    pub imported_devices: usize,
    /// 가져온 태그 수.
    pub imported_tags: usize,
    /// 이름 충돌로 자동 변경된 태그 목록.
    pub renamed_tags: Vec<ImportRenameItem>,
    /// 값이 바뀐 항목(주소 체계 변환, 스캔 주기 보정, 이름 변경 등).
    pub modified_items: Vec<ImportNotice>,
    /// 무시한 항목(헤더, 미지원 태그·드라이버 종류 등).
    pub ignored_items: Vec<ImportNotice>,
    /// 읽지 못한 항목(필드 부족, 숫자 파싱 오류 등).
    pub unreadable_items: Vec<ImportNotice>,
    /// 변환·병합이 적용된 결과 프로젝트 명세.
    pub project: ProjectSpec,
}

/// 쉼표 텍스트로 쓴 가져오기 파일을 읽어 기존 프로젝트 명세에 병합하거나 새로 변환한다(SQ-M02, SQ-M03).
/// 줄 형식은 NuriScada가 정한 것이며 기존 제품의 실제 파일 배치와 다를 수 있다.
///
/// 지원 레코드 형식:
/// - 장치(`DEVICE` / `SCAN` / `PORT`):
///   `DEVICE,<이름>,<드라이버>,<호스트:포트>[,<스캔_ms>[,<타임아웃_ms>]]`
/// - 아날로그 태그(`AI` / `AO` / `ANALOG`):
///   `AI,<이름>,<장치>,<영역 또는 MODBUS 5자리 주소>,<주소>[,<단위>,<plc_base>,<plc_full>,<base>,<full>,<알람종류>,<hihi>,<high>,<low>,<lolo>,<gap>,<쓰기허용>]`
/// - 디지털 태그(`DI` / `DO` / `DIGITAL`):
///   `DI,<이름>,<장치>,<영역 또는 MODBUS 5자리 주소>,<주소>[,<알람종류>,<쓰기허용>]`
pub fn import_legacy_text(
    base: Option<&ProjectSpec>,
    text: &str,
    default_device: Option<&str>,
) -> Result<ImportReport, ConfigError> {
    let mut project = base.cloned().unwrap_or_else(|| ProjectSpec {
        server: ServerSpec::default(),
        devices: Vec::new(),
        tags: Vec::new(),
        screens: Vec::new(),
    });

    let mut imported_devices = 0usize;
    let mut imported_tags = 0usize;
    let mut renamed_tags = Vec::new();
    let mut modified_items = Vec::new();
    let mut ignored_items = Vec::new();
    let mut unreadable_items = Vec::new();

    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with(';')
            || trimmed.starts_with("//")
        {
            continue;
        }

        let fields = parse_comma_text_line(raw_line);
        if fields.is_empty() {
            continue;
        }

        let kind_raw = fields[0].trim();
        let kind_upper = kind_raw.to_ascii_uppercase();

        if matches!(
            kind_upper.as_str(),
            "TYPE" | "KIND" | "TAGTYPE" | "RECORD" | "구분" | "종류"
        ) {
            ignored_items.push(ImportNotice {
                line: line_no,
                item: kind_raw.to_string(),
                detail: "머리글(헤더) 줄을 건너뛰었습니다.".to_string(),
            });
            continue;
        }

        match kind_upper.as_str() {
            "DEVICE" | "SCAN" | "PORT" => {
                if fields.len() < 4 {
                    unreadable_items.push(ImportNotice {
                        line: line_no,
                        item: kind_raw.to_string(),
                        detail: "장치 줄은 최소 4개 필드(DEVICE,이름,드라이버,주소)가 필요합니다."
                            .to_string(),
                    });
                    continue;
                }
                let dev_name = fields[1].trim();
                if dev_name.is_empty() {
                    unreadable_items.push(ImportNotice {
                        line: line_no,
                        item: kind_raw.to_string(),
                        detail: "장치 이름이 비어 있습니다.".to_string(),
                    });
                    continue;
                }
                let drv_raw = fields[2].trim();
                let drv_upper = drv_raw.to_ascii_uppercase();
                let driver = if drv_raw == "modbus-tcp" {
                    "modbus-tcp".to_string()
                } else if matches!(
                    drv_upper.as_str(),
                    "MODBUS-TCP" | "MODBUS_TCP" | "MODBUSTCP" | "MODBUS"
                ) {
                    modified_items.push(ImportNotice {
                        line: line_no,
                        item: dev_name.to_string(),
                        detail: format!("드라이버 이름 '{drv_raw}'을 'modbus-tcp'로 변환했습니다."),
                    });
                    "modbus-tcp".to_string()
                } else {
                    ignored_items.push(ImportNotice {
                        line: line_no,
                        item: dev_name.to_string(),
                        detail: format!(
                            "아직 지원하지 않는 드라이버('{drv_raw}') 장치를 무시했습니다."
                        ),
                    });
                    continue;
                };

                let address = fields[3].trim().to_string();
                if address.is_empty() {
                    unreadable_items.push(ImportNotice {
                        line: line_no,
                        item: dev_name.to_string(),
                        detail: "장치 통신 주소(host:port)가 비어 있습니다.".to_string(),
                    });
                    continue;
                }

                let mut scan_ms = fields
                    .get(4)
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(100);
                if scan_ms < MIN_SCAN_MS {
                    modified_items.push(ImportNotice {
                        line: line_no,
                        item: dev_name.to_string(),
                        detail: format!(
                            "스캔 주기 {scan_ms} ms가 최소치({MIN_SCAN_MS} ms)보다 작아 100 ms로 보정했습니다."
                        ),
                    });
                    scan_ms = 100;
                }

                let mut timeout_ms = fields
                    .get(5)
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(500);
                if timeout_ms == 0 {
                    modified_items.push(ImportNotice {
                        line: line_no,
                        item: dev_name.to_string(),
                        detail: "제한 시간 0 ms를 기본값 500 ms로 보정했습니다.".to_string(),
                    });
                    timeout_ms = 500;
                }

                let spec = DeviceSpec {
                    name: dev_name.to_string(),
                    driver,
                    address,
                    timeout_ms,
                    scan_ms,
                };
                if let Some(existing) = project
                    .devices
                    .iter_mut()
                    .find(|d| d.name.eq_ignore_ascii_case(dev_name))
                {
                    modified_items.push(ImportNotice {
                        line: line_no,
                        item: dev_name.to_string(),
                        detail: format!("기존 장치 '{}'의 설정을 덮어썼습니다.", existing.name),
                    });
                    *existing = spec;
                } else {
                    project.devices.push(spec);
                }
                imported_devices += 1;
            }
            "AI" | "AO" | "ANALOG" | "DI" | "DO" | "DIGITAL" => {
                if fields.len() < 4 {
                    unreadable_items.push(ImportNotice {
                        line: line_no,
                        item: kind_raw.to_string(),
                        detail: "태그 줄은 최소 4개 필드(종류,이름,장치,주소/영역)가 필요합니다."
                            .to_string(),
                    });
                    continue;
                }

                let is_analog = matches!(kind_upper.as_str(), "AI" | "AO" | "ANALOG");
                let is_output_kind = matches!(kind_upper.as_str(), "AO" | "DO");

                let raw_name = fields[1].trim();
                if raw_name.is_empty() {
                    unreadable_items.push(ImportNotice {
                        line: line_no,
                        item: kind_raw.to_string(),
                        detail: "태그 이름이 비어 있습니다.".to_string(),
                    });
                    continue;
                }

                let existing_names: Vec<String> =
                    project.tags.iter().map(|t| t.name.clone()).collect();
                let resolved_name = resolve_tag_name(&existing_names, raw_name);
                if resolved_name != raw_name {
                    renamed_tags.push(ImportRenameItem {
                        line: line_no,
                        original: raw_name.to_string(),
                        resolved: resolved_name.clone(),
                    });
                    modified_items.push(ImportNotice {
                        line: line_no,
                        item: raw_name.to_string(),
                        detail: format!(
                            "태그 이름 충돌로 '{raw_name}'을 '{resolved_name}'(으)로 변경했습니다."
                        ),
                    });
                }

                let dev_field = fields[2].trim();
                let device_name = if !dev_field.is_empty() {
                    if !project
                        .devices
                        .iter()
                        .any(|d| d.name.eq_ignore_ascii_case(dev_field))
                    {
                        project.devices.push(DeviceSpec {
                            name: dev_field.to_string(),
                            driver: "modbus-tcp".to_string(),
                            address: "127.0.0.1:5020".to_string(),
                            timeout_ms: 500,
                            scan_ms: 100,
                        });
                        imported_devices += 1;
                        modified_items.push(ImportNotice {
                            line: line_no,
                            item: resolved_name.clone(),
                            detail: format!(
                                "참조된 장치 '{dev_field}'가 없어 기본 주소(127.0.0.1:5020)로 자동 생성했습니다."
                            ),
                        });
                        dev_field.to_string()
                    } else {
                        project
                            .devices
                            .iter()
                            .find(|d| d.name.eq_ignore_ascii_case(dev_field))
                            .map(|d| d.name.clone())
                            .unwrap_or_else(|| dev_field.to_string())
                    }
                } else if let Some(def) = default_device.filter(|s| !s.trim().is_empty()) {
                    let def = def.trim();
                    if !project.devices.iter().any(|d| d.name == def) {
                        project.devices.push(DeviceSpec {
                            name: def.to_string(),
                            driver: "modbus-tcp".to_string(),
                            address: "127.0.0.1:5020".to_string(),
                            timeout_ms: 500,
                            scan_ms: 100,
                        });
                        imported_devices += 1;
                    }
                    def.to_string()
                } else if let Some(first) = project.devices.first() {
                    first.name.clone()
                } else {
                    project.devices.push(DeviceSpec {
                        name: "sim".to_string(),
                        driver: "modbus-tcp".to_string(),
                        address: "127.0.0.1:5020".to_string(),
                        timeout_ms: 500,
                        scan_ms: 100,
                    });
                    imported_devices += 1;
                    modified_items.push(ImportNotice {
                        line: line_no,
                        item: resolved_name.clone(),
                        detail: "장치가 지정되지 않아 기본 장치 'sim'(127.0.0.1:5020)을 자동 생성해 연결했습니다.".to_string(),
                    });
                    "sim".to_string()
                };

                // 영역 + 주소 파싱: (1) fields[3]이 영역 문자열이고 fields[4]가 주소이거나,
                // (2) fields[3]이 5자리 MODBUS 주소(40001, 30001, 10001, 00001)인 경우 지원
                let (mut space, address, rest_start) = match parse_space_and_address(
                    &fields,
                    is_analog,
                    line_no,
                    &resolved_name,
                    &mut modified_items,
                ) {
                    Ok(v) => v,
                    Err(reason) => {
                        unreadable_items.push(ImportNotice {
                            line: line_no,
                            item: resolved_name,
                            detail: reason,
                        });
                        continue;
                    }
                };

                if is_analog && matches!(space.as_str(), "coil" | "discrete") {
                    modified_items.push(ImportNotice {
                        line: line_no,
                        item: resolved_name.clone(),
                        detail: format!(
                            "아날로그 태그에 비트 영역('{space}')이 지정되어 'holding'으로 보정했습니다."
                        ),
                    });
                    space = "holding".to_string();
                } else if !is_analog && matches!(space.as_str(), "holding" | "input") {
                    modified_items.push(ImportNotice {
                        line: line_no,
                        item: resolved_name.clone(),
                        detail: format!(
                            "디지털 태그에 워드 영역('{space}')이 지정되어 'coil'로 보정했습니다."
                        ),
                    });
                    space = "coil".to_string();
                }

                if is_analog {
                    let unit_text = fields
                        .get(rest_start)
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let plc_base = parse_opt_f64(&fields, rest_start + 1).unwrap_or(0.0);
                    let plc_full = parse_opt_f64(&fields, rest_start + 2).unwrap_or(100.0);
                    let scale_base = parse_opt_f64(&fields, rest_start + 3).unwrap_or(0.0);
                    let scale_full = parse_opt_f64(&fields, rest_start + 4).unwrap_or(100.0);

                    let alarm = if let Some(ak) = parse_opt_u8(&fields, rest_start + 5) {
                        if ak <= 9 {
                            let hihi =
                                parse_opt_f64(&fields, rest_start + 6).or(Some(scale_full * 0.9));
                            let high =
                                parse_opt_f64(&fields, rest_start + 7).or(Some(scale_full * 0.8));
                            let low =
                                parse_opt_f64(&fields, rest_start + 8).or(Some(scale_full * 0.2));
                            let lolo =
                                parse_opt_f64(&fields, rest_start + 9).or(Some(scale_full * 0.1));
                            let gap = parse_opt_f64(&fields, rest_start + 10).unwrap_or(1.0);
                            Some(AlarmSpec {
                                kind: ak,
                                hihi,
                                high,
                                low,
                                lolo,
                                gap,
                            })
                        } else {
                            ignored_items.push(ImportNotice {
                                line: line_no,
                                item: resolved_name.clone(),
                                detail: format!(
                                    "지원하지 않는 아날로그 알람 종류({ak})를 무시했습니다."
                                ),
                            });
                            None
                        }
                    } else {
                        None
                    };

                    let writable_flag = parse_opt_bool(&fields, rest_start + 11)
                        .unwrap_or(is_output_kind && space == "holding");

                    project.tags.push(TagSpec {
                        writable: writable_flag,
                        name: resolved_name,
                        device: device_name,
                        unit: 1,
                        space,
                        address,
                        kind: "analog".to_string(),
                        scale: Some(ScaleSpec {
                            plc_base,
                            plc_full,
                            base: scale_base,
                            full: scale_full,
                            profile: 0,
                            delay: 0,
                            range: 0,
                        }),
                        alarm,
                        unit_text,
                    });
                } else {
                    let alarm = if let Some(dk) = parse_opt_u8(&fields, rest_start) {
                        if dk <= 3 {
                            Some(AlarmSpec {
                                kind: dk,
                                hihi: None,
                                high: None,
                                low: None,
                                lolo: None,
                                gap: 0.0,
                            })
                        } else {
                            ignored_items.push(ImportNotice {
                                line: line_no,
                                item: resolved_name.clone(),
                                detail: format!(
                                    "지원하지 않는 디지털 알람 종류({dk})를 무시했습니다."
                                ),
                            });
                            None
                        }
                    } else {
                        None
                    };
                    let writable_flag = parse_opt_bool(&fields, rest_start + 1)
                        .unwrap_or(is_output_kind && space == "coil");

                    project.tags.push(TagSpec {
                        writable: writable_flag,
                        name: resolved_name,
                        device: device_name,
                        unit: 1,
                        space,
                        address,
                        kind: "digital".to_string(),
                        scale: None,
                        alarm,
                        unit_text: String::new(),
                    });
                }
                imported_tags += 1;
            }
            _ => {
                ignored_items.push(ImportNotice {
                    line: line_no,
                    item: kind_raw.to_string(),
                    detail: format!("지원하지 않는 레코드 종류('{kind_raw}')를 무시했습니다."),
                });
            }
        }
    }

    let validated = Project::from_spec(project)?;
    Ok(ImportReport {
        imported_devices,
        imported_tags,
        renamed_tags,
        modified_items,
        ignored_items,
        unreadable_items,
        project: validated.to_spec(),
    })
}

fn parse_space_and_address(
    fields: &[String],
    is_analog: bool,
    line_no: usize,
    tag_name: &str,
    modified_items: &mut Vec<ImportNotice>,
) -> Result<(String, u16, usize), String> {
    let f3 = fields[3].trim();
    let f3_lower = f3.to_ascii_lowercase();
    if matches!(
        f3_lower.as_str(),
        "holding" | "input" | "coil" | "discrete" | "hr" | "ir" | "di" | "do"
    ) {
        let space = match f3_lower.as_str() {
            "holding" | "hr" => "holding",
            "input" | "ir" => "input",
            "coil" | "do" => "coil",
            "discrete" | "di" => "discrete",
            _ => unreachable!(),
        }
        .to_string();
        let addr_str = fields
            .get(4)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "태그 주소 필드가 비어 있습니다.".to_string())?;
        let addr = addr_str
            .parse::<u16>()
            .map_err(|_| format!("태그 주소 '{addr_str}'가 올바른 0~65535 정수가 아닙니다."))?;
        return Ok((space, addr, 5));
    }

    // MODBUS 5자리 관례 주소(40001~49999, 30001~39999, 10001~19999, 1~9999) 지원
    if let Ok(num) = f3.parse::<u32>() {
        if (40001..=49999).contains(&num) {
            let addr = (num - 40001) as u16;
            modified_items.push(ImportNotice {
                line: line_no,
                item: tag_name.to_string(),
                detail: format!(
                    "MODBUS 관례 주소 {num}을 holding 영역 주소 {addr}로 변환했습니다."
                ),
            });
            return Ok(("holding".to_string(), addr, 4));
        } else if (30001..=39999).contains(&num) {
            let addr = (num - 30001) as u16;
            modified_items.push(ImportNotice {
                line: line_no,
                item: tag_name.to_string(),
                detail: format!("MODBUS 관례 주소 {num}을 input 영역 주소 {addr}로 변환했습니다."),
            });
            return Ok(("input".to_string(), addr, 4));
        } else if (10001..=19999).contains(&num) {
            let addr = (num - 10001) as u16;
            modified_items.push(ImportNotice {
                line: line_no,
                item: tag_name.to_string(),
                detail: format!(
                    "MODBUS 관례 주소 {num}을 discrete 영역 주소 {addr}로 변환했습니다."
                ),
            });
            return Ok(("discrete".to_string(), addr, 4));
        } else if num <= u16::MAX as u32 {
            // 영역을 적지 않은 순수 주소는 아날로그는 보유 레지스터, 디지털은 코일로 본다.
            let default_space = if is_analog { "holding" } else { "coil" };
            return Ok((default_space.to_string(), num as u16, 4));
        }
    }

    Err(format!("영역 또는 주소 형식('{f3}')을 해석할 수 없습니다."))
}

fn parse_opt_f64(fields: &[String], idx: usize) -> Option<f64> {
    fields
        .get(idx)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

fn parse_opt_u8(fields: &[String], idx: usize) -> Option<u8> {
    fields
        .get(idx)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u8>().ok())
}

fn parse_opt_bool(fields: &[String], idx: usize) -> Option<bool> {
    let raw = fields.get(idx)?.trim();
    if raw.is_empty() {
        return None;
    }
    match raw.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" => Some(true),
        "false" | "0" | "no" | "n" => Some(false),
        _ => None,
    }
}
