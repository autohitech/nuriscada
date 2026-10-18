// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 프로젝트 설정(서버, 장치, 태그)의 읽기와 검증.
//!
//! 슬라이스용 단순 TOML 형식이다(설계 0001 8절). 원본 프로젝트 파일 읽기는 다음 조각에서
//! 별도 모듈로 추가한다.

use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use nuriscada_core::alarm::{AlarmError, AnalogAlarm, DigitalKind, Limits};
use nuriscada_core::scaling::{Profile, RangeMode, ScaleParams};
use nuriscada_driver_api::DataKind;
use serde::{Deserialize, Serialize};

pub mod legacy;
pub mod tagx;

/// 지원하는 가장 짧은 스캔 주기(밀리초). 제품 목표는 100 ms이고 시험을 위해 더 짧은 값을 허용한다.
pub const MIN_SCAN_MS: u64 = 10;
/// 가장 짧은 방송 주기(밀리초).
pub const MIN_PUBLISH_MS: u64 = 20;

/// 설정 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// TOML 형식 오류.
    Parse(String),
    /// 의미 오류(중복 이름, 없는 장치 등).
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(m) => write!(f, "설정 형식 오류: {m}"),
            Self::Invalid(m) => write!(f, "설정 오류: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn invalid<T>(msg: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::Invalid(msg.into()))
}

/// 서버 설정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// 수신 주소. 기본은 루프백이다.
    pub listen: String,
    /// 변경된 태그를 웹소켓으로 묶어 보내는 주기. 한 주기에 묶음 하나를 보낸다.
    pub publish: Duration,
}

/// 장치 드라이버 종류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverConfig {
    /// MODBUS TCP.
    ModbusTcp {
        /// `host:port`.
        address: String,
    },
}

/// 장치.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// 장치 이름.
    pub name: String,
    /// 드라이버.
    pub driver: DriverConfig,
    /// 연결·요청 제한 시간.
    pub timeout: Duration,
    /// 스캔 주기.
    pub scan: Duration,
}

/// 메모리 영역(MODBUS 용어 기준).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    /// 코일.
    Coil,
    /// 이산 입력.
    Discrete,
    /// 보유 레지스터.
    Holding,
    /// 입력 레지스터.
    Input,
}

impl Space {
    /// 영역의 데이터 종류.
    pub fn kind(self) -> DataKind {
        match self {
            Self::Coil | Self::Discrete => DataKind::Bit,
            Self::Holding | Self::Input => DataKind::Word,
        }
    }

    /// 드라이버 인터페이스의 영역 번호(0 코일, 1 이산 입력, 2 보유 레지스터, 3 입력 레지스터).
    pub fn code(self) -> u16 {
        match self {
            Self::Coil => 0,
            Self::Discrete => 1,
            Self::Holding => 2,
            Self::Input => 3,
        }
    }

    /// 설정 파일·API에서 쓰는 영역 이름.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Coil => "coil",
            Self::Discrete => "discrete",
            Self::Holding => "holding",
            Self::Input => "input",
        }
    }
}

/// 아날로그 알람 설정.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalogAlarmSpec {
    /// 알람 종류(0~9).
    pub kind: u8,
    /// 한계 값.
    pub limits: Limits,
}

/// 태그 본문.
#[derive(Debug, Clone, PartialEq)]
pub enum TagBody {
    /// 아날로그 입력.
    Analog {
        /// 환산 매개변수.
        scale: ScaleParams,
        /// 알람(없을 수 있음).
        alarm: Option<AnalogAlarmSpec>,
        /// 단위 표시 문자열.
        unit_text: String,
    },
    /// 디지털 입력.
    Digital {
        /// 알람 종류(없을 수 있음).
        alarm_kind: Option<u8>,
    },
}

/// 태그.
#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// 명시적으로 쓰기를 허용한 태그. 기본은 false다.
    pub writable: bool,
    /// 태그 이름(프로젝트 안에서 유일).
    pub name: String,
    /// 장치 목록에서의 위치.
    pub device: usize,
    /// 장치 번호(유닛 식별자).
    pub unit: u8,
    /// 메모리 영역.
    pub space: Space,
    /// 주소.
    pub address: u16,
    /// 태그 본문.
    pub body: TagBody,
}

/// 검증을 마친 프로젝트.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// 서버 설정.
    pub server: ServerConfig,
    /// 장치 목록.
    pub devices: Vec<Device>,
    /// 태그 목록.
    pub tags: Vec<Tag>,
    /// 그래픽 화면 목록.
    pub screens: Vec<ScreenSpec>,
}

// ---- 파일·편집 형식 ----

/// 편집·직렬화용 프로젝트 명세.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSpec {
    /// 서버 설정.
    #[serde(default)]
    pub server: ServerSpec,
    /// 장치 목록.
    #[serde(default, rename = "device")]
    pub devices: Vec<DeviceSpec>,
    /// 태그 목록.
    #[serde(default, rename = "tag")]
    pub tags: Vec<TagSpec>,
    /// 그래픽 화면 목록.
    #[serde(default, rename = "screen")]
    pub screens: Vec<ScreenSpec>,
}

/// 편집·직렬화용 서버 설정.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerSpec {
    /// 수신 주소.
    #[serde(default = "default_listen")]
    pub listen: String,
    /// 방송 주기(ms).
    #[serde(default = "default_publish_ms")]
    pub publish_ms: u64,
}

impl Default for ServerSpec {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            publish_ms: default_publish_ms(),
        }
    }
}

fn default_publish_ms() -> u64 {
    100
}

fn default_listen() -> String {
    "127.0.0.1:8080".to_string()
}

/// 편집·직렬화용 장치 명세.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSpec {
    /// 장치 이름.
    pub name: String,
    /// 드라이버(`modbus-tcp`).
    pub driver: String,
    /// 주소(`host:port`).
    pub address: String,
    /// 제한 시간(ms).
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// 스캔 주기(ms).
    #[serde(default = "default_scan_ms")]
    pub scan_ms: u64,
}

fn default_timeout_ms() -> u64 {
    500
}

fn default_scan_ms() -> u64 {
    100
}

/// 편집·직렬화용 태그 명세.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagSpec {
    /// 쓰기 허용 여부.
    #[serde(default)]
    pub writable: bool,
    /// 태그 이름.
    pub name: String,
    /// 장치 이름.
    pub device: String,
    /// 유닛 번호.
    #[serde(default = "default_unit")]
    pub unit: u8,
    /// 메모리 영역(`coil`, `discrete`, `holding`, `input`).
    pub space: String,
    /// 주소.
    pub address: u16,
    /// 종류(`analog`, `digital`).
    #[serde(rename = "type")]
    pub kind: String,
    /// 환산 설정(아날로그 전용).
    pub scale: Option<ScaleSpec>,
    /// 알람 설정.
    pub alarm: Option<AlarmSpec>,
    /// 단위 문자열.
    #[serde(default)]
    pub unit_text: String,
}

fn default_unit() -> u8 {
    1
}

/// 편집·직렬화용 환산 설정.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleSpec {
    /// 장치 하한.
    pub plc_base: f64,
    /// 장치 상한.
    pub plc_full: f64,
    /// 표시 하한.
    pub base: f64,
    /// 표시 상한.
    pub full: f64,
    /// 환산 프로파일 코드.
    #[serde(default)]
    pub profile: i32,
    /// 1차 지연 계수.
    #[serde(default)]
    pub delay: u32,
    /// 범위 처리 코드.
    #[serde(default)]
    pub range: i32,
}

/// 편집·직렬화용 알람 설정.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlarmSpec {
    /// 알람 종류.
    pub kind: u8,
    /// 상상한.
    pub hihi: Option<f64>,
    /// 상한.
    pub high: Option<f64>,
    /// 하한.
    pub low: Option<f64>,
    /// 하하한.
    pub lolo: Option<f64>,
    /// 복귀 여유.
    #[serde(default)]
    pub gap: f64,
}

/// 그래픽 화면 명세.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenSpec {
    /// 화면 이름(프로젝트 안에서 유일).
    pub name: String,
    /// 캔버스 가로 폭(px).
    #[serde(default = "default_screen_width")]
    pub width: u32,
    /// 캔버스 세로 높이(px).
    #[serde(default = "default_screen_height")]
    pub height: u32,
    /// 화면에 배치된 그래픽 위젯 목록.
    #[serde(default, rename = "widget")]
    pub widgets: Vec<GraphicWidgetSpec>,
}

fn default_screen_width() -> u32 {
    800
}

fn default_screen_height() -> u32 {
    480
}

/// 그래픽 화면 위젯 명세.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphicWidgetSpec {
    /// 화면 안에서 유일한 위젯 식별자.
    pub id: String,
    /// 위젯 종류(`value`, `gauge`, `tank`, `lamp`, `pump`, `valve`, `label`).
    pub kind: String,
    /// 표시 이름.
    #[serde(default)]
    pub label: String,
    /// 연결할 태그 이름(선택).
    pub tag: Option<String>,
    /// X 좌표.
    pub x: f64,
    /// Y 좌표.
    pub y: f64,
    /// 가로 크기.
    #[serde(default = "default_widget_width")]
    pub width: f64,
    /// 세로 크기.
    #[serde(default = "default_widget_height")]
    pub height: f64,
    /// 게이지·탱크 표시 하한.
    #[serde(default)]
    pub min: f64,
    /// 게이지·탱크 표시 상한.
    #[serde(default = "default_widget_max")]
    pub max: f64,
}

fn default_widget_width() -> f64 {
    160.0
}

fn default_widget_height() -> f64 {
    110.0
}

fn default_widget_max() -> f64 {
    100.0
}

fn parse_space(s: &str) -> Result<Space, ConfigError> {
    match s {
        "coil" => Ok(Space::Coil),
        "discrete" => Ok(Space::Discrete),
        "holding" => Ok(Space::Holding),
        "input" => Ok(Space::Input),
        other => invalid(format!(
            "알 수 없는 영역 `{other}` (coil, discrete, holding, input 중 하나)"
        )),
    }
}

fn alarm_error(tag: &str, e: AlarmError) -> ConfigError {
    ConfigError::Invalid(format!("태그 `{tag}`의 알람: {e}"))
}

fn check_finite(label: &str, v: f64) -> Result<f64, ConfigError> {
    if v.is_finite() {
        Ok(v)
    } else {
        invalid(format!("{label}: 유한한 숫자여야 한다"))
    }
}

fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn toml_f64(v: f64) -> String {
    if v == 0.0 {
        return "0.0".to_string();
    }
    let s = v.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

impl Project {
    /// TOML 문자열에서 프로젝트를 읽고 검증한다.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let raw: ProjectSpec =
            toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        Self::from_spec(raw)
    }

    /// 편집 명세를 검증해 프로젝트를 만든다.
    pub fn from_spec(raw: ProjectSpec) -> Result<Self, ConfigError> {
        let mut devices = Vec::new();
        let mut names = HashSet::new();
        for d in raw.devices {
            if d.name.trim().is_empty() {
                return invalid("장치 이름이 비어 있다");
            }
            if !names.insert(d.name.clone()) {
                return invalid(format!("장치 이름이 중복된다: {}", d.name));
            }
            if d.driver != "modbus-tcp" {
                return invalid(format!(
                    "장치 `{}`: 지원하지 않는 드라이버 `{}` (modbus-tcp만 지원)",
                    d.name, d.driver
                ));
            }
            if d.address.trim().is_empty() {
                return invalid(format!("장치 `{}`: 주소가 비어 있다", d.name));
            }
            if d.scan_ms < MIN_SCAN_MS {
                return invalid(format!(
                    "장치 `{}`: 스캔 주기 {} ms는 최소 {MIN_SCAN_MS} ms보다 짧다",
                    d.name, d.scan_ms
                ));
            }
            if d.timeout_ms == 0 {
                return invalid(format!("장치 `{}`: 제한 시간이 0이다", d.name));
            }
            devices.push(Device {
                name: d.name,
                driver: DriverConfig::ModbusTcp { address: d.address },
                timeout: Duration::from_millis(d.timeout_ms),
                scan: Duration::from_millis(d.scan_ms),
            });
        }

        let mut tags = Vec::new();
        let mut tag_names = HashSet::new();
        for t in raw.tags {
            if t.name.trim().is_empty() {
                return invalid("태그 이름이 비어 있다");
            }
            if !tag_names.insert(t.name.clone()) {
                return invalid(format!("태그 이름이 중복된다: {}", t.name));
            }
            let device = devices
                .iter()
                .position(|d| d.name == t.device)
                .ok_or_else(|| {
                    ConfigError::Invalid(format!("태그 `{}`: 없는 장치 `{}`", t.name, t.device))
                })?;
            let space = parse_space(&t.space)?;
            if t.writable && (!matches!(space, Space::Coil | Space::Holding) || t.unit == 0) {
                return invalid(format!(
                    "태그 `{}`: 쓰기는 유닛 1 이상인 coil·holding만 허용한다",
                    t.name
                ));
            }
            let body = match t.kind.as_str() {
                "analog" => {
                    if space.kind() != DataKind::Word {
                        return invalid(format!(
                            "태그 `{}`: 아날로그 태그는 holding 또는 input 영역이어야 한다",
                            t.name
                        ));
                    }
                    let Some(s) = t.scale else {
                        return invalid(format!("태그 `{}`: scale이 필요하다", t.name));
                    };
                    let label = format!("태그 `{}` 환산", t.name);
                    let plc_base = check_finite(&label, s.plc_base)?;
                    let plc_full = check_finite(&label, s.plc_full)?;
                    let base = check_finite(&label, s.base)?;
                    let full = check_finite(&label, s.full)?;
                    let alarm = match t.alarm {
                        None => None,
                        Some(a) => {
                            let al = format!("태그 `{}` 알람", t.name);
                            let limits = Limits {
                                hihi: a.hihi.map(|v| check_finite(&al, v)).transpose()?,
                                high: a.high.map(|v| check_finite(&al, v)).transpose()?,
                                low: a.low.map(|v| check_finite(&al, v)).transpose()?,
                                lolo: a.lolo.map(|v| check_finite(&al, v)).transpose()?,
                                gap: check_finite(&al, a.gap)?,
                            };
                            AnalogAlarm::new(a.kind, limits)
                                .map_err(|e| alarm_error(&t.name, e))?;
                            Some(AnalogAlarmSpec {
                                kind: a.kind,
                                limits,
                            })
                        }
                    };
                    TagBody::Analog {
                        scale: ScaleParams {
                            plc_base,
                            plc_full,
                            base,
                            full,
                            profile: Profile::from_code(s.profile),
                            delay: s.delay,
                            range: RangeMode::from_code(s.range),
                        },
                        alarm,
                        unit_text: t.unit_text,
                    }
                }
                "digital" => {
                    if space.kind() != DataKind::Bit {
                        return invalid(format!(
                            "태그 `{}`: 디지털 태그는 coil 또는 discrete 영역이어야 한다",
                            t.name
                        ));
                    }
                    if t.scale.is_some() {
                        return invalid(format!("태그 `{}`: 디지털 태그에는 scale이 없다", t.name));
                    }
                    let alarm_kind = match t.alarm {
                        None => None,
                        Some(a) => {
                            DigitalKind::new(a.kind).map_err(|e| alarm_error(&t.name, e))?;
                            Some(a.kind)
                        }
                    };
                    TagBody::Digital { alarm_kind }
                }
                other => {
                    return invalid(format!(
                        "태그 `{}`: 알 수 없는 type `{other}` (analog, digital)",
                        t.name
                    ));
                }
            };
            tags.push(Tag {
                writable: t.writable,
                name: t.name,
                device,
                unit: t.unit,
                space,
                address: t.address,
                body,
            });
        }

        let mut screens = Vec::new();
        let mut screen_names = HashSet::new();
        for mut s in raw.screens {
            if s.name.trim().is_empty() || s.name.len() > 64 {
                return invalid("화면 이름은 1~64자여야 한다");
            }
            if !screen_names.insert(s.name.clone()) {
                return invalid(format!("화면 이름이 중복된다: {}", s.name));
            }
            if !(200..=3840).contains(&s.width) || !(120..=2160).contains(&s.height) {
                return invalid(format!(
                    "화면 `{}`: 크기는 200~3840 x 120~2160 범위여야 한다",
                    s.name
                ));
            }
            let mut widget_ids = HashSet::new();
            for w in &mut s.widgets {
                if w.id.trim().is_empty() || w.id.len() > 64 {
                    return invalid(format!("화면 `{}`: 위젯 id는 1~64자여야 한다", s.name));
                }
                if !widget_ids.insert(w.id.clone()) {
                    return invalid(format!("화면 `{}`: 위젯 id가 중복된다: {}", s.name, w.id));
                }
                if !matches!(
                    w.kind.as_str(),
                    "value" | "gauge" | "tank" | "lamp" | "pump" | "valve" | "label"
                ) {
                    return invalid(format!(
                        "화면 `{}` 위젯 `{}`: 지원하지 않는 kind `{}`",
                        s.name, w.id, w.kind
                    ));
                }
                let wl = format!("화면 `{}` 위젯 `{}`", s.name, w.id);
                check_finite(&wl, w.x)?;
                check_finite(&wl, w.y)?;
                check_finite(&wl, w.width)?;
                check_finite(&wl, w.height)?;
                check_finite(&wl, w.min)?;
                check_finite(&wl, w.max)?;
                if w.x < 0.0 || w.y < 0.0 || w.width < 20.0 || w.height < 20.0 {
                    return invalid(format!("{wl}: 좌표는 0 이상이고 크기는 20 이상이어야 한다"));
                }
                if matches!(w.kind.as_str(), "gauge" | "tank") && w.max <= w.min {
                    return invalid(format!("{wl}: max는 min보다 커야 한다"));
                }
                if let Some(tag_name) = &w.tag {
                    let trimmed = tag_name.trim();
                    if trimmed.is_empty() {
                        w.tag = None;
                    } else if !tag_names.contains(trimmed) {
                        return invalid(format!("{wl}: 없는 태그 `{trimmed}`"));
                    } else {
                        w.tag = Some(trimmed.to_string());
                    }
                }
            }
            screens.push(s);
        }

        if raw.server.publish_ms < MIN_PUBLISH_MS {
            return invalid(format!(
                "방송 주기 {} ms는 최소 {MIN_PUBLISH_MS} ms보다 짧다",
                raw.server.publish_ms
            ));
        }
        Ok(Self {
            server: ServerConfig {
                listen: raw.server.listen,
                publish: Duration::from_millis(raw.server.publish_ms),
            },
            devices,
            tags,
            screens,
        })
    }

    /// 현재 프로젝트를 편집 명세로 변환한다.
    pub fn to_spec(&self) -> ProjectSpec {
        ProjectSpec {
            server: ServerSpec {
                listen: self.server.listen.clone(),
                publish_ms: u64::try_from(self.server.publish.as_millis()).unwrap_or(100),
            },
            devices: self
                .devices
                .iter()
                .map(|d| {
                    let DriverConfig::ModbusTcp { address } = &d.driver;
                    DeviceSpec {
                        name: d.name.clone(),
                        driver: "modbus-tcp".to_string(),
                        address: address.clone(),
                        timeout_ms: u64::try_from(d.timeout.as_millis()).unwrap_or(500),
                        scan_ms: u64::try_from(d.scan.as_millis()).unwrap_or(100),
                    }
                })
                .collect(),
            tags: self
                .tags
                .iter()
                .map(|t| {
                    let (kind, scale, alarm, unit_text) = match &t.body {
                        TagBody::Analog {
                            scale,
                            alarm,
                            unit_text,
                        } => (
                            "analog".to_string(),
                            Some(ScaleSpec {
                                plc_base: scale.plc_base,
                                plc_full: scale.plc_full,
                                base: scale.base,
                                full: scale.full,
                                profile: scale.profile.code(),
                                delay: scale.delay,
                                range: scale.range.code(),
                            }),
                            alarm.map(|a| AlarmSpec {
                                kind: a.kind,
                                hihi: a.limits.hihi,
                                high: a.limits.high,
                                low: a.limits.low,
                                lolo: a.limits.lolo,
                                gap: a.limits.gap,
                            }),
                            unit_text.clone(),
                        ),
                        TagBody::Digital { alarm_kind } => (
                            "digital".to_string(),
                            None,
                            alarm_kind.map(|kind| AlarmSpec {
                                kind,
                                hihi: None,
                                high: None,
                                low: None,
                                lolo: None,
                                gap: 0.0,
                            }),
                            String::new(),
                        ),
                    };
                    TagSpec {
                        writable: t.writable,
                        name: t.name.clone(),
                        device: self.devices[t.device].name.clone(),
                        unit: t.unit,
                        space: t.space.as_str().to_string(),
                        address: t.address,
                        kind,
                        scale,
                        alarm,
                        unit_text,
                    }
                })
                .collect(),
            screens: self.screens.clone(),
        }
    }

    /// 프로젝트를 표준 TOML 문자열로 직렬화한다.
    pub fn to_toml(&self) -> String {
        let spec = self.to_spec();
        let mut out = String::new();
        out.push_str("[server]\n");
        out.push_str(&format!("listen = {}\n", toml_str(&spec.server.listen)));
        out.push_str(&format!("publish_ms = {}\n", spec.server.publish_ms));
        for d in &spec.devices {
            out.push_str("\n[[device]]\n");
            out.push_str(&format!("name = {}\n", toml_str(&d.name)));
            out.push_str(&format!("driver = {}\n", toml_str(&d.driver)));
            out.push_str(&format!("address = {}\n", toml_str(&d.address)));
            out.push_str(&format!("timeout_ms = {}\n", d.timeout_ms));
            out.push_str(&format!("scan_ms = {}\n", d.scan_ms));
        }
        for t in &spec.tags {
            out.push_str("\n[[tag]]\n");
            out.push_str(&format!("name = {}\n", toml_str(&t.name)));
            if t.writable {
                out.push_str("writable = true\n");
            }
            out.push_str(&format!("device = {}\n", toml_str(&t.device)));
            if t.unit != 1 {
                out.push_str(&format!("unit = {}\n", t.unit));
            }
            out.push_str(&format!("space = {}\n", toml_str(&t.space)));
            out.push_str(&format!("address = {}\n", t.address));
            out.push_str(&format!("type = {}\n", toml_str(&t.kind)));
            if !t.unit_text.is_empty() {
                out.push_str(&format!("unit_text = {}\n", toml_str(&t.unit_text)));
            }
            if let Some(s) = &t.scale {
                let mut fields = vec![
                    format!("plc_base = {}", toml_f64(s.plc_base)),
                    format!("plc_full = {}", toml_f64(s.plc_full)),
                    format!("base = {}", toml_f64(s.base)),
                    format!("full = {}", toml_f64(s.full)),
                ];
                if s.profile != 0 {
                    fields.push(format!("profile = {}", s.profile));
                }
                if s.delay != 0 {
                    fields.push(format!("delay = {}", s.delay));
                }
                if s.range != 0 {
                    fields.push(format!("range = {}", s.range));
                }
                out.push_str(&format!("scale = {{ {} }}\n", fields.join(", ")));
            }
            if let Some(a) = &t.alarm {
                let mut fields = vec![format!("kind = {}", a.kind)];
                if let Some(v) = a.hihi {
                    fields.push(format!("hihi = {}", toml_f64(v)));
                }
                if let Some(v) = a.high {
                    fields.push(format!("high = {}", toml_f64(v)));
                }
                if let Some(v) = a.low {
                    fields.push(format!("low = {}", toml_f64(v)));
                }
                if let Some(v) = a.lolo {
                    fields.push(format!("lolo = {}", toml_f64(v)));
                }
                if a.gap != 0.0 {
                    fields.push(format!("gap = {}", toml_f64(a.gap)));
                }
                out.push_str(&format!("alarm = {{ {} }}\n", fields.join(", ")));
            }
        }
        for s in &spec.screens {
            out.push_str("\n[[screen]]\n");
            out.push_str(&format!("name = {}\n", toml_str(&s.name)));
            out.push_str(&format!("width = {}\n", s.width));
            out.push_str(&format!("height = {}\n", s.height));
            for w in &s.widgets {
                out.push_str("\n[[screen.widget]]\n");
                out.push_str(&format!("id = {}\n", toml_str(&w.id)));
                out.push_str(&format!("kind = {}\n", toml_str(&w.kind)));
                if !w.label.is_empty() {
                    out.push_str(&format!("label = {}\n", toml_str(&w.label)));
                }
                if let Some(tag) = &w.tag {
                    out.push_str(&format!("tag = {}\n", toml_str(tag)));
                }
                out.push_str(&format!("x = {}\n", toml_f64(w.x)));
                out.push_str(&format!("y = {}\n", toml_f64(w.y)));
                out.push_str(&format!("width = {}\n", toml_f64(w.width)));
                out.push_str(&format!("height = {}\n", toml_f64(w.height)));
                if w.min != 0.0 {
                    out.push_str(&format!("min = {}\n", toml_f64(w.min)));
                }
                if w.max != 100.0 {
                    out.push_str(&format!("max = {}\n", toml_f64(w.max)));
                }
            }
        }
        out
    }
}
