// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시험용 FINS/TCP 시뮬레이터. 개발·CI 전용이며 배포물에 포함하지 않는다.
//!
//! 노드 주소 교환(명령 0/1)과 메모리 영역 읽기(0101, DM 워드·CIO/WR/HR 비트)·쓰기(0102, DM 워드·CIO/WR/HR 비트)를 지원한다.
//! 실행 중에 메모리 값을 바꾸거나 응답을 늦추고 끊을 수 있어서
//! 드라이버와 스케줄러의 시험에 쓴다.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// 시뮬레이터의 메모리와 동작 설정.
#[derive(Debug)]
pub struct State {
    /// DM 워드.
    pub dm: Vec<u16>,
    /// CIO 워드(비트 읽기는 마스크로 계산).
    pub cio: Vec<u16>,
    /// WR 워드(CIO와 같은 크기).
    pub wr: Vec<u16>,
    /// HR 워드(CIO와 같은 크기).
    pub hr: Vec<u16>,
    /// 응답 전에 기다리는 시간.
    pub response_delay: Duration,
    /// true이면 요청을 받아도 응답하지 않고 연결을 닫는다.
    pub drop_connections: bool,
    /// 받은 요청 수.
    pub requests_served: u64,
}

impl State {
    fn new(word_size: usize, cio_size: usize) -> Self {
        Self {
            dm: vec![0; word_size],
            cio: vec![0; cio_size],
            wr: vec![0; cio_size],
            hr: vec![0; cio_size],
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
    pub async fn start(bind: &str, word_size: usize, cio_size: usize) -> io::Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(Mutex::new(State::new(word_size, cio_size)));
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

    /// DM 값을 쓴다.
    pub fn set_dm(&self, start: usize, values: &[u16]) {
        self.state().dm[start..start + values.len()].copy_from_slice(values);
    }

    /// CIO 워드 값을 쓴다.
    pub fn set_cio(&self, start: usize, values: &[u16]) {
        self.state().cio[start..start + values.len()].copy_from_slice(values);
    }

    /// WR 워드 값을 쓴다.
    pub fn set_wr(&self, start: usize, values: &[u16]) {
        self.state().wr[start..start + values.len()].copy_from_slice(values);
    }

    /// HR 워드 값을 쓴다.
    pub fn set_hr(&self, start: usize, values: &[u16]) {
        self.state().hr[start..start + values.len()].copy_from_slice(values);
    }
}

impl Drop for Simulator {
    fn drop(&mut self) {
        self.task.abort();
    }
}

const MAGIC: [u8; 4] = [0x46, 0x49, 0x4E, 0x53];
const SERVER_NODE: u8 = 0x0F;
const CLIENT_NODE: u8 = 0xFB;

/// FINS 오류 응답(명령 2, 빈 본문, 종료 코드)을 만든다.
fn error_response(end: u16, src: u8) -> Vec<u8> {
    // 오류 응답도 에코 10 + 명령 2 + 종료 2를 담는다.
    let mut out = Vec::with_capacity(30);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&22u32.to_be_bytes());
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    let echo = [
        0xC0,
        0x00,
        0x02,
        0x00,
        SERVER_NODE,
        0x00,
        0x00,
        CLIENT_NODE,
        0x00,
        0x01,
    ];
    out.extend_from_slice(&echo);
    out.extend_from_slice(&[0x01, src]);
    out.extend_from_slice(&end.to_be_bytes());
    out
}

/// 요청 프레임(머리 포함)에 대한 응답을 만든다. 응답하지 않으면 `None`이다.
fn respond(frame: &[u8], state: &mut State) -> Option<Vec<u8>> {
    if frame.len() < 16 || frame[..4] != MAGIC {
        return None;
    }
    let cmd = u32::from_be_bytes(frame[8..12].try_into().ok()?);
    if cmd == 0 {
        // 노드 주소 교환: 클라이언트·서버 노드를 돌려준다.
        let mut out = Vec::with_capacity(24);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&16u32.to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&[0x00, 0x00, 0x00, CLIENT_NODE]);
        out.extend_from_slice(&[0x00, 0x00, 0x00, SERVER_NODE]);
        return Some(out);
    }
    if cmd != 2 || frame.len() < 16 + 18 {
        return Some(error_response(0x0401, 0x01));
    }
    let body = &frame[16..];
    if body[0] != 0x80 || body[10] != 0x01 {
        return Some(error_response(0x0401, 0x01));
    }
    if body[11] == 0x02 {
        return respond_write(frame, state);
    }
    if body[11] != 0x01 {
        return Some(error_response(0x0401, 0x01));
    }
    let area = body[12];
    let addr = u16::from_be_bytes([body[13], body[14]]);
    let bit = body[15];
    let count = usize::from(u16::from_be_bytes([body[16], body[17]]));
    if count == 0 || count > 128 {
        return Some(error_response(0x0203, 0x01));
    }
    // 데이터를 읽는다.
    let mut data = Vec::with_capacity(count * 2);
    match area {
        0x82 => {
            if usize::from(addr) + count > state.dm.len() {
                return Some(error_response(0x0104, 0x01));
            }
            for &v in &state.dm[usize::from(addr)..usize::from(addr) + count] {
                data.extend_from_slice(&v.to_be_bytes());
            }
        }
        0x30..=0x32 => {
            if bit > 15 {
                return Some(error_response(0x0104, 0x01));
            }
            let bank: &[u16] = match area {
                0x30 => &state.cio,
                0x31 => &state.wr,
                _ => &state.hr,
            };
            for i in 0..count {
                // 비트 읽기는 시작 비트에서 count개 순차 비트를 읽는다.
                let total_bit = usize::from(addr) * 16 + usize::from(bit) + i;
                let w = bank.get(total_bit / 16).copied().unwrap_or(0);
                let on = (w >> (total_bit % 16)) & 1 != 0;
                data.extend_from_slice(&(u16::from(on)).to_be_bytes());
            }
        }
        _ => return Some(error_response(0x0401, 0x01)),
    }
    let mut out = Vec::with_capacity(16 + 22 + data.len());
    out.extend_from_slice(&MAGIC);
    // 길이 = 명령 4 + 오류 4 + 에코 10 + 명령 2 + 데이터 + 종료 2.
    out.extend_from_slice(&(22u32 + data.len() as u32).to_be_bytes());
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    let mut echo = [
        0xC0,
        0x00,
        0x02,
        0x00,
        SERVER_NODE,
        0x00,
        0x00,
        CLIENT_NODE,
        0x00,
        0x00,
    ];
    echo[9] = body[9];
    out.extend_from_slice(&echo);
    out.extend_from_slice(&[0x01, 0x01]);
    out.extend_from_slice(&data);
    out.extend_from_slice(&0u16.to_be_bytes());
    Some(out)
}

/// 메모리 영역 쓰기(0102) 요청을 메모리에 반영한다. DM 워드와 CIO 비트를 지원한다.
fn respond_write(frame: &[u8], state: &mut State) -> Option<Vec<u8>> {
    // 쓰기 요청은 읽기보다 값이 더 있다(DM은 count×2바이트, CIO 비트는 count바이트).
    if frame.len() < 16 + 18 {
        return Some(error_response(0x0401, 0x02));
    }
    let body = &frame[16..];
    let area = body[12];
    let addr = usize::from(u16::from_be_bytes([body[13], body[14]]));
    let bit = body[15];
    let count = usize::from(u16::from_be_bytes([body[16], body[17]]));
    if count == 0 || count > 128 {
        return Some(error_response(0x0203, 0x02));
    }
    match area {
        0x82 => {
            if frame.len() != 16 + 18 + count * 2 {
                return Some(error_response(0x0401, 0x02));
            }
            if addr + count > state.dm.len() {
                return Some(error_response(0x0104, 0x02));
            }
            for (i, pair) in frame[34..].as_chunks::<2>().0.iter().enumerate() {
                state.dm[addr + i] = u16::from_be_bytes([pair[0], pair[1]]);
            }
        }
        0x30..=0x32 => {
            if bit > 15 {
                return Some(error_response(0x0104, 0x02));
            }
            if frame.len() != 16 + 18 + count {
                return Some(error_response(0x0401, 0x02));
            }
            let bank: &mut Vec<u16> = match area {
                0x30 => &mut state.cio,
                0x31 => &mut state.wr,
                _ => &mut state.hr,
            };
            let start_bit = addr * 16 + usize::from(bit);
            if start_bit + count > bank.len() * 16 {
                return Some(error_response(0x0104, 0x02));
            }
            for (i, &raw) in frame[34..34 + count].iter().enumerate() {
                if raw > 1 {
                    return Some(error_response(0x0203, 0x02));
                }
                let total_bit = start_bit + i;
                let slot = &mut bank[total_bit / 16];
                if raw == 1 {
                    *slot |= 1 << (total_bit % 16);
                } else {
                    *slot &= !(1 << (total_bit % 16));
                }
            }
        }
        _ => return Some(error_response(0x0401, 0x02)),
    }
    // 응답: 머리 + 에코 10 + 명령 2 + 종료 2.
    let mut out = Vec::with_capacity(16 + 14);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&22u32.to_be_bytes());
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    let echo = [
        0xC0,
        0x00,
        0x02,
        0x00,
        SERVER_NODE,
        0x00,
        0x00,
        CLIENT_NODE,
        0x00,
        body[9],
    ];
    out.extend_from_slice(&echo);
    out.extend_from_slice(&[0x01, 0x02]);
    out.extend_from_slice(&0u16.to_be_bytes());
    Some(out)
}

async fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>) -> io::Result<()> {
    loop {
        // 머리(16바이트)를 먼저 읽고 길이에 따라 본문을 읽는다.
        let mut head = [0u8; 16];
        stream.read_exact(&mut head).await?;
        if head[..4] != MAGIC {
            return Ok(());
        }
        let len = u32::from_be_bytes(head[4..8].try_into().expect("4바이트")) as usize;
        if !(8..=4096).contains(&len) {
            return Ok(());
        }
        // 머리 16바이트에는 명령·오류가 이미 들어 있다. 남은 파라미터만 읽는다.
        let param_len = len - 8;
        let mut params = vec![0u8; param_len];
        stream.read_exact(&mut params).await?;
        let mut frame = Vec::with_capacity(16 + param_len);
        frame.extend_from_slice(&head);
        frame.extend_from_slice(&params);

        let (delay, drop_now, reply) = {
            let mut s = state.lock().expect("시뮬레이터 상태 잠금");
            s.requests_served += 1;
            (
                s.response_delay,
                s.drop_connections,
                respond(&frame, &mut s),
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

    fn request(area: u8, word: u16, bit: u8, count: u16) -> Vec<u8> {
        let mut body = vec![
            0x80,
            0x00,
            0x02,
            0x00,
            SERVER_NODE,
            0x00,
            0x00,
            CLIENT_NODE,
            0x00,
            0x01,
            0x01,
            0x01,
            area,
            (word >> 8) as u8,
            (word & 0xFF) as u8,
            bit,
        ];
        body.extend_from_slice(&count.to_be_bytes());
        let mut frame = vec![
            0x46, 0x49, 0x4E, 0x53, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x00,
        ];
        let len = (8 + body.len()) as u32;
        frame[4..8].copy_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(&body);
        frame
    }

    async fn handshake(s: &mut TcpStream) {
        s.write_all(&[
            0x46, 0x49, 0x4E, 0x53, 0x00, 0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .await
        .unwrap();
        let mut res = [0u8; 24];
        s.read_exact(&mut res).await.unwrap();
        assert_eq!(
            &res[..12],
            &[
                0x46, 0x49, 0x4E, 0x53, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x01
            ]
        );
        assert_eq!(res[19], CLIENT_NODE);
        assert_eq!(res[23], SERVER_NODE);
    }

    async fn read_response(s: &mut TcpStream) -> Vec<u8> {
        let mut head = [0u8; 16];
        s.read_exact(&mut head).await.unwrap();
        let len = u32::from_be_bytes(head[4..8].try_into().unwrap()) as usize;
        let mut rest = vec![0u8; len - 8];
        s.read_exact(&mut rest).await.unwrap();
        let mut frame = head.to_vec();
        frame.extend_from_slice(&rest);
        frame
    }

    #[tokio::test]
    async fn dm와_cio_읽기_오류응답() {
        let sim = Simulator::start("127.0.0.1:0", 200, 64).await.unwrap();
        sim.set_dm(20, &[0x1234, 0x5678]);
        sim.set_cio(1, &[0x0008]);
        let mut s = TcpStream::connect(sim.addr()).await.unwrap();
        handshake(&mut s).await;

        s.write_all(&request(0x82, 20, 0, 2)).await.unwrap();
        let resp = read_response(&mut s).await;
        assert_eq!(&resp[16..20], &[0xC0, 0x00, 0x02, 0x00]);
        assert_eq!(&resp[28..32], &[0x12, 0x34, 0x56, 0x78]);

        // CIO 1.03: 워드 1의 bit 3 → 0x0008이므로 true.
        s.write_all(&request(0x30, 1, 3, 1)).await.unwrap();
        let resp = read_response(&mut s).await;
        assert_eq!(&resp[28..30], &[0x00, 0x01]);

        // 범위 밖 D 주소 → 0104.
        s.write_all(&request(0x82, 199, 0, 2)).await.unwrap();
        let resp = read_response(&mut s).await;
        assert_eq!(&resp[resp.len() - 2..], &[0x01, 0x04]);
        assert_eq!(sim.state().requests_served, 4);
    }

    #[tokio::test]
    async fn 영역_쓰기가_메모리에_반영된다() {
        let sim = Simulator::start("127.0.0.1:0", 300, 64).await.unwrap();
        let mut s = TcpStream::connect(sim.addr()).await.unwrap();
        handshake(&mut s).await;
        // D20에 0x1234, 0x5678 쓰기.
        let body = vec![
            0x80,
            0x00,
            0x02,
            0x00,
            SERVER_NODE,
            0x00,
            0x00,
            CLIENT_NODE,
            0x00,
            0x01,
            0x01,
            0x02,
            0x82,
            0x00,
            0x14,
            0x00,
            0x00,
            0x02,
            0x12,
            0x34,
            0x56,
            0x78,
        ];
        let mut req = vec![
            0x46, 0x49, 0x4E, 0x53, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x00,
        ];
        req[4..8].copy_from_slice(&(8 + body.len() as u32).to_be_bytes());
        req.extend_from_slice(&body);
        s.write_all(&req).await.unwrap();
        let resp = read_response(&mut s).await;
        assert_eq!(&resp[resp.len() - 2..], &[0x00, 0x00]);
        assert_eq!(sim.state().dm[20], 0x1234);
        assert_eq!(sim.state().dm[21], 0x5678);
        // 범위 밖 D299 쓰기 → 0104.
        let mut body = body.clone();
        body[13] = 0x01;
        body[14] = 0x2B;
        let mut req = vec![
            0x46, 0x49, 0x4E, 0x53, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x00,
        ];
        req[4..8].copy_from_slice(&(8 + body.len() as u32).to_be_bytes());
        req.extend_from_slice(&body);
        s.write_all(&req).await.unwrap();
        let resp = read_response(&mut s).await;
        assert_eq!(&resp[resp.len() - 2..], &[0x01, 0x04]);
        assert_eq!(sim.state().requests_served, 3);
    }
}
