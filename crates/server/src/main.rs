// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 서버 실행 파일.
//!
//! 사용: `nuriscada-server --config project.toml [--listen 127.0.0.1:8080] [--allow-external]`
//!
//! 이 조각에는 인증이 없다. 루프백이 아닌 주소에서 수신하려면 `--allow-external`을 줘야 하며,
//! 신뢰할 수 있는 네트워크에서 시험하는 용도로만 써야 한다.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use nuriscada_project::Project;
use nuriscada_scheduler::Hub;
use nuriscada_server::{router, start_scanners};
use tokio::net::TcpListener;

fn usage() -> ExitCode {
    eprintln!(
        "사용: nuriscada-server --config <project.toml> [--listen <주소:포트>] [--allow-external]"
    );
    ExitCode::from(2)
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut config = None;
    let mut listen = None;
    let mut allow_external = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = args.next(),
            "--listen" => listen = args.next(),
            "--allow-external" => allow_external = true,
            _ => return usage(),
        }
    }
    let Some(config) = config else {
        return usage();
    };
    let text = match std::fs::read_to_string(&config) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("설정 파일을 읽을 수 없다 ({config}): {e}");
            return ExitCode::FAILURE;
        }
    };
    let project = match Project::from_toml(&text) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let listen = listen.unwrap_or_else(|| project.server.listen.clone());
    let addr: SocketAddr = match listen.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("수신 주소가 올바르지 않다 ({listen}): {e}");
            return ExitCode::FAILURE;
        }
    };
    if !addr.ip().is_loopback() && !allow_external {
        eprintln!(
            "루프백이 아닌 주소({addr})에서 수신하려면 --allow-external이 필요하다. \
             이 버전에는 인증이 없으므로 신뢰할 수 있는 네트워크에서만 쓰라."
        );
        return ExitCode::FAILURE;
    }
    if !addr.ip().is_loopback() {
        eprintln!("경고: 인증 없이 {addr}에서 수신한다.");
    }

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("바인딩하지 못했다 ({addr}): {e}");
            return ExitCode::FAILURE;
        }
    };
    let bound = listener.local_addr().unwrap_or(addr);
    let hub = Hub::new(&project);
    let scanners = start_scanners(&project, &hub);
    println!("수신 시작: http://{bound}/api/v1/status");

    let app = router(hub, bound);
    let served = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    scanners.stop().await;
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("서버 오류: {e}");
            ExitCode::FAILURE
        }
    }
}
