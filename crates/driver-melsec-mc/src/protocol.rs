// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! MELSEC MC 프로토콜 3E 바이너리 프레임의 부호화와 해석. I/O 없는 순수 함수다.
//!
//! 공개된 MELSEC 커뮤니케이션 규격의 일괄 읽기(0401)에서 출발했다.
//! 워드 일괄 읽기(서브 0000)와 비트 일괄 읽기(서브 0001)만 지원한다.

use nuriscada_driver_api::{BlockData, DataKind, DriverError};

/// 요청 서브헤더(3E 바이너리).
pub const REQ_SUBHEADER: [u8; 2] = [0x50, 0x00];
/// 응답 서브헤더(3E 바이너리).
pub const RES_SUBHEADER: [u8; 2] = [0xD0, 0x00];
/// Q/L 헤더: 네트워크 번호 0, PC 번호 0xFF, 요구 대상 모듈 I/O 번호 0x03FF, 국 번호 0.
pub const ACCESS_ROUTE: [u8; 5] = [0x00, 0xFF, 0xFF, 0x03, 0x00];
/// CPU 감시 타이머(250 ms 단위, 0x10 = 4초).
pub const MONITOR_TIMER: u16 = 0x0010;
/// 일괄 읽기 명령(0401).
pub const CMD_BATCH_READ: u16 = 0x0401;
/// 응답 머리 길이(서브헤더 2 + 경로 5 + 길이 2 + 종료 코드 2).
pub const RESPONSE_HEADER_LEN: usize = 11;

/// 워드 장치 코드.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordDevice {
    /// D.
    D = 0xA8,
    /// W.
    W = 0xB4,
    /// R.
    R = 0xAF,
}

/// 비트 장치 코드.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDevice {
    /// M.
    M = 0x90,
    /// X(16진 주소).
    X = 0x9C,
    /// Y(16진 주소).
    Y = 0x9D,
}

/// 일괄 읽기 요청.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRequest {
    /// 워드 장치 읽기(개수는 워드 수).
    Words {
        /// 장치 코드.
        code: u8,
        /// 선두 장치 번호(24비트).
        head: u32,
        /// 읽을 워드 수(1~960).
        count: u16,
    },
    /// 비트 장치 읽기(개수는 비트 수).
    Bits {
        /// 장치 코드.
        code: u8,
        /// 선두 장치 번호(24비트).
        head: u32,
        /// 읽을 비트 수.
        count: u16,
    },
}

/// 규격의 한 번 요청 한도보다 작게 잡은 보수적 한도(응답 프레임 여유 포함).
pub const MAX_WORD_POINTS: u16 = 240;
/// 비트 일괄 읽기의 보수적 한도.
pub const MAX_BIT_POINTS: u16 = 512;

impl ReadRequest {
    /// 읽기 종류.
    pub fn kind(self) -> DataKind {
        match self {
            Self::Words { .. } => DataKind::Word,
            Self::Bits { .. } => DataKind::Bit,
        }
    }

    /// 읽을 개수.
    pub fn count(self) -> u16 {
        match self {
            Self::Words { count, .. } | Self::Bits { count, .. } => count,
        }
    }
}

/// 요청을 3E 바이너리 프레임으로 만든다. 개수가 한도를 벗어나면 오류다.
pub fn encode_read_request(req: &ReadRequest) -> Result<Vec<u8>, DriverError> {
    let (subcommand, head, code, count) = match *req {
        ReadRequest::Words { code, head, count } => {
            if count == 0 || count > MAX_WORD_POINTS {
                return Err(DriverError::InvalidRequest(format!(
                    "워드 읽기 개수 {count}는 1~{MAX_WORD_POINTS}이어야 한다"
                )));
            }
            (0x0000u16, head, code, count)
        }
        ReadRequest::Bits { code, head, count } => {
            if count == 0 || count > MAX_BIT_POINTS {
                return Err(DriverError::InvalidRequest(format!(
                    "비트 읽기 개수 {count}는 1~{MAX_BIT_POINTS}이어야 한다"
                )));
            }
            (0x0001u16, head, code, count)
        }
    };
    if head > 0xFF_FFFF {
        return Err(DriverError::InvalidRequest(format!(
            "장치 번호 {head}는 24비트를 넘어선다"
        )));
    }
    // 데이터 길이 = 타이머 2 + 명령 2 + 서브 2 + 선두 번호 3 + 코드 1 + 개수 2.
    let mut frame = Vec::with_capacity(21);
    frame.extend_from_slice(&REQ_SUBHEADER);
    frame.extend_from_slice(&ACCESS_ROUTE);
    frame.extend_from_slice(&12u16.to_le_bytes());
    frame.extend_from_slice(&MONITOR_TIMER.to_le_bytes());
    frame.extend_from_slice(&CMD_BATCH_READ.to_le_bytes());
    frame.extend_from_slice(&subcommand.to_le_bytes());
    frame.extend_from_slice(&head.to_le_bytes()[..3]);
    frame.push(code);
    frame.extend_from_slice(&count.to_le_bytes());
    Ok(frame)
}

/// 고정 머리(서브헤더+경로+길이+종료 코드) 뒤에 따라올 데이터 바이트 수를 읽는다.
///
/// 길이는 종료 코드부터 끝까지이므로 데이터는 `길이 - 2`다.
pub fn remaining_after_header(header: &[u8]) -> Result<usize, DriverError> {
    if header.len() != RESPONSE_HEADER_LEN {
        return Err(DriverError::Protocol(format!(
            "응답 머리 길이가 {RESPONSE_HEADER_LEN}이 아니다: {}",
            header.len()
        )));
    }
    if header[0..2] != RES_SUBHEADER {
        return Err(DriverError::Protocol(format!(
            "서브헤더가 D0 00이 아니다: {:02X} {:02X}",
            header[0], header[1]
        )));
    }
    if header[2..7] != ACCESS_ROUTE {
        return Err(DriverError::Protocol("응답 경로가 요청과 다르다".into()));
    }
    let len = u16::from_le_bytes([header[7], header[8]]);
    if len < 2 {
        return Err(DriverError::Protocol(format!(
            "응답 길이가 너무 짧다: {len}"
        )));
    }
    Ok(usize::from(len) - 2)
}

/// 종료 코드의 설명(자주 보는 것만, 나머지는 16진 표기).
pub fn end_code_text(code: u16) -> String {
    match code {
        0x0050 => "0050: 요청 데이터 길이가 범위를 벗어났다".into(),
        0x0051 => "0051: 통신 데이터 길이가 범위를 벗어났다".into(),
        0xC050 => "C050: 요청 주소가 범위를 벗어났다".into(),
        0xC051 => "C051: 읽을 개수가 범위를 벗어났다".into(),
        0xC059 => "C059: 지정한 장치가 범위를 벗어났다".into(),
        0xC05C => "C05C: 요청 내용이 잘못되었다".into(),
        other => format!("종료 코드 {other:04X}"),
    }
}

/// 응답 프레임 전체(머리 포함)를 해석한다.
pub fn decode_read_response(req: &ReadRequest, frame: &[u8]) -> Result<BlockData, DriverError> {
    if frame.len() < RESPONSE_HEADER_LEN {
        return Err(DriverError::Protocol(format!(
            "응답이 머리보다 짧다: {}",
            frame.len()
        )));
    }
    let (header, data) = frame.split_at(RESPONSE_HEADER_LEN);
    if header[0..2] != RES_SUBHEADER {
        return Err(DriverError::Protocol("서브헤더가 D0 00이 아니다".into()));
    }
    let end = u16::from_le_bytes([header[9], header[10]]);
    if end != 0 {
        return Err(DriverError::Device {
            code: (end & 0xFF) as u8,
            text: format!("MC {}", end_code_text(end)),
        });
    }
    match *req {
        ReadRequest::Words { count, .. } => {
            let want = usize::from(count) * 2;
            if data.len() < want {
                return Err(DriverError::Protocol(format!(
                    "워드 데이터가 부족하다: {} < {want}",
                    data.len()
                )));
            }
            let mut out = Vec::with_capacity(usize::from(count));
            let (pairs, _) = data[..want].as_chunks::<2>();
            for pair in pairs {
                out.push(u16::from_le_bytes([pair[0], pair[1]]));
            }
            Ok(BlockData::Words(out))
        }
        ReadRequest::Bits { count, .. } => {
            let want = usize::from(count).div_ceil(2);
            if data.len() < want {
                return Err(DriverError::Protocol(format!(
                    "비트 데이터가 부족하다: {} < {want}",
                    data.len()
                )));
            }
            // 비트 장치는 1점당 4비트(니블의 bit 0)에 2점씩 들어 있다.
            let mut out = Vec::with_capacity(usize::from(count));
            for (i, &byte) in data[..want].iter().enumerate() {
                let base = i * 2;
                out.push(byte & 0x01 != 0);
                if base + 1 < usize::from(count) {
                    out.push(byte & 0x10 != 0);
                }
            }
            Ok(BlockData::Bits(out))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 워드_읽기_요청_인코딩_d100_2점() {
        let req = ReadRequest::Words {
            code: WordDevice::D as u8,
            head: 100,
            count: 2,
        };
        let frame = encode_read_request(&req).unwrap();
        assert_eq!(
            frame,
            vec![
                0x50, 0x00, // 서브헤더
                0x00, 0xFF, 0xFF, 0x03, 0x00, // 경로
                0x0C, 0x00, // 데이터 길이 12
                0x10, 0x00, // 타이머
                0x01, 0x04, // 0401
                0x00, 0x00, // 서브 0000
                0x64, 0x00, 0x00, // D100
                0xA8, // D
                0x02, 0x00, // 2점
            ]
        );
    }

    #[test]
    fn 비트_읽기_요청_인코딩_m0_3점() {
        let req = ReadRequest::Bits {
            code: BitDevice::M as u8,
            head: 0,
            count: 3,
        };
        let frame = encode_read_request(&req).unwrap();
        assert_eq!(frame[11..15], [0x01, 0x04, 0x01, 0x00]);
        assert_eq!(frame[15..21], [0x00, 0x00, 0x00, 0x90, 0x03, 0x00]);
    }

    #[test]
    fn 개수_0은_거부한다() {
        let req = ReadRequest::Words {
            code: WordDevice::D as u8,
            head: 0,
            count: 0,
        };
        assert!(matches!(
            encode_read_request(&req),
            Err(DriverError::InvalidRequest(_))
        ));
    }

    #[test]
    fn 워드_응답_해석() {
        let req = ReadRequest::Words {
            code: WordDevice::D as u8,
            head: 100,
            count: 2,
        };
        let frame = vec![
            0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00, 0x06, 0x00, 0x00, 0x00, 0x34, 0x12, 0x78,
            0x56,
        ];
        assert_eq!(
            decode_read_response(&req, &frame).unwrap(),
            BlockData::Words(vec![0x1234, 0x5678])
        );
    }

    #[test]
    fn 비트_응답_해석_니블_포장() {
        let req = ReadRequest::Bits {
            code: BitDevice::M as u8,
            head: 0,
            count: 3,
        };
        // 바이트1: M0=1(하위 니블), M1=0(상위 니블 bit4=0). 바이트2: M2=1.
        let frame = vec![
            0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01,
        ];
        assert_eq!(
            decode_read_response(&req, &frame).unwrap(),
            BlockData::Bits(vec![true, false, true])
        );
    }

    #[test]
    fn 장치_오류는_종료_코드를_담는다() {
        let req = ReadRequest::Words {
            code: WordDevice::D as u8,
            head: 0,
            count: 1,
        };
        let frame = vec![
            0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00, 0x02, 0x00, 0x50, 0xC0,
        ];
        let err = decode_read_response(&req, &frame).unwrap_err();
        assert!(matches!(err, DriverError::Device { .. }));
        assert!(err.to_string().contains("C050"));
    }

    #[test]
    fn 서브헤더_불일치는_프로토콜_오류다() {
        let req = ReadRequest::Words {
            code: WordDevice::D as u8,
            head: 0,
            count: 1,
        };
        let frame = vec![
            0x50, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(matches!(
            decode_read_response(&req, &frame).unwrap_err(),
            DriverError::Protocol(_)
        ));
    }
}
