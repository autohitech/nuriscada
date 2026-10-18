// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! MODBUS TCP 마스터 드라이버.
//!
//! 프레임 부호화·해석은 [`protocol`]의 순수 함수이고, 네트워크 입출력은 [`client`]에 있다.

pub mod client;
pub mod protocol;

pub use client::{
    ModbusTcpClient, SPACE_COILS, SPACE_DISCRETE_INPUTS, SPACE_HOLDING_REGISTERS,
    SPACE_INPUT_REGISTERS,
};
