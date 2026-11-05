// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시험용 XGT 전용 프로토콜(FEnet) 시뮬레이터. 개발·CI 전용이며 배포물에 포함하지 않는다.
//!
//! 개별 읽기(0x5400)의 워드·비트를 지원한다. 변수 이름은 `%MWn`·`%MXn` ASCII다.
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
    /// %MW 워드.
    pub mw: Vec<u16>,
    /// %MX 비트.
    pub mx: Vec<bool>,
    /// 응답 전에 기다리는 시간.
    pub response_delay: Duration,
    /// true이면 요청을 받아도 응답하지 않고 연결을 닫는다.
    pub drop_connections: bool,
    /// 받은 요청 수.
    pub requests_served: u64,
}

impl State {
    fn new(word_size: usize, bit_size: usize) -> Self {
        Self {
            mw: vec![0; word_size],
            mx: vec![false; bit_size],
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
    pub async fn start(bind: &str, word_size: usize, bit_size: usize) -> io::Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(Mutex::new(State::new(word_size, bit_size)));
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

    /// %MW 값을 쓴다.
    pub fn set_mw(&self, start: usize, values: &[u16]) {
        self.state().mw[start..start + values.len()].copy_from_slice(values);
    }

    /// %MX 값을 쓴다.
    pub fn set_mx(&self, start: usize, values: &[bool]) {
        self.state().mx[start..start + values.len()].copy_from_slice(values);
    }
}

impl Drop for Simulator {
    fn drop(&mut self) {
        self.task.abort();
    }
}

const COMPANY: [u8; 10] = [0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00];

/// 오류 응답(명령 에코+오류 상태, 블록 없음)을 만든다.
fn error_response(invoke: u16, cmd: [u8; 2], dtype: u16, error: u16) -> Vec<u8> {
    let body = cmd;
    let mut out = Vec::with_capacity(30);
    out.extend_from_slice(&COMPANY);
    out.extend_from_slice(&[0x00, 0x00]);
    out.push(0xA0);
    out.push(0x11);
    out.extend_from_slice(&invoke.to_le_bytes());
    out.extend_from_slice(&10u16.to_le_bytes());
    out.push(0x00);
    out.push(0x00);
    out.extend_from_slice(&body);
    out.extend_from_slice(&dtype.to_le_bytes());
    out.extend_from_slice(&[0x00, 0x00]);
    out.extend_from_slice(&error.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// 변수 이름(`%MW100`, `%MX7`)을 (워드인가, 번호)로 푼다.
fn parse_name(name: &[u8]) -> Option<(bool, usize)> {
    if name.len() < 4 || name[0] != b'%' || name[1] != b'M' {
        return None;
    }
    let (is_word, digits) = match name[2] {
        b'W' => (true, &name[3..]),
        b'X' => (false, &name[3..]),
        _ => return None,
    };
    if digits.is_empty() || digits.len() > 6 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let num = std::str::from_utf8(digits).ok()?.parse::<usize>().ok()?;
    Some((is_word, num))
}

/// 요청 프레임에 대한 응답을 만든다. 응답하지 않으면 `None`이다.
fn respond(frame: &[u8], state: &mut State) -> Option<Vec<u8>> {
    if frame.len() < 28 || frame[..8] != COMPANY[..8] || frame[13] != 0x33 {
        return Some(error_response(0, [0x55, 0x00], 0x0002, 0x0001));
    }
    let invoke = u16::from_le_bytes([frame[14], frame[15]]);
    let cmd = u16::from_le_bytes([frame[20], frame[21]]);
    if cmd == 0x0058 {
        return respond_write(frame, invoke, state);
    }
    if cmd != 0x0054 {
        return Some(error_response(invoke, [0x55, 0x00], 0x0002, 0x0001));
    }
    let dtype = u16::from_le_bytes([frame[22], frame[23]]);
    let is_word = match dtype {
        0x0002 => true,
        0x0000 => false,
        _ => return Some(error_response(invoke, [0x55, 0x00], dtype, 0x0001)),
    };
    let blocks = usize::from(u16::from_le_bytes([frame[26], frame[27]]));
    if blocks == 0 || blocks > 16 {
        return Some(error_response(invoke, [0x55, 0x00], dtype, 0x0001));
    }
    // 블록 목록을 읽는다.
    let mut pos = 28;
    let mut vars = Vec::with_capacity(blocks);
    for _ in 0..blocks {
        if pos + 2 > frame.len() {
            return Some(error_response(invoke, [0x55, 0x00], dtype, 0x0001));
        }
        let len = usize::from(u16::from_le_bytes([frame[pos], frame[pos + 1]]));
        pos += 2;
        if len == 0 || len > 16 || pos + len > frame.len() {
            return Some(error_response(invoke, [0x55, 0x00], dtype, 0x0001));
        }
        let (w, num) = parse_name(&frame[pos..pos + len])?;
        pos += len;
        if w != is_word {
            return Some(error_response(invoke, [0x55, 0x00], dtype, 0x0001));
        }
        vars.push(num);
    }
    // 값을 읽는다.
    let mut data = Vec::new();
    if is_word {
        for &num in &vars {
            let Some(&v) = state.mw.get(num) else {
                return Some(error_response(invoke, [0x55, 0x00], dtype, 0x0011));
            };
            data.extend_from_slice(&2u16.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
    } else {
        for &num in &vars {
            let Some(&b) = state.mx.get(num) else {
                return Some(error_response(invoke, [0x55, 0x00], dtype, 0x0011));
            };
            data.extend_from_slice(&1u16.to_le_bytes());
            data.push(u8::from(b));
        }
    }
    // 응답: 머리 + 0x5500 + 종류 + 예약 + 오류 0 + 블록 수 + 블록별 데이터.
    let body_len = 2 + 2 + 2 + 2 + 2 + data.len();
    let mut out = Vec::with_capacity(20 + body_len);
    out.extend_from_slice(&COMPANY);
    out.extend_from_slice(&[0x00, 0x00]);
    out.push(0xA0);
    out.push(0x11);
    out.extend_from_slice(&invoke.to_le_bytes());
    out.extend_from_slice(&(body_len as u16).to_le_bytes());
    out.push(0x00);
    out.push(0x00);
    out.extend_from_slice(&[0x55, 0x00]);
    out.extend_from_slice(&dtype.to_le_bytes());
    out.extend_from_slice(&[0x00, 0x00]);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(vars.len() as u16).to_le_bytes());
    out.extend_from_slice(&data);
    Some(out)
}

/// 개별 쓰기(0x5800) 요청을 메모리에 반영한다. 성공하면 오류 상태 0 응답을 만든다.
fn respond_write(frame: &[u8], invoke: u16, state: &mut State) -> Option<Vec<u8>> {
    let dtype = u16::from_le_bytes([frame[22], frame[23]]);
    let is_word = match dtype {
        0x0002 => true,
        0x0000 => false,
        _ => return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0001)),
    };
    let blocks = usize::from(u16::from_le_bytes([frame[26], frame[27]]));
    if blocks == 0 || blocks > 16 {
        return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0001));
    }
    let mut pos = 28;
    let mut updates: Vec<(usize, u16, bool)> = Vec::with_capacity(blocks);
    for _ in 0..blocks {
        if pos + 2 > frame.len() {
            return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0001));
        }
        let len = usize::from(u16::from_le_bytes([frame[pos], frame[pos + 1]]));
        pos += 2;
        if len == 0 || len > 16 || pos + len > frame.len() {
            return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0001));
        }
        let (w, num) = parse_name(&frame[pos..pos + len])?;
        pos += len;
        if w != is_word {
            return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0001));
        }
        if pos + 2 > frame.len() {
            return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0001));
        }
        let count = usize::from(u16::from_le_bytes([frame[pos], frame[pos + 1]]));
        pos += 2;
        if is_word {
            if count != 2 || pos + 2 > frame.len() {
                return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0001));
            }
            let v = u16::from_le_bytes([frame[pos], frame[pos + 1]]);
            pos += 2;
            updates.push((num, v, false));
        } else {
            if count != 1 || pos + 1 > frame.len() {
                return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0001));
            }
            updates.push((num, 0, frame[pos] != 0));
            pos += 1;
        }
    }
    // 전부 검증한 뒤에 반영한다.
    for (num, v, b) in updates {
        if is_word {
            let Some(slot) = state.mw.get_mut(num) else {
                return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0011));
            };
            *slot = v;
        } else {
            let Some(slot) = state.mx.get_mut(num) else {
                return Some(error_response(invoke, [0x59, 0x00], dtype, 0x0011));
            };
            *slot = b;
        }
    }
    // 응답: 머리 + 0x5900 + 종류 + 예약 + 오류 0 + 블록 0.
    let body_len = 2 + 2 + 2 + 2 + 2;
    let mut out = Vec::with_capacity(20 + body_len);
    out.extend_from_slice(&COMPANY);
    out.extend_from_slice(&[0x00, 0x00]);
    out.push(0xA0);
    out.push(0x11);
    out.extend_from_slice(&invoke.to_le_bytes());
    out.extend_from_slice(&(body_len as u16).to_le_bytes());
    out.push(0x00);
    out.push(0x00);
    out.extend_from_slice(&[0x59, 0x00]);
    out.extend_from_slice(&dtype.to_le_bytes());
    out.extend_from_slice(&[0x00, 0x00]);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Some(out)
}

async fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>) -> io::Result<()> {
    loop {
        // 머리(20바이트)를 먼저 읽고 길이에 따라 본문을 읽는다.
        let mut head = [0u8; 20];
        stream.read_exact(&mut head).await?;
        let len = usize::from(u16::from_le_bytes([head[16], head[17]]));
        if len > 4096 {
            return Ok(());
        }
        let mut rest = vec![0u8; len];
        stream.read_exact(&mut rest).await?;
        let mut frame = Vec::with_capacity(20 + len);
        frame.extend_from_slice(&head);
        frame.extend_from_slice(&rest);

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

    fn request(vars: &[&str], dtype: u16) -> Vec<u8> {
        let mut body = vec![0x54, 0x00];
        body.extend_from_slice(&dtype.to_le_bytes());
        body.extend_from_slice(&[0x00, 0x00]);
        body.extend_from_slice(&(vars.len() as u16).to_le_bytes());
        for v in vars {
            body.extend_from_slice(&(v.len() as u16).to_le_bytes());
            body.extend_from_slice(v.as_bytes());
        }
        let mut frame = vec![
            0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00, 0x00, 0x00, 0xA0, 0x33,
            0x00, 0x00,
        ];
        frame.extend_from_slice(&(body.len() as u16).to_le_bytes());
        frame.extend_from_slice(&[0x00, 0x00]);
        frame.extend_from_slice(&body);
        frame
    }

    #[tokio::test]
    async fn mw와_mx_읽기_오류응답() {
        let sim = Simulator::start("127.0.0.1:0", 200, 64).await.unwrap();
        sim.set_mw(0, &[0x1234, 0x5678]);
        sim.set_mx(0, &[true, false]);
        let mut s = TcpStream::connect(sim.addr()).await.unwrap();

        s.write_all(&request(&["%MW0", "%MW1"], 0x0002))
            .await
            .unwrap();
        let mut head = [0u8; 20];
        s.read_exact(&mut head).await.unwrap();
        let len = usize::from(u16::from_le_bytes([head[16], head[17]]));
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).await.unwrap();
        assert_eq!(
            &body[..8],
            &[0x55, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            &body[8..16],
            &[0x02, 0x00, 0x02, 0x00, 0x34, 0x12, 0x02, 0x00]
        );
        assert_eq!(&body[16..18], &[0x78, 0x56]);

        s.write_all(&request(&["%MX0", "%MX1"], 0x0000))
            .await
            .unwrap();
        s.read_exact(&mut head).await.unwrap();
        let len = usize::from(u16::from_le_bytes([head[16], head[17]]));
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).await.unwrap();
        assert_eq!(
            &body[..10],
            &[0x55, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00]
        );
        assert_eq!(&body[10..16], &[0x01, 0x00, 0x01, 0x01, 0x00, 0x00]);

        // 없는 변수 → 오류 상태 0x0011.
        s.write_all(&request(&["%MW199", "%MW200"], 0x0002))
            .await
            .unwrap();
        s.read_exact(&mut head).await.unwrap();
        let len = usize::from(u16::from_le_bytes([head[16], head[17]]));
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).await.unwrap();
        assert_eq!(&body[6..8], &[0x11, 0x00]);
        assert_eq!(sim.state().requests_served, 3);
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;

    #[tokio::test]
    async fn 개별_쓰기가_메모리에_반영된다() {
        let sim = Simulator::start("127.0.0.1:0", 200, 64).await.unwrap();
        let mut s = TcpStream::connect(sim.addr()).await.unwrap();
        // %MW10에 0x1234 쓰기.
        let mut body = vec![0x58, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00];
        body.extend_from_slice(&[0x05, 0x00]);
        body.extend_from_slice(b"%MW10");
        body.extend_from_slice(&[0x02, 0x00, 0x34, 0x12]);
        let mut req = vec![
            0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00, 0x00, 0x00, 0xA0, 0x33,
            0x00, 0x00,
        ];
        req.extend_from_slice(&(body.len() as u16).to_le_bytes());
        req.extend_from_slice(&[0x00, 0x00]);
        req.extend_from_slice(&body);
        s.write_all(&req).await.unwrap();
        let mut head = [0u8; 20];
        s.read_exact(&mut head).await.unwrap();
        let len = usize::from(u16::from_le_bytes([head[16], head[17]]));
        let mut resp = vec![0u8; len];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(
            &resp[..8],
            &[0x59, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(sim.state().mw[10], 0x1234);
        // %MX0에 true 쓰기.
        let mut body = vec![0x58, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00];
        body.extend_from_slice(&[0x04, 0x00]);
        body.extend_from_slice(b"%MX0");
        body.extend_from_slice(&[0x01, 0x00, 0x01]);
        let mut req = vec![
            0x4C, 0x53, 0x49, 0x53, 0x2D, 0x58, 0x47, 0x54, 0x00, 0x00, 0x00, 0x00, 0xA0, 0x33,
            0x00, 0x00,
        ];
        req.extend_from_slice(&(body.len() as u16).to_le_bytes());
        req.extend_from_slice(&[0x00, 0x00]);
        req.extend_from_slice(&body);
        s.write_all(&req).await.unwrap();
        s.read_exact(&mut head).await.unwrap();
        let len = usize::from(u16::from_le_bytes([head[16], head[17]]));
        let mut resp = vec![0u8; len];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(&resp[6..8], &[0x00, 0x00]);
        assert!(sim.state().mx[0]);
    }
}
