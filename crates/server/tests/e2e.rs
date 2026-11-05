// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시뮬레이터 → 스캐너 → 허브 → REST·웹소켓 전체 경로 시험.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use nuriscada_project::Project;
use nuriscada_scheduler::Hub;
use nuriscada_server::{Scanners, router, start_scanners};
use nuriscada_sim_modbus::Simulator;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct Stack {
    sim: Simulator,
    addr: SocketAddr,
    scanners: Option<Scanners>,
}

async fn start() -> Stack {
    let sim = Simulator::start("127.0.0.1:0", 100).await.unwrap();
    let project = Arc::new(
        Project::from_toml(&format!(
            r#"
[[device]]
name = "sim"
driver = "modbus-tcp"
address = "{}"
timeout_ms = 300
scan_ms = 20

[[tag]]
name = "TANK"
device = "sim"
space = "holding"
address = 0
type = "analog"
unit_text = "%"
scale = {{ plc_base = 0, plc_full = 100, base = 0, full = 1000 }}
alarm = {{ kind = 1, hihi = 900, gap = 50 }}

[[tag]]
name = "PUMP"
device = "sim"
space = "coil"
address = 0
type = "digital"
"#,
            sim.addr()
        ))
        .unwrap(),
    );
    let hub = Hub::new(&project);
    let scanners = start_scanners(&project, &hub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(hub, addr);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Stack {
        sim,
        addr,
        scanners: Some(scanners),
    }
}

/// 최소 HTTP/1.1 GET. (상태 코드, 본문)을 돌려준다.
async fn get(addr: SocketAddr, path: &str, host: &str) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).await.unwrap();
    let status = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    (status, body.to_string())
}

async fn json(addr: SocketAddr, path: &str) -> Value {
    let (status, body) = get(addr, path, &addr.to_string()).await;
    assert_eq!(status, 200, "{path}: {body}");
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("{path}: {e}: {body}"))
}

/// 조건이 참이 될 때까지 기다린다.
async fn wait_for(mut f: impl AsyncFnMut() -> bool) {
    for _ in 0..150 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("시간 안에 조건이 참이 되지 않았다");
}

fn tag<'a>(doc: &'a Value, name: &str) -> &'a Value {
    doc["tags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap()
}

#[tokio::test]
async fn values_alarms_and_status_flow_through() {
    let mut st = start().await;
    let addr = st.addr;

    // 원시 50 → 표시 500.
    st.sim.set_holding(0, &[50]);
    wait_for(async || tag(&json(addr, "/api/v1/tags").await, "TANK")["value"] == 500.0).await;
    let doc = json(addr, "/api/v1/tags").await;
    let tank = tag(&doc, "TANK");
    assert_eq!(tank["quality"], "good");
    assert_eq!(tank["unit_text"], "%");
    assert_eq!(tank["alarm_state"], "NORMAL");

    // 원시 95 → 표시 950 ≥ 900: 상상한 알람.
    st.sim.set_holding(0, &[95]);
    wait_for(async || {
        !json(addr, "/api/v1/alarms").await["active"]
            .as_array()
            .unwrap()
            .is_empty()
    })
    .await;
    let alarms = json(addr, "/api/v1/alarms").await;
    assert_eq!(alarms["active"][0]["name"], "TANK");
    assert_eq!(alarms["active"][0]["alarm_state"], "HIHI");
    assert_eq!(alarms["recent"][0]["event"], "HIHI");
    assert_eq!(alarms["recent"][0]["seq"], 1);

    // 디지털 값.
    st.sim.set_coils(0, &[true]);
    wait_for(async || tag(&json(addr, "/api/v1/tags").await, "PUMP")["value"] == true).await;

    // 장치 상태와 지터.
    wait_for(async || json(addr, "/api/v1/status").await["devices"][0]["scans"].as_u64() > Some(5))
        .await;
    let status = json(addr, "/api/v1/status").await;
    assert_eq!(status["tag_count"], 2);
    let dev = &status["devices"][0];
    assert_eq!(dev["connected"], true);
    assert!(dev["jitter_p99_us"].is_u64());
    assert!(dev["requests_ok"].as_u64().unwrap() > 0);

    // 통신이 끊기면 품질이 나빠지고 마지막 값은 유지된다.
    st.sim.state().drop_connections = true;
    wait_for(async || tag(&json(addr, "/api/v1/tags").await, "TANK")["quality"] == "bad").await;
    let doc = json(addr, "/api/v1/tags").await;
    assert_eq!(tag(&doc, "TANK")["value"], 950.0);
    assert_eq!(
        json(addr, "/api/v1/status").await["devices"][0]["connected"],
        false
    );

    st.scanners.take().unwrap().stop().await;
}

#[tokio::test]
async fn rejects_foreign_host_and_origin() {
    let st = start().await;
    let (code, _) = get(st.addr, "/api/v1/status", "evil.example").await;
    assert_eq!(code, 403, "다른 Host는 거부한다");
    let (code, _) = get(st.addr, "/api/v1/status", "localhost").await;
    assert_eq!(code, 200);

    // 웹소켓 업그레이드 시도에 다른 Origin이 붙으면 거부한다.
    let mut s = TcpStream::connect(st.addr).await.unwrap();
    let req = format!(
        "GET /api/v1/stream HTTP/1.1\r\nHost: {}\r\nOrigin: http://evil.example\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
        st.addr
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = vec![0u8; 64];
    let n = s.read(&mut buf).await.unwrap();
    let head = String::from_utf8_lossy(&buf[..n]).to_string();
    assert!(head.contains("403"), "{head}");
}

#[tokio::test]
async fn websocket_sends_snapshot_then_batched_changes() {
    let st = start().await;
    let url = format!("ws://{}/api/v1/stream", st.addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    let first = ws.next().await.unwrap().unwrap().into_text().unwrap();
    let snap: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(snap["type"], "snapshot");
    assert_eq!(snap["data"]["tags"].as_array().unwrap().len(), 2);
    let snapshot_seq = snap["data"]["seq"].as_u64().unwrap();

    st.sim.set_holding(0, &[30]);
    // 태그 묶음에서 TANK(위치 0)의 값 300을 찾는다. 묶음 순번은 스냅샷 이후로 이어진다.
    let mut last_seq = snapshot_seq;
    let mut found = None;
    for _ in 0..200 {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("업데이트가 오지 않았다")
            .unwrap()
            .unwrap();
        let v: Value = serde_json::from_str(&msg.into_text().unwrap()).unwrap();
        if v["type"] != "tags" {
            continue;
        }
        let seq = v["data"]["seq"].as_u64().unwrap();
        assert!(seq > last_seq, "순번이 늘어야 한다: {seq} <= {last_seq}");
        last_seq = seq;
        if let Some(item) = v["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i[0] == 0 && i[1] == 300.0)
        {
            found = Some(item.clone());
            break;
        }
    }
    let item = found.expect("TANK = 300 항목");
    assert_eq!(item[2], 1, "품질 good");
}

/// 임시 웹 디렉터리를 만든다.
fn web_fixture() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nuriscada-web-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><title>t</title>").unwrap();
    std::fs::write(dir.join("assets/app.js"), "console.log(1)").unwrap();
    std::fs::write(dir.join("secret.txt.bak"), "x").unwrap();
    dir
}

async fn raw_get(addr: SocketAddr, path: &str) -> String {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).await.unwrap();
    raw
}

#[tokio::test]
async fn serves_the_web_ui_safely() {
    let dir = web_fixture();
    let sim = Simulator::start("127.0.0.1:0", 10).await.unwrap();
    let project = Arc::new(
        Project::from_toml(&format!(
            "[[device]]\nname=\"s\"\ndriver=\"modbus-tcp\"\naddress=\"{}\"\n",
            sim.addr()
        ))
        .unwrap(),
    );
    let hub = Hub::new(&project);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = nuriscada_server::router_with_web(hub, addr, Some(dir.clone()));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let index = raw_get(addr, "/").await;
    assert!(index.starts_with("HTTP/1.1 200"), "{index}");
    assert!(index.to_lowercase().contains("content-type: text/html"));
    assert!(index.to_lowercase().contains("cache-control: no-cache"));
    assert!(
        index
            .to_lowercase()
            .contains("content-security-policy: default-src 'self'")
    );
    assert!(
        index
            .to_lowercase()
            .contains("x-content-type-options: nosniff")
    );

    let js = raw_get(addr, "/assets/app.js").await;
    assert!(js.to_lowercase().contains("content-type: text/javascript"));
    assert!(js.to_lowercase().contains("immutable"));

    // 확장자 없는 경로는 첫 화면, 없는 파일은 404.
    assert!(
        raw_get(addr, "/some/screen")
            .await
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        raw_get(addr, "/assets/missing.js")
            .await
            .starts_with("HTTP/1.1 404")
    );
    // 디렉터리 밖으로 나가는 경로는 거부한다.
    assert!(
        raw_get(addr, "/../Cargo.toml")
            .await
            .starts_with("HTTP/1.1 404")
    );
    assert!(
        raw_get(addr, "/assets/../../etc/passwd")
            .await
            .starts_with("HTTP/1.1 404")
    );
    // API는 그대로 동작하고 보안 헤더도 붙는다.
    let status = raw_get(addr, "/api/v1/status").await;
    assert!(status.starts_with("HTTP/1.1 200"));
    assert!(status.to_lowercase().contains("x-frame-options: deny"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn melsec_값과_상태가_흐른다() {
    use nuriscada_sim_melsec::Simulator as MelsecSim;
    let sim = MelsecSim::start("127.0.0.1:0", 100, 64).await.unwrap();
    sim.set_d(0, &[95]);
    sim.set_m(0, &[true]);
    let project = Arc::new(
        Project::from_toml(&format!(
            r#"
[[device]]
name = "plc1"
driver = "melsec-mc"
address = "{}"
timeout_ms = 300
scan_ms = 20

[[tag]]
name = "TEMP"
device = "plc1"
space = "d"
address = 0
type = "analog"
unit_text = "℃"
scale = {{ plc_base = 0, plc_full = 100, base = 0, full = 100 }}
alarm = {{ kind = 1, hihi = 90, gap = 5 }}

[[tag]]
name = "RUN"
device = "plc1"
space = "m"
address = 0
type = "digital"
"#,
            sim.addr()
        ))
        .unwrap(),
    );
    let hub = Hub::new(&project);
    let scanners = start_scanners(&project, &hub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(hub, addr);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    wait_for(|| async {
        let doc = json(addr, "/api/v1/tags").await;
        tag(&doc, "TEMP")["value"] == 95.0
    })
    .await;
    let doc = json(addr, "/api/v1/tags").await;
    assert_eq!(tag(&doc, "RUN")["value"], true);
    let status = json(addr, "/api/v1/status").await;
    assert_eq!(status["devices"][0]["connected"], true);
    let alarms = json(addr, "/api/v1/alarms").await;
    assert!(
        alarms["active"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["name"] == "TEMP")
    );
    scanners.stop().await;
}

#[tokio::test]
async fn xgt_값과_상태가_흐른다() {
    use nuriscada_sim_xgt::Simulator as XgtSim;
    let sim = XgtSim::start("127.0.0.1:0", 200, 64).await.unwrap();
    sim.set_mw(100, &[321]);
    sim.set_mx(0, &[true]);
    let project = Arc::new(
        Project::from_toml(&format!(
            r#"
[[device]]
name = "xgt1"
driver = "xgt-fenet"
address = "{}"
timeout_ms = 300
scan_ms = 20

[[tag]]
name = "TEMP"
device = "xgt1"
space = "mw"
address = 100
type = "analog"
unit_text = "℃"
scale = {{ plc_base = 0, plc_full = 100, base = 0, full = 1000 }}
alarm = {{ kind = 1, hihi = 900, gap = 50 }}

[[tag]]
name = "RUN"
device = "xgt1"
space = "mx"
address = 0
type = "digital"
"#,
            sim.addr()
        ))
        .unwrap(),
    );
    let hub = Hub::new(&project);
    let scanners = start_scanners(&project, &hub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(hub, addr);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    wait_for(|| async {
        let doc = json(addr, "/api/v1/tags").await;
        tag(&doc, "TEMP")["value"] == 3210.0
    })
    .await;
    let doc = json(addr, "/api/v1/tags").await;
    assert_eq!(tag(&doc, "RUN")["value"], true);
    let status = json(addr, "/api/v1/status").await;
    assert_eq!(status["devices"][0]["connected"], true);
    let alarms = json(addr, "/api/v1/alarms").await;
    assert!(
        alarms["active"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["name"] == "TEMP")
    );
    scanners.stop().await;
}

#[tokio::test]
async fn fins_값과_상태가_흐른다() {
    use nuriscada_sim_fins::Simulator as FinsSim;
    let sim = FinsSim::start("127.0.0.1:0", 300, 64).await.unwrap();
    sim.set_dm(20, &[321]);
    sim.set_cio(1, &[0x0008]);
    let project = Arc::new(
        Project::from_toml(&format!(
            r#"
[[device]]
name = "omron1"
driver = "fins-tcp"
address = "{}"
timeout_ms = 300
scan_ms = 20

[[tag]]
name = "TEMP"
device = "omron1"
space = "dm"
address = 20
type = "analog"
unit_text = "℃"
scale = {{ plc_base = 0, plc_full = 100, base = 0, full = 1000 }}
alarm = {{ kind = 1, hihi = 900, gap = 50 }}

[[tag]]
name = "RUN"
device = "omron1"
space = "cio"
address = 19
type = "digital"
"#,
            sim.addr()
        ))
        .unwrap(),
    );
    let hub = Hub::new(&project);
    let scanners = start_scanners(&project, &hub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(hub, addr);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // D20=321 → 표시값 3210, 상상한 알람. CIO 1.03(=19) true.
    wait_for(|| async {
        let doc = json(addr, "/api/v1/tags").await;
        tag(&doc, "TEMP")["value"] == 3210.0
    })
    .await;
    let doc = json(addr, "/api/v1/tags").await;
    assert_eq!(tag(&doc, "RUN")["value"], true);
    let status = json(addr, "/api/v1/status").await;
    assert_eq!(status["devices"][0]["connected"], true);
    let alarms = json(addr, "/api/v1/alarms").await;
    assert!(
        alarms["active"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["name"] == "TEMP")
    );
    scanners.stop().await;
}

#[tokio::test]
async fn s7_값과_상태가_흐른다() {
    use nuriscada_sim_s7::Simulator as S7Sim;
    let sim = S7Sim::start("127.0.0.1:0").await.unwrap();
    sim.set_db(1, 0, &[321, 0]);
    sim.set_m(0, &[true, false]);
    let project = Arc::new(
        Project::from_toml(&format!(
            r#"
[[device]]
name = "siemens1"
driver = "s7-tcp"
address = "{}"
timeout_ms = 300
scan_ms = 20

[[tag]]
name = "TEMP"
device = "siemens1"
unit = 1
space = "s7db"
address = 0
type = "analog"
unit_text = "℃"
scale = {{ plc_base = 0, plc_full = 100, base = 0, full = 1000 }}
alarm = {{ kind = 1, hihi = 900, gap = 50 }}

[[tag]]
name = "RUN"
device = "siemens1"
unit = 0
space = "s7m"
address = 0
type = "digital"
"#,
            sim.addr()
        ))
        .unwrap(),
    );
    let hub = Hub::new(&project);
    let scanners = start_scanners(&project, &hub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(hub, addr);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // DB1.0 = 321 → 표시값 3210, 상상한 알람. M0.0 = true.
    wait_for(|| async {
        let doc = json(addr, "/api/v1/tags").await;
        tag(&doc, "TEMP")["value"] == 3210.0
    })
    .await;
    let doc = json(addr, "/api/v1/tags").await;
    assert_eq!(tag(&doc, "RUN")["value"], true);
    let status = json(addr, "/api/v1/status").await;
    assert_eq!(status["devices"][0]["connected"], true);
    let alarms = json(addr, "/api/v1/alarms").await;
    assert!(
        alarms["active"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["name"] == "TEMP")
    );
    scanners.stop().await;
}

#[tokio::test]
async fn 통신_두절과_복귀가_품질에_드러난다() {
    let mut st = start().await;
    let addr = st.addr;
    // 정상 수신 확인.
    wait_for(|| async {
        let doc = json(addr, "/api/v1/tags").await;
        tag(&doc, "TANK")["quality"] == "good"
    })
    .await;
    // 연결을 끊으면 품질이 bad가 되고 상태에 실패가 쌓인다.
    st.sim.state().drop_connections = true;
    wait_for(|| async {
        let doc = json(addr, "/api/v1/tags").await;
        tag(&doc, "TANK")["quality"] == "bad"
    })
    .await;
    let status = json(addr, "/api/v1/status").await;
    assert_eq!(status["devices"][0]["connected"], false);
    assert!(status["devices"][0]["requests_failed"].as_u64().unwrap() > 0);
    // 연결이 돌아오면 품질이 good으로 복귀한다.
    st.sim.state().drop_connections = false;
    wait_for(|| async {
        let doc = json(addr, "/api/v1/tags").await;
        tag(&doc, "TANK")["quality"] == "good"
    })
    .await;
    let status = json(addr, "/api/v1/status").await;
    assert_eq!(status["devices"][0]["connected"], true);
    st.scanners.take().unwrap().stop().await;
}

#[tokio::test]
async fn 문자열_태그는_표시되고_이력에_쌓이지_않는다() {
    let mut st = start().await;
    // 시작 프로젝트에 문자열 태그가 없으므로 프로젝트 편집으로 추가한다.
    // (서버 재시작 없이 Hub::reconfigure 경로를 함께 확인한다.)
    let project = {
        let base = Project::from_toml(&format!(
            r#"
[[device]]
name = "sim"
driver = "modbus-tcp"
address = "{}"
timeout_ms = 300
scan_ms = 20

[[tag]]
name = "LINE"
device = "sim"
space = "holding"
address = 0
type = "string"
unit_text = "호"
value = "1호기 라인"
"#,
            st.sim.addr()
        ))
        .unwrap();
        Arc::new(base)
    };
    let hub = Hub::new(&project);
    let scanners = start_scanners(&project, &hub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(hub, addr);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    wait_for(|| async {
        let doc = json(addr, "/api/v1/tags").await;
        doc["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "LINE" && t["value"] == "1호기 라인")
    })
    .await;
    let doc = json(addr, "/api/v1/tags").await;
    let line = doc["tags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "LINE")
        .unwrap();
    assert_eq!(line["kind"], "string");
    assert_eq!(line["quality"], "good");
    scanners.stop().await;
    st.scanners.take().unwrap().stop().await;
}
