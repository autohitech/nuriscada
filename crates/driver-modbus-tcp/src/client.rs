// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! MODBUS TCP 마스터 클라이언트.

use std::time::{Duration, Instant};

use nuriscada_driver_api::{BlockData, Driver, DriverError, LinkState, LinkStatus, ReadBlock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::protocol::{
    MBAP_LEN, ReadFunction, ReadRequest, decode_read_response, encode_read_request,
    remaining_after_header,
};

/// 코일(기능 01)의 영역 번호.
pub const SPACE_COILS: u16 = 0;
/// 이산 입력(기능 02)의 영역 번호.
pub const SPACE_DISCRETE_INPUTS: u16 = 1;
/// 보유 레지스터(기능 03)의 영역 번호.
pub const SPACE_HOLDING_REGISTERS: u16 = 2;
/// 입력 레지스터(기능 04)의 영역 번호.
pub const SPACE_INPUT_REGISTERS: u16 = 3;

fn function_for(space: u16) -> Option<ReadFunction> {
    match space {
        SPACE_COILS => Some(ReadFunction::Coils),
        SPACE_DISCRETE_INPUTS => Some(ReadFunction::DiscreteInputs),
        SPACE_HOLDING_REGISTERS => Some(ReadFunction::HoldingRegisters),
        SPACE_INPUT_REGISTERS => Some(ReadFunction::InputRegisters),
        _ => None,
    }
}

/// MODBUS TCP 클라이언트. 연결이 끊어지면 다음 읽기에서 다시 연결을 시도한다.
#[derive(Debug)]
pub struct ModbusTcpClient {
    address: String,
    timeout: Duration,
    stream: Option<TcpStream>,
    transaction: u16,
    status: LinkStatus,
}

impl ModbusTcpClient {
    /// 클라이언트를 만든다. `timeout`은 연결과 요청 한 번(송신과 수신)의 제한 시간이다.
    pub fn new(address: impl Into<String>, timeout: Duration) -> Self {
        Self {
            address: address.into(),
            timeout,
            stream: None,
            transaction: 0,
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
        let mut header = [0u8; MBAP_LEN];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let rest = remaining_after_header(&header)?;
        let mut frame = vec![0u8; MBAP_LEN + rest];
        frame[..MBAP_LEN].copy_from_slice(&header);
        stream
            .read_exact(&mut frame[MBAP_LEN..])
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        decode_read_response(req, &frame)
    }
}

impl Driver for ModbusTcpClient {
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
        let function = function_for(block.space).ok_or_else(|| {
            DriverError::InvalidRequest(format!("지원하지 않는 영역 번호: {}", block.space))
        })?;
        if function.kind() != block.kind {
            return Err(DriverError::InvalidRequest(
                "영역과 데이터 종류가 맞지 않는다".into(),
            ));
        }
        self.transaction = self.transaction.wrapping_add(1);
        let req = ReadRequest {
            transaction: self.transaction,
            unit: block.unit,
            function,
            start: block.start,
            count: block.count,
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
