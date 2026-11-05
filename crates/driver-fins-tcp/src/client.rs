// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! FINS/TCP 메모리 영역 읽기·쓰기 클라이언트. 연결할 때 노드 주소 교환을 수행한다.

use std::time::{Duration, Instant};

use nuriscada_driver_api::{BlockData, Driver, DriverError, LinkState, LinkStatus, ReadBlock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::protocol::{
    ReadRequest, TCP_HEADER_LEN, WriteRequest, decode_node_response, decode_read_response,
    decode_write_response, encode_node_request, encode_read_request, encode_write_request,
    remaining_after_header,
};

/// DM 워드 영역 번호.
pub const SPACE_DM: u16 = 40;
/// CIO 비트 영역 번호(비트 번호 = 워드×16 + 비트).
pub const SPACE_CIO: u16 = 41;

/// FINS/TCP 클라이언트. 연결할 때 노드 주소 교환을 수행하고 노드를 캐시한다.
///
/// FINS 프레임에는 국번이 없으므로 `ReadBlock.unit`은 무시한다.
#[derive(Debug)]
pub struct FinsTcpClient {
    address: String,
    timeout: Duration,
    stream: Option<TcpStream>,
    client_node: u8,
    server_node: u8,
    sid: u8,
    status: LinkStatus,
}

impl FinsTcpClient {
    /// 클라이언트를 만든다. `timeout`은 연결과 요청 한 번(송신과 수신)의 제한 시간이다.
    pub fn new(address: impl Into<String>, timeout: Duration) -> Self {
        Self {
            address: address.into(),
            timeout,
            stream: None,
            client_node: 0,
            server_node: 0,
            sid: 0,
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
        let mut header = [0u8; TCP_HEADER_LEN];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let rest = remaining_after_header(&header)?;
        let mut frame = vec![0u8; TCP_HEADER_LEN + rest];
        frame[..TCP_HEADER_LEN].copy_from_slice(&header);
        stream
            .read_exact(&mut frame[TCP_HEADER_LEN..])
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        decode_read_response(req, &frame)
    }

    async fn exchange_write(stream: &mut TcpStream, request: &[u8]) -> Result<(), DriverError> {
        stream
            .write_all(request)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let mut header = [0u8; TCP_HEADER_LEN];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let rest = remaining_after_header(&header)?;
        let mut frame = vec![0u8; TCP_HEADER_LEN + rest];
        frame[..TCP_HEADER_LEN].copy_from_slice(&header);
        stream
            .read_exact(&mut frame[TCP_HEADER_LEN..])
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        decode_write_response(&frame)
    }

    /// 단일 값을 쓴다. DM은 워드, CIO는 비트(값 0/1)이다. `unit`은 무시한다.
    /// 불명확한 결과를 자동 재시도하지 않는다.
    pub async fn write_single(
        &mut self,
        _unit: u8,
        space: u16,
        address: u16,
        value: u16,
    ) -> Result<(), DriverError> {
        let req = match space {
            SPACE_DM => WriteRequest::DmWords {
                start: address,
                values: vec![value],
            },
            SPACE_CIO => {
                if value > 1 {
                    return Err(DriverError::InvalidRequest(format!(
                        "CIO 비트 값 {value}는 0 또는 1이어야 한다"
                    )));
                }
                WriteRequest::CioBits {
                    start_bit: u32::from(address),
                    values: vec![value == 1],
                }
            }
            _ => {
                return Err(DriverError::InvalidRequest(format!(
                    "쓰기를 지원하지 않는 영역이다: {space}"
                )));
            }
        };
        self.sid = self.sid.wrapping_add(1);
        let frame = encode_write_request(self.client_node, self.server_node, self.sid, &req)?;

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

impl Driver for FinsTcpClient {
    async fn connect(&mut self) -> Result<(), DriverError> {
        if self.stream.is_some() {
            return Ok(());
        }
        let mut connected = timeout(self.timeout, TcpStream::connect(&self.address))
            .await
            .map_err(|_| DriverError::Timeout)?
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        // 요청이 작으므로 지연 없이 보낸다.
        connected
            .set_nodelay(true)
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        // 노드 주소 교환.
        let req = encode_node_request();
        timeout(self.timeout, connected.write_all(&req))
            .await
            .map_err(|_| DriverError::Timeout)?
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let mut res = [0u8; 24];
        timeout(self.timeout, connected.read_exact(&mut res))
            .await
            .map_err(|_| DriverError::Timeout)?
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let (client, server) = decode_node_response(&res)?;
        self.client_node = client;
        self.server_node = server;
        self.stream = Some(connected);
        self.status.state = LinkState::Connected;
        Ok(())
    }

    async fn read(&mut self, block: &ReadBlock) -> Result<BlockData, DriverError> {
        let req = match block.space {
            SPACE_DM => {
                if block.kind != nuriscada_driver_api::DataKind::Word {
                    return Err(DriverError::InvalidRequest(
                        "DM 영역은 워드 종류이어야 한다".into(),
                    ));
                }
                ReadRequest::DmWords {
                    start: block.start,
                    count: block.count,
                }
            }
            SPACE_CIO => {
                if block.kind != nuriscada_driver_api::DataKind::Bit {
                    return Err(DriverError::InvalidRequest(
                        "CIO 영역은 비트 종류이어야 한다".into(),
                    ));
                }
                ReadRequest::CioBits {
                    start_bit: u32::from(block.start),
                    count: block.count,
                }
            }
            _ => {
                return Err(DriverError::InvalidRequest(format!(
                    "지원하지 않는 영역 번호: {}",
                    block.space
                )));
            }
        };
        self.sid = self.sid.wrapping_add(1);
        let frame = encode_read_request(self.client_node, self.server_node, self.sid, &req)?;

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
