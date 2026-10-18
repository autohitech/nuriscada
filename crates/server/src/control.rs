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
use nuriscada_driver_modbus_tcp::ModbusTcpClient;
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
    eprintln!("조작 API 오류: {e}");
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

pub(super) async fn control_tags(State(state): State<AppState>) -> Result<Json<Value>> {
    let ops = operations(&state)?;
    let project = ops.project();
    Ok(Json(
        json!({"tags":project.tags.iter().filter(|t|t.writable).map(|t|json!({"name":t.name,"kind":if t.space==Space::Coil{"bool"}else{"word"}})).collect::<Vec<_>>()}),
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
    let raw = match tag.space {
        Space::Coil => input.value.as_bool().map(u16::from),
        Space::Holding => input.value.as_u64().and_then(|v| u16::try_from(v).ok()),
        _ => None,
    }
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
    let old = serde_json::to_value(current.value).map_err(internal)?;
    let requested = input.value.clone();
    let db = ops.clone();
    let actor = user.username.clone();
    let source = peer.source.clone();
    let target = name.clone();
    blocking(move||db.audit(Event{actor:&actor,source:&source,action:"write.request",target:&target,
        detail:json!({"previous_display_value":old,"requested_raw_value":requested,"outcome":"pending"})})).await?;
    let device = &project.devices[tag.device];
    let DriverConfig::ModbusTcp { address } = &device.driver;
    let mut driver = ModbusTcpClient::new(address.clone(), device.timeout);
    let outcome = driver
        .write_single(tag.unit, tag.space.code(), tag.address, raw)
        .await;
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
