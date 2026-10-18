// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 아날로그 입력 값 환산, 1차 지연, 범위 처리.

/// 환산 프로파일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// 0: 선형.
    Linear,
    /// 1: 코사인.
    Cosine,
    /// 3: 구간 반전.
    SplitMirror,
    /// 4 및 그 밖: 무변환.
    Passthrough,
}

impl Profile {
    /// 숫자 코드에서 프로파일을 만든다. 정의되지 않은 코드는 무변환이다.
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Linear,
            1 => Self::Cosine,
            3 => Self::SplitMirror,
            _ => Self::Passthrough,
        }
    }
}

/// 범위 처리 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeMode {
    /// 0: 처리하지 않는다.
    None,
    /// 1: 표시 범위로 자른다.
    Clamp,
    /// 2: 표시 범위 밖이면 값을 버린다.
    Drop,
}

impl RangeMode {
    /// 숫자 코드에서 범위 처리 방식을 만든다. 정의되지 않은 코드는 처리하지 않는다.
    pub fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Clamp,
            2 => Self::Drop,
            _ => Self::None,
        }
    }
}

/// 환산 매개변수.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleParams {
    /// 장치 값 하한.
    pub plc_base: f64,
    /// 장치 값 상한.
    pub plc_full: f64,
    /// 표시 값 하한.
    pub base: f64,
    /// 표시 값 상한.
    pub full: f64,
    /// 환산 프로파일.
    pub profile: Profile,
    /// 1차 지연 계수. 0이면 지연 없음, 상한 1000.
    pub delay: u32,
    /// 범위 처리 방식.
    pub range: RangeMode,
}

/// 지연 계수의 상한.
pub const MAX_DELAY: u32 = 1000;

/// 프로파일에 따라 한 값을 환산한다(지연과 범위 처리 제외).
pub fn scale(v: f64, p: &ScaleParams) -> f64 {
    let span = p.plc_full - p.plc_base;
    let width = p.full - p.base;
    match p.profile {
        Profile::Linear => {
            if span != 0.0 {
                (v - p.plc_base) * width / span + p.base
            } else {
                0.0
            }
        }
        Profile::Cosine => {
            if span == 0.0 {
                return 0.0;
            }
            let deg = ((v - p.plc_base) * 180.0 / span - 90.0).clamp(-90.0, 90.0);
            let out = deg.abs().to_radians().cos() * width + p.base;
            if deg < 0.0 { -out } else { out }
        }
        Profile::SplitMirror => {
            if span == 0.0 {
                return 0.0;
            }
            let mid = span / 2.0 + p.plc_base;
            if v < mid {
                -((v - p.plc_base) * 2.0 * width / span + p.base)
            } else {
                let shifted = v - span;
                let out = (shifted - p.plc_base) * 2.0 * width / span + p.base;
                p.full - out
            }
        }
        Profile::Passthrough => v,
    }
}

/// 값 하나의 상태: 1차 지연과 범위 처리를 포함한 갱신.
#[derive(Debug, Clone)]
pub struct AiValue {
    params: ScaleParams,
    delay: u32,
    previous: f64,
    current: f64,
}

impl AiValue {
    /// 새 값 상태를 만든다. 지연 계수는 [`MAX_DELAY`]로 제한한다.
    pub fn new(params: ScaleParams) -> Self {
        let delay = params.delay.min(MAX_DELAY);
        Self {
            params,
            delay,
            previous: 0.0,
            current: 0.0,
        }
    }

    /// 현재 반영된 값.
    pub fn current(&self) -> f64 {
        self.current
    }

    /// 새 원시 값을 넣는다. 반영된 값을 돌려주고, 버려지면 `None`이다.
    ///
    /// 버려진 경우 내부 상태는 바뀌지 않는다.
    pub fn update(&mut self, raw: f64) -> Option<f64> {
        let mut v = scale(raw, &self.params);
        if self.delay > 0 {
            let d = f64::from(self.delay);
            // 지연 필터의 결과는 32비트 부동소수 정밀도로 반올림한다.
            v = f64::from(((self.previous * d + v) / (d + 1.0)) as f32);
        }
        let (base, full) = (self.params.base, self.params.full);
        match self.params.range {
            RangeMode::Clamp => v = v.max(base).min(full),
            RangeMode::Drop if v < base || v > full => return None,
            _ => {}
        }
        self.previous = v;
        self.current = v;
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> ScaleParams {
        ScaleParams {
            plc_base: 0.0,
            plc_full: 100.0,
            base: 0.0,
            full: 100.0,
            profile: Profile::Linear,
            delay: 0,
            range: RangeMode::None,
        }
    }

    #[test]
    fn undefined_codes_fall_back() {
        assert_eq!(Profile::from_code(2), Profile::Passthrough);
        assert_eq!(Profile::from_code(-1), Profile::Passthrough);
        assert_eq!(RangeMode::from_code(7), RangeMode::None);
    }

    #[test]
    fn delay_is_capped() {
        let v = AiValue::new(ScaleParams {
            delay: 5000,
            ..params()
        });
        assert_eq!(v.delay, MAX_DELAY);
    }

    #[test]
    fn dropped_value_keeps_state() {
        let mut v = AiValue::new(ScaleParams {
            range: RangeMode::Drop,
            ..params()
        });
        assert_eq!(v.update(50.0), Some(50.0));
        assert_eq!(v.update(120.0), None);
        assert_eq!(v.current(), 50.0);
    }
}
