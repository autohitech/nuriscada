// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! MELSEC MC 프로토콜(3E 바이너리 프레임) 읽기 드라이버.
//!
//! 프레임 부호화·해석은 [`protocol`]의 순수 함수이고, 네트워크 입출력은 [`client`]에 있다.
//! 공개된 MELSEC 커뮤니케이션 규격의 일괄 읽기(0401)만 구현한다. 쓰기는 지원하지 않는다.

pub mod client;
pub mod protocol;

pub use client::{MelsecMcClient, SPACE_D, SPACE_M, SPACE_R, SPACE_W, SPACE_X, SPACE_Y};
