// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! MELSEC MC 프로토콜(3E 바이너리) 클라이언트. 일괄 읽기·쓰기를 지원한다.

use std::time::{Duration, Instant};

use nuriscada_driver_api::{BlockData, Driver, DriverError, LinkState, LinkStatus, ReadBlock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::protocol::{
    RESPONSE_HEADER_LEN, ReadRequest, WriteRequest, decode_read_response, decode_write_response,
    encode_read_request, encode_write_request, remaining_after_header,
};

/// D 레지스터 영역 번호.
pub const SPACE_D: u16 = 10;
/// W 링크 레지스터 영역 번호.
pub const SPACE_W: u16 = 11;
/// R 파일 레지스터 영역 번호.
pub const SPACE_R: u16 = 12;
/// M 내부 릴레이 영역 번호.
pub const SPACE_M: u16 = 20;
/// X 입력 영역 번호(16진 주소).
pub const SPACE_X: u16 = 21;
/// Y 출력 영역 번호(16진 주소).
pub const SPACE_Y: u16 = 22;

/// 영역 번호에서 장치 코드를 찾는다.
fn code_for(space: u16) -> Option<(u8, bool)> {
    // (장치 코드, 워드인가)
    match space {
        SPACE_D => Some((0xA8, true)),
        SPACE_W => Some((0xB4, true)),
        SPACE_R => Some((0xAF, true)),
        SPACE_M => Some((0x90, false)),
        SPACE_X => Some((0x9C, false)),
        SPACE_Y => Some((0x9D, false)),
        _ => None,
    }
}

/// MELSEC MC 클라이언트. 연결이 끊어지면 다음 읽기에서 다시 연결을 시도한다.
///
/// MC 프레임에는 국번이 없으므로 `ReadBlock.unit`은 무시한다.
#[derive(Debug)]
pub struct MelsecMcClient {
    address: String,
    timeout: Duration,
    stream: Option<TcpStream>,
    status: LinkStatus,
}

impl MelsecMcClient {
    /// 클라이언트를 만든다. `timeout`은 연결과 요청 한 번(송신과 수신)의 제한 시간이다.
    pub fn new(address: impl Into<String>, timeout: Duration) -> Self {
        Self {
            address: address.into(),
            timeout,
            stream: None,
            status: LinkStatus::initial(),
        }
    }

    fn drop_connection(&mut self) {
        self.stream = None;
        self.status.state = LinkState::Disconnected;
    }

    async fn exchange(
        stream: &mut TcpStream,
        request: &[u8],
        req: &ReadRequest,
    ) -> Result<BlockData, DriverError> {
        stream
            .write_all(request)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let mut header = [0u8; RESPONSE_HEADER_LEN];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let rest = remaining_after_header(&header)?;
        let mut frame = vec![0u8; RESPONSE_HEADER_LEN + rest];
        frame[..RESPONSE_HEADER_LEN].copy_from_slice(&header);
        stream
            .read_exact(&mut frame[RESPONSE_HEADER_LEN..])
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        decode_read_response(req, &frame)
    }

    async fn exchange_write(stream: &mut TcpStream, request: &[u8]) -> Result<(), DriverError> {
        stream
            .write_all(request)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let mut header = [0u8; RESPONSE_HEADER_LEN];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let rest = remaining_after_header(&header)?;
        let mut frame = vec![0u8; RESPONSE_HEADER_LEN + rest];
        frame[..RESPONSE_HEADER_LEN].copy_from_slice(&header);
        stream
            .read_exact(&mut frame[RESPONSE_HEADER_LEN..])
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        decode_write_response(&frame)
    }

    /// 단일 워드(D·W·R) 또는 비트(M·X·Y)를 쓴다. `unit`은 무시한다(MC 프레임에 국번이 없다).
    /// 불명확한 결과를 자동 재시도하지 않는다.
    pub async fn write_single(
        &mut self,
        _unit: u8,
        space: u16,
        address: u16,
        value: u16,
    ) -> Result<(), DriverError> {
        let req = match space {
            SPACE_D | SPACE_W | SPACE_R => WriteRequest::Words {
                code: code_for(space).expect("읽기 영역 번호").0,
                head: u32::from(address),
                values: vec![value],
            },
            SPACE_M | SPACE_X | SPACE_Y => WriteRequest::Bits {
                code: code_for(space).expect("읽기 영역 번호").0,
                head: u32::from(address),
                values: vec![value != 0],
            },
            _ => {
                return Err(DriverError::InvalidRequest(format!(
                    "쓰기는 D·W·R·M·X·Y 영역만 지원한다: {space}"
                )));
            }
        };
        let frame = encode_write_request(&req)?;

        let started = Instant::now();
        let result = async {
            self.connect().await?;
            let stream = self.stream.as_mut().expect("방금 연결했다");
            timeout(self.timeout, Self::exchange_write(stream, &frame))
                .await
                .map_err(|_| DriverError::Timeout)?
        }
        .await;

        match &result {
            Ok(()) => {
                self.status.requests_ok += 1;
                self.status.last_round_trip = Some(started.elapsed());
            }
            Err(e) => {
                self.status.requests_failed += 1;
                // 장치가 오류를 응답한 경우 연결은 정상이다. 그 밖의 오류는 연결을 버린다.
                if !matches!(
                    e,
                    DriverError::Device { .. } | DriverError::InvalidRequest(_)
                ) {
                    self.drop_connection();
                }
            }
        }
        result
    }

    /// 다중 워드(D·W·R) 또는 비트(M·X·Y)를 한 번에 쓴다. `unit`은 무시한다.
    /// 불명확한 결과를 자동 재시도하지 않는다.
    pub async fn write_multiple(
        &mut self,
        _unit: u8,
        space: u16,
        address: u16,
        words: &[u16],
        bits: &[bool],
    ) -> Result<(), DriverError> {
        let req = match space {
            SPACE_D | SPACE_W | SPACE_R => WriteRequest::Words {
                code: code_for(space).expect("읽기 영역 번호").0,
                head: u32::from(address),
                values: words.to_vec(),
            },
            SPACE_M | SPACE_X | SPACE_Y => WriteRequest::Bits {
                code: code_for(space).expect("읽기 영역 번호").0,
                head: u32::from(address),
                values: bits.to_vec(),
            },
            _ => {
                return Err(DriverError::InvalidRequest(format!(
                    "다중 쓰기는 D·W·R·M·X·Y 영역만 지원한다: {space}"
                )));
            }
        };
        let frame = encode_write_request(&req)?;

        let started = Instant::now();
        let result = async {
            self.connect().await?;
            let stream = self.stream.as_mut().expect("방금 연결했다");
            timeout(self.timeout, Self::exchange_write(stream, &frame))
                .await
                .map_err(|_| DriverError::Timeout)?
        }
        .await;

        match &result {
            Ok(()) => {
                self.status.requests_ok += 1;
                self.status.last_round_trip = Some(started.elapsed());
            }
            Err(e) => {
                self.status.requests_failed += 1;
                // 장치가 오류를 응답한 경우 연결은 정상이다. 그 밖의 오류는 연결을 버린다.
                if !matches!(
                    e,
                    DriverError::Device { .. } | DriverError::InvalidRequest(_)
                ) {
                    self.drop_connection();
                }
            }
        }
        result
    }
}

impl Driver for MelsecMcClient {
    async fn connect(&mut self) -> Result<(), DriverError> {
        if self.stream.is_some() {
            return Ok(());
        }
        let connected = timeout(self.timeout, TcpStream::connect(&self.address))
            .await
            .map_err(|_| DriverError::Timeout)?
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        // 요청이 작으므로 지연 없이 보낸다.
        connected
            .set_nodelay(true)
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        self.stream = Some(connected);
        self.status.state = LinkState::Connected;
        Ok(())
    }

    async fn read(&mut self, block: &ReadBlock) -> Result<BlockData, DriverError> {
        let (code, is_word) = code_for(block.space).ok_or_else(|| {
            DriverError::InvalidRequest(format!("지원하지 않는 영역 번호: {}", block.space))
        })?;
        let want_word = block.kind == nuriscada_driver_api::DataKind::Word;
        if is_word != want_word {
            return Err(DriverError::InvalidRequest(
                "영역과 데이터 종류가 맞지 않는다".into(),
            ));
        }
        let req = if is_word {
            ReadRequest::Words {
                code,
                head: u32::from(block.start),
                count: block.count,
            }
        } else {
            ReadRequest::Bits {
                code,
                head: u32::from(block.start),
                count: block.count,
            }
        };
        let frame = encode_read_request(&req)?;

        let started = Instant::now();
        let result = async {
            self.connect().await?;
            let stream = self.stream.as_mut().expect("방금 연결했다");
            timeout(self.timeout, Self::exchange(stream, &frame, &req))
                .await
                .map_err(|_| DriverError::Timeout)?
        }
        .await;

        match &result {
            Ok(_) => {
                self.status.requests_ok += 1;
                self.status.last_round_trip = Some(started.elapsed());
            }
            Err(e) => {
                self.status.requests_failed += 1;
                // 장치가 오류를 응답한 경우 연결은 정상이다. 그 밖의 오류는 연결을 버린다.
                if !matches!(
                    e,
                    DriverError::Device { .. } | DriverError::InvalidRequest(_)
                ) {
                    self.drop_connection();
                }
            }
        }
        result
    }

    fn status(&self) -> LinkStatus {
        self.status
    }
}
