// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! S7 (ISO-on-TCP) 읽기·쓰기 변수 클라이언트. 연결할 때 COTP 접속과 통신 설정을 수행한다.

use std::time::{Duration, Instant};

use nuriscada_driver_api::{BlockData, Driver, DriverError, LinkState, LinkStatus, ReadBlock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::protocol::{
    ReadRequest, WriteRequest, decode_read_response, decode_setup, decode_write_response,
    encode_cotp_connect, encode_read_request, encode_setup, encode_write_request, strip_transport,
};

/// DB 워드 영역 번호(DB 번호 = 유닛).
pub const SPACE_S7DB: u16 = 50;
/// M 비트 영역 번호(비트 번호 = 워드×8 + 비트).
pub const SPACE_S7M: u16 = 51;

/// S7 클라이언트. 연결할 때 COTP 접속과 S7 통신 설정을 수행한다.
///
/// DB 번호는 `ReadBlock.unit`에서 가져온다.
#[derive(Debug)]
pub struct S7Client {
    address: String,
    timeout: Duration,
    stream: Option<TcpStream>,
    pdu_ref: u16,
    status: LinkStatus,
    // 마지막으로 주고받은 프레임(앞 256바이트까지). 교신 전에는 비어 있다.
    last_tx: Vec<u8>,
    last_rx: Vec<u8>,
}

impl S7Client {
    /// 클라이언트를 만든다. `timeout`은 연결·설정과 요청 한 번(송신과 수신)의 제한 시간이다.
    pub fn new(address: impl Into<String>, timeout: Duration) -> Self {
        Self {
            address: address.into(),
            timeout,
            stream: None,
            pdu_ref: 0,
            status: LinkStatus::initial(),
            last_tx: Vec::new(),
            last_rx: Vec::new(),
        }
    }

    fn drop_connection(&mut self) {
        self.stream = None;
        self.status.state = LinkState::Disconnected;
    }

    async fn transact(stream: &mut TcpStream, request: &[u8]) -> Result<Vec<u8>, DriverError> {
        stream
            .write_all(request)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let mut head = [0u8; 4];
        stream
            .read_exact(&mut head)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        if head[0] != 0x03 {
            return Err(DriverError::Protocol("TPKT가 아니다".into()));
        }
        let total = usize::from(u16::from_be_bytes([head[2], head[3]]));
        if !(4..=4096).contains(&total) {
            return Err(DriverError::Protocol(format!(
                "TPKT 길이가 범위를 벗어났다: {total}"
            )));
        }
        let mut rest = vec![0u8; total - 4];
        stream
            .read_exact(&mut rest)
            .await
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        let mut frame = head.to_vec();
        frame.extend_from_slice(&rest);
        Ok(frame)
    }

    async fn exchange(
        stream: &mut TcpStream,
        request: &[u8],
        pdu_ref: u16,
        req: &ReadRequest,
    ) -> Result<(BlockData, Vec<u8>), DriverError> {
        let frame = Self::transact(stream, request).await?;
        let pdu = strip_transport(&frame)?;
        if pdu.len() < 6 || u16::from_le_bytes([pdu[4], pdu[5]]) != pdu_ref {
            return Err(DriverError::Protocol("PDU 참조가 다르다".into()));
        }
        let data = decode_read_response(req, &frame)?;
        Ok((data, frame))
    }

    async fn exchange_write(
        stream: &mut TcpStream,
        request: &[u8],
        pdu_ref: u16,
    ) -> Result<(), DriverError> {
        let frame = Self::transact(stream, request).await?;
        let pdu = strip_transport(&frame)?;
        if pdu.len() < 6 || u16::from_le_bytes([pdu[4], pdu[5]]) != pdu_ref {
            return Err(DriverError::Protocol("PDU 참조가 다르다".into()));
        }
        decode_write_response(&frame)
    }

    /// 단일 값을 쓴다. DB는 워드, M은 비트(값 0/1)이다. DB 번호는 `unit`에서 가져온다.
    /// 불명확한 결과를 자동 재시도하지 않는다.
    pub async fn write_single(
        &mut self,
        unit: u8,
        space: u16,
        address: u16,
        value: u16,
    ) -> Result<(), DriverError> {
        let req = match space {
            SPACE_S7DB => WriteRequest::DbWords {
                db: u16::from(unit),
                start: address,
                values: vec![value],
            },
            SPACE_S7M => {
                if value > 1 {
                    return Err(DriverError::InvalidRequest(format!(
                        "M 비트 값 {value}는 0 또는 1이어야 한다"
                    )));
                }
                WriteRequest::MBits {
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
        // 접속(필요하면 통신 설정까지)이 참조 번호를 올리므로 먼저 연결한다.
        self.connect().await?;
        self.pdu_ref = self.pdu_ref.wrapping_add(1);
        let frame = encode_write_request(self.pdu_ref, &req)?;
        let pdu_ref = self.pdu_ref;

        let started = Instant::now();
        let result = async {
            let stream = self.stream.as_mut().expect("방금 연결했다");
            timeout(self.timeout, Self::exchange_write(stream, &frame, pdu_ref))
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

impl Driver for S7Client {
    async fn connect(&mut self) -> Result<(), DriverError> {
        if self.stream.is_some() {
            return Ok(());
        }
        let mut connected = timeout(self.timeout, TcpStream::connect(&self.address))
            .await
            .map_err(|_| DriverError::Timeout)?
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        connected
            .set_nodelay(true)
            .map_err(|e| DriverError::Connection(e.to_string()))?;
        // COTP 접속. 목적 TSAP 0x01 0x02(랙 0·슬롯 2)로 고정한다.
        let frame = Self::transact(&mut connected, &encode_cotp_connect(0x02)).await?;
        crate::protocol::decode_cotp_confirm(&frame)?;
        // S7 통신 설정.
        self.pdu_ref = self.pdu_ref.wrapping_add(1);
        let frame = Self::transact(&mut connected, &encode_setup(self.pdu_ref)).await?;
        decode_setup(&frame, self.pdu_ref)?;
        self.stream = Some(connected);
        self.status.state = LinkState::Connected;
        Ok(())
    }

    async fn read(&mut self, block: &ReadBlock) -> Result<BlockData, DriverError> {
        let req = match block.space {
            SPACE_S7DB => {
                if block.kind != nuriscada_driver_api::DataKind::Word {
                    return Err(DriverError::InvalidRequest(
                        "s7db 영역은 워드 종류이어야 한다".into(),
                    ));
                }
                ReadRequest::DbWords {
                    db: u16::from(block.unit),
                    start: block.start,
                    count: block.count,
                }
            }
            SPACE_S7M => {
                if block.kind != nuriscada_driver_api::DataKind::Bit {
                    return Err(DriverError::InvalidRequest(
                        "s7m 영역은 비트 종류이어야 한다".into(),
                    ));
                }
                ReadRequest::MBits {
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
        // 접속(필요하면 통신 설정까지)이 참조 번호를 올리므로 먼저 연결한다.
        self.connect().await?;
        self.pdu_ref = self.pdu_ref.wrapping_add(1);
        let frame = encode_read_request(self.pdu_ref, &req)?;
        let pdu_ref = self.pdu_ref;

        let started = Instant::now();
        let result = async {
            let stream = self.stream.as_mut().expect("방금 연결했다");
            timeout(self.timeout, Self::exchange(stream, &frame, pdu_ref, &req))
                .await
                .map_err(|_| DriverError::Timeout)?
        }
        .await;

        match &result {
            Ok((_, rx)) => {
                self.status.requests_ok += 1;
                self.status.last_round_trip = Some(started.elapsed());
                // 진단용으로 마지막 송수신 프레임을 앞 256바이트까지 남긴다.
                self.last_tx = frame[..frame.len().min(256)].to_vec();
                self.last_rx = rx[..rx.len().min(256)].to_vec();
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
        result.map(|(data, _)| data)
    }

    fn status(&self) -> LinkStatus {
        self.status
    }

    fn last_frames(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        // 아직 교신하지 않았으면 `None`이다.
        if self.last_tx.is_empty() && self.last_rx.is_empty() {
            None
        } else {
            Some((self.last_tx.clone(), self.last_rx.clone()))
        }
    }
}
