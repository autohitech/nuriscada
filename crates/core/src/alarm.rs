// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 아날로그 알람(종류 0~9)과 디지털 알람(종류 0~3)의 상태 전이.

use std::fmt;

/// 알람 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 정상.
    Normal,
    /// 하하한 상태.
    Lolo,
    /// 하한 상태.
    Low,
    /// 상한 상태.
    High,
    /// 상상한 상태.
    Hihi,
    /// 범위 안(종류 9 전용).
    Middle,
}

impl Status {
    /// 표시용 이름.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Lolo => "LOLO",
            Self::Low => "LOW",
            Self::High => "HIGH",
            Self::Hihi => "HIHI",
            Self::Middle => "MIDDLE",
        }
    }
}

/// 알람 기록의 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// 상상한 도달.
    Hihi,
    /// 상한 도달.
    High,
    /// 하한 도달.
    Low,
    /// 하하한 도달.
    Lolo,
    /// 복귀 직전의 확인 기록.
    Cnf,
    /// 정상 복귀.
    Return,
    /// 디지털 켜짐.
    DiOn,
    /// 디지털 꺼짐.
    DiOff,
}

impl Event {
    /// 기대값 데이터가 쓰는 이름.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hihi => "HIHI",
            Self::High => "HIGH",
            Self::Low => "LOW",
            Self::Lolo => "LOLO",
            Self::Cnf => "CNF",
            Self::Return => "RETURN",
            Self::DiOn => "DI_ON",
            Self::DiOff => "DI_OFF",
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 알람 구성 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlarmError {
    /// 정의되지 않은 알람 종류.
    UnknownKind(u8),
    /// 해당 종류에 필요한 한계 값이 없다.
    MissingLimit {
        /// 알람 종류.
        kind: u8,
        /// 없는 한계의 이름.
        limit: &'static str,
    },
}

impl fmt::Display for AlarmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKind(k) => write!(f, "정의되지 않은 알람 종류: {k}"),
            Self::MissingLimit { kind, limit } => {
                write!(f, "알람 종류 {kind}에 한계 값 {limit}이 필요하다")
            }
        }
    }
}

impl std::error::Error for AlarmError {}

/// 아날로그 알람 한계 값.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Limits {
    /// 상상한.
    pub hihi: Option<f64>,
    /// 상한.
    pub high: Option<f64>,
    /// 하한.
    pub low: Option<f64>,
    /// 하하한.
    pub lolo: Option<f64>,
    /// 복귀 여유(히스테리시스).
    pub gap: f64,
}

/// 종류별로 필요한 한계.
fn required(kind: u8) -> Result<&'static [&'static str], AlarmError> {
    Ok(match kind {
        0 => &["hihi", "lolo"],
        1 => &["hihi"],
        2 => &["lolo"],
        3 => &["high", "low"],
        4 => &["high"],
        5 => &["low"],
        6 => &["hihi", "high", "low", "lolo"],
        7 => &["hihi", "high"],
        8 => &["lolo", "low"],
        9 => &["high", "low"],
        k => return Err(AlarmError::UnknownKind(k)),
    })
}

/// 아날로그 알람. 알람 사용이 켜진 상태를 가정한다.
#[derive(Debug, Clone)]
pub struct AnalogAlarm {
    kind: u8,
    hihi: f64,
    high: f64,
    low: f64,
    lolo: f64,
    gap: f64,
    status: Status,
    events: Vec<Event>,
}

impl AnalogAlarm {
    /// 알람을 만든다. 종류에 필요한 한계가 없으면 오류다.
    pub fn new(kind: u8, limits: Limits) -> Result<Self, AlarmError> {
        let need = required(kind)?;
        let get = |name: &'static str, value: Option<f64>| -> Result<f64, AlarmError> {
            match value {
                Some(v) => Ok(v),
                None if need.contains(&name) => Err(AlarmError::MissingLimit { kind, limit: name }),
                None => Ok(f64::NAN),
            }
        };
        Ok(Self {
            kind,
            hihi: get("hihi", limits.hihi)?,
            high: get("high", limits.high)?,
            low: get("low", limits.low)?,
            lolo: get("lolo", limits.lolo)?,
            gap: limits.gap,
            status: Status::Normal,
            events: Vec::new(),
        })
    }

    /// 현재 상태.
    pub fn status(&self) -> Status {
        self.status
    }

    fn set(&mut self, event: Event, status: Status) {
        self.events.push(event);
        self.status = status;
    }

    /// 복귀: 확인 기록 뒤에 복귀 기록을 남기고 정상으로 간다.
    fn ret(&mut self) {
        self.events.push(Event::Cnf);
        self.set(Event::Return, Status::Normal);
    }

    /// 값 하나를 평가한다. 새로 생긴 기록을 돌려준다.
    pub fn check(&mut self, v: f64) -> Vec<Event> {
        let start = self.events.len();
        let (s, g) = (self.status, self.gap);
        let (hihi, high, low, lolo) = (self.hihi, self.high, self.low, self.lolo);
        use Status::{High, Hihi, Lolo, Low, Middle, Normal};
        match self.kind {
            0 => {
                if v >= hihi {
                    if s != Hihi {
                        self.set(Event::Hihi, Hihi);
                    }
                } else if v <= lolo {
                    if s != Lolo {
                        self.set(Event::Lolo, Lolo);
                    }
                } else if s != Normal {
                    if s == Hihi {
                        if v < hihi - g {
                            self.ret();
                        }
                    } else if v > lolo + g {
                        self.ret();
                    }
                }
            }
            1 => {
                if v >= hihi {
                    if s != Hihi {
                        self.set(Event::Hihi, Hihi);
                    }
                } else if s != Normal && v < hihi - g {
                    self.ret();
                }
            }
            2 => {
                if v <= lolo {
                    if s != Lolo {
                        self.set(Event::Lolo, Lolo);
                    }
                } else if s != Normal && v > lolo + g {
                    self.ret();
                }
            }
            3 => {
                if v >= high {
                    if s != High {
                        self.set(Event::High, High);
                    }
                } else if v <= low {
                    if s != Low {
                        self.set(Event::Low, Low);
                    }
                } else if s != Normal {
                    if s == High {
                        if v < high - g {
                            self.ret();
                        }
                    } else if v > low + g {
                        self.ret();
                    }
                }
            }
            4 => {
                if v >= high {
                    if s != High {
                        self.set(Event::High, High);
                    }
                } else if s != Normal && v < high - g {
                    self.ret();
                }
            }
            5 => {
                if v <= low {
                    if s != Low {
                        self.set(Event::Low, Low);
                    }
                } else if s != Normal && v > low + g {
                    self.ret();
                }
            }
            6 => {
                if v >= hihi {
                    if s != Hihi {
                        self.set(Event::Hihi, Hihi);
                    }
                } else if v <= lolo {
                    if s != Lolo {
                        self.set(Event::Lolo, Lolo);
                    }
                } else if v >= high {
                    if s == Hihi {
                        if v < hihi - g {
                            self.set(Event::High, High);
                        }
                    } else if s != High {
                        self.set(Event::High, High);
                    }
                } else if v <= low {
                    if s == Lolo {
                        if v > lolo + g {
                            self.set(Event::Low, Low);
                        }
                    } else if s != Low {
                        self.set(Event::Low, Low);
                    }
                } else if s != Normal {
                    if matches!(s, Hihi | High) {
                        if v < high - g {
                            self.ret();
                        }
                    } else if v > low + g {
                        self.ret();
                    }
                }
            }
            7 => {
                if v >= hihi {
                    if s != Hihi {
                        self.set(Event::Hihi, Hihi);
                    }
                } else if v >= high {
                    if s == Hihi {
                        if v < hihi - g {
                            self.set(Event::High, High);
                        }
                    } else if s != High {
                        self.set(Event::High, High);
                    }
                } else if s != Normal && v < high - g {
                    self.ret();
                }
            }
            8 => {
                if v <= lolo {
                    if s != Lolo {
                        self.set(Event::Lolo, Lolo);
                    }
                } else if v <= low {
                    if s == Lolo {
                        if v > lolo + g {
                            self.set(Event::Low, Low);
                        }
                    } else if s != Low {
                        self.set(Event::Low, Low);
                    }
                } else if s != Normal && v > low + g {
                    self.ret();
                }
            }
            9 => {
                if low <= v && v <= high {
                    if s != Middle {
                        // 상태와 기록 종류가 다르다: 상태는 MIDDLE, 기록은 HIGH.
                        self.set(Event::High, Middle);
                    }
                } else if s != Normal {
                    self.set(Event::Return, Normal);
                }
            }
            _ => unreachable!("생성자가 종류를 검증한다"),
        }
        self.events[start..].to_vec()
    }

    /// 값 열을 순서대로 평가해 값마다 새로 생긴 기록을 돌려준다.
    pub fn run(&mut self, series: &[f64]) -> Vec<Vec<Event>> {
        series.iter().map(|&v| self.check(v)).collect()
    }
}

/// 디지털 알람 종류(0~3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigitalKind(u8);

impl DigitalKind {
    /// 종류 코드에서 만든다. 0~3만 허용한다.
    pub fn new(kind: u8) -> Result<Self, AlarmError> {
        if kind <= 3 {
            Ok(Self(kind))
        } else {
            Err(AlarmError::UnknownKind(kind))
        }
    }

    /// 값이 `value`로 바뀔 때 남기는 기록.
    fn event_on_change(self, value: bool) -> Option<Event> {
        match (self.0, value) {
            (0, true) => Some(Event::DiOn),
            (0, false) => Some(Event::Return),
            (1, true) => Some(Event::Return),
            (1, false) => Some(Event::DiOff),
            (2, false) => Some(Event::DiOff),
            (3, true) => Some(Event::DiOn),
            _ => None,
        }
    }
}

/// 디지털 알람. 값이 **변할 때만** 평가한다.
#[derive(Debug, Clone)]
pub struct DigitalAlarm {
    kind: DigitalKind,
    previous: bool,
}

impl DigitalAlarm {
    /// 입력 직전 값을 `initial`로 두고 만든다.
    pub fn new(kind: DigitalKind, initial: bool) -> Self {
        Self {
            kind,
            previous: initial,
        }
    }

    /// 값 하나를 평가한다.
    pub fn check(&mut self, value: bool) -> Option<Event> {
        let event = if value == self.previous {
            None
        } else {
            self.kind.event_on_change(value)
        };
        self.previous = value;
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_kind_is_rejected() {
        assert_eq!(
            AnalogAlarm::new(10, Limits::default()).unwrap_err(),
            AlarmError::UnknownKind(10)
        );
        assert_eq!(DigitalKind::new(4).unwrap_err(), AlarmError::UnknownKind(4));
    }

    #[test]
    fn missing_required_limit_is_rejected() {
        let limits = Limits {
            hihi: Some(90.0),
            ..Limits::default()
        };
        // 종류 0은 hihi와 lolo가 모두 필요하다.
        assert_eq!(
            AnalogAlarm::new(0, limits).unwrap_err(),
            AlarmError::MissingLimit {
                kind: 0,
                limit: "lolo"
            }
        );
        // 종류 1은 hihi만 필요하다.
        assert!(AnalogAlarm::new(1, limits).is_ok());
    }

    #[test]
    fn same_state_does_not_repeat_events() {
        let mut a = AnalogAlarm::new(
            1,
            Limits {
                hihi: Some(90.0),
                gap: 5.0,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(a.check(95.0), vec![Event::Hihi]);
        assert!(a.check(99.0).is_empty());
        assert_eq!(a.status(), Status::Hihi);
    }

    #[test]
    fn digital_first_equal_value_is_not_evaluated() {
        let mut d = DigitalAlarm::new(DigitalKind::new(0).unwrap(), false);
        assert_eq!(d.check(false), None);
        assert_eq!(d.check(true), Some(Event::DiOn));
    }
}
