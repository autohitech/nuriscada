// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! Siemens S7 (ISO-on-TCP) 읽기 변수(0x04)·쓰기 변수(0x05) 드라이버.
//!
//! 프레임 부호화·해석은 [`protocol`]의 순수 함수이고, 네트워크 입출력은 [`client`]에 있다.
//! 공개 문서(RFC 1006, 접속 절차와 PDU·항목 배치의 공개된 설명)에서 출발했다.
//! S7-300/400의 DB 워드와 M 비트 읽기·쓰기를 지원한다. S7CommPlus(0x72)는 지원하지 않는다.

pub mod client;
pub mod protocol;

pub use client::{S7Client, SPACE_S7DB, SPACE_S7M};
