// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시험용 S7 (ISO-on-TCP) 시뮬레이터. 개발·CI 전용이며 배포물에 포함하지 않는다.
//!
//! COTP 접속, S7 통신 설정, 읽기 변수(0x04, DB 워드·M 비트)를 지원한다.
//! 실행 중에 메모리 값을 바꾸거나 응답을 늦추고 끊을 수 있어서
//! 드라이버와 스케줄러의 시험에 쓴다.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// 시뮬레이터의 메모리와 동작 설정.
#[derive(Debug)]
pub struct State {
    /// DB 번호별 워드 메모리.
    pub db: HashMap<u16, Vec<u16>>,
    /// M 비트.
    pub m: Vec<bool>,
    /// 응답 전에 기다리는 시간.
    pub response_delay: Duration,
    /// true이면 요청을 받아도 응답하지 않고 연결을 닫는다.
    pub drop_connections: bool,
    /// 받은 요청 수.
    pub requests_served: u64,
}

impl State {
    fn new() -> Self {
        Self {
            db: HashMap::new(),
            m: vec![false; 1024],
            response_delay: Duration::ZERO,
            drop_connections: false,
            requests_served: 0,
        }
    }
}

/// 실행 중인 시뮬레이터. 버리면 서버가 멈춘다.
#[derive(Debug)]
pub struct Simulator {
    addr: std::net::SocketAddr,
    state: Arc<Mutex<State>>,
    task: JoinHandle<()>,
}

impl Simulator {
    /// `bind` 주소(예: `127.0.0.1:0`)에서 시작한다.
    pub async fn start(bind: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(Mutex::new(State::new()));
        let shared = Arc::clone(&state);
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let conn_state = Arc::clone(&shared);
                tokio::spawn(async move {
                    let _ = serve(stream, conn_state).await;
                });
            }
        });
        Ok(Self { addr, state, task })
    }

    /// 접속 주소.
    pub fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    /// 상태를 잠그고 돌려준다. 짧게만 쥐고 있어야 한다.
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("시뮬레이터 상태 잠금")
    }

    /// DB 워드 영역을 만든다(없는 번호면 생성).
    pub fn set_db(&self, db: u16, start: usize, values: &[u16]) {
        let mut s = self.state();
        let regs = s.db.entry(db).or_insert_with(|| vec![0; 1024]);
        if regs.len() < start + values.len() {
            regs.resize(start + values.len(), 0);
        }
        regs[start..start + values.len()].copy_from_slice(values);
    }

    /// M 비트 값을 쓴다.
    pub fn set_m(&self, start: usize, values: &[bool]) {
        let mut s = self.state();
        if s.m.len() < start + values.len() {
            s.m.resize(start + values.len(), false);
        }
        s.m[start..start + values.len()].copy_from_slice(values);
    }
}

impl Drop for Simulator {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn tpkt(payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x03, 0x00];
    out.extend_from_slice(&(4 + payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn wrap(pdu: &[u8]) -> Vec<u8> {
    let mut full = vec![0x02, 0xF0, 0x80];
    full.extend_from_slice(pdu);
    tpkt(&full)
}

fn s7_header(rosctr: u8, pdu_ref: u16, param_len: u16, data_len: u16) -> Vec<u8> {
    let mut h = vec![0x32, rosctr, 0x00, 0x00];
    h.extend_from_slice(&pdu_ref.to_le_bytes());
    h.extend_from_slice(&param_len.to_be_bytes());
    h.extend_from_slice(&data_len.to_be_bytes());
    h
}

/// S7 PDU에 대한 응답을 만든다. 응답하지 않으면 `None`이다.
fn respond(pdu: &[u8], state: &mut State) -> Option<Vec<u8>> {
    if pdu.len() < 10 || pdu[0] != 0x32 || pdu[1] != 0x01 {
        return None;
    }
    let pdu_ref = u16::from_le_bytes([pdu[4], pdu[5]]);
    let func = pdu[10];
    if func == 0xF0 {
        // 통신 설정: PDU 480 수락.
        let param = [0xF0, 0x00, 0x00, 0x01, 0x00, 0x01, 0x01, 0xE0];
        let mut out = s7_header(0x03, pdu_ref, 8, 0);
        out.extend_from_slice(&param);
        return Some(wrap(&out));
    }
    if func != 0x04 || pdu.len() < 12 + 12 {
        return None;
    }
    let item_count = pdu[11];
    if item_count != 1 {
        return None;
    }
    let transport = pdu[15];
    let data_len = u16::from_be_bytes([pdu[16], pdu[17]]);
    let db = u16::from_be_bytes([pdu[18], pdu[19]]);
    let area = pdu[20];
    let addr = u32::from_be_bytes([0, pdu[21], pdu[22], pdu[23]]);
    let (is_word, bytes) = match (area, transport) {
        (0x84, 0x02) => (true, data_len as usize),
        (0x83, 0x01) => (false, data_len.div_ceil(8) as usize),
        _ => return None,
    };
    let _ = bytes;
    // 데이터를 읽는다.
    let mut data = Vec::new();
    if is_word {
        let count = usize::from(data_len) / 2;
        let regs = state.db.get(&db).cloned().unwrap_or_default();
        let start = addr as usize;
        if start + count > regs.len() {
            // 항목 오류 0x05(범위 밖).
            let mut out = s7_header(0x03, pdu_ref, 2, 2 + count as u16 * 2);
            out.extend_from_slice(&[0x00, 0x00, 0x05, 0x04]);
            out.extend_from_slice(&0u16.to_be_bytes());
            return Some(wrap(&out));
        }
        data.push(0xFF);
        data.push(0x04);
        data.extend_from_slice(&(count as u16 * 2).to_be_bytes());
        for &v in &regs[start..start + count] {
            data.extend_from_slice(&v.to_be_bytes());
        }
    } else {
        let count = usize::from(data_len);
        let total_bits = state.m.len();
        let start = addr as usize;
        if start + count > total_bits {
            let mut out = s7_header(0x03, pdu_ref, 2, 3);
            out.extend_from_slice(&[0x00, 0x00, 0x05, 0x04]);
            out.extend_from_slice(&0u16.to_be_bytes());
            return Some(wrap(&out));
        }
        data.push(0xFF);
        data.push(0x03);
        let nbytes = count.div_ceil(8);
        data.extend_from_slice(&(nbytes as u16).to_be_bytes());
        let mut cur = 0u8;
        let mut n = 0;
        for i in 0..count {
            if state.m[start + i] {
                cur |= 1 << n;
            }
            n += 1;
            if n == 8 {
                data.push(cur);
                cur = 0;
                n = 0;
            }
        }
        if n > 0 {
            data.push(cur);
        }
    }
    let mut out = s7_header(0x03, pdu_ref, 2, data.len() as u16);
    out.extend_from_slice(&[0x00, 0x00]);
    out.extend_from_slice(&data);
    Some(wrap(&out))
}

async fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>) -> io::Result<()> {
    loop {
        // TPKT 머리(4바이트)를 먼저 읽고 길이에 따라 본문을 읽는다.
        let mut head = [0u8; 4];
        stream.read_exact(&mut head).await?;
        if head[0] != 0x03 {
            return Ok(());
        }
        let len = usize::from(u16::from_be_bytes([head[2], head[3]]));
        if !(4..=4096).contains(&len) {
            return Ok(());
        }
        let mut rest = vec![0u8; len - 4];
        stream.read_exact(&mut rest).await?;
        let mut frame = Vec::with_capacity(len);
        frame.extend_from_slice(&head);
        frame.extend_from_slice(&rest);

        // COTP 접속 요청이면 확인을 돌려준다.
        if rest.first() == Some(&0x11) && rest.get(1) == Some(&0xE0) {
            let mut out = vec![0x03, 0x00];
            out.extend_from_slice(&25u16.to_be_bytes());
            out.extend_from_slice(&[
                0x0E, 0xD0, 0x00, 0x00, 0x00, 0x01, 0x00, 0xC1, 0x02, 0x01, 0x00, 0xC2, 0x02, 0x01,
                0x00, 0xC0, 0x01, 0x09, 0x00, 0x00, 0x00,
            ]);
            stream.write_all(&out).await?;
            continue;
        }
        // COTP 데이터면 S7 PDU를 처리한다(머리 4 + COTP 3).
        if frame.len() < 7 || frame[4] != 0x02 || frame[5] != 0xF0 {
            return Ok(());
        }
        let (delay, drop_now, reply) = {
            let mut s = state.lock().expect("시뮬레이터 상태 잠금");
            s.requests_served += 1;
            (
                s.response_delay,
                s.drop_connections,
                respond(&frame[7..], &mut s),
            )
        };
        if drop_now {
            return Ok(());
        }
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if let Some(reply) = reply {
            stream.write_all(&reply).await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn handshake(s: &mut TcpStream) {
        // COTP 접속 요청.
        s.write_all(&[
            0x03, 0x00, 0x00, 0x16, 0x11, 0xE0, 0x00, 0x00, 0x00, 0x01, 0x00, 0xC1, 0x02, 0x01,
            0x00, 0xC2, 0x02, 0x01, 0x02, 0xC0, 0x01, 0x09,
        ])
        .await
        .unwrap();
        let mut head = [0u8; 4];
        s.read_exact(&mut head).await.unwrap();
        let len = usize::from(u16::from_be_bytes([head[2], head[3]]));
        let mut rest = vec![0u8; len - 4];
        s.read_exact(&mut rest).await.unwrap();
        assert_eq!(rest[1], 0xD0);
        // S7 통신 설정.
        let mut setup = vec![0x03, 0x00, 0x00, 0x19, 0x02, 0xF0, 0x80];
        setup.extend_from_slice(&[
            0x32, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x08, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x01,
            0x00, 0x01, 0x01, 0xE0,
        ]);
        s.write_all(&setup).await.unwrap();
        let mut head = [0u8; 4];
        s.read_exact(&mut head).await.unwrap();
        let len = usize::from(u16::from_be_bytes([head[2], head[3]]));
        let mut rest = vec![0u8; len - 4];
        s.read_exact(&mut rest).await.unwrap();
        assert_eq!(rest[3 + 1], 0x03);
    }

    async fn read_pdu(s: &mut TcpStream) -> Vec<u8> {
        let mut head = [0u8; 4];
        s.read_exact(&mut head).await.unwrap();
        let len = usize::from(u16::from_be_bytes([head[2], head[3]]));
        let mut rest = vec![0u8; len - 4];
        s.read_exact(&mut rest).await.unwrap();
        let mut frame = head.to_vec();
        frame.extend_from_slice(&rest);
        frame[7..].to_vec()
    }

    fn read_req(pdu_ref: u16, item: &[u8]) -> Vec<u8> {
        let mut pdu = vec![0x32, 0x01, 0x00, 0x00];
        pdu.extend_from_slice(&pdu_ref.to_le_bytes());
        pdu.extend_from_slice(&(2 + item.len() as u16).to_be_bytes());
        pdu.extend_from_slice(&0u16.to_be_bytes());
        pdu.extend_from_slice(&[0x04, 0x01]);
        pdu.extend_from_slice(item);
        let mut out = vec![0x03, 0x00];
        out.extend_from_slice(&(4 + 3 + pdu.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0x02, 0xF0, 0x80]);
        out.extend_from_slice(&pdu);
        out
    }

    #[tokio::test]
    async fn db와_m_읽기_오류응답() {
        let sim = Simulator::start("127.0.0.1:0").await.unwrap();
        sim.set_db(1, 0, &[0x1234, 0x5678]);
        sim.set_m(80, &[true, false, true]);
        let mut s = TcpStream::connect(sim.addr()).await.unwrap();
        handshake(&mut s).await;

        // DB1 0부터 2워드.
        let item = [
            0x12, 0x0A, 0x10, 0x02, 0x00, 0x04, 0x00, 0x01, 0x84, 0x00, 0x00, 0x00,
        ];
        s.write_all(&read_req(1, &item)).await.unwrap();
        let pdu = read_pdu(&mut s).await;
        assert_eq!(pdu[1], 0x03);
        assert_eq!(&pdu[10..12], &[0x00, 0x00]);
        assert_eq!(&pdu[12..16], &[0xFF, 0x04, 0x00, 0x04]);
        assert_eq!(&pdu[16..20], &[0x12, 0x34, 0x56, 0x78]);

        // M80부터 3비트: true, false, true → 0b101.
        let item = [
            0x12, 0x0A, 0x10, 0x01, 0x00, 0x03, 0x00, 0x00, 0x83, 0x00, 0x00, 0x50,
        ];
        s.write_all(&read_req(2, &item)).await.unwrap();
        let pdu = read_pdu(&mut s).await;
        assert_eq!(&pdu[10..17], &[0x00, 0x00, 0xFF, 0x03, 0x00, 0x01, 0x05]);

        // 없는 DB → 항목 오류 0x05.
        let item = [
            0x12, 0x0A, 0x10, 0x02, 0x00, 0x02, 0x00, 0x09, 0x84, 0x00, 0x00, 0x00,
        ];
        s.write_all(&read_req(3, &item)).await.unwrap();
        let pdu = read_pdu(&mut s).await;
        assert_eq!(&pdu[10..13], &[0x00, 0x00, 0x05]);
        assert_eq!(sim.state().requests_served, 4);
    }
}
