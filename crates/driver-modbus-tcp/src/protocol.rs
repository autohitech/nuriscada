// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! MODBUS TCP 프레임의 부호화와 해석. I/O 없는 순수 함수다.
//!
//! 공개된 MODBUS 응용 프로토콜 규격(기능 코드 1~4)에서 출발했다.

use nuriscada_driver_api::{BlockData, DataKind, DriverError};

/// MBAP 머리의 길이(거래 식별자 2, 프로토콜 식별자 2, 길이 2, 유닛 1).
pub const MBAP_LEN: usize = 7;
/// 한 프레임에서 허용하는 PDU의 최대 길이.
pub const MAX_PDU_LEN: usize = 253;

/// 읽기 기능 코드.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadFunction {
    /// 01: 코일 읽기.
    Coils = 1,
    /// 02: 이산 입력 읽기.
    DiscreteInputs = 2,
    /// 03: 보유 레지스터 읽기.
    HoldingRegisters = 3,
    /// 04: 입력 레지스터 읽기.
    InputRegisters = 4,
}

impl ReadFunction {
    /// 코드에서 만든다.
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Coils),
            2 => Some(Self::DiscreteInputs),
            3 => Some(Self::HoldingRegisters),
            4 => Some(Self::InputRegisters),
            _ => None,
        }
    }

    /// 비트를 읽는 기능인지.
    pub fn is_bit(self) -> bool {
        matches!(self, Self::Coils | Self::DiscreteInputs)
    }

    /// 한 요청에서 읽을 수 있는 최대 개수.
    pub fn max_count(self) -> u16 {
        if self.is_bit() { 2000 } else { 125 }
    }

    /// 데이터 종류.
    pub fn kind(self) -> DataKind {
        if self.is_bit() {
            DataKind::Bit
        } else {
            DataKind::Word
        }
    }
}

/// 읽기 요청.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadRequest {
    /// 거래 식별자.
    pub transaction: u16,
    /// 유닛 식별자.
    pub unit: u8,
    /// 기능.
    pub function: ReadFunction,
    /// 시작 주소.
    pub start: u16,
    /// 개수.
    pub count: u16,
}

/// 요청을 프레임으로 만든다. 개수가 범위를 벗어나면 오류다.
pub fn encode_read_request(req: &ReadRequest) -> Result<Vec<u8>, DriverError> {
    if req.count == 0 || req.count > req.function.max_count() {
        return Err(DriverError::InvalidRequest(format!(
            "개수 {}는 1..={} 범위를 벗어난다",
            req.count,
            req.function.max_count()
        )));
    }
    let mut frame = Vec::with_capacity(MBAP_LEN + 5);
    frame.extend_from_slice(&req.transaction.to_be_bytes());
    frame.extend_from_slice(&0u16.to_be_bytes()); // 프로토콜 식별자
    frame.extend_from_slice(&6u16.to_be_bytes()); // 유닛 + PDU(5)
    frame.push(req.unit);
    frame.push(req.function as u8);
    frame.extend_from_slice(&req.start.to_be_bytes());
    frame.extend_from_slice(&req.count.to_be_bytes());
    Ok(frame)
}

/// 머리에서 뒤따르는 바이트 수(유닛 식별자 이후)를 읽는다.
///
/// 길이 필드는 유닛 식별자부터 끝까지의 바이트 수이므로, 머리를 읽은 뒤 더 읽을 양은 `길이 - 1`이다.
pub fn remaining_after_header(header: &[u8; MBAP_LEN]) -> Result<usize, DriverError> {
    let proto = u16::from_be_bytes([header[2], header[3]]);
    if proto != 0 {
        return Err(DriverError::Protocol(format!(
            "프로토콜 식별자가 0이 아니다: {proto}"
        )));
    }
    let len = usize::from(u16::from_be_bytes([header[4], header[5]]));
    if !(2..=MAX_PDU_LEN + 1).contains(&len) {
        return Err(DriverError::Protocol(format!(
            "길이 필드가 이상하다: {len}"
        )));
    }
    Ok(len - 1)
}

/// 오류 코드의 설명.
pub fn exception_text(code: u8) -> &'static str {
    match code {
        1 => "지원하지 않는 기능",
        2 => "잘못된 데이터 주소",
        3 => "잘못된 데이터 값",
        4 => "장치 고장",
        5 => "확인(처리 중)",
        6 => "장치 사용 중",
        8 => "메모리 패리티 오류",
        10 => "게이트웨이 경로 없음",
        11 => "게이트웨이 대상 장치 응답 없음",
        _ => "알 수 없는 오류",
    }
}

/// 응답 프레임 전체(머리 포함)를 해석한다.
pub fn decode_read_response(req: &ReadRequest, frame: &[u8]) -> Result<BlockData, DriverError> {
    if frame.len() < MBAP_LEN + 2 {
        return Err(DriverError::Protocol("응답이 너무 짧다".into()));
    }
    let transaction = u16::from_be_bytes([frame[0], frame[1]]);
    if transaction != req.transaction {
        return Err(DriverError::Protocol(format!(
            "거래 식별자가 다르다: 요청 {}, 응답 {transaction}",
            req.transaction
        )));
    }
    let header: [u8; MBAP_LEN] = frame[..MBAP_LEN].try_into().expect("길이 검사");
    let rest = remaining_after_header(&header)?;
    if frame.len() != MBAP_LEN + rest {
        return Err(DriverError::Protocol(format!(
            "프레임 길이가 길이 필드와 다르다: {} != {}",
            frame.len(),
            MBAP_LEN + rest
        )));
    }
    if frame[6] != req.unit {
        return Err(DriverError::Protocol(format!(
            "유닛 식별자가 다르다: 요청 {}, 응답 {}",
            req.unit, frame[6]
        )));
    }
    let function = frame[7];
    if function == req.function as u8 | 0x80 {
        let code = *frame
            .get(8)
            .ok_or_else(|| DriverError::Protocol("오류 응답에 코드가 없다".into()))?;
        return Err(DriverError::Device {
            code,
            text: exception_text(code).to_string(),
        });
    }
    if function != req.function as u8 {
        return Err(DriverError::Protocol(format!(
            "기능 코드가 다르다: 요청 {}, 응답 {function}",
            req.function as u8
        )));
    }
    let byte_count = usize::from(frame[8]);
    let data = &frame[9..];
    if data.len() != byte_count {
        return Err(DriverError::Protocol(format!(
            "바이트 수 필드 {byte_count}와 실제 데이터 {}가 다르다",
            data.len()
        )));
    }
    let count = usize::from(req.count);
    if req.function.is_bit() {
        if byte_count != count.div_ceil(8) {
            return Err(DriverError::Protocol(format!(
                "비트 {count}개에 바이트 {byte_count}개는 맞지 않는다"
            )));
        }
        let bits = (0..count)
            .map(|i| (data[i / 8] >> (i % 8)) & 1 == 1)
            .collect();
        Ok(BlockData::Bits(bits))
    } else {
        if byte_count != count * 2 {
            return Err(DriverError::Protocol(format!(
                "워드 {count}개에 바이트 {byte_count}개는 맞지 않는다"
            )));
        }
        let words = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        Ok(BlockData::Words(words))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(function: ReadFunction, start: u16, count: u16) -> ReadRequest {
        ReadRequest {
            transaction: 1,
            unit: 1,
            function,
            start,
            count,
        }
    }

    #[test]
    fn encodes_spec_example_request() {
        // 규격 예: 보유 레지스터 108(0x6B)부터 3개.
        let frame = encode_read_request(&req(ReadFunction::HoldingRegisters, 0x6B, 3)).unwrap();
        assert_eq!(
            frame,
            [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x6B, 0x00, 0x03
            ]
        );
    }

    #[test]
    fn decodes_registers() {
        let r = req(ReadFunction::HoldingRegisters, 0x6B, 3);
        let frame = [
            0x00, 0x01, 0x00, 0x00, 0x00, 0x09, 0x01, 0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00,
            0x64,
        ];
        assert_eq!(
            decode_read_response(&r, &frame).unwrap(),
            BlockData::Words(vec![0x022B, 0x0000, 0x0064])
        );
    }

    #[test]
    fn decodes_coils_lsb_first() {
        // 규격 예: 코일 20~38(19개), 데이터 CD 6B 05.
        let r = req(ReadFunction::Coils, 20, 19);
        let frame = [
            0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x01, 0x01, 0x03, 0xCD, 0x6B, 0x05,
        ];
        let BlockData::Bits(bits) = decode_read_response(&r, &frame).unwrap() else {
            panic!("비트여야 한다");
        };
        assert_eq!(bits.len(), 19);
        // 0xCD = 1100_1101 → 낮은 비트부터: 1,0,1,1,0,0,1,1
        assert_eq!(
            &bits[..8],
            &[true, false, true, true, false, false, true, true]
        );
        // 0x05 = 101 → 17번째부터 1,0,1
        assert_eq!(&bits[16..], &[true, false, true]);
    }

    #[test]
    fn exception_response_becomes_device_error() {
        let r = req(ReadFunction::HoldingRegisters, 0, 1);
        let frame = [0x00, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x83, 0x02];
        assert_eq!(
            decode_read_response(&r, &frame).unwrap_err(),
            DriverError::Device {
                code: 2,
                text: "잘못된 데이터 주소".into()
            }
        );
    }

    #[test]
    fn rejects_mismatched_responses() {
        let r = req(ReadFunction::HoldingRegisters, 0, 1);
        let base = [
            0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x01, 0x03, 0x02, 0x00, 0x07,
        ];
        assert!(decode_read_response(&r, &base).is_ok());
        let mut bad = base;
        bad[1] = 9; // 거래 식별자
        assert!(decode_read_response(&r, &bad).is_err());
        let mut bad = base;
        bad[6] = 2; // 유닛
        assert!(decode_read_response(&r, &bad).is_err());
        let mut bad = base;
        bad[7] = 4; // 기능
        assert!(decode_read_response(&r, &bad).is_err());
        let mut bad = base;
        bad[3] = 1; // 프로토콜 식별자
        assert!(decode_read_response(&r, &bad).is_err());
        let mut bad = base;
        bad[8] = 4; // 바이트 수
        assert!(decode_read_response(&r, &bad).is_err());
        assert!(decode_read_response(&r, &base[..9]).is_err());
    }

    #[test]
    fn rejects_out_of_range_counts() {
        assert!(encode_read_request(&req(ReadFunction::HoldingRegisters, 0, 0)).is_err());
        assert!(encode_read_request(&req(ReadFunction::HoldingRegisters, 0, 126)).is_err());
        assert!(encode_read_request(&req(ReadFunction::HoldingRegisters, 0, 125)).is_ok());
        assert!(encode_read_request(&req(ReadFunction::Coils, 0, 2001)).is_err());
        assert!(encode_read_request(&req(ReadFunction::Coils, 0, 2000)).is_ok());
    }

    #[test]
    fn header_length_bounds() {
        let ok = [0, 1, 0, 0, 0, 6, 1];
        assert_eq!(remaining_after_header(&ok).unwrap(), 5);
        assert!(remaining_after_header(&[0, 1, 0, 0, 0, 1, 1]).is_err());
        assert!(remaining_after_header(&[0, 1, 0, 0, 0xFF, 0xFF, 1]).is_err());
    }
}
