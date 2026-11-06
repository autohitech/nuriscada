// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! MODBUS RTU·ASCII 프레임 부호화와 검사. I/O 없는 순수 함수다.
//!
//! 전송(시리얼 포트)은 아직 없으므로 실제 장치와 주고받지 않는다.
//! 규격의 프레임 배치와 검사합(CRC-16/Modbus, LRC)에서 출발했다.

use nuriscada_driver_api::DriverError;

use super::protocol::exception_text;

/// CRC-16/Modbus (다항식 0xA001, 초기값 0xFFFF, 하위 바이트 먼저).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

/// LRC (바이트 합을 256으로 나눈 나머지의 2의 보수).
pub fn lrc(data: &[u8]) -> u8 {
    data.iter()
        .fold(0u8, |acc, &b| acc.wrapping_add(b))
        .wrapping_neg()
}

/// RTU 요청 ADU를 만든다: 유닛 + PDU + CRC(하위 바이트 먼저).
pub fn encode_rtu_request(unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut addr_and_pdu = Vec::with_capacity(1 + pdu.len());
    addr_and_pdu.push(unit);
    addr_and_pdu.extend_from_slice(pdu);
    let mut out = addr_and_pdu.clone();
    out.extend_from_slice(&crc16(&addr_and_pdu).to_le_bytes());
    out
}

/// RTU 응답 ADU를 검사한다. 유닛·기능·예상 PDU 길이가 맞고 CRC가 맞아야 한다.
/// 예외 응답이면 장치 오류로 돌려준다.
pub fn decode_rtu_response(
    request_unit: u8,
    request_function: u8,
    expected_pdu_len: usize,
    frame: &[u8],
) -> Result<Vec<u8>, DriverError> {
    if frame.len() < 4 {
        return Err(DriverError::Protocol(format!(
            "RTU 응답이 너무 짧다: {}",
            frame.len()
        )));
    }
    let (body, crc_bytes) = frame.split_at(frame.len() - 2);
    let received = u16::from_le_bytes([crc_bytes[0], crc_bytes[1]]);
    if crc16(body) != received {
        return Err(DriverError::Protocol("RTU CRC가 맞지 않다".into()));
    }
    if body[0] != request_unit {
        return Err(DriverError::Protocol(format!(
            "유닛 식별자가 다르다: 요청 {request_unit:#04X}, 응답 {:#04X}",
            body[0]
        )));
    }
    let function = body[1];
    if function == request_function | 0x80 {
        let code = *body
            .get(2)
            .ok_or_else(|| DriverError::Protocol("오류 응답에 코드가 없다".into()))?;
        return Err(DriverError::Device {
            code,
            text: exception_text(code).to_string(),
        });
    }
    if function != request_function {
        return Err(DriverError::Protocol(format!(
            "기능 코드가 다르다: 요청 {request_function:#04X}, 응답 {function:#04X}"
        )));
    }
    let pdu = &body[1..];
    if pdu.len() != expected_pdu_len {
        return Err(DriverError::Protocol(format!(
            "RTU PDU 길이 {}가 예상 {}과 다르다",
            pdu.len(),
            expected_pdu_len
        )));
    }
    Ok(pdu.to_vec())
}

/// ASCII 요청 문자열을 만든다: `:` + 16진수 + LRC 16진수 + CRLF.
pub fn encode_ascii_request(unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(1 + pdu.len());
    raw.push(unit);
    raw.extend_from_slice(pdu);
    raw.push(lrc(&raw));
    let mut out = Vec::with_capacity(1 + raw.len() * 2 + 2);
    out.push(b':');
    for b in raw {
        out.extend_from_slice(format!("{b:02X}").as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// ASCII 응답 문자열을 검사한다. 16진수·LRC·유닛·기능·PDU 길이를 본다.
pub fn decode_ascii_response(
    request_unit: u8,
    request_function: u8,
    expected_pdu_len: usize,
    frame: &[u8],
) -> Result<Vec<u8>, DriverError> {
    if frame.len() < 9 || frame[0] != b':' || frame[frame.len() - 2..] != *b"\r\n" {
        return Err(DriverError::Protocol("ASCII 프레임 모양이 아니다".into()));
    }
    let hex = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    let body = &frame[1..frame.len() - 2];
    if !body.len().is_multiple_of(2) {
        return Err(DriverError::Protocol("ASCII 16진수 길이가 홀수다".into()));
    }
    let mut raw = Vec::with_capacity(body.len() / 2);
    for pair in body.as_chunks::<2>().0 {
        let (Some(hi), Some(lo)) = (hex(pair[0]), hex(pair[1])) else {
            return Err(DriverError::Protocol("ASCII 16진수가 아니다".into()));
        };
        raw.push(hi << 4 | lo);
    }
    if raw.len() < 3 {
        return Err(DriverError::Protocol("ASCII 응답이 너무 짧다".into()));
    }
    let (data, [lrc_byte]) = raw.split_at(raw.len() - 1) else {
        return Err(DriverError::Protocol("ASCII 응답이 너무 짧다".into()));
    };
    if lrc(data) != *lrc_byte {
        return Err(DriverError::Protocol("ASCII LRC가 맞지 않다".into()));
    }
    if data[0] != request_unit {
        return Err(DriverError::Protocol(format!(
            "유닛 식별자가 다르다: 요청 {request_unit:#04X}, 응답 {:#04X}",
            data[0]
        )));
    }
    let function = data[1];
    if function == request_function | 0x80 {
        let code = *data
            .get(2)
            .ok_or_else(|| DriverError::Protocol("오류 응답에 코드가 없다".into()))?;
        return Err(DriverError::Device {
            code,
            text: exception_text(code).to_string(),
        });
    }
    if function != request_function {
        return Err(DriverError::Protocol(format!(
            "기능 코드가 다르다: 요청 {request_function:#04X}, 응답 {function:#04X}"
        )));
    }
    let pdu = &data[1..];
    if pdu.len() != expected_pdu_len {
        return Err(DriverError::Protocol(format!(
            "ASCII PDU 길이 {}가 예상 {}과 다르다",
            pdu.len(),
            expected_pdu_len
        )));
    }
    Ok(pdu.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_검사값이_규격과_같다() {
        // CRC-16/Modbus 확인값: "123456789" -> 0x4B37.
        assert_eq!(crc16(b"123456789"), 0x4B37);
        assert_eq!(crc16(&[]), 0xFFFF);
    }

    #[test]
    fn lrc_합의_2의_보수를_낸다() {
        assert_eq!(lrc(&[0x01]), 0xFF);
        assert_eq!(lrc(&[0x01, 0x03, 0x00, 0x6B, 0x00, 0x03]), 0x8E);
    }

    #[test]
    fn rtu_읽기_요청이_규격_예시와_같다() {
        // Simply Modbus 예시: 슬레이브 0x11, 03 006B 0003 + CRC 7687.
        assert_eq!(
            encode_rtu_request(0x11, &[0x03, 0x00, 0x6B, 0x00, 0x03]),
            vec![0x11, 0x03, 0x00, 0x6B, 0x00, 0x03, 0x76, 0x87]
        );
    }

    #[test]
    fn rtu_응답을_검사하고_깨진_crc를_거부한다() {
        // 와트 2개 읽기 응답: 유닛 1, 기능 3, 바이트 수 4, 값 2개.
        let mut frame = vec![0x01, 0x03, 0x04, 0x02, 0x2B, 0x00, 0x00];
        let crc = crc16(&frame).to_le_bytes();
        frame.extend_from_slice(&crc);
        let pdu = decode_rtu_response(0x01, 0x03, 6, &frame).unwrap();
        assert_eq!(pdu, [0x03, 0x04, 0x02, 0x2B, 0x00, 0x00]);
        let mut bad = frame.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        assert!(decode_rtu_response(0x01, 0x03, 6, &bad).is_err());
        // 예외 응답은 장치 오류다.
        let mut err = vec![0x01, 0x83, 0x02];
        let crc = crc16(&err).to_le_bytes();
        err.extend_from_slice(&crc);
        assert!(matches!(
            decode_rtu_response(0x01, 0x03, 6, &err),
            Err(DriverError::Device { code: 2, .. })
        ));
    }

    #[test]
    fn ascii_요청이_모양을_갖추고_왕복한다() {
        let frame = encode_ascii_request(0x01, &[0x03, 0x00, 0x6B, 0x00, 0x03]);
        assert_eq!(&frame[..1], b":");
        assert_eq!(&frame[frame.len() - 2..], b"\r\n");
        // LRC 바이트는 CRLF 앞 2자리다.
        let lrc_hex = &frame[frame.len() - 4..frame.len() - 2];
        assert_eq!(lrc_hex, b"8E");
        // 같은 PDU의 인위 응답을 왕복한다.
        let resp = encode_ascii_request(0x01, &[0x03, 0x02, 0x00, 0x0A]);
        let pdu = decode_ascii_response(0x01, 0x03, 4, &resp).unwrap();
        assert_eq!(pdu, [0x03, 0x02, 0x00, 0x0A]);
        // LRC를 깨면 거부한다.
        let mut bad = resp.clone();
        let n = bad.len() - 3;
        bad[n] = if bad[n] == b'0' { b'1' } else { b'0' };
        assert!(decode_ascii_response(0x01, 0x03, 4, &bad).is_err());
    }
}
