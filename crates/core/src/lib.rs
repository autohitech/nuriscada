// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 태그 모델, 값 환산, 알람 엔진. I/O 없는 순수 로직.
//!
//! 이 크레이트는 첫 수직 조각(설계 0001)의 골격이다. 구현은 해당 단계(S1~S3)에서 채운다.

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
