// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! S7 (ISO-on-TCP) 읽기의 부호화와 해석. I/O 없는 순수 함수다.
//!
//! 공개 문서에서 출발했다: TPKT·COTP 접속은 RFC 1006과 공개된 접속 절차,
//! S7 PDU·읽기 항목 배치는 공개된 필드 정의. S7CommPlus(0x72)는 범위 밖이다.

use nuriscada_driver_api::{BlockData, DataKind, DriverError};

/// TPKT 버전.
pub const TPKT_VERSION: u8 = 3;
/// TPKT 머리 길이.
pub const TPKT_LEN: usize = 4;
/// COTP 데이터 PDU 종류.
pub const COTP_DT: u8 = 0xF0;
/// S7 프로토콜 식별자.
pub const S7_PROTO: u8 = 0x32;
/// S7 작업 요청.
pub const ROSCTR_JOB: u8 = 0x01;
/// S7 응답(데이터 있음).
pub const ROSCTR_ACK_DATA: u8 = 0x03;
/// 통신 설정 기능.
pub const FUNC_SETUP: u8 = 0xF0;
/// 읽기 변수 기능.
pub const FUNC_READ: u8 = 0x04;
/// 요청 PDU 크기(바이트).
pub const PDU_SIZE: u16 = 480;
/// M 영역 코드.
pub const AREA_M: u8 = 0x83;
/// DB 영역 코드.
pub const AREA_DB: u8 = 0x84;
/// 항목 지정자(S7ANY).
pub const ITEM_ANY: u8 = 0x12;
/// S7ANY 구문.
pub const SYNTAX_ANY: u8 = 0x10;
/// 전송 크기 바이트.
pub const TRANS_BYTE: u8 = 0x02;
/// 전송 크기 비트.
pub const TRANS_BIT: u8 = 0x01;
/// 한 요청의 보수적 한도(워드 32, 비트 32).
pub const MAX_WORD_ITEMS: u16 = 32;
/// 비트 읽기의 보수적 한도.
pub const MAX_BIT_ITEMS: u16 = 32;

/// TPKT 머리를 붙인다.
fn tpkt(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(TPKT_LEN + payload.len());
    out.push(TPKT_VERSION);
    out.push(0x00);
    out.extend_from_slice(&(TPKT_LEN as u16 + payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// COTP 데이터 래퍼를 붙인다(순서 번호 0).
fn cotp_dt(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + payload.len());
    out.push(0x02);
    out.push(COTP_DT);
    out.push(0x80);
    out.extend_from_slice(payload);
    out
}

/// TPKT·COTP 머리를 벗긴 S7 PDU를 돌려준다.
pub fn strip_transport(frame: &[u8]) -> Result<&[u8], DriverError> {
    if frame.len() < TPKT_LEN + 3 {
        return Err(DriverError::Protocol(format!(
            "프레임이 너무 짧다: {}",
            frame.len()
        )));
    }
    if frame[0] != TPKT_VERSION {
        return Err(DriverError::Protocol(format!(
            "TPKT 버전이 3이 아니다: {}",
            frame[0]
        )));
    }
    let total = usize::from(u16::from_be_bytes([frame[2], frame[3]]));
    if total != frame.len() {
        return Err(DriverError::Protocol(format!(
            "TPKT 길이 {}와 실제 {}가 다르다",
            total,
            frame.len()
        )));
    }
    if frame[TPKT_LEN] != 0x02 || frame[TPKT_LEN + 1] != COTP_DT {
        return Err(DriverError::Protocol("COTP 데이터가 아니다".into()));
    }
    Ok(&frame[TPKT_LEN + 3..])
}

/// COTP 접속 요청(소스 TSAP 0x0100, 목적 TSAP 0x01 + unit).
/// `unit` 하위 5비트가 슬롯, 상위 3비트가 랙이다.
pub fn encode_cotp_connect(unit: u8) -> Vec<u8> {
    let payload = vec![
        0x11, 0xE0, 0x00, 0x00, 0x00, 0x01, 0x00, 0xC1, 0x02, 0x01, 0x00, 0xC2, 0x02, 0x01, unit,
        0xC0, 0x01, 0x09,
    ];
    tpkt(&payload)
}

/// COTP 접속 확인을 검사한다.
pub fn decode_cotp_confirm(frame: &[u8]) -> Result<(), DriverError> {
    if frame.len() < TPKT_LEN + 2 || frame[0] != TPKT_VERSION {
        return Err(DriverError::Protocol("COTP 접속 확인이 아니다".into()));
    }
    // TPKT 뒤: COTP 머리 길이 + PDU 종류(CC 0xD0).
    if frame[TPKT_LEN + 1] != 0xD0 {
        return Err(DriverError::Protocol("COTP 접속 확인이 아니다".into()));
    }
    Ok(())
}

/// S7 머리(10바이트)를 만든다. PDU 참조는 리틀엔디안, 길이는 빅엔디안이다.
fn s7_header(rosctr: u8, pdu_ref: u16, param_len: u16, data_len: u16) -> Vec<u8> {
    let mut h = vec![S7_PROTO, rosctr, 0x00, 0x00];
    h.extend_from_slice(&pdu_ref.to_le_bytes());
    h.extend_from_slice(&param_len.to_be_bytes());
    h.extend_from_slice(&data_len.to_be_bytes());
    h
}

/// 통신 설정 요청을 만든다(PDU 480, 병렬 1).
pub fn encode_setup(pdu_ref: u16) -> Vec<u8> {
    let param = vec![FUNC_SETUP, 0x00, 0x00, 0x01, 0x00, 0x01, 0x01, 0xE0];
    let mut pdu = s7_header(ROSCTR_JOB, pdu_ref, param.len() as u16, 0);
    pdu.extend_from_slice(&param);
    tpkt(&cotp_dt(&pdu))
}

/// 통신 설정 응답을 검사한다.
pub fn decode_setup(frame: &[u8], pdu_ref: u16) -> Result<(), DriverError> {
    let pdu = strip_transport(frame)?;
    if pdu.len() < 18 || pdu[0] != S7_PROTO || pdu[1] != ROSCTR_ACK_DATA {
        return Err(DriverError::Protocol("설정 응답이 아니다".into()));
    }
    if u16::from_le_bytes([pdu[4], pdu[5]]) != pdu_ref {
        return Err(DriverError::Protocol("PDU 참조가 다르다".into()));
    }
    Ok(())
}

/// 읽기 요청.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRequest {
    /// DB 워드 읽기.
    DbWords {
        /// DB 번호.
        db: u16,
        /// 시작 바이트.
        start: u16,
        /// 읽을 워드 수.
        count: u16,
    },
    /// M 비트 읽기.
    MBits {
        /// 시작 비트 번호(바이트×8 + 비트).
        start_bit: u32,
        /// 읽을 비트 수.
        count: u16,
    },
}

impl ReadRequest {
    /// 읽기 종류.
    pub fn kind(self) -> DataKind {
        match self {
            Self::DbWords { .. } => DataKind::Word,
            Self::MBits { .. } => DataKind::Bit,
        }
    }
}

/// 읽기 변수 요청을 만든다. 항목은 하나다.
pub fn encode_read_request(pdu_ref: u16, req: &ReadRequest) -> Result<Vec<u8>, DriverError> {
    let (transport, data_len, db, area, addr_field) = match *req {
        ReadRequest::DbWords { db, start, count } => {
            if count == 0 || count > MAX_WORD_ITEMS {
                return Err(DriverError::InvalidRequest(format!(
                    "워드 읽기 개수 {count}는 1~{MAX_WORD_ITEMS}이어야 한다"
                )));
            }
            if db > 255 {
                return Err(DriverError::InvalidRequest(format!(
                    "DB 번호 {db}는 255 이하여야 한다"
                )));
            }
            (
                TRANS_BYTE,
                u32::from(count) * 2,
                db,
                AREA_DB,
                [
                    (u32::from(start) >> 16) as u8,
                    (u32::from(start) >> 8) as u8,
                    (start & 0xFF) as u8,
                ],
            )
        }
        ReadRequest::MBits { start_bit, count } => {
            if count == 0 || count > MAX_BIT_ITEMS {
                return Err(DriverError::InvalidRequest(format!(
                    "비트 읽기 개수 {count}는 1~{MAX_BIT_ITEMS}이어야 한다"
                )));
            }
            let byte = start_bit / 8;
            let bit = (start_bit % 8) as u8;
            if byte > 0x1F_FFFF {
                return Err(DriverError::InvalidRequest(format!(
                    "비트 번호 {start_bit}가 범위를 벗어났다"
                )));
            }
            let addr24 = (byte << 3) | u32::from(bit);
            (
                TRANS_BIT,
                u32::from(count),
                0,
                AREA_M,
                [
                    (addr24 >> 16) as u8,
                    (addr24 >> 8) as u8,
                    (addr24 & 0xFF) as u8,
                ],
            )
        }
    };
    let mut param = vec![FUNC_READ, 0x01];
    param.extend_from_slice(&[
        ITEM_ANY,
        0x0A,
        SYNTAX_ANY,
        transport,
        (data_len >> 8) as u8,
        (data_len & 0xFF) as u8,
        (db >> 8) as u8,
        (db & 0xFF) as u8,
        area,
        addr_field[0],
        addr_field[1],
        addr_field[2],
    ]);
    let mut pdu = s7_header(ROSCTR_JOB, pdu_ref, param.len() as u16, 0);
    pdu.extend_from_slice(&param);
    Ok(tpkt(&cotp_dt(&pdu)))
}

/// 읽기 응답을 해석한다.
pub fn decode_read_response(req: &ReadRequest, frame: &[u8]) -> Result<BlockData, DriverError> {
    let pdu = strip_transport(frame)?;
    if pdu.len() < 14 {
        return Err(DriverError::Protocol(format!(
            "S7 PDU가 너무 짧다: {}",
            pdu.len()
        )));
    }
    if pdu[0] != S7_PROTO || pdu[1] != ROSCTR_ACK_DATA {
        return Err(DriverError::Protocol(format!(
            "읽기 응답이 아니다: {:02X} {:02X}",
            pdu[0], pdu[1]
        )));
    }
    let err_class = pdu[10];
    let err_code = pdu[11];
    if err_class != 0 || err_code != 0 {
        return Err(DriverError::Device {
            code: err_code,
            text: format!("S7 오류 {err_class:02X}{err_code:02X}"),
        });
    }
    let mut body = &pdu[12..];
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
    let ret = take(&mut body, 1)?[0];
    if ret != 0xFF {
        return Err(DriverError::Device {
            code: ret,
            text: format!("S7 항목 오류 {ret:02X}"),
        });
    }
    let _transport = take(&mut body, 1)?;
    let len = u16::from_be_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
    match *req {
        ReadRequest::DbWords { count, .. } => {
            if usize::from(len) != usize::from(count) * 2 {
                return Err(DriverError::Protocol(format!(
                    "워드 길이가 맞지 않다: {len}"
                )));
            }
            let mut out = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let bytes = take(&mut body, 2)?;
                out.push(u16::from_be_bytes([bytes[0], bytes[1]]));
            }
            // 홀수 길이면 채움 바이트를 둔다.
            Ok(BlockData::Words(out))
        }
        ReadRequest::MBits { count, .. } => {
            if usize::from(len) != usize::from(count).div_ceil(8) {
                return Err(DriverError::Protocol(format!(
                    "비트 길이가 맞지 않다: {len}"
                )));
            }
            let mut out = Vec::with_capacity(usize::from(count));
            let mut left = usize::from(count);
            while left > 0 {
                let byte = take(&mut body, 1)?[0];
                for i in 0..8 {
                    if left == 0 {
                        break;
                    }
                    out.push(byte & (1 << i) != 0);
                    left -= 1;
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
    fn cotp_접속_요청_인코딩() {
        // 목적 TSAP 0x01 0x02(랙 0 슬롯 2).
        let frame = encode_cotp_connect(0x02);
        assert_eq!(
            frame,
            vec![
                0x03, 0x00, 0x00, 0x16, 0x11, 0xE0, 0x00, 0x00, 0x00, 0x01, 0x00, 0xC1, 0x02, 0x01,
                0x00, 0xC2, 0x02, 0x01, 0x02, 0xC0, 0x01, 0x09,
            ]
        );
    }

    #[test]
    fn 설정_요청_인코딩() {
        let frame = encode_setup(0x0001);
        // TPKT + COTP DT + S7 머리 10 + 파라미터 8.
        assert_eq!(frame.len(), 4 + 3 + 10 + 8);
        assert_eq!(frame[0..4], [0x03, 0x00, 0x00, 0x19]);
        assert_eq!(frame[4..7], [0x02, 0xF0, 0x80]);
        assert_eq!(frame[7..11], [0x32, 0x01, 0x00, 0x00]);
        assert_eq!(frame[11..13], [0x01, 0x00]);
        assert_eq!(frame[13..17], [0x00, 0x08, 0x00, 0x00]);
        assert_eq!(
            frame[17..25],
            [0xF0, 0x00, 0x00, 0x01, 0x00, 0x01, 0x01, 0xE0]
        );
    }

    #[test]
    fn db_읽기_요청_인코딩() {
        let req = ReadRequest::DbWords {
            db: 1,
            start: 0,
            count: 2,
        };
        let frame = encode_read_request(1, &req).unwrap();
        let pdu = strip_transport(&frame).unwrap();
        assert_eq!(pdu[0..4], [0x32, 0x01, 0x00, 0x00]);
        assert_eq!(pdu[4..6], [0x01, 0x00]);
        // 항목: 0x12 0x0A 0x10 BYTE len DB area addr.
        assert_eq!(
            pdu[10..24],
            [
                0x04, 0x01, 0x12, 0x0A, 0x10, 0x02, 0x00, 0x04, 0x00, 0x01, 0x84, 0x00, 0x00, 0x00,
            ]
        );
    }

    #[test]
    fn m_비트_읽기_요청_인코딩() {
        // M10.3: 바이트 10, 비트 3.
        let req = ReadRequest::MBits {
            start_bit: 10 * 8 + 3,
            count: 2,
        };
        let frame = encode_read_request(1, &req).unwrap();
        let pdu = strip_transport(&frame).unwrap();
        assert_eq!(
            pdu[10..24],
            [
                0x04, 0x01, 0x12, 0x0A, 0x10, 0x01, 0x00, 0x02, 0x00, 0x00, 0x83, 0x00, 0x00, 0x53,
            ]
        );
    }

    #[test]
    fn 개수_0과_db_초과는_거부한다() {
        let bad = ReadRequest::DbWords {
            db: 0,
            start: 0,
            count: 0,
        };
        assert!(matches!(
            encode_read_request(1, &bad),
            Err(DriverError::InvalidRequest(_))
        ));
        let bad_db = ReadRequest::DbWords {
            db: 300,
            start: 0,
            count: 1,
        };
        assert!(matches!(
            encode_read_request(1, &bad_db),
            Err(DriverError::InvalidRequest(_))
        ));
    }

    #[test]
    fn db_응답_해석() {
        let req = ReadRequest::DbWords {
            db: 1,
            start: 0,
            count: 2,
        };
        // 머리 12 + 항목(반환 1 + 전송 1 + 길이 2 + 데이터 4).
        let body = vec![
            0x32, 0x03, 0x00, 0x00, 0x01, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x04,
            0x00, 0x04, 0x12, 0x34, 0x56, 0x78,
        ];
        let mut frame = vec![0x03, 0x00, 0x00, 0x1B];
        frame.extend_from_slice(&[0x02, 0xF0, 0x80]);
        frame.extend_from_slice(&body);
        assert_eq!(
            decode_read_response(&req, &frame).unwrap(),
            BlockData::Words(vec![0x1234, 0x5678])
        );
        // 항목 오류(반환 코드 위치).
        let mut bad = frame.clone();
        bad[19] = 0x05;
        let err = decode_read_response(&req, &bad).unwrap_err();
        assert!(matches!(err, DriverError::Device { .. }));
    }

    #[test]
    fn m_비트_응답_해석() {
        let req = ReadRequest::MBits {
            start_bit: 0,
            count: 10,
        };
        // 10비트 = 2바이트.
        let body = vec![
            0x32,
            0x03,
            0x00,
            0x00,
            0x01,
            0x00,
            0x00,
            0x04,
            0x00,
            0x00,
            0x00,
            0x00,
            0xFF,
            0x04,
            0x00,
            0x02,
            0b0000_0101,
            0b0000_0010,
        ];
        let mut frame = vec![0x03, 0x00, 0x00, 0x19];
        frame.extend_from_slice(&[0x02, 0xF0, 0x80]);
        frame.extend_from_slice(&body);
        assert_eq!(
            decode_read_response(&req, &frame).unwrap(),
            BlockData::Bits(vec![
                true, false, true, false, false, false, false, false, false, true
            ])
        );
    }
}
