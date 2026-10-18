// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 인증·권한·감사·실제 MODBUS 쓰기 경로의 통합 시험.

use nuriscada_project::Project;
use nuriscada_scheduler::Hub;
use nuriscada_server::{
    operations::{Operations, Role},
    router_with_operations, start_scanners,
};
use nuriscada_sim_modbus::Simulator;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const PASSWORD: &str = "integration-test-password";

async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    cookie: &str,
    body: Value,
    extra: &str,
) -> (u16, String, Value) {
    let body = body.to_string();
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nCookie: {cookie}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        head.into(),
        serde_json::from_str(body).unwrap_or(Value::Null),
    )
}

struct Stack {
    addr: SocketAddr,
    sim: Simulator,
    ops: Arc<Operations>,
    _task: tokio::task::JoinHandle<()>,
}
async fn start(writes: bool, public: bool) -> Stack {
    start_mode(writes, public, false).await
}

async fn start_mode(writes: bool, public: bool, read_only: bool) -> Stack {
    let sim = Simulator::start("127.0.0.1:0", 10).await.unwrap();
    let project = Arc::new(
        Project::from_toml(&format!(
            r#"
[[device]]
name="sim"
driver="modbus-tcp"
address="{}"
scan_ms=20
[[tag]]
name="LEVEL"
device="sim"
space="holding"
address=0
type="analog"
writable=true
scale={{plc_base=0,plc_full=100,base=0,full=1000}}
alarm={{kind=1,hihi=900,gap=50}}
[[tag]]
name="PUMP"
device="sim"
space="coil"
address=0
type="digital"
writable=true
"#,
            sim.addr()
        ))
        .unwrap(),
    );
    let ops = Operations::open_with_mode(
        std::path::Path::new(":memory:"),
        project.clone(),
        writes,
        read_only,
    )
    .unwrap();
    ops.create_user("admin", PASSWORD, Role::Admin, true, "cli", "cli")
        .unwrap();
    ops.create_user("viewer", PASSWORD, Role::Viewer, false, "admin", "cli")
        .unwrap();
    ops.create_user("operator", PASSWORD, Role::Operator, false, "admin", "cli")
        .unwrap();
    let hub = Hub::new(&project);
    let archive = ops.start_archive(hub.clone());
    let scanners = start_scanners(&project, &hub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bound = if public {
        SocketAddr::from(([0, 0, 0, 0], addr.port()))
    } else {
        addr
    };
    let app = router_with_operations(hub, bound, None, ops.clone());
    let task = tokio::spawn(async move {
        let _archive = archive;
        let _scanners = scanners;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    Stack {
        addr,
        sim,
        ops,
        _task: task,
    }
}

async fn login(st: &Stack, name: &str) -> String {
    let (code, headers, _) = request(
        st.addr,
        "POST",
        "/api/v1/auth/login",
        "",
        json!({"username":name,"password":PASSWORD}),
        "X-NuriScada-Request: 1\r\n",
    )
    .await;
    assert_eq!(code, 200, "{headers}");
    assert!(headers.to_lowercase().contains("httponly"));
    assert!(headers.to_lowercase().contains("samesite=strict"));
    assert!(headers.to_lowercase().contains("cache-control: no-store"));
    if st.ops.public_read_only {
        assert!(headers.to_lowercase().contains("; secure"));
    }
    headers
        .lines()
        .find_map(|l| {
            l.to_lowercase().starts_with("set-cookie:").then(|| {
                l.split_once(':')
                    .unwrap()
                    .1
                    .trim()
                    .split(';')
                    .next()
                    .unwrap()
                    .to_string()
            })
        })
        .unwrap()
}

#[tokio::test]
async fn authentication_roles_raw_write_and_audit_are_enforced() {
    let st = start(true, false).await;
    assert_eq!(
        request(st.addr, "GET", "/api/v1/tags", "", json!({}), "")
            .await
            .0,
        401
    );
    assert_eq!(
        request(
            st.addr,
            "POST",
            "/api/v1/auth/login",
            "",
            json!({"username":"admin","password":PASSWORD}),
            ""
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(
            st.addr,
            "POST",
            "/api/v1/auth/login",
            "",
            json!({"username":"admin","password":PASSWORD}),
            "X-NuriScada-Request: 1\r\nOrigin: http://evil.example\r\n"
        )
        .await
        .0,
        403
    );
    let viewer = login(&st, "viewer").await;
    assert_eq!(
        request(st.addr, "GET", "/api/v1/tags", &viewer, json!({}), "")
            .await
            .0,
        200
    );
    assert_eq!(
        request(
            st.addr,
            "POST",
            "/api/v1/control/tags/LEVEL",
            &viewer,
            json!({"value":25}),
            "X-NuriScada-Request: 1\r\n"
        )
        .await
        .0,
        403
    );
    assert_eq!(st.sim.state().holding_registers[0], 0);
    let operator = login(&st, "operator").await;
    let result = request(
        st.addr,
        "POST",
        "/api/v1/control/tags/LEVEL",
        &operator,
        json!({"value":25}),
        "X-NuriScada-Request: 1\r\n",
    )
    .await;
    assert_eq!(result.0, 200, "{}", result.2);
    assert_eq!(st.sim.state().holding_registers[0], 25);
    assert_eq!(
        request(
            st.addr,
            "POST",
            "/api/v1/control/tags/PUMP",
            &operator,
            json!({"value":true}),
            "X-NuriScada-Request: 1\r\n"
        )
        .await
        .0,
        200
    );
    assert!(st.sim.state().coils[0]);
    for value in [json!(-1), json!(65536), json!(1.5), json!(true)] {
        assert_eq!(
            request(
                st.addr,
                "POST",
                "/api/v1/control/tags/LEVEL",
                &operator,
                json!({"value":value}),
                "X-NuriScada-Request: 1\r\n"
            )
            .await
            .0,
            400
        );
    }
    assert_eq!(
        request(st.addr, "GET", "/api/v1/audit", &operator, json!({}), "")
            .await
            .0,
        403
    );
    let admin = login(&st, "admin").await;
    let (code, _, audit) = request(st.addr, "GET", "/api/v1/audit", &admin, json!({}), "").await;
    assert_eq!(code, 200);
    assert!(
        audit["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["event"]["action"] == "write.result"
                && r["event"]["detail"]["outcome"] == "device_acknowledged")
    );
    assert!(!audit.to_string().contains(PASSWORD));
    st.ops.verify_audit().unwrap();
    assert_eq!(
        request(
            st.addr,
            "POST",
            "/api/v1/auth/logout",
            &admin,
            json!({}),
            "X-NuriScada-Request: 1\r\n"
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(st.addr, "GET", "/api/v1/tags", &admin, json!({}), "")
            .await
            .0,
        401
    );
}

#[tokio::test]
async fn disabled_and_public_modes_reject_writes_even_with_admin_and_forged_proxy() {
    for (enabled, public, read_only) in [
        (false, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        let st = start_mode(enabled, public, read_only).await;
        let admin = login(&st, "admin").await;
        assert_eq!(
            request(
                st.addr,
                "POST",
                "/api/v1/control/tags/LEVEL",
                &admin,
                json!({"value":100}),
                "X-NuriScada-Request: 1\r\nX-Forwarded-For: 127.0.0.1\r\n"
            )
            .await
            .0,
            403
        );
        assert_eq!(st.sim.state().holding_registers[0], 0);
    }
}

#[tokio::test]
async fn alarm_history_is_persisted_and_acknowledgement_is_separate_from_return() {
    let st = start(true, false).await;
    let operator = login(&st, "operator").await;
    st.sim.set_holding(0, &[95]);
    let mut rows = Vec::new();
    for _ in 0..50 {
        let result = request(
            st.addr,
            "GET",
            "/api/v1/history/alarms",
            &operator,
            json!({}),
            "",
        )
        .await;
        assert_eq!(result.0, 200);
        rows = result.2["records"].as_array().unwrap().clone();
        if !rows.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let record = rows.iter().find(|r| r["data"]["event"] == "HIHI").unwrap();
    let path = format!("/api/v1/history/alarms/{}/ack", record["id"]);
    let viewer = login(&st, "viewer").await;
    assert_eq!(
        request(
            st.addr,
            "POST",
            &path,
            &viewer,
            json!({}),
            "X-NuriScada-Request: 1\r\n"
        )
        .await
        .0,
        403
    );
    let ack = request(
        st.addr,
        "POST",
        &path,
        &operator,
        json!({}),
        "X-NuriScada-Request: 1\r\n",
    )
    .await;
    assert_eq!(ack.0, 200);
    assert_eq!(ack.2["changed"], true);
    assert_eq!(
        request(
            st.addr,
            "POST",
            &path,
            &operator,
            json!({}),
            "X-NuriScada-Request: 1\r\n"
        )
        .await
        .2["changed"],
        false
    );
    let history = st.ops.alarm_page(0, Some("LEVEL".into())).unwrap();
    assert_eq!(history[0]["ack_actor"], "operator");
    let current = request(st.addr, "GET", "/api/v1/alarms", &operator, json!({}), "").await;
    assert_eq!(
        current.2["active"][0]["alarm_state"], "HIHI",
        "확인만으로 정상 복귀하지 않는다"
    );
    assert_eq!(
        st.ops
            .audit_page(0)
            .unwrap()
            .iter()
            .filter(|r| r["event"]["action"] == "alarm.ack")
            .count(),
        1
    );
}

#[tokio::test]
async fn explicit_public_mode_accepts_proxy_host_but_keeps_authentication() {
    let st = start_mode(false, false, true).await;
    for (path, expected) in [("/api/v1/auth/session", 200), ("/api/v1/tags", 401)] {
        let mut stream = TcpStream::connect(st.addr).await.unwrap();
        stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: panel.example\r\nOrigin: https://panel.example\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        assert!(
            response.starts_with(&format!("HTTP/1.1 {expected}")),
            "{response}"
        );
    }
}
