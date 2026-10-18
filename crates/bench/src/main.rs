// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 부하·지터 측정 도구 (설계 0001의 AC-3, AC-4, 결정 0005). 개발·CI 전용이며 배포물에 포함하지 않는다.
//!
//! 한 프로세스 안에서 시뮬레이터, 스캐너, 서버, 웹소켓 클라이언트를 모두 돌린다. 같은 기계의 CPU를
//! 함께 쓰므로 실제 배치보다 **불리한** 조건이다.
//!
//! 사용:
//! `nuriscada-bench [--tags 10000] [--clients 50] [--devices 10] [--scan-ms 100] [--duration-s 30]
//!  [--change-pct 10] [--check] [--json] [--web-dir web/dist --port 8080]`

use std::net::SocketAddr;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use nuriscada_project::Project;
use nuriscada_scheduler::{DeviceStatus, Hub};
use nuriscada_server::{router_with_web, start_scanners};
use nuriscada_sim_modbus::Simulator;
use tokio::net::TcpListener;

struct Args {
    tags: usize,
    clients: usize,
    devices: usize,
    scan_ms: u64,
    duration_s: u64,
    change_pct: u64,
    check: bool,
    json: bool,
    /// 지정하면 웹 화면도 제공해 브라우저를 붙여 볼 수 있다(포트는 `--port`).
    web_dir: Option<String>,
    port: u16,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        tags: 10_000,
        clients: 50,
        devices: 10,
        scan_ms: 100,
        duration_s: 30,
        change_pct: 10,
        check: false,
        json: false,
        web_dir: None,
        port: 0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut num = |name: &str| -> Result<u64, String> {
            it.next()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| format!("{name} 값이 필요하다"))
        };
        match arg.as_str() {
            "--tags" => a.tags = num("--tags")? as usize,
            "--clients" => a.clients = num("--clients")? as usize,
            "--devices" => a.devices = num("--devices")? as usize,
            "--scan-ms" => a.scan_ms = num("--scan-ms")?,
            "--duration-s" => a.duration_s = num("--duration-s")?,
            "--change-pct" => a.change_pct = num("--change-pct")?.min(100),
            "--check" => a.check = true,
            "--json" => a.json = true,
            "--web-dir" => a.web_dir = it.next(),
            "--port" => {
                a.port =
                    u16::try_from(num("--port")?).map_err(|_| "--port 범위 오류".to_string())?
            }
            other => return Err(format!("알 수 없는 인자: {other}")),
        }
    }
    if a.devices == 0 || a.tags < a.devices {
        return Err("태그 수는 장치 수 이상이어야 한다".into());
    }
    Ok(a)
}

/// 작은 의사 난수 생성기(xorshift). 외부 의존성을 피한다.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// 프로젝트 설정을 만든다: 장치마다 태그를 균등하게 나눈다. 10개마다 하나는 알람 종류 6을 가진다.
fn build_project(sims: &[Simulator], tags: usize, scan_ms: u64) -> String {
    let mut toml = String::new();
    for (d, sim) in sims.iter().enumerate() {
        toml.push_str(&format!(
            "[[device]]\nname = \"dev{d}\"\ndriver = \"modbus-tcp\"\naddress = \"{}\"\ntimeout_ms = 1000\nscan_ms = {scan_ms}\n\n",
            sim.addr()
        ));
    }
    let per = tags / sims.len();
    for d in 0..sims.len() {
        for i in 0..per {
            let alarm = if i % 10 == 0 {
                "alarm = { kind = 6, hihi = 90, high = 70, low = 30, lolo = 10, gap = 5 }\n"
            } else {
                ""
            };
            toml.push_str(&format!(
                "[[tag]]\nname = \"D{d}_T{i}\"\ndevice = \"dev{d}\"\nspace = \"holding\"\naddress = {i}\ntype = \"analog\"\nunit_text = \"%\"\nscale = {{ plc_base = 0, plc_full = 100, base = 0, full = 100 }}\n{alarm}\n"
            ));
        }
    }
    toml
}

/// (RSS KiB, 누적 CPU 초). 지원하지 않는 플랫폼이면 `None`.
fn process_usage() -> Option<(u64, f64)> {
    let out = Command::new("ps")
        .args(["-o", "rss=,time=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut parts = text.split_whitespace();
    let rss: u64 = parts.next()?.parse().ok()?;
    let time = parts.next()?;
    // 형식: [[HH:]MM:]SS[.ff]
    let mut secs = 0.0;
    for part in time.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().ok()?;
    }
    Some((rss, secs))
}

#[derive(Default)]
struct ClientCounters {
    frames: AtomicU64,
    bytes: AtomicU64,
    snapshots: AtomicU64,
    connected: AtomicU64,
}

async fn client(addr: SocketAddr, counters: Arc<ClientCounters>) {
    let url = format!("ws://{addr}/api/v1/stream");
    let Ok((mut ws, _)) = tokio_tungstenite::connect_async(url).await else {
        return;
    };
    counters.connected.fetch_add(1, Ordering::Relaxed);
    while let Some(Ok(msg)) = ws.next().await {
        if let Ok(text) = msg.into_text() {
            counters.frames.fetch_add(1, Ordering::Relaxed);
            counters
                .bytes
                .fetch_add(text.len() as u64, Ordering::Relaxed);
            // 프레임은 키 순서로 정렬되어 `"type"`이 끝에 온다.
            if text.ends_with("\"type\":\"snapshot\"}") {
                counters.snapshots.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

struct Report {
    args: Args,
    devices: Vec<DeviceStatus>,
    frames: u64,
    bytes: u64,
    snapshots: u64,
    connected: u64,
    rss_start: Option<u64>,
    rss_end: Option<u64>,
    rss_max: Option<u64>,
    cpu_pct: Option<f64>,
}

impl Report {
    fn period_us(&self) -> u64 {
        self.args.scan_ms * 1000
    }

    fn worst_p99(&self) -> u64 {
        self.devices
            .iter()
            .filter_map(|d| d.jitter_p99_us)
            .max()
            .unwrap_or(0)
    }

    fn worst_max(&self) -> u64 {
        self.devices
            .iter()
            .map(|d| d.jitter_max_us)
            .max()
            .unwrap_or(0)
    }

    fn overruns(&self) -> u64 {
        self.devices.iter().map(|d| d.overruns).sum()
    }

    fn failed(&self) -> u64 {
        self.devices.iter().map(|d| d.requests_failed).sum()
    }

    fn scans_min(&self) -> u64 {
        self.devices.iter().map(|d| d.scans).min().unwrap_or(0)
    }

    /// 합격 기준(결정 0005): 지터 99번째 백분위수가 주기의 20% 이내이고, 놓친 주기와 실패한 요청이 없고,
    /// 모든 클라이언트가 연결해 값을 받았다.
    fn failures(&self) -> Vec<String> {
        let mut f = Vec::new();
        let limit = self.period_us() / 5;
        if self.worst_p99() > limit {
            f.push(format!(
                "지터 p99 {} µs > 한도 {limit} µs",
                self.worst_p99()
            ));
        }
        if self.overruns() > 0 {
            f.push(format!("놓친 주기 {}회", self.overruns()));
        }
        if self.failed() > 0 {
            f.push(format!("실패한 요청 {}회", self.failed()));
        }
        if self.connected < self.args.clients as u64 {
            f.push(format!(
                "연결한 클라이언트 {} < {}",
                self.connected, self.args.clients
            ));
        }
        if self.args.clients > 0 && self.frames == 0 {
            f.push("클라이언트가 아무 것도 받지 못했다".into());
        }
        f
    }

    fn print(&self) {
        let a = &self.args;
        println!("== NuriScada 부하 측정 ==");
        println!(
            "구성: 태그 {} · 장치 {} · 클라이언트 {} · 주기 {} ms · 변화율 {}%/주기 · {} 초",
            a.tags, a.devices, a.clients, a.scan_ms, a.change_pct, a.duration_s
        );
        println!(
            "스캔: 장치당 최소 {} 회, 놓친 주기 {} 회, 실패한 요청 {} 회",
            self.scans_min(),
            self.overruns(),
            self.failed()
        );
        println!(
            "지터: p99 {:.2} ms (한도 {:.1} ms), 최댓값 {:.2} ms",
            self.worst_p99() as f64 / 1000.0,
            self.period_us() as f64 / 5000.0,
            self.worst_max() as f64 / 1000.0
        );
        println!(
            "웹소켓: 연결 {}/{}, 프레임 {}, 전송량 {:.1} MiB, 스냅샷 {}",
            self.connected,
            a.clients,
            self.frames,
            self.bytes as f64 / 1_048_576.0,
            self.snapshots
        );
        if let (Some(s), Some(e), Some(m)) = (self.rss_start, self.rss_end, self.rss_max) {
            println!(
                "메모리(RSS): 시작 {:.0} MiB → 끝 {:.0} MiB, 최대 {:.0} MiB",
                s as f64 / 1024.0,
                e as f64 / 1024.0,
                m as f64 / 1024.0
            );
        }
        if let Some(c) = self.cpu_pct {
            println!("CPU: 평균 {c:.0}% (코어 하나 = 100%)");
        }
        let failures = self.failures();
        if failures.is_empty() {
            println!("판정: 합격");
        } else {
            println!("판정: 불합격 — {}", failures.join("; "));
        }
    }

    fn print_json(&self) {
        let a = &self.args;
        let v = serde_json::json!({
            "tags": a.tags, "devices": a.devices, "clients": a.clients, "scan_ms": a.scan_ms,
            "change_pct": a.change_pct, "duration_s": a.duration_s,
            "scans_min": self.scans_min(), "overruns": self.overruns(), "requests_failed": self.failed(),
            "jitter_p99_us": self.worst_p99(), "jitter_max_us": self.worst_max(),
            "ws_connected": self.connected, "ws_frames": self.frames, "ws_bytes": self.bytes,
            "ws_snapshots": self.snapshots,
            "rss_start_kib": self.rss_start, "rss_end_kib": self.rss_end, "rss_max_kib": self.rss_max,
            "cpu_pct": self.cpu_pct, "pass": self.failures().is_empty(),
        });
        println!("{v}");
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    let per = args.tags / args.devices;
    let mut sims = Vec::new();
    for _ in 0..args.devices {
        match Simulator::start("127.0.0.1:0", per.max(1)).await {
            Ok(s) => sims.push(s),
            Err(e) => {
                eprintln!("시뮬레이터를 시작하지 못했다: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    let toml = build_project(&sims, args.tags, args.scan_ms);
    let project = match Project::from_toml(&toml) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let hub = Hub::new(&project);
    let scanners = start_scanners(&project, &hub);
    let listener = TcpListener::bind(("127.0.0.1", args.port))
        .await
        .expect("바인딩");
    let addr = listener.local_addr().expect("주소");
    let app = router_with_web(
        Arc::clone(&hub),
        addr,
        args.web_dir.as_ref().map(std::path::PathBuf::from),
    );
    if args.web_dir.is_some() {
        println!("웹 화면: http://{addr}/");
    }
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let counters = Arc::new(ClientCounters::default());
    for _ in 0..args.clients {
        tokio::spawn(client(addr, Arc::clone(&counters)));
    }

    let usage_start = process_usage();
    let wall_start = Instant::now();
    let mut rss_max = usage_start.map(|u| u.0);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let deadline = wall_start + Duration::from_secs(args.duration_s);
    let mut tick = tokio::time::interval(Duration::from_millis(args.scan_ms));
    let mut last_sample = Instant::now();

    // 값 갱신: 주기마다 레지스터의 일부를 무작위 걸음으로 바꾼다(알람을 오르내리게 한다).
    while Instant::now() < deadline {
        tick.tick().await;
        for sim in &sims {
            let n = per;
            let changes = n * usize::try_from(args.change_pct).unwrap_or(0) / 100;
            let mut state = sim.state();
            for _ in 0..changes {
                let idx = (rng.next() as usize) % n;
                let cur = i64::from(state.holding_registers[idx]);
                let step = (rng.next() % 21) as i64 - 10;
                state.holding_registers[idx] = (cur + step).clamp(0, 100) as u16;
            }
        }
        if last_sample.elapsed() >= Duration::from_secs(5) {
            last_sample = Instant::now();
            if let (Some((rss, _)), Some(m)) = (process_usage(), rss_max.as_mut()) {
                *m = (*m).max(rss);
            }
        }
    }

    let usage_end = process_usage();
    let wall = wall_start.elapsed().as_secs_f64();
    let devices = hub.devices();
    let cpu_pct = match (usage_start, usage_end) {
        (Some((_, a)), Some((_, b))) if wall > 0.0 => Some((b - a) / wall * 100.0),
        _ => None,
    };
    if let (Some((rss, _)), Some(m)) = (usage_end, rss_max.as_mut()) {
        *m = (*m).max(rss);
    }
    scanners.stop().await;

    let report = Report {
        args,
        devices,
        frames: counters.frames.load(Ordering::Relaxed),
        bytes: counters.bytes.load(Ordering::Relaxed),
        snapshots: counters.snapshots.load(Ordering::Relaxed),
        connected: counters.connected.load(Ordering::Relaxed),
        rss_start: usage_start.map(|u| u.0),
        rss_end: usage_end.map(|u| u.0),
        rss_max,
        cpu_pct,
    };
    if report.args.json {
        report.print_json();
    } else {
        report.print();
    }
    if report.args.check && !report.failures().is_empty() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
