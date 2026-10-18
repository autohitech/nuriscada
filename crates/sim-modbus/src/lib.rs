// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시험용 MODBUS TCP 시뮬레이터. 개발·CI 전용이며 배포물에 포함하지 않는다.
//!
//! 읽기 기능 코드 1~4를 지원한다. 실행 중에 메모리 값을 바꾸거나 응답을 늦추고 끊을 수 있어서
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
    /// 코일.
    pub coils: Vec<bool>,
    /// 이산 입력.
    pub discrete_inputs: Vec<bool>,
    /// 보유 레지스터.
    pub holding_registers: Vec<u16>,
    /// 입력 레지스터.
    pub input_registers: Vec<u16>,
    /// 응답 전에 기다리는 시간.
    pub response_delay: Duration,
    /// true이면 요청을 받아도 응답하지 않고 연결을 닫는다.
    pub drop_connections: bool,
    /// 받은 요청 수.
    pub requests_served: u64,
}

impl State {
    fn new(size: usize) -> Self {
        Self {
            coils: vec![false; size],
            discrete_inputs: vec![false; size],
            holding_registers: vec![0; size],
            input_registers: vec![0; size],
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
    /// `bind` 주소(예: `127.0.0.1:0`)에서 시작한다. 각 메모리 영역의 크기는 `size`다.
    pub async fn start(bind: &str, size: usize) -> io::Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(Mutex::new(State::new(size)));
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

    /// 보유 레지스터 값을 쓴다.
    pub fn set_holding(&self, start: usize, values: &[u16]) {
        self.state().holding_registers[start..start + values.len()].copy_from_slice(values);
    }

    /// 입력 레지스터 값을 쓴다.
    pub fn set_input(&self, start: usize, values: &[u16]) {
        self.state().input_registers[start..start + values.len()].copy_from_slice(values);
    }

    /// 코일 값을 쓴다.
    pub fn set_coils(&self, start: usize, values: &[bool]) {
        self.state().coils[start..start + values.len()].copy_from_slice(values);
    }

    /// 이산 입력 값을 쓴다.
    pub fn set_discrete(&self, start: usize, values: &[bool]) {
        self.state().discrete_inputs[start..start + values.len()].copy_from_slice(values);
    }
}

impl Drop for Simulator {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 오류 응답을 만든다.
fn exception(txn: [u8; 2], unit: u8, function: u8, code: u8) -> Vec<u8> {
    vec![txn[0], txn[1], 0, 0, 0, 3, unit, function | 0x80, code]
}

/// 요청 프레임(머리 포함)에 대한 응답을 만든다. 응답하지 않으면 `None`이다.
fn respond(frame: &[u8], state: &mut State) -> Option<Vec<u8>> {
    let txn = [frame[0], frame[1]];
    let unit = frame[6];
    let function = frame[7];
    if frame.len() != 12 || !(1..=6).contains(&function) {
        return Some(exception(txn, unit, function, 1));
    }
    let start = usize::from(u16::from_be_bytes([frame[8], frame[9]]));
    let count = usize::from(u16::from_be_bytes([frame[10], frame[11]]));
    if function == 5 || function == 6 {
        let size = if function == 5 {
            state.coils.len()
        } else {
            state.holding_registers.len()
        };
        if start >= size {
            return Some(exception(txn, unit, function, 2));
        }
        if function == 5 {
            if count != 0 && count != 0xff00 {
                return Some(exception(txn, unit, function, 3));
            }
            state.coils[start] = count == 0xff00;
        } else {
            state.holding_registers[start] = count as u16;
        }
        return Some(frame.to_vec());
    }
    let is_bit = function <= 2;
    let max = if is_bit { 2000 } else { 125 };
    if count == 0 || count > max {
        return Some(exception(txn, unit, function, 3));
    }
    let size = match function {
        1 => state.coils.len(),
        2 => state.discrete_inputs.len(),
        3 => state.holding_registers.len(),
        _ => state.input_registers.len(),
    };
    if start + count > size {
        return Some(exception(txn, unit, function, 2));
    }
    let mut data = Vec::new();
    if is_bit {
        let bits = if function == 1 {
            &state.coils
        } else {
            &state.discrete_inputs
        };
        data.resize(count.div_ceil(8), 0u8);
        for (i, &bit) in bits[start..start + count].iter().enumerate() {
            if bit {
                data[i / 8] |= 1 << (i % 8);
            }
        }
    } else {
        let regs = if function == 3 {
            &state.holding_registers
        } else {
            &state.input_registers
        };
        for &r in &regs[start..start + count] {
            data.extend_from_slice(&r.to_be_bytes());
        }
    }
    let byte_count = u8::try_from(data.len()).ok()?;
    let length = u16::try_from(3 + data.len()).ok()?;
    let mut out = vec![txn[0], txn[1], 0, 0];
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&[unit, function, byte_count]);
    out.extend_from_slice(&data);
    Some(out)
}

async fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>) -> io::Result<()> {
    loop {
        let mut header = [0u8; 7];
        stream.read_exact(&mut header).await?;
        let len = usize::from(u16::from_be_bytes([header[4], header[5]]));
        if !(2..=254).contains(&len) {
            return Ok(());
        }
        let mut frame = vec![0u8; 6 + len];
        frame[..7].copy_from_slice(&header);
        stream.read_exact(&mut frame[7..]).await?;

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
    async fn answers_raw_read_and_exceptions() {
        let sim = Simulator::start("127.0.0.1:0", 100).await.unwrap();
        sim.set_holding(10, &[7, 8]);
        let mut s = TcpStream::connect(sim.addr()).await.unwrap();

        // 보유 레지스터 10부터 2개.
        s.write_all(&[0, 1, 0, 0, 0, 6, 1, 3, 0, 10, 0, 2])
            .await
            .unwrap();
        let mut resp = [0u8; 13];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp, [0, 1, 0, 0, 0, 7, 1, 3, 4, 0, 7, 0, 8]);

        // 범위 밖 주소 → 오류 코드 2.
        s.write_all(&[0, 2, 0, 0, 0, 6, 1, 3, 0, 99, 0, 2])
            .await
            .unwrap();
        let mut err = [0u8; 9];
        s.read_exact(&mut err).await.unwrap();
        assert_eq!(err, [0, 2, 0, 0, 0, 3, 1, 0x83, 2]);

        // 지원하지 않는 기능 → 오류 코드 1.
        s.write_all(&[0, 3, 0, 0, 0, 6, 1, 7, 0, 0, 0, 1])
            .await
            .unwrap();
        s.read_exact(&mut err).await.unwrap();
        assert_eq!(err, [0, 3, 0, 0, 0, 3, 1, 0x87, 1]);
        assert_eq!(sim.state().requests_served, 3);
    }
}
