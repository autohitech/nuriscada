// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! XGT 전용 프로토콜(FEnet) 개별 읽기 클라이언트.

use std::time::{Duration, Instant};

use nuriscada_driver_api::{BlockData, Driver, DriverError, LinkState, LinkStatus, ReadBlock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::protocol::{
    HEADER_LEN, ReadRequest, Variable, decode_read_response, encode_read_request,
    remaining_after_header,
};

/// %MW 워드 영역 번호.
pub const SPACE_MW: u16 = 30;
/// %MX 비트 영역 번호(10진 비트 번호).
pub const SPACE_MX: u16 = 31;

/// 영역 번호에서 변수 이름 머리를 찾는다.
fn prefix_for(space: u16) -> Option<(&'static str, bool)> {
    match space {
        SPACE_MW => Some(("MW", true)),
        SPACE_MX => Some(("MX", false)),
        _ => None,
    }
}

/// XGT FEnet 클라이언트. 연결이 끊어지면 다음 읽기에서 다시 연결을 시도한다.
///
/// FEnet 프레임에는 국번이 없으므로 `ReadBlock.unit`은 무시한다.
#[derive(Debug)]
pub struct XgtFenetClient {
    address: String,
    timeout: Duration,
    stream: Option<TcpStream>,
    invoke: u16,
    status: LinkStatus,
}

impl XgtFenetClient {
    /// 클라이언트를 만든다. `timeout`은 연결과 요청 한 번(송신과 수신)의 제한 시간이다.
    pub fn new(address: impl Into<String>, timeout: Duration) -> Self {
        Self {
            address: address.into(),
            timeout,
            stream: None,
            invoke: 0,
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
        let mut header = [0u8; HEADER_LEN];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let rest = remaining_after_header(&header)?;
        let mut frame = vec![0u8; HEADER_LEN + rest];
        frame[..HEADER_LEN].copy_from_slice(&header);
        stream
            .read_exact(&mut frame[HEADER_LEN..])
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        decode_read_response(req, &frame)
    }
}

impl Driver for XgtFenetClient {
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
        let (prefix, is_word) = prefix_for(block.space).ok_or_else(|| {
            DriverError::InvalidRequest(format!("지원하지 않는 영역 번호: {}", block.space))
        })?;
        let want_word = block.kind == nuriscada_driver_api::DataKind::Word;
        if is_word != want_word {
            return Err(DriverError::InvalidRequest(
                "영역과 데이터 종류가 맞지 않는다".into(),
            ));
        }
        if block.count == 0 || usize::from(block.count) > crate::protocol::MAX_BLOCKS as usize {
            return Err(DriverError::InvalidRequest(format!(
                "읽기 개수 {}는 1~{}이어야 한다",
                block.count,
                crate::protocol::MAX_BLOCKS
            )));
        }
        self.invoke = self.invoke.wrapping_add(1);
        let mut vars = Vec::with_capacity(usize::from(block.count));
        for i in 0..block.count {
            let addr = u32::from(block.start) + u32::from(i);
            let name = format!("%{prefix}{addr}");
            vars.push(Variable {
                name: name.into_bytes(),
                is_word,
            });
        }
        let req = ReadRequest {
            invoke: self.invoke,
            vars,
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
