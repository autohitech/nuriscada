// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! OMRON FINS/TCP 메모리 영역 읽기·쓰기 드라이버.
//!
//! 프레임 부호화·해석은 [`protocol`]의 순수 함수이고, 네트워크 입출력은 [`client`]에 있다.
//! 공개된 FINS/TCP 절차(노드 주소 교환, 메모리 영역 읽기 0101·쓰기 0102)에서 출발했다.
//! DM 워드와 CIO 비트 읽기, DM 워드와 CIO 비트 쓰기를 지원한다.

pub mod client;
pub mod protocol;

pub use client::{FinsTcpClient, SPACE_CIO, SPACE_DM};
