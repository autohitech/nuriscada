// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 서버 실행 파일.
//!
//! 사용: `nuriscada-server --config project.toml [--listen 127.0.0.1:8080] [--web-dir web/dist] [--allow-external]`
//!
//! `--web-dir`을 주지 않으면 실행 파일 옆의 `web`(배포 묶음), 실행 파일 상위의 `web`(`bin/` 아래 설치),
//! 현재 디렉터리의 `web/dist`(개발) 순서로 찾는다.
//!
//! 운영 서버는 로컬 계정 인증을 요구한다. 최초 관리자 설정은 --init-admin으로 수행한다.
//! 외부 수신은 --allow-external과 HTTPS 프록시를 사용하며, 공개 배치는 --public-read-only로 고정한다.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use nuriscada_project::Project;
use nuriscada_scheduler::Hub;
use nuriscada_server::{
    operations::{Operations, Role},
    router_with_operations, start_scanners,
};
use tokio::net::TcpListener;

/// 웹 화면 디렉터리를 찾는다: 배포 묶음의 `web`, 개발 중의 `web/dist`.
fn default_web_dir() -> Option<std::path::PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
    let mut candidates = Vec::new();
    if let Some(dir) = exe_dir {
        candidates.push(dir.join("web"));
        candidates.push(dir.join("..").join("web"));
    }
    candidates.push(std::path::PathBuf::from("web/dist"));
    candidates
        .into_iter()
        .find(|c| c.join("index.html").is_file())
}

fn usage() -> ExitCode {
    eprintln!(
        "사용: nuriscada-server --config <project.toml> [--data-dir <경로>] [--init-admin <계정>] [--password-stdin] [--enable-writes] [--public-read-only] [--listen <주소:포트>] [--allow-external] [--web-dir <경로>]"
    );
    ExitCode::from(2)
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut config = None;
    let mut listen = None;
    let mut web_dir: Option<String> = None;
    let mut allow_external = false;
    let mut data_dir: Option<std::path::PathBuf> = None;
    let mut init_admin = None;
    let mut password_stdin = false;
    let mut enable_writes = false;
    let mut public_read_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = args.next(),
            "--listen" => listen = args.next(),
            "--web-dir" => web_dir = args.next(),
            "--allow-external" => allow_external = true,
            "--data-dir" => data_dir = args.next().map(Into::into),
            "--init-admin" => init_admin = args.next(),
            "--password-stdin" => password_stdin = true,
            "--enable-writes" => enable_writes = true,
            "--public-read-only" => public_read_only = true,
            "--version" | "-V" => {
                println!("nuriscada-server {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                usage();
                return ExitCode::SUCCESS;
            }
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

    let data_dir = data_dir.unwrap_or_else(|| {
        std::path::Path::new(&config)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".nuriscada")
    });
    let new_dir = !data_dir.exists();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("데이터 디렉터리 생성 실패: {e}");
        return ExitCode::FAILURE;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if new_dir
            && let Err(e) =
                std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))
        {
            eprintln!("데이터 디렉터리 권한 설정 실패: {e}");
            return ExitCode::FAILURE;
        }
        if std::fs::metadata(&data_dir)
            .map(|m| m.permissions().mode() & 0o077 != 0)
            .unwrap_or(true)
        {
            eprintln!("데이터 디렉터리는 소유자만 접근할 수 있어야 한다 (0700)");
            return ExitCode::FAILURE;
        }
    }
    #[cfg(not(unix))]
    let _ = new_dir;
    let operations = match Operations::open_with_config(
        &data_dir.join("operations.sqlite3"),
        Arc::clone(&project),
        Some(std::path::PathBuf::from(&config)),
        enable_writes,
        public_read_only,
    ) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("운영 저장소 열기 실패: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(name) = init_admin {
        let password = if password_stdin {
            let mut line = String::new();
            if let Err(e) = std::io::stdin().read_line(&mut line) {
                eprintln!("비밀번호 읽기 실패: {e}");
                return ExitCode::FAILURE;
            }
            line.trim_end_matches(['\r', '\n']).to_string()
        } else {
            match rpassword::prompt_password("최초 관리자 비밀번호 (12~128바이트): ") {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("비밀번호 읽기 실패: {e}");
                    return ExitCode::FAILURE;
                }
            }
        };
        return match operations.create_user(
            &name,
            &password,
            Role::Admin,
            true,
            "local-cli",
            "local-cli",
        ) {
            Ok(()) => {
                println!("최초 관리자 설정 완료");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("관리자 설정 실패: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if operations.needs_setup().unwrap_or(true) {
        eprintln!("먼저 같은 설정·데이터 경로로 --init-admin <계정>을 실행하라. 기본 계정은 없다.");
        return ExitCode::FAILURE;
    }

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
             외부 구성은 HTTPS와 읽기 전용 공개 모드를 사용하라."
        );
        return ExitCode::FAILURE;
    }
    if !addr.ip().is_loopback() {
        eprintln!("인증을 사용하는 외부 읽기 전용 수신: {addr}. HTTPS 프록시가 필요하다.");
    }
    if enable_writes && !addr.ip().is_loopback() {
        eprintln!("쓰기 활성화는 루프백 바인딩에서만 가능하다");
        return ExitCode::FAILURE;
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
    let archive = operations.start_archive(Arc::clone(&hub));
    let scanners = start_scanners(&project, &hub);
    operations.attach_scanners(scanners).await;
    println!("수신 시작: http://{bound}/api/v1/status");

    let web_dir = web_dir
        .map(std::path::PathBuf::from)
        .or_else(default_web_dir);
    if let Some(dir) = &web_dir {
        if let (Ok(web), Ok(data)) = (std::fs::canonicalize(dir), std::fs::canonicalize(&data_dir))
            && data.starts_with(web)
        {
            eprintln!("데이터 디렉터리는 웹 자원 디렉터리 밖에 있어야 한다");
            operations.stop_scanners().await;
            return ExitCode::FAILURE;
        }
        println!("웹 화면: http://{bound}/ ({})", dir.display());
    }
    let app = router_with_operations(hub, bound, web_dir, Arc::clone(&operations));
    let served = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await;
    operations.stop_scanners().await;
    archive.stop().await;
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("서버 오류: {e}");
            ExitCode::FAILURE
        }
    }
}
