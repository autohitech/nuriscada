// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! XGT 전용 프로토콜(FEnet, TCP) 개별 읽기 드라이버.
//!
//! 프레임 부호화·해석은 [`protocol`]의 순수 함수이고, 네트워크 입출력은 [`client`]에 있다.
//! 공개된 XGT FEnet 매뉴얼의 개별 읽기(0x5400, 워드·비트)만 구현한다. 쓰기는 지원하지 않는다.

pub mod client;
pub mod protocol;

pub use client::{SPACE_MW, SPACE_MX, XgtFenetClient};
