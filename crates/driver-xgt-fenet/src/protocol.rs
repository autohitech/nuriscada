// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! XGT 전용 프로토콜(FEnet) 개별 읽기의 부호화와 해석. I/O 없는 순수 함수다.
//!
//! 공개된 XGT FEnet 매뉴얼의 개별 읽기(0x5400, 워드·비트)에서 출발했다.
//! 응답의 블록별 구성(블록 수, 데이터 개수+데이터)은 매뉴얼의 응답 예시를 따른다.

use nuriscada_driver_api::{BlockData, DataKind, DriverError};

/// 회사 ID("LSIS-XGT" + NULL 2바이트).
pub const COMPANY_ID: [u8; 10] = [0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00];
/// CPU 정보(XGK).
pub const CPU_INFO: u8 = 0xA0;
/// 프레임 방향: 클라이언트→서버.
pub const SOURCE_REQUEST: u8 = 0x33;
/// 프레임 방향: 서버→클라이언트.
pub const SOURCE_RESPONSE: u8 = 0x11;
/// 머리 길이(회사 10 + PLC 정보 2 + CPU 1 + 방향 1 + 호출 번호 2 + 길이 2 + 위치 1 + 검사합 1).
pub const HEADER_LEN: usize = 20;
/// 개별 읽기 명령(매뉴얼 표기 "5400", 바이트 순서 그대로 리틀엔디안 수치 0x0054).
pub const CMD_READ: u16 = 0x0054;
/// 개별 읽기 응답(매뉴얼 표기 "5500").
pub const CMD_READ_RESPONSE: u16 = 0x0055;
/// 워드 데이터 종류.
pub const DTYPE_WORD: u16 = 0x0002;
/// 비트 데이터 종류.
pub const DTYPE_BIT: u16 = 0x0000;
/// 한 요청의 최대 블록 수(매뉴얼의 블록 수 범위 1~16).
pub const MAX_BLOCKS: u16 = 16;

/// 읽을 변수 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    /// 변수 이름(`%MW100`처럼 `%` 포함 ASCII).
    pub name: Vec<u8>,
    /// 워드인가(거짓이면 비트).
    pub is_word: bool,
}

/// 개별 읽기 요청. 모든 변수는 같은 데이터 종류여야 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRequest {
    /// 호출 번호(응답 대조용).
    pub invoke: u16,
    /// 읽을 변수 목록(1~16개).
    pub vars: Vec<Variable>,
}

impl ReadRequest {
    /// 읽기 종류.
    pub fn kind(&self) -> DataKind {
        if self.vars.iter().all(|v| v.is_word) {
            DataKind::Word
        } else {
            DataKind::Bit
        }
    }
}

/// 요청을 프레임으로 만든다. 변수 개수·이름을 검사한다.
pub fn encode_read_request(req: &ReadRequest) -> Result<Vec<u8>, DriverError> {
    if req.vars.is_empty() || req.vars.len() > usize::from(MAX_BLOCKS) {
        return Err(DriverError::InvalidRequest(format!(
            "변수 개수 {}는 1~{MAX_BLOCKS}이어야 한다",
            req.vars.len()
        )));
    }
    let word = req.vars[0].is_word;
    if req.vars.iter().any(|v| v.is_word != word) {
        return Err(DriverError::InvalidRequest(
            "한 요청의 변수는 같은 데이터 종류이어야 한다".into(),
        ));
    }
    let mut body = Vec::new();
    body.extend_from_slice(&CMD_READ.to_le_bytes());
    body.extend_from_slice(&(if word { DTYPE_WORD } else { DTYPE_BIT }).to_le_bytes());
    body.extend_from_slice(&[0x00, 0x00]);
    body.extend_from_slice(&(req.vars.len() as u16).to_le_bytes());
    for v in &req.vars {
        if v.name.is_empty() || v.name.len() > 16 || !v.name.is_ascii() {
            return Err(DriverError::InvalidRequest(format!(
                "변수 이름 길이가 범위를 벗어났다: {}",
                v.name.len()
            )));
        }
        body.extend_from_slice(&(v.name.len() as u16).to_le_bytes());
        body.extend_from_slice(&v.name);
    }
    let len = u16::try_from(body.len())
        .map_err(|_| DriverError::InvalidRequest("요청이 너무 길다".into()))?;
    let mut frame = Vec::with_capacity(HEADER_LEN + body.len());
    frame.extend_from_slice(&COMPANY_ID);
    frame.extend_from_slice(&[0x00, 0x00]);
    frame.push(CPU_INFO);
    frame.push(SOURCE_REQUEST);
    frame.extend_from_slice(&req.invoke.to_le_bytes());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.push(0x00);
    frame.push(0x00);
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// 머리에서 뒤따를 본문 길이를 읽는다.
pub fn remaining_after_header(header: &[u8]) -> Result<usize, DriverError> {
    if header.len() != HEADER_LEN {
        return Err(DriverError::Protocol(format!(
            "응답 머리가 {HEADER_LEN}바이트가 아니다: {}",
            header.len()
        )));
    }
    if header[..8] != COMPANY_ID[..8] {
        return Err(DriverError::Protocol("회사 ID가 LSIS-XGT가 아니다".into()));
    }
    if header[13] != SOURCE_RESPONSE {
        return Err(DriverError::Protocol(format!(
            "프레임 방향이 서버→클라이언트가 아니다: {:02X}",
            header[13]
        )));
    }
    Ok(usize::from(u16::from_le_bytes([header[16], header[17]])))
}

/// 오류 상태의 설명.
pub fn error_text(code: u16) -> String {
    match code {
        0x0000 => "정상".into(),
        0x0001 => "0001: 요청 형식이 잘못되었다".into(),
        0x0011 => "0011: 존재하지 않는 변수이다".into(),
        other => format!("오류 상태 {other:04X}"),
    }
}

/// 응답 프레임 전체(머리 포함)를 해석한다. 변수별 바이트 수를 순서대로 돌려준다.
pub fn decode_read_response(req: &ReadRequest, frame: &[u8]) -> Result<BlockData, DriverError> {
    if frame.len() < HEADER_LEN + 10 {
        return Err(DriverError::Protocol(format!(
            "응답이 너무 짧다: {}",
            frame.len()
        )));
    }
    let (header, mut body) = frame.split_at(HEADER_LEN);
    if header[..8] != COMPANY_ID[..8] {
        return Err(DriverError::Protocol("회사 ID가 LSIS-XGT가 아니다".into()));
    }
    if header[13] != SOURCE_RESPONSE {
        return Err(DriverError::Protocol(
            "프레임 방향이 서버→클라이언트가 아니다".into(),
        ));
    }
    let take = |body: &mut &[u8], n: usize| -> Result<Vec<u8>, DriverError> {
        if body.len() < n {
            return Err(DriverError::Protocol(format!(
                "본문이 짧다: {} < {n}",
                body.len()
            )));
        }
        let (head, rest) = body.split_at(n);
        *body = rest;
        Ok(head.to_vec())
    };
    let cmd = u16::from_le_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
    if cmd != CMD_READ_RESPONSE {
        return Err(DriverError::Protocol(format!(
            "응답 명령이 0x5500이 아니다: {cmd:04X}"
        )));
    }
    let dtype = u16::from_le_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
    let want_dtype = if req.vars[0].is_word {
        DTYPE_WORD
    } else {
        DTYPE_BIT
    };
    if dtype != want_dtype {
        return Err(DriverError::Protocol(format!(
            "데이터 종류가 요청과 다르다: {dtype:04X}"
        )));
    }
    let _reserved = take(&mut body, 2)?;
    let error = u16::from_le_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
    if error != 0 {
        return Err(DriverError::Device {
            code: (error & 0xFF) as u8,
            text: format!("XGT {}", error_text(error)),
        });
    }
    let blocks = u16::from_le_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
    if usize::from(blocks) != req.vars.len() {
        return Err(DriverError::Protocol(format!(
            "블록 수 {}가 요청 {}과 다르다",
            blocks,
            req.vars.len()
        )));
    }
    let word = req.vars[0].is_word;
    if word {
        let mut out = Vec::new();
        for _ in &req.vars {
            let count = u16::from_le_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
            if count != 2 {
                return Err(DriverError::Protocol(format!(
                    "워드 데이터 개수가 2바이트가 아니다: {count}"
                )));
            }
            let bytes = take(&mut body, 2)?;
            out.push(u16::from_le_bytes([bytes[0], bytes[1]]));
        }
        Ok(BlockData::Words(out))
    } else {
        let mut out = Vec::new();
        for _ in &req.vars {
            let count = u16::from_le_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
            if count != 1 {
                return Err(DriverError::Protocol(format!(
                    "비트 데이터 개수가 1바이트가 아니다: {count}"
                )));
            }
            let bytes = take(&mut body, 1)?;
            out.push(bytes[0] != 0);
        }
        Ok(BlockData::Bits(out))
    }
}

/// 개별 쓰기 명령(매뉴얼 표기 "5800").
pub const CMD_WRITE: u16 = 0x0058;
/// 개별 쓰기 응답(매뉴얼 표기 "5900").
pub const CMD_WRITE_RESPONSE: u16 = 0x0059;

/// 개별 쓰기 요청. 모든 변수는 같은 데이터 종류여야 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteRequest {
    /// 워드 쓰기(이름과 값 목록, 개수가 같아야 한다).
    Words {
        /// 변수 이름 목록(`%MW100`처럼 `%` 포함 ASCII).
        names: Vec<Vec<u8>>,
        /// 쓸 워드 값.
        values: Vec<u16>,
    },
    /// 비트 쓰기(이름과 값 목록, 개수가 같아야 한다).
    Bits {
        /// 변수 이름 목록.
        names: Vec<Vec<u8>>,
        /// 쓸 비트 값.
        values: Vec<bool>,
    },
}

/// 요청을 프레임으로 만든다.
pub fn encode_write_request(invoke: u16, req: &WriteRequest) -> Result<Vec<u8>, DriverError> {
    let (is_word, names, data) = match req {
        WriteRequest::Words { names, values } => {
            if names.is_empty()
                || names.len() > usize::from(MAX_BLOCKS)
                || names.len() != values.len()
            {
                return Err(DriverError::InvalidRequest(
                    "워드 쓰기 변수·값 개수가 1~16이고 같아야 한다".into(),
                ));
            }
            let mut data = Vec::with_capacity(values.len() * 4);
            for v in values {
                data.extend_from_slice(&2u16.to_le_bytes());
                data.extend_from_slice(&v.to_le_bytes());
            }
            (true, names, data)
        }
        WriteRequest::Bits { names, values } => {
            if names.is_empty()
                || names.len() > usize::from(MAX_BLOCKS)
                || names.len() != values.len()
            {
                return Err(DriverError::InvalidRequest(
                    "비트 쓰기 변수·값 개수가 1~16이고 같아야 한다".into(),
                ));
            }
            let mut data = Vec::with_capacity(values.len() * 3);
            for b in values {
                data.extend_from_slice(&1u16.to_le_bytes());
                data.push(u8::from(*b));
            }
            (false, names, data)
        }
    };
    let mut body = Vec::new();
    body.extend_from_slice(&CMD_WRITE.to_le_bytes());
    body.extend_from_slice(&(if is_word { DTYPE_WORD } else { DTYPE_BIT }).to_le_bytes());
    body.extend_from_slice(&[0x00, 0x00]);
    body.extend_from_slice(&(names.len() as u16).to_le_bytes());
    for name in names {
        if name.is_empty() || name.len() > 16 || !name.is_ascii() {
            return Err(DriverError::InvalidRequest(format!(
                "변수 이름 길이가 범위를 벗어났다: {}",
                name.len()
            )));
        }
        body.extend_from_slice(&(name.len() as u16).to_le_bytes());
        body.extend_from_slice(name);
    }
    body.extend_from_slice(&data);
    let len = u16::try_from(body.len())
        .map_err(|_| DriverError::InvalidRequest("요청이 너무 길다".into()))?;
    let mut frame = Vec::with_capacity(HEADER_LEN + body.len());
    frame.extend_from_slice(&COMPANY_ID);
    frame.extend_from_slice(&[0x00, 0x00]);
    frame.push(CPU_INFO);
    frame.push(SOURCE_REQUEST);
    frame.extend_from_slice(&invoke.to_le_bytes());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.push(0x00);
    frame.push(0x00);
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// 쓰기 응답 프레임(머리 포함)을 해석한다. 오류 상태만 본다.
pub fn decode_write_response(frame: &[u8]) -> Result<(), DriverError> {
    if frame.len() < HEADER_LEN + 8 {
        return Err(DriverError::Protocol(format!(
            "응답이 너무 짧다: {}",
            frame.len()
        )));
    }
    let (header, mut body) = frame.split_at(HEADER_LEN);
    if header[..8] != COMPANY_ID[..8] {
        return Err(DriverError::Protocol("회사 ID가 LSIS-XGT가 아니다".into()));
    }
    if header[13] != SOURCE_RESPONSE {
        return Err(DriverError::Protocol(
            "프레임 방향이 서버→클라이언트가 아니다".into(),
        ));
    }
    let take = |body: &mut &[u8], n: usize| -> Result<Vec<u8>, DriverError> {
        if body.len() < n {
            return Err(DriverError::Protocol(format!(
                "본문이 짧다: {} < {n}",
                body.len()
            )));
        }
        let (head, rest) = body.split_at(n);
        *body = rest;
        Ok(head.to_vec())
    };
    let cmd = u16::from_le_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
    if cmd != CMD_WRITE_RESPONSE {
        return Err(DriverError::Protocol(format!(
            "응답 명령이 0x5900이 아니다: {cmd:04X}"
        )));
    }
    let _dtype = take(&mut body, 2)?;
    let _reserved = take(&mut body, 2)?;
    let error = u16::from_le_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
    if error != 0 {
        return Err(DriverError::Device {
            code: (error & 0xFF) as u8,
            text: format!("XGT {}", error_text(error)),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str, is_word: bool) -> Variable {
        Variable {
            name: name.as_bytes().to_vec(),
            is_word,
        }
    }

    #[test]
    fn 워드_읽기_요청_인코딩_매뉴얼_예시() {
        // 매뉴얼의 %MW0 개별 읽기 예시(검사합 바이트는 0x00으로 보낸다).
        let req = ReadRequest {
            invoke: 0,
            vars: vec![var("%MW0", true)],
        };
        let frame = encode_read_request(&req).unwrap();
        assert_eq!(
            frame,
            vec![
                0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00, // 회사 ID
                0x00, 0x00, // PLC 정보
                0xA0, // CPU
                0x33, // 방향
                0x00, 0x00, // 호출 번호
                0x0E, 0x00, // 길이 14
                0x00, // 위치
                0x00, // 검사합
                0x54, 0x00, // 0x5400
                0x02, 0x00, // 워드
                0x00, 0x00, // 예약
                0x01, 0x00, // 1블록
                0x04, 0x00, // 이름 길이
                0x25, 0x4D, 0x57, 0x30, // %MW0
            ]
        );
    }

    #[test]
    fn 비트_읽기_요청_인코딩() {
        let req = ReadRequest {
            invoke: 7,
            vars: vec![var("%MX0", false), var("%MX1", false)],
        };
        let frame = encode_read_request(&req).unwrap();
        assert_eq!(frame[20..26], [0x54, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(frame[26..28], [0x02, 0x00]);
        assert_eq!(frame[14..16], [0x07, 0x00]);
    }

    #[test]
    fn 변수_없음과_종류_혼용은_거부한다() {
        let empty = ReadRequest {
            invoke: 0,
            vars: vec![],
        };
        assert!(matches!(
            encode_read_request(&empty),
            Err(DriverError::InvalidRequest(_))
        ));
        let mixed = ReadRequest {
            invoke: 0,
            vars: vec![var("%MW0", true), var("%MX0", false)],
        };
        assert!(matches!(
            encode_read_request(&mixed),
            Err(DriverError::InvalidRequest(_))
        ));
    }

    #[test]
    fn 워드_응답_해석() {
        let req = ReadRequest {
            invoke: 0,
            vars: vec![var("%MW0", true)],
        };
        let mut frame = vec![
            0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00, 0x00, 0x00, 0xA0, 0x11,
            0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x55, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x02, 0x00, 0x34, 0x12,
        ];
        assert_eq!(
            decode_read_response(&req, &frame).unwrap(),
            BlockData::Words(vec![0x1234])
        );
        frame[26] = 0x11;
        let err = decode_read_response(&req, &frame).unwrap_err();
        assert!(matches!(err, DriverError::Device { .. }));
        assert!(err.to_string().contains("0011"));
    }

    #[test]
    fn 비트_응답_해석() {
        let req = ReadRequest {
            invoke: 0,
            vars: vec![var("%MX0", false), var("%MX1", false)],
        };
        let frame = vec![
            0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00, 0x00, 0x00, 0xA0, 0x11,
            0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x02, 0x00, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00,
        ];
        assert_eq!(
            decode_read_response(&req, &frame).unwrap(),
            BlockData::Bits(vec![true, false])
        );
    }

    #[test]
    fn 블록_수_불일치는_프로토콜_오류다() {
        let req = ReadRequest {
            invoke: 0,
            vars: vec![var("%MW0", true), var("%MW1", true)],
        };
        let frame = vec![
            0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00, 0x00, 0x00, 0xA0, 0x11,
            0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x55, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x02, 0x00, 0x34, 0x12,
        ];
        assert!(matches!(
            decode_read_response(&req, &frame),
            Err(DriverError::Protocol(_))
        ));
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;

    #[test]
    fn 워드_쓰기_요청_인코딩_매뉴얼_예시() {
        // 매뉴얼의 %MW0 워드 쓰기 예시(데이터 개수 02 00 + 값 2바이트).
        let req = WriteRequest::Words {
            names: vec![b"%MW0".to_vec()],
            values: vec![0x1234],
        };
        let frame = encode_write_request(0, &req).unwrap();
        assert_eq!(
            frame,
            vec![
                0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00, // 회사 ID
                0x00, 0x00, // PLC 정보
                0xA0, // CPU
                0x33, // 방향
                0x00, 0x00, // 호출 번호
                0x12, 0x00, // 길이 18
                0x00, // 위치
                0x00, // 검사합
                0x58, 0x00, // 0x5800
                0x02, 0x00, // 워드
                0x00, 0x00, // 예약
                0x01, 0x00, // 1블록
                0x04, 0x00, // 이름 길이
                0x25, 0x4D, 0x57, 0x30, // %MW0
                0x02, 0x00, // 데이터 개수
                0x34, 0x12, // 값
            ]
        );
    }

    #[test]
    fn 비트_쓰기_요청_인코딩() {
        let req = WriteRequest::Bits {
            names: vec![b"%MX0".to_vec(), b"%MX1".to_vec()],
            values: vec![true, false],
        };
        let frame = encode_write_request(7, &req).unwrap();
        assert_eq!(frame[14..16], [0x07, 0x00]);
        assert_eq!(frame[20..26], [0x58, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(
            &frame[frame.len() - 6..],
            &[0x01, 0x00, 0x01, 0x01, 0x00, 0x00]
        );
    }

    #[test]
    fn 개수_불일치와_빈_요청은_거부한다() {
        let bad = WriteRequest::Words {
            names: vec![b"%MW0".to_vec()],
            values: vec![],
        };
        assert!(matches!(
            encode_write_request(0, &bad),
            Err(DriverError::InvalidRequest(_))
        ));
        let empty = WriteRequest::Bits {
            names: vec![],
            values: vec![],
        };
        assert!(matches!(
            encode_write_request(0, &empty),
            Err(DriverError::InvalidRequest(_))
        ));
    }

    #[test]
    fn 쓰기_응답은_종료_상태만_본다() {
        let ok = vec![
            0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00, 0x00, 0x00, 0xA0, 0x11,
            0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x59, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(decode_write_response(&ok).is_ok());
        let mut bad = ok.clone();
        bad[26] = 0x11;
        let err = decode_write_response(&bad).unwrap_err();
        assert!(matches!(err, DriverError::Device { .. }));
        assert!(err.to_string().contains("0011"));
    }
}
