// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 드라이버 인터페이스와 공통 형식.
//!
//! 드라이버는 읽기와 상태만 갖는다(GUI 결합 없음, 요구사항 SQ-C01). 쓰기는 다음 조각에서 더한다.

use std::fmt;
use std::future::Future;
use std::time::Duration;

/// 읽는 데이터의 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataKind {
    /// 비트(참/거짓).
    Bit,
    /// 16비트 워드.
    Word,
}

/// 한 번에 읽는 연속 영역.
///
/// `space`는 드라이버가 정의하는 메모리 영역 번호다(예: MODBUS의 코일, 보유 레지스터).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadBlock {
    /// 장치 번호(국번, 유닛 식별자).
    pub unit: u8,
    /// 드라이버가 정의한 메모리 영역 번호.
    pub space: u16,
    /// 시작 주소.
    pub start: u16,
    /// 읽을 개수(비트 수 또는 워드 수).
    pub count: u16,
    /// 읽는 데이터의 종류.
    pub kind: DataKind,
}

/// 읽은 데이터.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockData {
    /// 비트 값.
    Bits(Vec<bool>),
    /// 16비트 워드 값.
    Words(Vec<u16>),
}

impl BlockData {
    /// 값의 개수.
    pub fn len(&self) -> usize {
        match self {
            Self::Bits(v) => v.len(),
            Self::Words(v) => v.len(),
        }
    }

    /// 값이 없는지.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 드라이버 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverError {
    /// 응답 시간 초과.
    Timeout,
    /// 연결 오류(연결 실패, 끊김).
    Connection(String),
    /// 프로토콜 위반(형식 오류, 일치하지 않는 응답).
    Protocol(String),
    /// 장치가 오류를 응답했다.
    Device {
        /// 장치가 돌려준 오류 코드.
        code: u8,
        /// 사람이 읽을 설명.
        text: String,
    },
    /// 요청이 잘못되었다(범위 초과, 지원하지 않는 영역).
    InvalidRequest(String),
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("응답 시간 초과"),
            Self::Connection(m) => write!(f, "연결 오류: {m}"),
            Self::Protocol(m) => write!(f, "프로토콜 오류: {m}"),
            Self::Device { code, text } => write!(f, "장치 오류 {code}: {text}"),
            Self::InvalidRequest(m) => write!(f, "잘못된 요청: {m}"),
        }
    }
}

impl std::error::Error for DriverError {}

/// 연결 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// 연결되지 않았다.
    Disconnected,
    /// 연결되어 있다.
    Connected,
}

/// 드라이버 상태(진단용).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkStatus {
    /// 연결 상태.
    pub state: LinkState,
    /// 마지막 성공한 요청의 왕복 시간.
    pub last_round_trip: Option<Duration>,
    /// 성공한 요청 수.
    pub requests_ok: u64,
    /// 실패한 요청 수.
    pub requests_failed: u64,
}

impl LinkStatus {
    /// 아무 요청도 하지 않은 초기 상태.
    pub const fn initial() -> Self {
        Self {
            state: LinkState::Disconnected,
            last_round_trip: None,
            requests_ok: 0,
            requests_failed: 0,
        }
    }
}

/// 장치 드라이버.
pub trait Driver: Send {
    /// 장치에 연결한다. 이미 연결되어 있으면 아무것도 하지 않는다.
    fn connect(&mut self) -> impl Future<Output = Result<(), DriverError>> + Send;

    /// 영역 하나를 읽는다. 연결이 끊어져 있으면 먼저 다시 연결을 시도한다.
    fn read(
        &mut self,
        block: &ReadBlock,
    ) -> impl Future<Output = Result<BlockData, DriverError>> + Send;

    /// 현재 상태.
    fn status(&self) -> LinkStatus;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_data_len() {
        assert_eq!(BlockData::Bits(vec![true, false]).len(), 2);
        assert!(BlockData::Words(vec![]).is_empty());
    }

    #[test]
    fn error_text_is_readable() {
        assert_eq!(DriverError::Timeout.to_string(), "응답 시간 초과");
        let e = DriverError::Device {
            code: 2,
            text: "주소 오류".into(),
        };
        assert_eq!(e.to_string(), "장치 오류 2: 주소 오류");
    }
}
