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
use serde::Deserialize;

/// 지원하는 가장 짧은 스캔 주기(밀리초). 제품 목표는 100 ms이고 시험을 위해 더 짧은 값을 허용한다.
pub const MIN_SCAN_MS: u64 = 10;

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
}

// ---- 파일 형식 ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProject {
    #[serde(default)]
    server: RawServer,
    #[serde(default, rename = "device")]
    devices: Vec<RawDevice>,
    #[serde(default, rename = "tag")]
    tags: Vec<RawTag>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServer {
    #[serde(default = "default_listen")]
    listen: String,
}

impl Default for RawServer {
    fn default() -> Self {
        Self {
            listen: default_listen(),
        }
    }
}

fn default_listen() -> String {
    "127.0.0.1:8080".to_string()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDevice {
    name: String,
    driver: String,
    address: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_scan_ms")]
    scan_ms: u64,
}

fn default_timeout_ms() -> u64 {
    500
}

fn default_scan_ms() -> u64 {
    100
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTag {
    name: String,
    device: String,
    #[serde(default = "default_unit")]
    unit: u8,
    space: String,
    address: u16,
    #[serde(rename = "type")]
    kind: String,
    scale: Option<RawScale>,
    alarm: Option<RawAlarm>,
    #[serde(default)]
    unit_text: String,
}

fn default_unit() -> u8 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScale {
    plc_base: f64,
    plc_full: f64,
    base: f64,
    full: f64,
    #[serde(default)]
    profile: i32,
    #[serde(default)]
    delay: u32,
    #[serde(default)]
    range: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAlarm {
    kind: u8,
    hihi: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    lolo: Option<f64>,
    #[serde(default)]
    gap: f64,
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

impl Project {
    /// TOML 문자열에서 프로젝트를 읽고 검증한다.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let raw: RawProject =
            toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;

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
                    let alarm = match t.alarm {
                        None => None,
                        Some(a) => {
                            let limits = Limits {
                                hihi: a.hihi,
                                high: a.high,
                                low: a.low,
                                lolo: a.lolo,
                                gap: a.gap,
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
                            plc_base: s.plc_base,
                            plc_full: s.plc_full,
                            base: s.base,
                            full: s.full,
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
                name: t.name,
                device,
                unit: t.unit,
                space,
                address: t.address,
                body,
            });
        }

        Ok(Self {
            server: ServerConfig {
                listen: raw.server.listen,
            },
            devices,
            tags,
        })
    }
}
