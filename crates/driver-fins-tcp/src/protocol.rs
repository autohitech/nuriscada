// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! FINS/TCP 프레임의 부호화와 해석. I/O 없는 순수 함수다.
//!
//! 공개된 FINS/TCP 절차에서 출발했다: 노드 주소 교환(명령 0/1)과
//! 메모리 영역 읽기(0101, DM 워드·CIO 비트). 모든 다바이트 정수는 빅엔디안이다.

use nuriscada_driver_api::{BlockData, DataKind, DriverError};

/// FINS/TCP 매직("FINS").
pub const MAGIC: [u8; 4] = [0x46, 0x49, 0x4E, 0x53];
/// TCP 머리 길이(매직 4 + 길이 4 + 명령 4 + 오류 4).
pub const TCP_HEADER_LEN: usize = 16;
/// 노드 주소 교환 요청(명령 0).
pub const CMD_NODE_REQ: u32 = 0;
/// 노드 주소 교환 응답(명령 1).
pub const CMD_NODE_RES: u32 = 1;
/// FINS 프레임 전송(명령 2).
pub const CMD_FINS: u32 = 2;
/// DM 워드 영역 코드.
pub const AREA_DM_WORD: u8 = 0x82;
/// CIO 비트 영역 코드.
pub const AREA_CIO_BIT: u8 = 0x30;
/// 메모리 영역 읽기(MRC 01 SRC 01).
pub const MRC_MEMORY: u8 = 0x01;
/// 읽기(SRC 01).
pub const SRC_READ: u8 = 0x01;
/// 한 요청의 보수적 한도(워드 128, 비트 128).
pub const MAX_WORD_ITEMS: u16 = 128;
/// 비트 읽기의 보수적 한도.
pub const MAX_BIT_ITEMS: u16 = 128;

/// 노드 주소 교환 요청(20바이트, 클라이언트 노드 0 = 자동 할당).
pub fn encode_node_request() -> Vec<u8> {
    let mut out = Vec::with_capacity(20);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&12u32.to_be_bytes());
    out.extend_from_slice(&CMD_NODE_REQ.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out
}
/// 노드 주소 교환 응답(24바이트)을 해석해 (클라이언트 노드, 서버 노드)를 돌려준다.
pub fn decode_node_response(frame: &[u8]) -> Result<(u8, u8), DriverError> {
    if frame.len() != 24 {
        return Err(DriverError::Protocol(format!(
            "노드 응답 길이가 24가 아니다: {}",
            frame.len()
        )));
    }
    if frame[..4] != MAGIC {
        return Err(DriverError::Protocol("매직이 FINS가 아니다".into()));
    }
    let cmd = u32::from_be_bytes(frame[8..12].try_into().expect("4바이트"));
    if cmd != CMD_NODE_RES {
        return Err(DriverError::Protocol(format!(
            "노드 응답 명령이 1이 아니다: {cmd}"
        )));
    }
    let error = u32::from_be_bytes(frame[12..16].try_into().expect("4바이트"));
    if error != 0 {
        return Err(DriverError::Connection(format!(
            "노드 주소 교환 실패: {error:#010X}"
        )));
    }
    Ok((frame[19], frame[23]))
}

/// 읽기 요청.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRequest {
    /// DM 워드 읽기.
    DmWords {
        /// 시작 주소.
        start: u16,
        /// 읽을 워드 수.
        count: u16,
    },
    /// CIO 비트 읽기.
    CioBits {
        /// 시작 비트 번호(워드×16 + 비트).
        start_bit: u32,
        /// 읽을 비트 수.
        count: u16,
    },
}

impl ReadRequest {
    /// 읽기 종류.
    pub fn kind(self) -> DataKind {
        match self {
            Self::DmWords { .. } => DataKind::Word,
            Self::CioBits { .. } => DataKind::Bit,
        }
    }
}

/// FINS 명령 프레임(머리 16바이트 뒤)을 만든다. 주소 필드는 워드 2바이트 + 비트 1바이트다.
fn fins_frame(
    client: u8,
    server: u8,
    sid: u8,
    area: u8,
    word: u16,
    bit: u8,
    count: u16,
) -> Vec<u8> {
    let mut f = vec![
        0x80,
        0x00,
        0x02,
        0x00,
        server,
        0x00,
        0x00,
        client,
        0x00,
        sid,
        MRC_MEMORY,
        SRC_READ,
        area,
        (word >> 8) as u8,
        (word & 0xFF) as u8,
        bit,
    ];
    f.extend_from_slice(&count.to_be_bytes());
    f
}

/// 요청 전체(TCP 머리 16바이트 + FINS 프레임)를 만든다.
pub fn encode_read_request(
    client: u8,
    server: u8,
    sid: u8,
    req: &ReadRequest,
) -> Result<Vec<u8>, DriverError> {
    let inner = match *req {
        ReadRequest::DmWords { start, count } => {
            if count == 0 || count > MAX_WORD_ITEMS {
                return Err(DriverError::InvalidRequest(format!(
                    "워드 읽기 개수 {count}는 1~{MAX_WORD_ITEMS}이어야 한다"
                )));
            }
            fins_frame(client, server, sid, AREA_DM_WORD, start, 0x00, count)
        }
        ReadRequest::CioBits { start_bit, count } => {
            if count == 0 || count > MAX_BIT_ITEMS {
                return Err(DriverError::InvalidRequest(format!(
                    "비트 읽기 개수 {count}는 1~{MAX_BIT_ITEMS}이어야 한다"
                )));
            }
            // 비트 번호 = 워드×16 + 비트. 주소 필드: 워드 2바이트 + 비트 1바이트.
            let word = start_bit / 16;
            let bit = (start_bit % 16) as u8;
            if word > 0xFFFF {
                return Err(DriverError::InvalidRequest(format!(
                    "비트 번호 {start_bit}가 범위를 벗어났다"
                )));
            }
            fins_frame(client, server, sid, AREA_CIO_BIT, word as u16, bit, count)
        }
    };
    let mut out = Vec::with_capacity(TCP_HEADER_LEN + inner.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&(8u32 + inner.len() as u32).to_be_bytes());
    out.extend_from_slice(&CMD_FINS.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&inner);
    Ok(out)
}

/// TCP 머리에서 뒤따를 바이트 수를 읽는다.
pub fn remaining_after_header(header: &[u8]) -> Result<usize, DriverError> {
    if header.len() != TCP_HEADER_LEN {
        return Err(DriverError::Protocol(format!(
            "TCP 머리가 {TCP_HEADER_LEN}바이트가 아니다: {}",
            header.len()
        )));
    }
    if header[..4] != MAGIC {
        return Err(DriverError::Protocol("매직이 FINS가 아니다".into()));
    }
    let cmd = u32::from_be_bytes(header[8..12].try_into().expect("4바이트"));
    if cmd != CMD_FINS {
        return Err(DriverError::Protocol(format!(
            "FINS 전송 명령(2)이 아니다: {cmd}"
        )));
    }
    Ok(u32::from_be_bytes(header[4..8].try_into().expect("4바이트")) as usize - 8)
}

/// 종료 코드의 설명.
pub fn end_code_text(code: u16) -> String {
    match code {
        0x0000 => "정상".into(),
        0x0104 => "0104: 주소가 범위를 벗어났다".into(),
        0x0203 => "0203: 개수가 범위를 벗어났다".into(),
        0x0401 => "0401: 명령이 잘못되었다".into(),
        0x110C => "110C: 비트 접근을 지원하지 않는 영역이다".into(),
        other => format!("종료 코드 {other:04X}"),
    }
}

/// 응답 프레임 전체(TCP 머리 포함)를 해석한다.
pub fn decode_read_response(req: &ReadRequest, frame: &[u8]) -> Result<BlockData, DriverError> {
    if frame.len() < TCP_HEADER_LEN + 14 {
        return Err(DriverError::Protocol(format!(
            "응답이 너무 짧다: {}",
            frame.len()
        )));
    }
    let (header, mut body) = frame.split_at(TCP_HEADER_LEN);
    if header[..4] != MAGIC {
        return Err(DriverError::Protocol("매직이 FINS가 아니다".into()));
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
    let icf = take(&mut body, 1)?[0];
    if icf != 0xC0 {
        return Err(DriverError::Protocol(format!(
            "응답 ICF가 0xC0이 아니다: {icf:02X}"
        )));
    }
    let _echo = take(&mut body, 9)?;
    let mrc = take(&mut body, 1)?[0];
    let src = take(&mut body, 1)?[0];
    if mrc != MRC_MEMORY || src != SRC_READ {
        return Err(DriverError::Protocol(format!(
            "응답 명령이 0101이 아니다: {mrc:02X}{src:02X}"
        )));
    }
    // 오류 응답은 데이터 없이 종료 코드만 있다(에코 10 + 명령 2 + 종료 2).
    if body.len() == 2 {
        let end = u16::from_be_bytes([body[0], body[1]]);
        if end != 0 {
            return Err(DriverError::Device {
                code: (end & 0xFF) as u8,
                text: format!("FINS {}", end_code_text(end)),
            });
        }
        return Err(DriverError::Protocol("응답에 데이터가 없다".into()));
    }
    match *req {
        ReadRequest::DmWords { count, .. } => {
            let mut out = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let bytes = take(&mut body, 2)?;
                out.push(u16::from_be_bytes([bytes[0], bytes[1]]));
            }
            let end = u16::from_be_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
            if end != 0 {
                return Err(DriverError::Device {
                    code: (end & 0xFF) as u8,
                    text: format!("FINS {}", end_code_text(end)),
                });
            }
            Ok(BlockData::Words(out))
        }
        ReadRequest::CioBits { count, .. } => {
            let mut out = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let bytes = take(&mut body, 2)?;
                out.push(u16::from_be_bytes([bytes[0], bytes[1]]) != 0);
            }
            let end = u16::from_be_bytes(take(&mut body, 2)?[..].try_into().expect("2바이트"));
            if end != 0 {
                return Err(DriverError::Device {
                    code: (end & 0xFF) as u8,
                    text: format!("FINS {}", end_code_text(end)),
                });
            }
            Ok(BlockData::Bits(out))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 노드_교환_프레임() {
        assert_eq!(
            encode_node_request(),
            vec![
                0x46, 0x49, 0x4E, 0x53, 0x00, 0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ]
        );
        let res = vec![
            0x46, 0x49, 0x4E, 0x53, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0xFB, 0x00, 0x00, 0x00, 0x0F,
        ];
        let (client, server) = decode_node_response(&res).unwrap();
        assert_eq!(client, 0xFB);
        assert_eq!(server, 0x0F);
        // 오류 응답은 연결 오류다.
        let mut bad = res.clone();
        bad[15] = 0x03;
        assert!(matches!(
            decode_node_response(&bad),
            Err(DriverError::Connection(_))
        ));
    }

    #[test]
    fn dm_읽기_요청_인코딩_매뉴얼_예시() {
        // 매뉴얼의 D20 1워드 읽기 예시(클라이언트 0xFB, 서버 0x0F, SID 1).
        // 길이 = 명령 4 + 오류 4 + FINS 프레임 18 = 26(헤더 정의·노드 예시와 일치).
        let req = ReadRequest::DmWords {
            start: 20,
            count: 1,
        };
        let frame = encode_read_request(0xFB, 0x0F, 1, &req).unwrap();
        assert_eq!(
            frame,
            vec![
                0x46, 0x49, 0x4E, 0x53, // 매직
                0x00, 0x00, 0x00, 0x1A, // 길이 26
                0x00, 0x00, 0x00, 0x02, // 명령 2
                0x00, 0x00, 0x00, 0x00, // 오류 0
                0x80, 0x00, 0x02, 0x00, 0x0F, 0x00, 0x00, 0xFB, 0x00, 0x01, // 머리+SID
                0x01, 0x01, // 0101
                0x82, // DM 워드
                0x00, 0x14, 0x00, // D20
                0x00, 0x01, // 1개
            ]
        );
    }

    #[test]
    fn cio_비트_읽기_요청_인코딩() {
        // CIO 2781.10: 워드 0x0ADD, 비트 0x0A.
        let req = ReadRequest::CioBits {
            start_bit: 2781 * 16 + 10,
            count: 1,
        };
        let frame = encode_read_request(0xFB, 0x0F, 1, &req).unwrap();
        assert_eq!(frame[28..32], [0x30, 0x0A, 0xDD, 0x0A]);
        assert_eq!(frame[32..34], [0x00, 0x01]);
    }

    #[test]
    fn 개수_0은_거부한다() {
        let req = ReadRequest::DmWords { start: 0, count: 0 };
        assert!(matches!(
            encode_read_request(0xFB, 0x0F, 1, &req),
            Err(DriverError::InvalidRequest(_))
        ));
    }

    #[test]
    fn dm_응답_해석() {
        let req = ReadRequest::DmWords {
            start: 20,
            count: 2,
        };
        let frame = vec![
            0x46, 0x49, 0x4E, 0x53, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x00, 0xC0, 0x00, 0x02, 0x00, 0x0F, 0x00, 0x00, 0xFB, 0x00, 0x01, 0x01, 0x01,
            0x12, 0x34, 0x56, 0x78, 0x00, 0x00,
        ];
        assert_eq!(
            decode_read_response(&req, &frame).unwrap(),
            BlockData::Words(vec![0x1234, 0x5678])
        );
        let mut bad = frame.clone();
        let last = bad.len() - 2;
        bad[last] = 0x01;
        bad[last + 1] = 0x04;
        let err = decode_read_response(&req, &bad).unwrap_err();
        assert!(matches!(err, DriverError::Device { .. }));
        assert!(err.to_string().contains("0104"));
    }

    #[test]
    fn cio_비트_응답_해석() {
        let req = ReadRequest::CioBits {
            start_bit: 16,
            count: 2,
        };
        let frame = vec![
            0x46, 0x49, 0x4E, 0x53, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x00, 0xC0, 0x00, 0x02, 0x00, 0x0F, 0x00, 0x00, 0xFB, 0x00, 0x01, 0x01, 0x01,
            0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(
            decode_read_response(&req, &frame).unwrap(),
            BlockData::Bits(vec![true, false])
        );
    }
}
