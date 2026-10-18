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
    ops.attach_scanners(scanners).await;
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

#[tokio::test]
async fn project_configuration_can_be_edited_and_reloaded_by_local_admin() {
    let st = start(true, false).await;
    let operator = login(&st, "operator").await;
    assert_eq!(
        request(st.addr, "GET", "/api/v1/project", &operator, json!({}), "")
            .await
            .0,
        403
    );
    let admin = login(&st, "admin").await;
    let (code, _, body) = request(st.addr, "GET", "/api/v1/project", &admin, json!({}), "").await;
    assert_eq!(code, 200);
    let mut spec = body["project"].clone();
    assert_eq!(spec["tag"].as_array().unwrap().len(), 2);

    // 잘못된 설정은 거부하고 기존 태그를 유지한다.
    let mut bad = spec.clone();
    bad["tag"][0]["device"] = json!("missing_device");
    assert_eq!(
        request(
            st.addr,
            "POST",
            "/api/v1/project",
            &admin,
            bad,
            "X-NuriScada-Request: 1\r\n"
        )
        .await
        .0,
        400
    );

    // 새 아날로그 태그 TEMP(주소 1)와 그래픽 화면을 추가하고 런타임 스캔·화면 조회가 반영되는지 확인한다.
    st.sim.set_holding(1, &[42]);
    spec["tag"].as_array_mut().unwrap().push(json!({
        "name": "TEMP",
        "writable": true,
        "device": "sim",
        "unit": 1,
        "space": "holding",
        "address": 1,
        "type": "analog",
        "unit_text": "C",
        "scale": { "plc_base": 0.0, "plc_full": 100.0, "base": 0.0, "full": 100.0, "profile": 0, "delay": 0, "range": 0 },
        "alarm": null
    }));
    spec["screen"] = json!([{
        "name": "온도 감시 화면",
        "width": 800,
        "height": 400,
        "widget": [{
            "id": "w-temp-tank",
            "kind": "tank",
            "label": "가열 탱크",
            "tag": "TEMP",
            "x": 40.0,
            "y": 40.0,
            "width": 160.0,
            "height": 200.0,
            "min": 0.0,
            "max": 100.0
        }]
    }]);
    let (code, _, updated) = request(
        st.addr,
        "POST",
        "/api/v1/project",
        &admin,
        spec,
        "X-NuriScada-Request: 1\r\n",
    )
    .await;
    assert_eq!(code, 200, "{updated}");
    assert!(
        updated["toml"]
            .as_str()
            .unwrap()
            .contains("name = \"TEMP\"")
    );
    let (scode, _, screens_doc) =
        request(st.addr, "GET", "/api/v1/screens", &operator, json!({}), "").await;
    assert_eq!(scode, 200);
    assert_eq!(screens_doc["screens"][0]["name"], "온도 감시 화면");
    assert_eq!(screens_doc["screens"][0]["widget"][0]["tag"], "TEMP");

    // 스캐너 재시작 후 새 태그 값이 읽히고 쓰기도 동작하는지 확인한다.
    let mut temp_val = Value::Null;
    for _ in 0..30 {
        let (_, _, tags_doc) = request(st.addr, "GET", "/api/v1/tags", &admin, json!({}), "").await;
        if let Some(t) = tags_doc["tags"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "TEMP")
            && t["quality"] == "good"
        {
            temp_val = t["value"].clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(temp_val, json!(42.0));
    assert_eq!(
        request(
            st.addr,
            "POST",
            "/api/v1/control/tags/TEMP",
            &admin,
            json!({"value": 77}),
            "X-NuriScada-Request: 1\r\n"
        )
        .await
        .0,
        200
    );
    assert_eq!(st.sim.state().holding_registers[1], 77);
    assert!(
        st.ops
            .audit_page(0)
            .unwrap()
            .iter()
            .any(|r| r["event"]["action"] == "project.update")
    );
    st.ops.verify_audit().unwrap();
}

#[tokio::test]
async fn tag_value_history_is_persisted_and_exportable_as_json_and_csv() {
    let st = start(true, false).await;
    let viewer = login(&st, "viewer").await;
    st.sim.set_holding(0, &[10]);
    tokio::time::sleep(Duration::from_millis(260)).await;
    st.sim.set_holding(0, &[35]);
    let mut samples = Vec::new();
    for _ in 0..30 {
        let (code, _, body) = request(
            st.addr,
            "GET",
            "/api/v1/history/tags?tag=LEVEL&limit=100",
            &viewer,
            json!({}),
            "",
        )
        .await;
        assert_eq!(code, 200);
        samples = body["samples"].as_array().unwrap().clone();
        if samples.iter().any(|s| s["value"] == json!(350.0)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(
        samples.iter().any(|s| s["value"] == json!(350.0)),
        "samples={samples:?}"
    );

    // CSV 내보내기 확인
    let mut stream = TcpStream::connect(st.addr).await.unwrap();
    stream
        .write_all(
            format!(
                "GET /api/v1/history/tags?tag=LEVEL&format=csv HTTP/1.1\r\nHost: {}\r\nCookie: {viewer}\r\nConnection: close\r\n\r\n",
                st.addr
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let (head, csv) = raw.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(head.to_lowercase().contains("content-type: text/csv"));
    assert!(csv.starts_with("id,time_ms,tag,kind,value,quality,alarm_state\n"));
    assert!(csv.contains("\"LEVEL\",analog,350.0,good"), "csv={csv}");
}

/// 기존 제품의 태그 파일 한 줄(AI)을 만든다: 공통 19개 + 고유 51개 필드.
fn tagx_ai_line(name: &str) -> String {
    let mut f: Vec<String> = vec![name.into(), "설명".into(), "1".into()];
    f.extend(std::iter::repeat_n("0".to_string(), 16));
    let mut s = vec!["0".to_string(); 51];
    s[1] = "1".into(); // 국번
    s[2] = "1".into(); // 주소
    s[10] = "%".into(); // 단위
    s[12] = "100".into(); // 표시 상한
    s[14] = "100".into(); // 장치 상한
    f.extend(s);
    format!("AI,{},", f.join(","))
}

#[tokio::test]
async fn tag_file_import_previews_then_merges_for_local_admin_only() {
    let st = start(true, false).await;
    let hdr = "X-NuriScada-Request: 1\r\n";
    let url = "/api/v1/project/import";
    let operator = login(&st, "operator").await;
    let body = json!({"text": tagx_ai_line("IMP.A"), "format": "tagx", "default_device": "sim"});
    assert_eq!(
        request(st.addr, "POST", url, &operator, body.clone(), hdr)
            .await
            .0,
        403
    );

    let admin = login(&st, "admin").await;
    let count = |doc: &Value| doc["project"]["tag"].as_array().unwrap().len();
    let (_, _, before) = request(st.addr, "GET", "/api/v1/project", &admin, json!({}), "").await;

    // 미리보기(기본)는 보고서만 돌려주고 프로젝트를 바꾸지 않는다.
    let (code, _, preview) = request(st.addr, "POST", url, &admin, body.clone(), hdr).await;
    assert_eq!(code, 200, "{preview}");
    assert_eq!(preview["applied"], false);
    assert_eq!(preview["report"]["imported_tags"], 1);
    let (_, _, same) = request(st.addr, "GET", "/api/v1/project", &admin, json!({}), "").await;
    assert_eq!(count(&same), count(&before));

    // 잘못된 요청은 거부한다.
    let bad_format = json!({"text": "", "format": "xml"});
    assert_eq!(
        request(st.addr, "POST", url, &admin, bad_format, hdr)
            .await
            .0,
        400
    );
    let no_device = json!({"text": tagx_ai_line("X"), "format": "tagx"});
    assert_eq!(
        request(st.addr, "POST", url, &admin, no_device, hdr)
            .await
            .0,
        400
    );
    let wrong_device = json!({"text": "", "format": "tagx", "default_device": "nope"});
    assert_eq!(
        request(st.addr, "POST", url, &admin, wrong_device, hdr)
            .await
            .0,
        400
    );

    // 병합하면 태그가 늘고 감사 기록에 남는다.
    let mut merge = body;
    merge["mode"] = json!("merge");
    let (code, _, done) = request(st.addr, "POST", url, &admin, merge, hdr).await;
    assert_eq!(code, 200, "{done}");
    assert_eq!(done["applied"], true);
    let (_, _, after) = request(st.addr, "GET", "/api/v1/project", &admin, json!({}), "").await;
    assert_eq!(count(&after), count(&before) + 1);
    assert!(
        after["project"]["tag"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "IMP.A")
    );
}
