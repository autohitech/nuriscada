// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시뮬레이터 실행 파일. 개발·시연 전용이다.
//!
//! 사용: `nuriscada-sim-modbus [--listen 127.0.0.1:5020] [--size 1000] [--demo]`
//!
//! `--demo`는 보유 레지스터 0번 값을 0~100 사이에서 100 ms마다 삼각파로 바꾸고,
//! 코일 0번을 5초마다 뒤집는다.

use std::time::Duration;

use nuriscada_sim_modbus::Simulator;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut listen = "127.0.0.1:5020".to_string();
    let mut size = 1000usize;
    let mut demo = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => listen = args.next().unwrap_or(listen),
            "--size" => size = args.next().and_then(|v| v.parse().ok()).unwrap_or(size),
            "--demo" => demo = true,
            "--version" | "-V" => {
                println!("nuriscada-sim-modbus {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                eprintln!("알 수 없는 인자: {other}");
                std::process::exit(2);
            }
        }
    }
    let sim = Simulator::start(&listen, size).await?;
    println!("시뮬레이터 시작: {}", sim.addr());
    if demo {
        let mut tick: u32 = 0;
        loop {
            // 0..=100..=0 삼각파(주기 200틱).
            let phase = tick % 200;
            let value = if phase <= 100 { phase } else { 200 - phase };
            sim.set_holding(0, &[u16::try_from(value).unwrap_or(0)]);
            if tick.is_multiple_of(50) {
                let on = (tick / 50).is_multiple_of(2);
                sim.set_coils(0, &[on]);
            }
            tick = tick.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    } else {
        std::future::pending::<()>().await;
    }
    Ok(())
}
