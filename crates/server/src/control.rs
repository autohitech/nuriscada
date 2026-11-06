// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 인증·관리·쓰기 API. 실제 TCP 접속 출처를 사용하며 프록시 헤더는 신뢰하지 않는다.

use std::net::SocketAddr;
use std::sync::Arc;

use crate::error_code::Code;
use axum::{
    Extension, Json,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use nuriscada_driver_api::DataKind;
use nuriscada_driver_fins_tcp::FinsTcpClient;
use nuriscada_driver_melsec_mc::MelsecMcClient;
use nuriscada_driver_modbus_tcp::ModbusTcpClient;
use nuriscada_driver_s7::S7Client;
use nuriscada_driver_xgt_fenet::XgtFenetClient;
use nuriscada_project::{DriverConfig, Space};
use nuriscada_scheduler::{Quality, now_ms};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AppState,
    operations::{Event, Operations, Principal, Role},
};

#[derive(Clone)]
pub(super) struct Peer {
    source: String,
    loopback: bool,
}

type Error = (StatusCode, Json<Value>);
type Result<T> = std::result::Result<T, Error>;
fn error(status: StatusCode, code: Code) -> Error {
    code.response(status, None)
}
fn error_detail(status: StatusCode, code: Code, detail: &str) -> Error {
    code.response(status, Some(detail))
}
fn internal(e: impl std::fmt::Display) -> Error {
    crate::log::emit("error", "api", &e.to_string());
    error(StatusCode::INTERNAL_SERVER_ERROR, Code::Internal)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> crate::operations::Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(internal)?
        .map_err(internal)
}

pub(super) fn token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == "nuriscada_session"
                && value.len() == 64
                && value.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| value.to_string())
        })
}

pub(super) async fn guard(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let Some(ops) = &state.operations else {
        return next.run(req).await;
    };
    if !req.uri().path().starts_with("/api/") {
        return next.run(req).await;
    }
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| Peer {
            source: c.0.to_string(),
            loopback: c.0.ip().is_loopback(),
        })
        .unwrap_or(Peer {
            source: "unknown".into(),
            loopback: false,
        });
    req.extensions_mut().insert(peer);
    if ops.public_read_only
        && !matches!(req.method().as_str(), "GET" | "HEAD")
        && !matches!(
            req.uri().path(),
            "/api/v1/auth/login" | "/api/v1/auth/logout"
        )
    {
        return error(StatusCode::FORBIDDEN, Code::PublicReadOnly).into_response();
    }
    if let Some(origin) = req.headers().get(header::ORIGIN) {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if !origin
            .to_str()
            .is_ok_and(|o| o == format!("http://{host}") || o == format!("https://{host}"))
        {
            return error(StatusCode::FORBIDDEN, Code::CrossOrigin).into_response();
        }
    }
    if !matches!(req.method().as_str(), "GET" | "HEAD")
        && req
            .headers()
            .get("x-nuriscada-request")
            .and_then(|v| v.to_str().ok())
            != Some("1")
    {
        return error(StatusCode::FORBIDDEN, Code::MissingHeader).into_response();
    }
    if matches!(
        req.uri().path(),
        "/api/v1/auth/login" | "/api/v1/auth/session"
    ) {
        return next.run(req).await;
    }
    let Some(token) = token(req.headers()) else {
        return error(StatusCode::UNAUTHORIZED, Code::LoginRequired).into_response();
    };
    let ops = Arc::clone(ops);
    match blocking(move || ops.authenticate(&token)).await {
        Ok(Some(principal)) => {
            req.extensions_mut().insert(principal);
            next.run(req).await
        }
        Ok(None) => error(StatusCode::UNAUTHORIZED, Code::SessionExpired).into_response(),
        Err(e) => e.into_response(),
    }
}

fn operations(state: &AppState) -> Result<Arc<Operations>> {
    state
        .operations
        .clone()
        .ok_or_else(|| error(StatusCode::NOT_FOUND, Code::ReadOnlyDemo))
}

pub(super) async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let Some(ops) = state.operations.clone() else {
        return Ok(Json(
            json!({"authentication_required":false,"writes_enabled":false,"user":null}),
        ));
    };
    let t = token(&headers).unwrap_or_default();
    let cloned = ops.clone();
    let (user, setup) =
        blocking(move || Ok((cloned.authenticate(&t)?, cloned.needs_setup()?))).await?;
    Ok(Json(
        json!({"authentication_required":true,"needs_setup":setup,"user":user,
        "local_management":!ops.public_read_only && state.guard.allowed_hosts.is_some(),
        "writes_enabled":ops.writes_enabled && state.guard.allowed_hosts.is_some()}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Credentials {
    username: String,
    password: String,
}

pub(super) async fn login(
    State(state): State<AppState>,
    Extension(peer): Extension<Peer>,
    Json(input): Json<Credentials>,
) -> Result<Response> {
    let ops = operations(&state)?;
    if input.username.len() > 64 || input.password.len() > 128 {
        return Err(error(StatusCode::BAD_REQUEST, Code::InputTooLong));
    }
    let permit = ops
        .login_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| error(StatusCode::TOO_MANY_REQUESTS, Code::RateLimited))?;
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        ops.login(&input.username, &input.password, &peer.source)
    })
    .await
    .map_err(internal)?;
    let t = match result {
        Ok(Some(t)) => t,
        Ok(None) => {
            return Err(error(StatusCode::UNAUTHORIZED, Code::BadCredentials));
        }
        Err(e) if e.to_string() == "로그인 시도 제한" => {
            return Err(error(StatusCode::TOO_MANY_REQUESTS, Code::LoginLocked));
        }
        Err(e) => return Err(internal(e)),
    };
    let secure = if state.guard.allowed_hosts.is_none() {
        "; Secure"
    } else {
        ""
    };
    Ok(([(header::SET_COOKIE,format!("nuriscada_session={t}; Path=/; HttpOnly; SameSite=Strict; Max-Age=28800{secure}")),(header::CACHE_CONTROL,"no-store".into())],Json(json!({"ok":true}))).into_response())
}

pub(super) async fn logout(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
    Extension(peer): Extension<Peer>,
    headers: HeaderMap,
) -> Result<Response> {
    let ops = operations(&state)?;
    let t = token(&headers).unwrap_or_default();
    blocking(move || ops.logout(&t, &user.username, &peer.source)).await?;
    Ok((
        [
            (
                header::SET_COOKIE,
                "nuriscada_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Json(json!({"ok":true})),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NewUser {
    username: String,
    password: String,
    role: Role,
}
pub(super) async fn create_user(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
    Extension(peer): Extension<Peer>,
    Json(input): Json<NewUser>,
) -> Result<Json<Value>> {
    if user.role != Role::Admin {
        return Err(error(StatusCode::FORBIDDEN, Code::AdminRequired));
    }
    if !peer.loopback || state.guard.allowed_hosts.is_none() {
        return Err(error(StatusCode::FORBIDDEN, Code::AccountsLocalOnly));
    }
    let ops = operations(&state)?;
    let permit = ops
        .login_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| error(StatusCode::TOO_MANY_REQUESTS, Code::RateLimited))?;
    blocking(move || {
        let _permit = permit;
        ops.create_user(
            &input.username,
            &input.password,
            input.role,
            false,
            &user.username,
            &peer.source,
        )
    })
    .await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
pub(super) struct Page {
    #[serde(default)]
    after: u64,
}

#[derive(Deserialize)]
pub(super) struct AlarmPage {
    #[serde(default)]
    before: u64,
    tag: Option<String>,
}

pub(super) async fn alarm_history(
    State(state): State<AppState>,
    Query(page): Query<AlarmPage>,
) -> Result<Json<Value>> {
    let ops = operations(&state)?;
    if page.tag.as_ref().is_some_and(|t| t.len() > 256) {
        return Err(error(StatusCode::BAD_REQUEST, Code::TagNameTooLong));
    }
    let rows = blocking(move || ops.alarm_page(page.before, page.tag)).await?;
    Ok(Json(json!({"records":rows})))
}

#[derive(Deserialize)]
pub(super) struct TagHistoryQuery {
    tag: String,
    from_ms: Option<u64>,
    to_ms: Option<u64>,
    #[serde(default = "default_history_limit")]
    limit: usize,
    format: Option<String>,
}

fn default_history_limit() -> usize {
    500
}

pub(super) async fn tag_history(
    State(state): State<AppState>,
    Query(q): Query<TagHistoryQuery>,
) -> Result<Response> {
    let ops = operations(&state)?;
    if q.tag.is_empty() || q.tag.len() > 256 {
        return Err(error(StatusCode::BAD_REQUEST, Code::TagNameInvalid));
    }
    let tag_name = q.tag.clone();
    let rows = blocking(move || ops.tag_history(&q.tag, q.from_ms, q.to_ms, q.limit)).await?;
    if q.format.as_deref() == Some("csv") {
        let mut csv = String::from("id,time_ms,tag,kind,value,quality,alarm_state\n");
        for r in &rows {
            let id = r["id"].as_i64().unwrap_or(0);
            let time_ms = r["time_ms"].as_i64().unwrap_or(0);
            let kind = r["kind"].as_str().unwrap_or("");
            let val = match &r["value"] {
                Value::Null => String::new(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                other => other.to_string(),
            };
            let quality = r["quality"].as_str().unwrap_or("");
            let alarm = r["alarm_state"].as_str().unwrap_or("");
            let escaped_tag = format!("\"{}\"", tag_name.replace('"', "\"\""));
            csv.push_str(&format!(
                "{id},{time_ms},{escaped_tag},{kind},{val},{quality},{alarm}\n"
            ));
        }
        return Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8")], csv).into_response());
    }
    Ok(Json(json!({"tag": tag_name, "samples": rows})).into_response())
}

pub(super) async fn acknowledge(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
    Extension(peer): Extension<Peer>,
    Path(id): Path<u64>,
) -> Result<Json<Value>> {
    let ops = operations(&state)?;
    if user.role == Role::Viewer
        || ops.public_read_only
        || !peer.loopback
        || state.guard.allowed_hosts.is_none()
    {
        return Err(error(StatusCode::FORBIDDEN, Code::AckForbidden));
    }
    let changed = blocking(move || ops.acknowledge(id, &user.username, &peer.source)).await?;
    Ok(Json(json!({"ok":true,"changed":changed})))
}
pub(super) async fn audit(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
    Query(page): Query<Page>,
) -> Result<Json<Value>> {
    if user.role != Role::Admin {
        return Err(error(StatusCode::FORBIDDEN, Code::AdminRequired));
    }
    let ops = operations(&state)?;
    let rows = blocking(move || {
        ops.verify_audit()?;
        ops.audit_page(page.after)
    })
    .await?;
    Ok(Json(json!({"events":rows})))
}
/// 진단 번들: 버전·가동 시간·장치 진단·보관 상태·프로젝트 원문. 관리자만 로컬에서 받는다.
pub(super) async fn diagnostics(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
    Extension(peer): Extension<Peer>,
) -> Result<Json<Value>> {
    let ops = operations(&state)?;
    if user.role != Role::Admin
        || ops.public_read_only
        || !peer.loopback
        || state.guard.allowed_hosts.is_none()
    {
        return Err(error(StatusCode::FORBIDDEN, Code::AdminRequired));
    }
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": state.hub.uptime_secs(),
        "devices": state.hub.devices(),
        "alarm_archive": ops.archive_status(),
        "project_toml": ops.project().to_toml(),
    })))
}

pub(super) async fn control_tags(State(state): State<AppState>) -> Result<Json<Value>> {
    let ops = operations(&state)?;
    let project = ops.project();
    Ok(Json(
        json!({"tags":project.tags.iter().filter(|t|t.writable).map(|t|json!({"name":t.name,"kind":if t.space.kind()==DataKind::Bit{"bool"}else{"word"}})).collect::<Vec<_>>()}),
    ))
}

pub(super) async fn get_project(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
) -> Result<Json<Value>> {
    if user.role != Role::Admin {
        return Err(error(StatusCode::FORBIDDEN, Code::AdminRequired));
    }
    let ops = operations(&state)?;
    let project = ops.project();
    Ok(Json(json!({
        "project": project.to_spec(),
        "toml": project.to_toml(),
    })))
}

pub(super) async fn update_project(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
    Extension(peer): Extension<Peer>,
    headers: HeaderMap,
    Json(spec): Json<nuriscada_project::ProjectSpec>,
) -> Result<Json<Value>> {
    let ops = operations(&state)?;
    if user.role != Role::Admin
        || ops.public_read_only
        || !peer.loopback
        || state.guard.allowed_hosts.is_none()
    {
        return Err(error(StatusCode::FORBIDDEN, Code::ProjectLocalAdminOnly));
    }
    let next = Arc::new(nuriscada_project::Project::from_spec(spec).map_err(|e| {
        error_detail(
            StatusCode::BAD_REQUEST,
            Code::ProjectInvalid,
            &e.to_string(),
        )
    })?);
    let _lock = ops.write_lock.lock().await;
    let check = ops.clone();
    let t = token(&headers).unwrap_or_default();
    if blocking(move || check.authenticate(&t)).await?.is_none() || user.expires_ms <= now_ms() {
        return Err(error(StatusCode::UNAUTHORIZED, Code::SessionExpired));
    }
    let db = ops.clone();
    let actor = user.username.clone();
    let source = peer.source.clone();
    let saved = Arc::clone(&next);
    blocking(move || db.save_project(saved, &actor, &source)).await?;
    ops.stop_scanners().await;
    state.hub.reconfigure(&next);
    ops.attach_scanners(crate::start_scanners(&next, &state.hub))
        .await;
    Ok(Json(json!({
        "ok": true,
        "project": next.to_spec(),
        "toml": next.to_toml(),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportProjectRequest {
    text: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    default_device: Option<String>,
    /// `simple`(기본, NuriScada 가져오기 형식) 또는 `tagx`(기존 제품의 태그 파일).
    #[serde(default)]
    format: Option<String>,
}

pub(super) async fn import_project(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
    Extension(peer): Extension<Peer>,
    headers: HeaderMap,
    Json(input): Json<ImportProjectRequest>,
) -> Result<Json<Value>> {
    let ops = operations(&state)?;
    if user.role != Role::Admin
        || ops.public_read_only
        || !peer.loopback
        || state.guard.allowed_hosts.is_none()
    {
        return Err(error(StatusCode::FORBIDDEN, Code::ImportLocalAdminOnly));
    }
    let mode = input.mode.as_deref().unwrap_or("preview");
    if !matches!(mode, "preview" | "merge" | "replace") {
        return Err(error(StatusCode::BAD_REQUEST, Code::ImportBadMode));
    }
    let format = input.format.as_deref().unwrap_or("simple");
    if !matches!(format, "simple" | "tagx") {
        return Err(error(StatusCode::BAD_REQUEST, Code::ImportBadFormat));
    }
    let current = ops.project();
    let base_spec = if mode == "replace" {
        nuriscada_project::ProjectSpec {
            server: current.to_spec().server,
            // 태그 파일은 장치를 정의하지 않으므로 교체해도 장치는 유지한다.
            devices: if format == "tagx" {
                current.to_spec().devices
            } else {
                Vec::new()
            },
            tags: Vec::new(),
            screens: current.to_spec().screens,
        }
    } else {
        current.to_spec()
    };
    let report = if format == "tagx" {
        let Some(device) = input.default_device.as_deref() else {
            return Err(error(StatusCode::BAD_REQUEST, Code::ImportDeviceRequired));
        };
        nuriscada_project::tagx::import_tagx(&base_spec, &input.text, device)
    } else {
        nuriscada_project::legacy::import_legacy_text(
            Some(&base_spec),
            &input.text,
            input.default_device.as_deref(),
        )
    }
    .map_err(|e| {
        error_detail(
            StatusCode::BAD_REQUEST,
            Code::ProjectInvalid,
            &e.to_string(),
        )
    })?;
    let next = Arc::new(
        nuriscada_project::Project::from_spec(report.project.clone()).map_err(|e| {
            error_detail(
                StatusCode::BAD_REQUEST,
                Code::ProjectInvalid,
                &e.to_string(),
            )
        })?,
    );
    let apply = matches!(mode, "merge" | "replace");
    if apply {
        let _lock = ops.write_lock.lock().await;
        let check = ops.clone();
        let t = token(&headers).unwrap_or_default();
        if blocking(move || check.authenticate(&t)).await?.is_none() || user.expires_ms <= now_ms()
        {
            return Err(error(StatusCode::UNAUTHORIZED, Code::SessionExpired));
        }
        let db = ops.clone();
        let actor = user.username.clone();
        let source = peer.source.clone();
        let saved = Arc::clone(&next);
        blocking(move || db.save_project(saved, &actor, &source)).await?;
        let db2 = ops.clone();
        let actor2 = user.username.clone();
        let source2 = peer.source.clone();
        let detail = json!({
            "mode": mode,
            "format": format,
            "imported_devices": report.imported_devices,
            "imported_tags": report.imported_tags,
            "renamed_tags": report.renamed_tags.len(),
            "modified_items": report.modified_items.len(),
            "ignored_items": report.ignored_items.len(),
            "unreadable_items": report.unreadable_items.len(),
        });
        blocking(move || {
            db2.audit(Event {
                actor: &actor2,
                source: &source2,
                action: "project.import",
                target: "project",
                detail,
            })
        })
        .await?;
        ops.stop_scanners().await;
        state.hub.reconfigure(&next);
        ops.attach_scanners(crate::start_scanners(&next, &state.hub))
            .await;
    }
    Ok(Json(json!({
        "ok": true,
        "applied": apply,
        "report": report,
        "project": next.to_spec(),
        "toml": next.to_toml(),
    })))
}

/// 값 JSON을 장치 원시값(u16)으로 바꾼다. 종류가 맞지 않으면 `None`이다.
fn raw_value(space: Space, value: &Value) -> Option<u16> {
    match space {
        Space::Coil
        | Space::MelsecM
        | Space::MelsecX
        | Space::MelsecY
        | Space::XgtMx
        | Space::FinsCio
        | Space::FinsWr
        | Space::FinsHr
        | Space::S7M => value.as_bool().map(u16::from),
        Space::Holding
        | Space::MelsecD
        | Space::MelsecW
        | Space::MelsecR
        | Space::XgtMw
        | Space::FinsDm
        | Space::S7Db => value.as_u64().and_then(|v| u16::try_from(v).ok()),
        _ => None,
    }
}

/// 단일 쓰기를 장치에 보낸다.
async fn execute_single(
    device: &nuriscada_project::Device,
    tag: &nuriscada_project::Tag,
    raw: u16,
) -> std::result::Result<(), nuriscada_driver_api::DriverError> {
    match &device.driver {
        DriverConfig::ModbusTcp { address } => {
            let mut driver = ModbusTcpClient::new(address.clone(), device.timeout);
            driver
                .write_single(tag.unit, tag.space.code(), tag.address, raw)
                .await
        }
        DriverConfig::MelsecMc { address } => {
            let mut driver = MelsecMcClient::new(address.clone(), device.timeout);
            driver
                .write_single(tag.unit, tag.space.code(), tag.address, raw)
                .await
        }
        DriverConfig::XgtFenet { address } => {
            let mut driver = XgtFenetClient::new(address.clone(), device.timeout);
            driver
                .write_single(tag.unit, tag.space.code(), tag.address, raw)
                .await
        }
        DriverConfig::FinsTcp { address } => {
            let mut driver = FinsTcpClient::new(address.clone(), device.timeout);
            driver
                .write_single(tag.unit, tag.space.code(), tag.address, raw)
                .await
        }
        DriverConfig::S7Tcp { address } => {
            let mut driver = S7Client::new(address.clone(), device.timeout);
            driver
                .write_single(tag.unit, tag.space.code(), tag.address, raw)
                .await
        }
    }
}

/// 인터록 조건을 검사한다. 모두 만족하면 `None`, 처음 어긴 조건의 설명을 돌려준다.
/// 조건 태그가 없거나 품질이 나쁘면 조건 불충족으로 본다(닫힘 실패).
fn check_interlock(
    project: &nuriscada_project::Project,
    hub_tags: &[nuriscada_scheduler::TagSnapshot],
    tag: &nuriscada_project::Tag,
) -> Option<String> {
    use nuriscada_project::InterlockValue;
    for cond in &tag.interlocks {
        let name = &project.tags[cond.tag].name;
        let current = hub_tags.iter().find(|t| t.name == *name);
        let met = match current {
            Some(cur) if cur.quality == nuriscada_scheduler::Quality::Good => {
                match (&cond.value, &cur.value) {
                    (
                        InterlockValue::Bool(want),
                        Some(nuriscada_scheduler::TagValue::Bool(have)),
                    ) => want == have,
                    (
                        InterlockValue::Int(want),
                        Some(nuriscada_scheduler::TagValue::Number(have)),
                    ) => *want as f64 == *have,
                    (
                        InterlockValue::Text(want),
                        Some(nuriscada_scheduler::TagValue::Text(have)),
                    ) => want == have,
                    _ => false,
                }
            }
            _ => false,
        };
        if !met {
            return Some(format!("{name} 조건 불충족"));
        }
    }
    None
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WriteValue {
    value: Value,
}

pub(super) async fn write_tag(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
    Extension(peer): Extension<Peer>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(input): Json<WriteValue>,
) -> Result<Json<Value>> {
    let ops = operations(&state)?;
    if user.role == Role::Viewer
        || !ops.writes_enabled
        || !peer.loopback
        || state.guard.allowed_hosts.is_none()
    {
        let db = ops.clone();
        let actor = user.username.clone();
        let source = peer.source.clone();
        let target = name.clone();
        blocking(move || {
            db.audit(Event {
                actor: &actor,
                source: &source,
                action: "write.denied",
                target: &target,
                detail: json!({"requested":input.value}),
            })
        })
        .await?;
        return Err(error(StatusCode::FORBIDDEN, Code::WriteNotAllowed));
    }
    let _slot = ops
        .write_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| error(StatusCode::TOO_MANY_REQUESTS, Code::WriteQueueFull))?;
    let _lock = ops.write_lock.lock().await;
    let check = ops.clone();
    let t = token(&headers).unwrap_or_default();
    if blocking(move || check.authenticate(&t)).await?.is_none() || user.expires_ms <= now_ms() {
        return Err(error(StatusCode::UNAUTHORIZED, Code::SessionExpired));
    }
    let project = ops.project();
    let tag = project
        .tags
        .iter()
        .find(|t| t.name == name && t.writable)
        .cloned()
        .ok_or_else(|| error(StatusCode::NOT_FOUND, Code::WriteTagNotWritable))?;
    let raw = raw_value(tag.space, &input.value)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, Code::WriteBadValue))?;
    let current = state
        .hub
        .tags()
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, Code::TagNotFound))?;
    if current.quality != Quality::Good {
        return Err(error(StatusCode::CONFLICT, Code::WriteBadQuality));
    }
    let hub_tags = state.hub.tags();
    if let Some(reason) = check_interlock(&project, &hub_tags, &tag) {
        audit_write(
            &ops,
            user.username.clone(),
            peer.source.clone(),
            "write.denied",
            name.clone(),
            json!({"requested":input.value,"reason":reason}),
        )
        .await?;
        return Err(error_detail(
            StatusCode::FORBIDDEN,
            Code::WriteInterlocked,
            &reason,
        ));
    }
    let old = serde_json::to_value(current.value).map_err(internal)?;
    let requested = input.value.clone();
    let db = ops.clone();
    let actor = user.username.clone();
    let source = peer.source.clone();
    let target = name.clone();
    blocking(move||db.audit(Event{actor:&actor,source:&source,action:"write.request",target:&target,
        detail:json!({"previous_display_value":old,"requested_raw_value":requested,"outcome":"pending"})})).await?;
    let device = &project.devices[tag.device];
    let outcome = execute_single(device, &tag, raw).await;
    let detail = match &outcome {
        Ok(()) => json!({"requested_raw_value":input.value,"outcome":"device_acknowledged"}),
        Err(e) => {
            json!({"requested_raw_value":input.value,"outcome":"failed_or_unknown","error":e.to_string()})
        }
    };
    let db = ops.clone();
    blocking(move || {
        db.audit(Event {
            actor: &user.username,
            source: &peer.source,
            action: "write.result",
            target: &name,
            detail,
        })
    })
    .await?;
    outcome.map_err(|e| {
        error_detail(
            StatusCode::BAD_GATEWAY,
            Code::WriteUnconfirmed,
            &e.to_string(),
        )
    })?;
    Ok(Json(
        json!({"ok":true,"outcome":"device_acknowledged","readback":"다음 스캔으로 확인"}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BatchWriteItem {
    name: String,
    value: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BatchWrite {
    writes: Vec<BatchWriteItem>,
}

/// 감사에 제어 결과를 남긴다.
async fn audit_write(
    ops: &Arc<Operations>,
    actor: String,
    source: String,
    action: &str,
    target: String,
    detail: Value,
) -> Result<()> {
    let db = ops.clone();
    let action = action.to_string();
    blocking(move || {
        db.audit(Event {
            actor: &actor,
            source: &source,
            action: &action,
            target: &target,
            detail,
        })
    })
    .await?;
    Ok(())
}

/// 배치 한 항목의 실행 준비 상태.
struct Ready {
    index: usize,
    tag: nuriscada_project::Tag,
    raw: u16,
    requested: Value,
}

/// 배치 실행 한 걸음. `Multi`의 위치들은 `ready` 안의 연속 묶음이다.
#[derive(Debug, PartialEq)]
enum BatchStep {
    Single(usize),
    Multi(Vec<usize>),
}

/// 배치 묶음 계획. 같은 장치·유닛·연속 주소만 묶고, 상한을 넘기면 나눈다. 순서는 바꾸지 않는다.
/// 묶음 대상: MODBUS 코일·보유 레지스터(15·16), MELSEC D·W·R·M·X·Y(1401).
fn batch_cap(device: &nuriscada_project::Device, space: Space) -> Option<usize> {
    use nuriscada_driver_melsec_mc::protocol::{MAX_BIT_POINTS, MAX_WORD_POINTS};
    use nuriscada_driver_modbus_tcp::protocol::{MAX_WRITE_COILS, MAX_WRITE_REGISTERS};
    match &device.driver {
        DriverConfig::ModbusTcp { .. } => match space {
            Space::Coil => Some(MAX_WRITE_COILS),
            Space::Holding => Some(MAX_WRITE_REGISTERS),
            _ => None,
        },
        DriverConfig::MelsecMc { .. } => match space {
            Space::MelsecD | Space::MelsecW | Space::MelsecR => Some(MAX_WORD_POINTS as usize),
            Space::MelsecM | Space::MelsecX | Space::MelsecY => Some(MAX_BIT_POINTS as usize),
            _ => None,
        },
        _ => None,
    }
}

fn plan_batch_steps(devices: &[nuriscada_project::Device], ready: &[Ready]) -> Vec<BatchStep> {
    let mut steps = Vec::new();
    let mut i = 0;
    while i < ready.len() {
        let first = &ready[i];
        let cap = batch_cap(&devices[first.tag.device], first.tag.space);
        let mut j = i + 1;
        if let Some(cap) = cap {
            while j < ready.len() {
                let next = &ready[j];
                if next.tag.device != first.tag.device
                    || next.tag.unit != first.tag.unit
                    || next.tag.space != first.tag.space
                    || next.tag.address != ready[j - 1].tag.address + 1
                    || j - i >= cap
                {
                    break;
                }
                j += 1;
            }
        }
        if j - i > 1 {
            steps.push(BatchStep::Multi((i..j).collect()));
        } else {
            steps.push(BatchStep::Single(i));
        }
        i = j;
    }
    steps
}

/// 한 항목의 실행 결과를 감사에 남기고 결과 칸을 채운다.
async fn finish_write(
    ops: &Arc<Operations>,
    actor: &str,
    source: &str,
    r: &Ready,
    outcome: &std::result::Result<(), nuriscada_driver_api::DriverError>,
    results: &mut [Value],
) -> Result<()> {
    let detail = match outcome {
        Ok(()) => json!({"requested_raw_value":r.requested,"outcome":"device_acknowledged"}),
        Err(e) => {
            json!({"requested_raw_value":r.requested,"outcome":"failed_or_unknown","error":e.to_string()})
        }
    };
    audit_write(
        ops,
        actor.to_string(),
        source.to_string(),
        "write.result",
        r.tag.name.clone(),
        detail,
    )
    .await?;
    results[r.index] = match outcome {
        Ok(()) => json!({"name":r.tag.name,"outcome":"device_acknowledged"}),
        Err(e) => json!({"name":r.tag.name,"outcome":"failed_or_unknown","error":e.to_string()}),
    };
    Ok(())
}

/// 여러 태그를 한 요청으로 쓴다. 원자적이지 않다: 항목마다 결과를 돌려준다.
/// MODBUS 코일·보유 레지스터는 같은 장치·유닛의 연속 주소를 한 번에 보내고(15·16),
/// 나머지는 단일 쓰기로 보낸다. 같은 태그가 두 번 나오면 순서대로 두 번 쓴다.
pub(super) async fn write_batch(
    State(state): State<AppState>,
    Extension(user): Extension<Principal>,
    Extension(peer): Extension<Peer>,
    headers: HeaderMap,
    Json(input): Json<BatchWrite>,
) -> Result<Json<Value>> {
    let ops = operations(&state)?;
    if input.writes.is_empty() || input.writes.len() > 64 {
        return Err(error(StatusCode::BAD_REQUEST, Code::InputTooLong));
    }
    if user.role == Role::Viewer
        || !ops.writes_enabled
        || !peer.loopback
        || state.guard.allowed_hosts.is_none()
    {
        for w in &input.writes {
            audit_write(
                &ops,
                user.username.clone(),
                peer.source.clone(),
                "write.denied",
                w.name.clone(),
                json!({"requested":w.value}),
            )
            .await?;
        }
        return Err(error(StatusCode::FORBIDDEN, Code::WriteNotAllowed));
    }
    let _slot = ops
        .write_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| error(StatusCode::TOO_MANY_REQUESTS, Code::WriteQueueFull))?;
    let _lock = ops.write_lock.lock().await;
    let check = ops.clone();
    let t = token(&headers).unwrap_or_default();
    if blocking(move || check.authenticate(&t)).await?.is_none() || user.expires_ms <= now_ms() {
        return Err(error(StatusCode::UNAUTHORIZED, Code::SessionExpired));
    }
    let project = ops.project();
    let hub_tags = state.hub.tags();
    // 1단계: 항목별 검증·품질 확인·요청 감사.
    let mut results: Vec<Value> = Vec::with_capacity(input.writes.len());
    let mut ready: Vec<Ready> = Vec::new();
    for (index, w) in input.writes.iter().enumerate() {
        let Some(tag) = project
            .tags
            .iter()
            .find(|t| t.name == w.name && t.writable)
            .cloned()
        else {
            results.push(json!({"name":w.name,"outcome":"not_writable"}));
            continue;
        };
        let Some(raw) = raw_value(tag.space, &w.value) else {
            results.push(json!({"name":w.name,"outcome":"bad_value"}));
            continue;
        };
        let Some(current) = hub_tags.iter().find(|t| t.name == w.name) else {
            results.push(json!({"name":w.name,"outcome":"not_found"}));
            continue;
        };
        if current.quality != Quality::Good {
            results.push(json!({"name":w.name,"outcome":"bad_quality"}));
            continue;
        }
        if let Some(reason) = check_interlock(&project, &hub_tags, &tag) {
            audit_write(
                &ops,
                user.username.clone(),
                peer.source.clone(),
                "write.denied",
                w.name.clone(),
                json!({"requested":w.value,"reason":reason}),
            )
            .await?;
            results.push(json!({"name":w.name,"outcome":"interlocked"}));
            continue;
        }
        let old = serde_json::to_value(current.value.clone()).map_err(internal)?;
        audit_write(
            &ops,
            user.username.clone(),
            peer.source.clone(),
            "write.request",
            w.name.clone(),
            json!({"previous_display_value":old,"requested_raw_value":w.value,"outcome":"pending"}),
        )
        .await?;
        results.push(Value::Null);
        ready.push(Ready {
            index,
            tag,
            raw,
            requested: w.value.clone(),
        });
    }
    // 2단계: MODBUS 연속 묶음과 단일 쓰기를 순서대로 보낸다.
    for step in plan_batch_steps(&project.devices, &ready) {
        match step {
            BatchStep::Single(pos) => {
                let r = &ready[pos];
                let device = &project.devices[r.tag.device];
                let outcome = execute_single(device, &r.tag, r.raw).await;
                finish_write(
                    &ops,
                    &user.username,
                    &peer.source,
                    r,
                    &outcome,
                    &mut results,
                )
                .await?;
            }
            BatchStep::Multi(positions) => {
                let first = &ready[positions[0]];
                let device = &project.devices[first.tag.device];
                let outcome = match &device.driver {
                    DriverConfig::ModbusTcp { address } => {
                        let mut driver = ModbusTcpClient::new(address.clone(), device.timeout);
                        match first.tag.space {
                            Space::Coil => {
                                let bits: Vec<bool> =
                                    positions.iter().map(|&p| ready[p].raw == 1).collect();
                                driver
                                    .write_multiple(
                                        first.tag.unit,
                                        Space::Coil.code(),
                                        first.tag.address,
                                        &bits,
                                        &[],
                                    )
                                    .await
                            }
                            _ => {
                                let words: Vec<u16> =
                                    positions.iter().map(|&p| ready[p].raw).collect();
                                driver
                                    .write_multiple(
                                        first.tag.unit,
                                        Space::Holding.code(),
                                        first.tag.address,
                                        &[],
                                        &words,
                                    )
                                    .await
                            }
                        }
                    }
                    DriverConfig::MelsecMc { address } => {
                        let mut driver = MelsecMcClient::new(address.clone(), device.timeout);
                        match first.tag.space {
                            Space::MelsecM | Space::MelsecX | Space::MelsecY => {
                                let bits: Vec<bool> =
                                    positions.iter().map(|&p| ready[p].raw == 1).collect();
                                driver
                                    .write_multiple(
                                        first.tag.unit,
                                        first.tag.space.code(),
                                        first.tag.address,
                                        &[],
                                        &bits,
                                    )
                                    .await
                            }
                            _ => {
                                let words: Vec<u16> =
                                    positions.iter().map(|&p| ready[p].raw).collect();
                                driver
                                    .write_multiple(
                                        first.tag.unit,
                                        first.tag.space.code(),
                                        first.tag.address,
                                        &words,
                                        &[],
                                    )
                                    .await
                            }
                        }
                    }
                    _ => unreachable!("묶음은 MODBUS·MELSEC 장치만 쓴다"),
                };
                for pos in positions {
                    let r = &ready[pos];
                    finish_write(
                        &ops,
                        &user.username,
                        &peer.source,
                        r,
                        &outcome,
                        &mut results,
                    )
                    .await?;
                }
            }
        }
    }
    let ok = results
        .iter()
        .all(|r| r.get("outcome").and_then(|o| o.as_str()) == Some("device_acknowledged"));
    Ok(Json(json!({"ok":ok,"results":results})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuriscada_project::{Device, Tag, TagBody};
    use std::time::Duration;

    fn device() -> Device {
        Device {
            name: "sim".to_string(),
            driver: DriverConfig::ModbusTcp {
                address: "127.0.0.1:502".to_string(),
            },
            timeout: Duration::from_millis(500),
            scan: Duration::from_millis(100),
        }
    }

    fn ready_tag(device: usize, space: Space, address: u16) -> Ready {
        Ready {
            index: 0,
            tag: Tag {
                writable: true,
                name: String::new(),
                device,
                unit: 1,
                space,
                address,
                body: TagBody::Digital { alarm_kind: None },
                interlocks: Vec::new(),
            },
            raw: 0,
            requested: Value::Null,
        }
    }

    #[test]
    fn 연속된_modbus_주소는_묶고_나머지는_낱개로_둔다() {
        let devices = vec![device()];
        let ready = vec![
            ready_tag(0, Space::Coil, 0),
            ready_tag(0, Space::Coil, 1),
            ready_tag(0, Space::Holding, 10),
            ready_tag(0, Space::Holding, 12),
            ready_tag(0, Space::Coil, 5),
        ];
        assert_eq!(
            plan_batch_steps(&devices, &ready),
            vec![
                BatchStep::Multi(vec![0, 1]),
                BatchStep::Single(2),
                BatchStep::Single(3),
                BatchStep::Single(4),
            ]
        );
    }

    #[test]
    fn 다른_장치나_영역이_끼면_묶음이_끊긴다() {
        let mut devices = vec![device(), device()];
        devices[1].name = "other".to_string();
        let ready = vec![
            ready_tag(0, Space::Coil, 0),
            ready_tag(1, Space::Coil, 1),
            ready_tag(0, Space::Holding, 0),
        ];
        assert_eq!(
            plan_batch_steps(&devices, &ready),
            vec![
                BatchStep::Single(0),
                BatchStep::Single(1),
                BatchStep::Single(2),
            ]
        );
    }

    #[test]
    fn 상한을_넘기면_묶음을_나눈다() {
        let devices = vec![device()];
        let ready: Vec<Ready> = (0..130u16)
            .map(|a| ready_tag(0, Space::Holding, a))
            .collect();
        // 보유 레지스터 상한 123개에서 끊긴다.
        assert_eq!(
            plan_batch_steps(&devices, &ready),
            vec![
                BatchStep::Multi((0..123).collect()),
                BatchStep::Multi((123..130).collect()),
            ]
        );
    }

    fn melsec_device() -> Device {
        Device {
            name: "plc".to_string(),
            driver: DriverConfig::MelsecMc {
                address: "127.0.0.1:5007".to_string(),
            },
            timeout: Duration::from_millis(500),
            scan: Duration::from_millis(100),
        }
    }

    #[test]
    fn melsec_연속_주소는_묶고_다른_드라이버와_섞지_않는다() {
        let devices = vec![device(), melsec_device()];
        let ready = vec![
            ready_tag(1, Space::MelsecD, 0),
            ready_tag(1, Space::MelsecD, 1),
            ready_tag(0, Space::Coil, 0),
            ready_tag(1, Space::MelsecM, 10),
            ready_tag(1, Space::MelsecM, 11),
        ];
        assert_eq!(
            plan_batch_steps(&devices, &ready),
            vec![
                BatchStep::Multi(vec![0, 1]),
                BatchStep::Single(2),
                BatchStep::Multi(vec![3, 4]),
            ]
        );
    }

    #[test]
    fn 문자열_조건은_정확히_일치할_때만_만족한다() {
        use nuriscada_project::Project;
        use nuriscada_scheduler::{TagSnapshot, TagValue};
        let project = Project::from_toml(
            r#"
[[device]]
name = "sim"
driver = "modbus-tcp"
address = "127.0.0.1:1"
scan_ms = 100

[[tag]]
name = "MODE"
device = "sim"
space = "holding"
address = 0
type = "string"
value = "AUTO"
length = 4

[[tag]]
name = "MOTOR"
device = "sim"
space = "coil"
address = 1
type = "digital"
writable = true

[[tag.interlock]]
tag = "MODE"
value = "AUTO"
"#,
        )
        .unwrap();
        let snap = |value: Option<TagValue>| TagSnapshot {
            name: "MODE".to_string(),
            device: "sim".to_string(),
            kind: "string",
            value,
            quality: Quality::Good,
            changed_ms: 1,
            unit_text: String::new(),
            alarm_state: None,
        };
        let motor = project.tags.iter().find(|t| t.name == "MOTOR").unwrap();
        assert!(
            check_interlock(
                &project,
                &[snap(Some(TagValue::Text("AUTO".into())))],
                motor
            )
            .is_none()
        );
        assert!(
            check_interlock(&project, &[snap(Some(TagValue::Text("MAN".into())))], motor).is_some()
        );
        assert!(check_interlock(&project, &[snap(None)], motor).is_some());
    }
}
