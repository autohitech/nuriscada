// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시험용 MELSEC MC 프로토콜(3E 바이너리) 시뮬레이터. 개발·CI 전용이며 배포물에 포함하지 않는다.
//!
//! 일괄 읽기(0401)의 워드(서브 0000)와 비트(서브 0001)를 지원한다.
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
    /// D 레지스터.
    pub d: Vec<u16>,
    /// W 링크 레지스터.
    pub w: Vec<u16>,
    /// R 파일 레지스터.
    pub r: Vec<u16>,
    /// M 내부 릴레이.
    pub m: Vec<bool>,
    /// X 입력(16진 주소).
    pub x: Vec<bool>,
    /// Y 출력(16진 주소).
    pub y: Vec<bool>,
    /// 응답 전에 기다리는 시간.
    pub response_delay: Duration,
    /// true이면 요청을 받아도 응답하지 않고 연결을 닫는다.
    pub drop_connections: bool,
    /// 받은 요청 수.
    pub requests_served: u64,
}

impl State {
    fn new(size: usize, bit_size: usize) -> Self {
        Self {
            d: vec![0; size],
            w: vec![0; size],
            r: vec![0; size],
            m: vec![false; bit_size],
            x: vec![false; bit_size],
            y: vec![false; bit_size],
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
    /// `bind` 주소(예: `127.0.0.1:0`)에서 시작한다. 워드 영역 크기는 `size`, 비트 영역 크기는 `bit_size`다.
    pub async fn start(bind: &str, size: usize, bit_size: usize) -> io::Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(Mutex::new(State::new(size, bit_size)));
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

    /// D 레지스터 값을 쓴다.
    pub fn set_d(&self, start: usize, values: &[u16]) {
        self.state().d[start..start + values.len()].copy_from_slice(values);
    }

    /// M 릴레이 값을 쓴다.
    pub fn set_m(&self, start: usize, values: &[bool]) {
        self.state().m[start..start + values.len()].copy_from_slice(values);
    }
}

impl Drop for Simulator {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 종료 코드를 담은 오류 응답을 만든다.
fn error_response(end: u16) -> Vec<u8> {
    vec![
        0xD0,
        0x00,
        0x00,
        0xFF,
        0xFF,
        0x03,
        0x00,
        0x02,
        0x00,
        (end & 0xFF) as u8,
        (end >> 8) as u8,
    ]
}

/// 요청 프레임(머리 포함)에 대한 응답을 만든다. 응답하지 않으면 `None`이다.
fn respond(frame: &[u8], state: &mut State) -> Option<Vec<u8>> {
    if frame.len() != 21 {
        return Some(error_response(0xC05C));
    }
    if frame[0..2] != [0x50, 0x00] {
        return Some(error_response(0xC05C));
    }
    let len = u16::from_le_bytes([frame[7], frame[8]]);
    if len != 12 {
        return Some(error_response(0x0050));
    }
    let command = u16::from_le_bytes([frame[11], frame[12]]);
    let subcommand = u16::from_le_bytes([frame[13], frame[14]]);
    if command != 0x0401 || (subcommand != 0x0000 && subcommand != 0x0001) {
        return Some(error_response(0xC05C));
    }
    let head = u32::from_le_bytes([frame[15], frame[16], frame[17], 0]);
    let code = frame[18];
    let count = usize::from(u16::from_le_bytes([frame[19], frame[20]]));
    let is_word = subcommand == 0x0000;
    if count == 0 {
        return Some(error_response(0xC051));
    }
    let start = head as usize;
    if is_word {
        let regs = match code {
            0xA8 => &state.d,
            0xB4 => &state.w,
            0xAF => &state.r,
            _ => return Some(error_response(0xC059)),
        };
        if start + count > regs.len() {
            return Some(error_response(0xC050));
        }
        let mut data = Vec::with_capacity(count * 2);
        for &v in &regs[start..start + count] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let len = u16::try_from(2 + data.len()).ok()?;
        let mut out = vec![0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00];
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&[0x00, 0x00]);
        out.extend_from_slice(&data);
        Some(out)
    } else {
        let bits = match code {
            0x90 => &state.m,
            0x9C => &state.x,
            0x9D => &state.y,
            _ => return Some(error_response(0xC059)),
        };
        if start + count > bits.len() {
            return Some(error_response(0xC050));
        }
        // 1점당 4비트(니블의 bit 0)에 2점씩 포장한다.
        let mut data = vec![0u8; count.div_ceil(2)];
        for (i, &bit) in bits[start..start + count].iter().enumerate() {
            if bit {
                if i % 2 == 0 {
                    data[i / 2] |= 0x01;
                } else {
                    data[i / 2] |= 0x10;
                }
            }
        }
        let len = u16::try_from(2 + data.len()).ok()?;
        let mut out = vec![0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00];
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&[0x00, 0x00]);
        out.extend_from_slice(&data);
        Some(out)
    }
}

async fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>) -> io::Result<()> {
    loop {
        // 요청은 항상 21바이트(서브헤더 2 + 경로 5 + 길이 2 + 데이터 12)다.
        let mut frame = [0u8; 21];
        stream.read_exact(&mut frame).await?;

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

    #[tokio::test]
    async fn d영역과_m영역_읽기_오류응답() {
        let sim = Simulator::start("127.0.0.1:0", 100, 64).await.unwrap();
        sim.set_d(10, &[7, 8]);
        sim.set_m(0, &[true, false, true]);
        let mut s = TcpStream::connect(sim.addr()).await.unwrap();

        // D10부터 2점 읽기.
        let req = [
            0x50, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00, 0x0C, 0x00, 0x10, 0x00, 0x01, 0x04, 0x00,
            0x00, 0x0A, 0x00, 0x00, 0xA8, 0x02, 0x00,
        ];
        s.write_all(&req).await.unwrap();
        let mut resp = [0u8; 15];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(
            resp,
            [
                0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00, 0x06, 0x00, 0x00, 0x00, 0x07, 0x00, 0x08,
                0x00
            ]
        );

        // M0부터 3점 읽기 → 0x01, 0x01.
        let req = [
            0x50, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00, 0x0C, 0x00, 0x10, 0x00, 0x01, 0x04, 0x01,
            0x00, 0x00, 0x00, 0x00, 0x90, 0x03, 0x00,
        ];
        s.write_all(&req).await.unwrap();
        let mut resp = [0u8; 13];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(
            resp,
            [
                0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01
            ]
        );

        // 범위 밖 주소 → C050.
        let req = [
            0x50, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00, 0x0C, 0x00, 0x10, 0x00, 0x01, 0x04, 0x00,
            0x00, 0x63, 0x00, 0x00, 0xA8, 0x02, 0x00,
        ];
        s.write_all(&req).await.unwrap();
        let mut err = [0u8; 11];
        s.read_exact(&mut err).await.unwrap();
        assert_eq!(&err[9..11], &[0x50, 0xC0]);

        // 지원하지 않는 명령 → C05C.
        let mut req = req;
        req[11] = 0x02;
        s.write_all(&req).await.unwrap();
        s.read_exact(&mut err).await.unwrap();
        assert_eq!(&err[9..11], &[0x5C, 0xC0]);
        assert_eq!(sim.state().requests_served, 4);
    }
}
