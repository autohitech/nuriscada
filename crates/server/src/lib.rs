// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! REST·웹소켓 서버와 스캐너 실행.
//!
//! 기본 바인딩은 루프백이다. 루프백에서는 `Host`와 `Origin` 헤더를 검사해 다른 웹 페이지가
//! 브라우저를 통해 이 서버에 접근하지 못하게 한다(DNS 재바인딩 방어).
//! 운영 실행 파일은 계정·세션 인증을 사용한다. router/router_with_web은 읽기 전용 시험·시연용이다.

use std::net::{IpAddr, SocketAddr};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use nuriscada_driver_api::{BlockData, Driver, DriverError, LinkStatus, ReadBlock};
use nuriscada_driver_modbus_tcp::ModbusTcpClient;
use nuriscada_project::{DriverConfig, Project};
use nuriscada_scheduler::wire::encode_snapshot;
use nuriscada_scheduler::{Hub, PlanLimits, plan, run_device};
use serde_json::json;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

mod control;
/// 계정·세션·감사 기록 저장소.
pub mod operations;

/// 설정이 고른 드라이버를 감싸는 열거형. `Driver` 트레이트가 동적 호출이 안 되므로 열거형으로 다룬다.
pub enum AnyDriver {
    /// MODBUS TCP.
    ModbusTcp(ModbusTcpClient),
}

impl Driver for AnyDriver {
    async fn connect(&mut self) -> Result<(), DriverError> {
        match self {
            Self::ModbusTcp(d) => d.connect().await,
        }
    }

    async fn read(&mut self, block: &ReadBlock) -> Result<BlockData, DriverError> {
        match self {
            Self::ModbusTcp(d) => d.read(block).await,
        }
    }

    fn status(&self) -> LinkStatus {
        match self {
            Self::ModbusTcp(d) => d.status(),
        }
    }
}

/// 실행 중인 스캐너들.
pub struct Scanners {
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl Scanners {
    /// 스캔을 멈추고 끝날 때까지 기다린다.
    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        for t in self.tasks {
            let _ = t.await;
        }
    }
}

/// 프로젝트의 장치마다 스캔 작업을 시작한다.
pub fn start_scanners(project: &Arc<Project>, hub: &Arc<Hub>) -> Scanners {
    let blocks = plan(project, PlanLimits::MODBUS);
    let (shutdown, rx) = watch::channel(false);
    let mut tasks = vec![hub.spawn_publisher(project.server.publish, rx.clone())];
    for (index, device) in project.devices.iter().enumerate() {
        let driver = match &device.driver {
            DriverConfig::ModbusTcp { address } => {
                AnyDriver::ModbusTcp(ModbusTcpClient::new(address.clone(), device.timeout))
            }
        };
        let device_blocks = blocks
            .iter()
            .filter(|b| b.device == index)
            .cloned()
            .collect();
        tasks.push(tokio::spawn(run_device(
            Arc::clone(hub),
            Arc::clone(project),
            index,
            driver,
            device_blocks,
            rx.clone(),
        )));
    }
    Scanners { shutdown, tasks }
}

/// 요청 검사 설정.
#[derive(Clone)]
struct Guard {
    /// `Some`이면 `Host` 헤더의 호스트 이름이 이 목록에 있어야 한다(루프백 바인딩).
    allowed_hosts: Option<Arc<Vec<String>>>,
}

#[derive(Clone)]
struct AppState {
    operations: Option<Arc<operations::Operations>>,
    hub: Arc<Hub>,
    guard: Guard,
    /// 웹 UI 빌드 결과 디렉터리. 없으면 API만 제공한다.
    web_dir: Option<Arc<PathBuf>>,
}

/// 루프백 바인딩이면 허용할 호스트 이름 목록을 만든다.
fn allowed_hosts_for(listen: SocketAddr) -> Option<Vec<String>> {
    let loopback = match listen.ip() {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
    };
    loopback.then(|| {
        vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            "[::1]".to_string(),
        ]
    })
}

/// 호스트 부분만 꺼낸다(`host:port` → `host`, `[::1]:80` → `[::1]`).
fn host_of(authority: &str) -> &str {
    if let Some(end) = authority.strip_prefix('[').and_then(|r| r.find(']')) {
        return &authority[..end + 2];
    }
    authority.split(':').next().unwrap_or(authority)
}

fn forbidden(reason: &str) -> Response {
    (StatusCode::FORBIDDEN, reason.to_string()).into_response()
}

async fn host_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(allowed) = &state.guard.allowed_hosts {
        let headers: &HeaderMap = req.headers();
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(host_of);
        if !host.is_some_and(|h| allowed.iter().any(|a| a.eq_ignore_ascii_case(h))) {
            return forbidden("허용되지 않은 Host");
        }
        // 브라우저가 보낸 Origin이 있으면 같은 호스트여야 한다.
        if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            let origin_host = origin
                .split_once("://")
                .map_or(origin, |(_, rest)| rest)
                .split('/')
                .next()
                .unwrap_or("");
            if !allowed
                .iter()
                .any(|a| a.eq_ignore_ascii_case(host_of(origin_host)))
            {
                return forbidden("허용되지 않은 Origin");
            }
        }
    }
    next.run(req).await
}

/// 서버의 경로를 만든다(API만). `listen`은 실제로 바인딩한 주소다(루프백 여부로 검사를 정한다).
pub fn router(hub: Arc<Hub>, listen: SocketAddr) -> Router {
    router_with_web(hub, listen, None)
}

/// API와 웹 UI 정적 파일을 함께 제공하는 경로를 만든다. 모든 자원은 같은 출처에서 나오며
/// 외부 CDN에 의존하지 않는다.
pub fn router_with_web(hub: Arc<Hub>, listen: SocketAddr, web_dir: Option<PathBuf>) -> Router {
    build_router(hub, listen, web_dir, None)
}

/// 인증과 영구 감사 기록을 사용하는 운영 서버. 읽기 전용 시연 라우터와 구분한다.
pub fn router_with_operations(
    hub: Arc<Hub>,
    listen: SocketAddr,
    web_dir: Option<PathBuf>,
    operations: Arc<operations::Operations>,
) -> Router {
    build_router(hub, listen, web_dir, Some(operations))
}

fn build_router(
    hub: Arc<Hub>,
    listen: SocketAddr,
    web_dir: Option<PathBuf>,
    operations: Option<Arc<operations::Operations>>,
) -> Router {
    let state = AppState {
        operations,
        hub,
        guard: Guard {
            allowed_hosts: allowed_hosts_for(listen).map(Arc::new),
        },
        web_dir: web_dir.map(Arc::new),
    };
    Router::new()
        .route("/api/v1/auth/session", get(control::session))
        .route("/api/v1/auth/login", post(control::login))
        .route("/api/v1/auth/logout", post(control::logout))
        .route("/api/v1/users", post(control::create_user))
        .route("/api/v1/audit", get(control::audit))
        .route("/api/v1/history/alarms", get(control::alarm_history))
        .route(
            "/api/v1/history/alarms/{id}/ack",
            post(control::acknowledge),
        )
        .route("/api/v1/control/tags", get(control::control_tags))
        .route("/api/v1/control/tags/{name}", post(control::write_tag))
        .route("/api/v1/tags", get(tags))
        .route("/api/v1/alarms", get(alarms))
        .route("/api/v1/status", get(status))
        .route("/api/v1/stream", get(stream))
        .fallback(static_files)
        .layer(axum::extract::DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            control::guard,
        ))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn_with_state(state.clone(), host_guard))
        .with_state(state)
}

/// 모든 응답에 붙이는 보안 헤더.
async fn security_headers(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; connect-src 'self'; img-src 'self' data:; \
             style-src 'self' 'unsafe-inline'; script-src 'self'; base-uri 'none'; \
             form-action 'none'; frame-ancestors 'none'",
        ),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if api {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// 웹 UI 정적 파일. 디렉터리 밖으로 나가는 경로는 거부하고, 확장자 없는 경로는 `index.html`로 돌린다.
async fn static_files(State(s): State<AppState>, uri: Uri) -> Response {
    let Some(root) = &s.web_dir else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let rel = uri.path().trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let rel_path = Path::new(rel);
    if rel_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let canonical_root = match tokio::fs::canonicalize(root.as_path()).await {
        Ok(r) => r,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let try_read = |p: PathBuf| {
        let canonical_root = canonical_root.clone();
        async move {
            // 심볼릭 링크로 디렉터리 밖을 가리키는 경우도 막는다.
            let real = tokio::fs::canonicalize(&p).await.ok()?;
            if !real.starts_with(&canonical_root) {
                return None;
            }
            let bytes = tokio::fs::read(&real).await.ok()?;
            Some((real, bytes))
        }
    };

    let found = match try_read(root.join(rel_path)).await {
        Some(f) => Some(f),
        // 확장자가 없는 경로는 화면 안의 경로로 보고 첫 화면을 돌려준다.
        None if rel_path.extension().is_none() => try_read(root.join("index.html")).await,
        None => None,
    };
    let Some((real, bytes)) = found else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let cache = if real.components().any(|c| c.as_os_str() == "assets") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, content_type(&real)),
            (header::CACHE_CONTROL, cache),
        ],
        bytes,
    )
        .into_response()
}

async fn tags(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "tags": s.hub.tags() }))
}

async fn alarms(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "active": s.hub.active_alarms(), "recent": s.hub.recent_alarms() }))
}

async fn status(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": s.hub.uptime_secs(),
        "tag_count": s.hub.tags().len(),
        "devices": s.hub.devices(),
        "alarm_archive": s.operations.as_ref().map(|o|o.archive_status()),
    }))
}

/// 큰 스냅샷을 복사·부호화하는 동안 비동기 스캔 작업을 막지 않는다.
async fn snapshot_message(hub: &Arc<Hub>) -> Result<(Message, u64), tokio::task::JoinError> {
    // 동시 접속이 몰려도 큰 스냅샷을 복사·부호화하는 작업 수를 제한한다.
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let permit = SLOTS.acquire().await.expect("스냅샷 세마포어");
    let hub = Arc::clone(hub);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let snap = hub.snapshot();
        let seq = snap.seq;
        (Message::Text(encode_snapshot(&snap).into()), seq)
    })
    .await
}

async fn stream(ws: WebSocketUpgrade, State(s): State<AppState>, headers: HeaderMap) -> Response {
    let auth = s
        .operations
        .map(|ops| (ops, control::token(&headers).unwrap_or_default()));
    ws.on_upgrade(move |socket| push_updates(socket, s.hub, auth))
}

/// 연결할 때 전체 스냅샷을 보내고, 이후 변경 묶음과 알람 기록을 밀어 보낸다.
///
/// 스냅샷보다 먼저 구독하고, 스냅샷에 이미 반영된 묶음(순번이 같거나 작은 것)은 버린다.
/// 그래서 연결 직후의 변경을 놓치지도, 오래된 값으로 되돌리지도 않는다.
async fn push_updates(
    mut socket: WebSocket,
    hub: Arc<Hub>,
    auth: Option<(Arc<operations::Operations>, String)>,
) {
    let mut rx = hub.subscribe_wire();
    let Ok((first, mut covered)) = snapshot_message(&hub).await else {
        return;
    };
    if socket.send(first).await.is_err() {
        return;
    }
    let mut expiry = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tokio::select! {
            _=expiry.tick(),if auth.is_some()=>{
                let (ops,token)=auth.as_ref().expect("인증 경로");
                let ops=ops.clone();let token=token.clone();
                if !matches!(tokio::task::spawn_blocking(move||ops.authenticate(&token)).await,Ok(Ok(Some(_)))) {return;}
            }
            frame = rx.recv() => match frame {
                Ok(frame) => {
                    if frame.seq.is_some_and(|seq| seq <= covered) {
                        continue;
                    }
                    if socket.send(Message::Text(frame.text.as_ref().into())).await.is_err() {
                        return;
                    }
                }
                // 너무 느려서 일부를 놓쳤다: 전체 스냅샷으로 다시 맞춘다.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let Ok((msg, seq)) = snapshot_message(&hub).await else {
                        return;
                    };
                    covered = seq;
                    if socket.send(msg).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            incoming = socket.recv() => match incoming {
                None | Some(Err(_) | Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_parsing() {
        assert_eq!(host_of("localhost:8080"), "localhost");
        assert_eq!(host_of("127.0.0.1"), "127.0.0.1");
        assert_eq!(host_of("[::1]:8080"), "[::1]");
        assert_eq!(host_of("evil.example:80"), "evil.example");
    }

    #[test]
    fn only_loopback_gets_a_host_guard() {
        assert!(allowed_hosts_for("127.0.0.1:1".parse().unwrap()).is_some());
        assert!(allowed_hosts_for("[::1]:1".parse().unwrap()).is_some());
        assert!(allowed_hosts_for("0.0.0.0:1".parse().unwrap()).is_none());
        assert!(allowed_hosts_for("192.168.0.5:1".parse().unwrap()).is_none());
    }
}
