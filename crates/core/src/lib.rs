// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 태그 모델, 값 환산, 알람 엔진. I/O 없는 순수 로직이다.
//!
//! 규칙의 규범은 `tests/fixtures/expected/`의 기대값 데이터이며, 설계 문서
//! (`docs/design/0001-vertical-slice-1.md` 5절)는 요약이다.

pub mod alarm;
pub mod scaling;

/// 크레이트 이름. 골격이 빌드되는지 확인하는 용도다.
pub const CRATE_NAME: &str = "nuriscada-core";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_stable() {
        assert_eq!(CRATE_NAME, "nuriscada-core");
    }
}
