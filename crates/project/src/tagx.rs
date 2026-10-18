// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 기존 제품의 태그 파일(`Local.tagx`) 읽기(SQ-M01, SQ-M02).
//!
//! 줄 형식은 분석 명세를 따른다: 줄의 첫 필드가 종류(`AI`, `AO`, `DI`, `DO`, `ST`, `GR`, `GDO`)이고,
//! 이어서 공통 필드 19개와 종류별 고유 필드가 온다. 빈 줄과 `;`로 시작하는 줄은 건너뛴다.
//!
//! **읽는 것**: 태그 이름, 설명 없이 이름·단위, 환산(장치 범위·표시 범위·프로파일·지연·범위 처리),
//! 알람 종류와 한계·복귀 여유, 쓰기 가능 여부(AO, DO).
//! **읽지 않는 것**: 통신 설정(포트, 국번, 주소를 해석하는 규칙은 통신 규격마다 다르다). 주소는
//! 보유 레지스터(아날로그) 또는 코일(디지털)로 **추정해 옮기며 검증되지 않았다**. 보고서에 모두 적는다.
//!
//! 형식의 알려진 한계: 숫자 변환이 잘못된 값을 어떻게 다루는지는 확인하지 않았다. 이 읽기는 보수적으로
//! 숫자가 아니면 그 줄을 "읽지 못함"으로 보고한다.

use crate::legacy::{
    ImportNotice, ImportRenameItem, ImportReport, parse_comma_text_line, resolve_tag_name,
};
use crate::{AlarmSpec, ConfigError, ProjectSpec, ScaleSpec, TagSpec};

/// 공통 필드 수.
const COMMON: usize = 19;

/// 필드 위치: `TYPE`이 0번이므로 표의 k번째 필드는 `COMMON + k`번이다.
fn at(k: usize) -> usize {
    COMMON + k
}

/// 종류별로 줄에 있어야 하는 최소 필드 수(`TYPE` 포함): 읽는 가장 뒤 필드까지.
///
/// 줄 끝의 빈 필드는 쉼표 텍스트 읽기에서 생략될 수 있으므로 **읽지 않는 뒤쪽 필드는 요구하지 않는다**.
fn min_fields(kind: &str) -> usize {
    match kind {
        "AI" => at(38) + 1,
        "AO" => at(19) + 1,
        "DI" => at(19) + 1,
        "DO" => at(3) + 1,
        _ => 1 + COMMON,
    }
}

struct Row<'a> {
    fields: &'a [String],
    line: usize,
}

impl Row<'_> {
    fn s(&self, i: usize) -> &str {
        self.fields.get(i).map_or("", |s| s.trim())
    }

    fn i64(&self, i: usize, what: &str) -> Result<i64, String> {
        let t = self.s(i);
        if t.is_empty() {
            return Ok(0);
        }
        t.parse::<i64>()
            .map_err(|_| format!("{what}({i}번째 필드) '{t}'은 정수가 아니다"))
    }

    fn f64(&self, i: usize, what: &str) -> Result<f64, String> {
        let t = self.s(i);
        if t.is_empty() {
            return Ok(0.0);
        }
        t.parse::<f64>()
            .map_err(|_| format!("{what}({i}번째 필드) '{t}'은 숫자가 아니다"))
    }

    /// 16진 표기(`h` 접미어를 허용한다).
    fn hex(&self, i: usize, what: &str) -> Result<i64, String> {
        let t = self.s(i);
        if t.is_empty() {
            return Ok(0);
        }
        let digits = t.trim_end_matches(['h', 'H']);
        i64::from_str_radix(digits, 16)
            .map_err(|_| format!("{what}({i}번째 필드) '{t}'은 16진수가 아니다"))
    }
}

fn notice(line: usize, item: &str, detail: impl Into<String>) -> ImportNotice {
    ImportNotice {
        line,
        item: item.to_string(),
        detail: detail.into(),
    }
}

/// 태그 파일을 읽어 `base`에 태그를 더한다. `device`는 가져온 태그를 붙일 장치 이름이며 `base`에 있어야 한다.
///
/// 태그 이름의 점(`.`)은 그대로 둔다(계층은 이름으로 표현된다). 이름이 겹치면 이름 충돌 규칙으로
/// 새 이름을 정하고 보고서에 적는다.
pub fn import_tagx(
    base: &ProjectSpec,
    text: &str,
    device: &str,
) -> Result<ImportReport, ConfigError> {
    if !base.devices.iter().any(|d| d.name == device) {
        return Err(ConfigError::Invalid(format!(
            "가져온 태그를 붙일 장치 `{device}`가 프로젝트에 없다"
        )));
    }
    let mut project = base.clone();
    let mut names: Vec<String> = project.tags.iter().map(|t| t.name.clone()).collect();
    let (mut renamed, mut modified, mut ignored, mut unreadable) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut imported = 0usize;
    let mut groups = 0usize;

    for (n, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = n + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        let fields = parse_comma_text_line(raw);
        let Some(first) = fields.first() else {
            continue;
        };
        let kind = first.trim().to_ascii_uppercase();

        match kind.as_str() {
            "GR" => {
                groups += 1;
                continue;
            }
            "GDO" => {
                ignored.push(notice(line, "GDO", "출력 그룹 줄은 가져오지 않았다"));
                continue;
            }
            "ST" => {
                ignored.push(notice(
                    line,
                    &name_of(&fields),
                    "문자열 태그는 아직 지원하지 않아 가져오지 않았다",
                ));
                continue;
            }
            "AI" | "AO" | "DI" | "DO" => {}
            other => {
                ignored.push(notice(line, other, "알 수 없는 줄 종류라 건너뛰었다"));
                continue;
            }
        }

        if fields.len() < min_fields(&kind) {
            unreadable.push(notice(
                line,
                &name_of(&fields),
                format!(
                    "{kind} 줄은 필드가 최소 {}개 필요하지만 {}개뿐이다",
                    min_fields(&kind),
                    fields.len()
                ),
            ));
            continue;
        }

        let row = Row {
            fields: &fields,
            line,
        };
        match build_tag(&kind, &row, device, &mut modified) {
            Ok(mut tag) => {
                let resolved = resolve_tag_name(&names, &tag.name);
                if resolved != tag.name {
                    renamed.push(ImportRenameItem {
                        line,
                        original: tag.name.clone(),
                        resolved: resolved.clone(),
                    });
                    tag.name = resolved;
                }
                names.push(tag.name.clone());
                project.tags.push(tag);
                imported += 1;
            }
            Err(reason) => unreadable.push(notice(line, &name_of(&fields), reason)),
        }
    }

    if groups > 0 {
        ignored.push(notice(
            0,
            "GR",
            format!("그룹 줄 {groups}개는 이름의 점(.)으로 이미 표현되어 가져오지 않았다"),
        ));
    }
    if imported > 0 {
        modified.push(notice(
            0,
            "통신 매핑",
            "포트·국번·주소를 통신 규격에 맞게 해석하지 않았다. 아날로그는 보유 레지스터, 디지털은 코일로 \
             추정해 옮겼으며 검증되지 않았다. 장치와 주소를 확인하라",
        ));
    }

    Ok(ImportReport {
        imported_devices: 0,
        imported_tags: imported,
        renamed_tags: renamed,
        modified_items: modified,
        ignored_items: ignored,
        unreadable_items: unreadable,
        project,
    })
}

fn name_of(fields: &[String]) -> String {
    fields
        .get(1)
        .map_or_else(String::new, |s| s.trim().to_string())
}

fn build_tag(
    kind: &str,
    row: &Row<'_>,
    device: &str,
    modified: &mut Vec<ImportNotice>,
) -> Result<TagSpec, String> {
    let name = row.s(1).to_string();
    if name.is_empty() {
        return Err("태그 이름이 비어 있다".into());
    }
    let line = row.line;
    if row.s(3) == "0" {
        modified.push(notice(
            line,
            &name,
            "사용 안 함(act=0)으로 저장된 태그이지만 가져왔다",
        ));
    }

    // 국번 → 유닛. 0~255가 아니면 1로 두고 알린다.
    let station = row.i64(at(2), "국번")?;
    let unit = u8::try_from(station).unwrap_or_else(|_| {
        modified.push(notice(
            line,
            &name,
            format!("국번 {station}은 유닛 번호 범위(0~255) 밖이라 1로 바꿨다"),
        ));
        1
    });

    match kind {
        "AI" | "AO" => {
            let writable = kind == "AO";
            // AI는 10진 주소, AO는 16진 주소.
            let addr = if writable {
                row.hex(at(3), "주소")?
            } else {
                row.i64(at(3), "주소")?
            };
            let address =
                u16::try_from(addr).map_err(|_| format!("주소 {addr}는 0~65535 범위 밖이다"))?;
            let (unit_text, base, full, plc_base, plc_full) = (
                row.s(at(11)).to_string(),
                row.f64(at(12), "표시 하한")?,
                row.f64(at(13), "표시 상한")?,
                row.f64(at(14), "장치 하한")?,
                row.f64(at(15), "장치 상한")?,
            );
            let (filter, cut, delay) = if writable {
                (
                    row.i64(at(16), "환산 필터")?,
                    row.i64(at(19), "범위 처리")?,
                    0,
                )
            } else {
                (
                    row.i64(at(32), "환산 필터")?,
                    row.i64(at(33), "범위 처리")?,
                    row.i64(at(34), "지연")?,
                )
            };
            if !matches!(filter, 0 | 1 | 3 | 4) {
                modified.push(notice(
                    line,
                    &name,
                    format!("환산 필터 {filter}은 지원하지 않아 무변환으로 가져왔다"),
                ));
            }
            let alarm = if writable {
                None
            } else {
                let enabled = row.i64(at(22), "알람 사용")? != 0;
                if enabled {
                    let kind = row.i64(at(29), "알람 종류")?;
                    let kind = u8::try_from(kind)
                        .ok()
                        .filter(|k| *k <= 9)
                        .ok_or_else(|| format!("알람 종류 {kind}는 0~9 범위 밖이다"))?;
                    Some(AlarmSpec {
                        kind,
                        hihi: Some(row.f64(at(16), "상상한")?),
                        high: Some(row.f64(at(17), "상한")?),
                        low: Some(row.f64(at(18), "하한")?),
                        lolo: Some(row.f64(at(19), "하하한")?),
                        gap: row.f64(at(38), "복귀 여유")?,
                    })
                } else {
                    None
                }
            };
            Ok(TagSpec {
                writable,
                name,
                device: device.to_string(),
                unit,
                space: "holding".into(),
                address,
                kind: "analog".into(),
                scale: Some(ScaleSpec {
                    plc_base,
                    plc_full,
                    base,
                    full,
                    profile: i32::try_from(filter).unwrap_or(4),
                    delay: u32::try_from(delay.clamp(0, 1000)).unwrap_or(0),
                    range: i32::try_from(cut).unwrap_or(0),
                }),
                alarm,
                unit_text,
            })
        }
        _ => {
            let writable = kind == "DO";
            let address = if writable {
                let a = row.hex(at(3), "주소")?;
                u16::try_from(a).map_err(|_| format!("주소 {a}는 0~65535 범위 밖이다"))?
            } else {
                // 디지털 입력의 주소는 `워드.비트` 문자열이다. 비트 번호로 합쳐 코일 주소로 추정한다.
                parse_word_bit(row.s(at(3)))?
            };
            let alarm = if writable {
                None
            } else if row.i64(at(13), "알람 사용")? != 0 {
                let k = row.i64(at(19), "알람 종류")?;
                let k = u8::try_from(k)
                    .ok()
                    .filter(|k| *k <= 3)
                    .ok_or_else(|| format!("디지털 알람 종류 {k}는 0~3 범위 밖이다"))?;
                Some(AlarmSpec {
                    kind: k,
                    hihi: None,
                    high: None,
                    low: None,
                    lolo: None,
                    gap: 0.0,
                })
            } else {
                None
            };
            Ok(TagSpec {
                writable,
                name,
                device: device.to_string(),
                unit,
                space: "coil".into(),
                address,
                kind: "digital".into(),
                scale: None,
                alarm,
                unit_text: String::new(),
            })
        }
    }
}

/// `워드.비트`(끝이 `h`면 비트를 16진수)를 비트 번호(워드 × 16 + 비트)로 바꾼다. 점이 없으면 숫자 그대로.
fn parse_word_bit(text: &str) -> Result<u16, String> {
    let (word, bit) = text.split_once('.').unwrap_or((text, "0"));
    let word: u32 = word
        .trim()
        .parse()
        .map_err(|_| format!("주소 '{text}'의 워드 부분이 정수가 아니다"))?;
    let bit_text = bit.trim();
    let bit = if bit_text.ends_with(['h', 'H']) {
        u32::from_str_radix(bit_text.trim_end_matches(['h', 'H']), 16)
    } else {
        bit_text.trim_end_matches(';').parse()
    }
    .map_err(|_| format!("주소 '{text}'의 비트 부분이 숫자가 아니다"))?;
    u16::try_from(word * 16 + bit).map_err(|_| format!("주소 '{text}'는 코일 주소 범위를 넘는다"))
}
