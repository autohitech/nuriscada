// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 로컬 계정, 만료 세션과 순서·해시 체인 감사 기록. DB 작업은 blocking 스레드에서 호출한다.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use nuriscada_project::Project;
use nuriscada_scheduler::{AlarmRecord, Hub, Quality, TagSnapshot, TagValue, now_ms};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// 저장·암호 처리 오류.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// 계정 역할. 역할을 알 수 없으면 거부한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// 계정 관리·감사 조회·조작.
    Admin,
    /// 조작.
    Operator,
    /// 읽기 전용.
    Viewer,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Arc<Project> {
        Arc::new(Project::from_toml("").unwrap())
    }

    #[test]
    fn sessions_expire_and_failed_logins_are_bounded_and_audited() {
        let ops = Operations::open(Path::new(":memory:"), project(), false).unwrap();
        assert!(ops.needs_setup().unwrap());
        ops.create_user(
            "admin",
            "long-test-password",
            Role::Admin,
            true,
            "cli",
            "cli",
        )
        .unwrap();
        assert!(
            ops.create_user(
                "again",
                "long-test-password",
                Role::Admin,
                true,
                "cli",
                "cli"
            )
            .is_err()
        );
        let token = ops
            .login("admin", "long-test-password", "test")
            .unwrap()
            .unwrap();
        assert_eq!(ops.authenticate(&token).unwrap().unwrap().role, Role::Admin);
        ops.db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET expires=0", [])
            .unwrap();
        assert!(ops.authenticate(&token).unwrap().is_none());
        for _ in 0..9 {
            assert!(ops.login("admin", "wrong", "test").unwrap().is_none());
        }
        assert!(ops.login("admin", "wrong", "test").is_err());
        let audit = ops.audit_page(0).unwrap();
        assert_eq!(
            audit
                .iter()
                .filter(|r| r["event"]["action"] == "login")
                .count(),
            10
        );
        assert!(
            !serde_json::to_string(&audit)
                .unwrap()
                .contains("long-test-password")
        );
        ops.verify_audit().unwrap();
    }

    #[test]
    fn accounts_and_audit_survive_restart_and_tampering_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "nuriscada-ops-{}-{}.sqlite3",
            std::process::id(),
            random_hex().unwrap()
        ));
        {
            let ops = Operations::open(&path, project(), false).unwrap();
            ops.create_user(
                "admin",
                "long-test-password",
                Role::Admin,
                true,
                "cli",
                "cli",
            )
            .unwrap();
            ops.save_alarms(&[AlarmRecord {
                seq: 1,
                time_ms: now_ms(),
                tag: "LEVEL".into(),
                event: "HIHI",
                value: 95.0,
            }])
            .unwrap();
            assert!(ops.acknowledge(1, "admin", "test").unwrap());
        }
        {
            let ops = Operations::open(&path, project(), false).unwrap();
            assert!(!ops.needs_setup().unwrap());
            assert_eq!(ops.alarm_page(0, None).unwrap()[0]["ack_actor"], "admin");
            assert!(
                ops.login("admin", "long-test-password", "restart")
                    .unwrap()
                    .is_some()
            );
            ops.db
                .lock()
                .unwrap()
                .execute("UPDATE audit SET payload='{}' WHERE seq=1", [])
                .unwrap();
            assert!(ops.verify_audit().is_err());
        }
        assert!(Operations::open(&path, project(), false).is_err());
        let _ = std::fs::remove_file(path);
    }
}

impl Role {
    /// DB에 저장하는 역할 이름.
    pub fn name(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }
}

/// 인증된 세션. 원문 세션 토큰은 응답에 직렬화하지 않는다.
#[derive(Debug, Clone, Serialize)]
pub struct Principal {
    /// 사용자 이름.
    pub username: String,
    /// 역할.
    pub role: Role,
    /// 절대 만료 시각(UTC ms).
    pub expires_ms: u64,
}

/// 감사 사건. 비밀번호·세션 토큰을 넣지 않는다.
pub struct Event<'a> {
    /// 행위자.
    pub actor: &'a str,
    /// 실제 접속 출처 또는 로컬 CLI.
    pub source: &'a str,
    /// 행위 종류.
    pub action: &'a str,
    /// 대상.
    pub target: &'a str,
    /// 이전·새 값과 결과.
    pub detail: Value,
}

fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

fn random_hex() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn hash_password(password: &str) -> Result<String> {
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| e.to_string())?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| e.to_string())?
        .to_string())
}

fn append(conn: &Connection, event: Event<'_>) -> Result<()> {
    let previous: Option<(i64, String)> = conn
        .query_row(
            "SELECT seq, hash FROM audit ORDER BY seq DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (seq, previous) = previous.map_or((1, String::new()), |(n, h)| (n + 1, h));
    let payload = json!({"time_ms": now_ms(), "actor": event.actor, "source": event.source,
        "action": event.action, "target": event.target, "detail": event.detail})
    .to_string();
    let hash = digest(format!("{seq}\n{previous}\n{payload}"));
    conn.execute(
        "INSERT INTO audit(seq,payload,previous,hash) VALUES (?1,?2,?3,?4)",
        params![seq, payload, previous, hash],
    )?;
    Ok(())
}

/// SQLite와 유한한 로그인·쓰기 동시 처리 제어를 소유한다.
pub struct Operations {
    run_id: String,
    archive_error: Mutex<Option<String>>,
    /// 공개 모드는 계정·조작 변경을 서버에서 거부한다.
    pub public_read_only: bool,
    db: Mutex<Connection>,
    attempts: Mutex<VecDeque<Instant>>,
    dummy_hash: String,
    project: Mutex<Arc<Project>>,
    config_path: Option<std::path::PathBuf>,
    scanners: tokio::sync::Mutex<Option<crate::Scanners>>,
    /// 로컬 쓰기를 명시적으로 허용했는지. 접속 경로 검사도 별도로 수행한다.
    pub writes_enabled: bool,
    /// 비밀번호 연산 동시 작업 한도.
    pub login_slots: Arc<tokio::sync::Semaphore>,
    /// 대기 중·실행 중 쓰기 요청 한도.
    pub write_slots: Arc<tokio::sync::Semaphore>,
    /// 쓰기는 수락 순서대로 한 번에 하나만 실행한다.
    pub write_lock: tokio::sync::Mutex<()>,
}

impl Operations {
    /// DB를 열고 스키마·감사 체인을 검사한다. 손상된 감사 기록은 덮어쓰지 않는다.
    pub fn open(path: &Path, project: Arc<Project>, writes_enabled: bool) -> Result<Arc<Self>> {
        Self::open_with_mode(path, project, writes_enabled, false)
    }

    /// 프록시 뒤의 루프백 바인딩에도 공개 읽기 전용 모드를 명시할 수 있다.
    pub fn open_with_mode(
        path: &Path,
        project: Arc<Project>,
        writes_enabled: bool,
        public_read_only: bool,
    ) -> Result<Arc<Self>> {
        Self::open_with_config(path, project, None, writes_enabled, public_read_only)
    }

    /// 설정 파일 경로를 함께 연결해 런타임 편집 시 파일 저장까지 수행한다.
    pub fn open_with_config(
        path: &Path,
        project: Arc<Project>,
        config_path: Option<std::path::PathBuf>,
        writes_enabled: bool,
        public_read_only: bool,
    ) -> Result<Arc<Self>> {
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS users(name TEXT PRIMARY KEY, password TEXT NOT NULL, role TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS sessions(token TEXT PRIMARY KEY, name TEXT NOT NULL, expires INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS audit(seq INTEGER PRIMARY KEY, payload TEXT NOT NULL, previous TEXT NOT NULL, hash TEXT NOT NULL);")?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS alarms(id INTEGER PRIMARY KEY, run TEXT NOT NULL, seq INTEGER NOT NULL, tag TEXT NOT NULL, data TEXT NOT NULL, ack_actor TEXT, ack_ms INTEGER, UNIQUE(run,seq));
            CREATE INDEX IF NOT EXISTS alarms_tag_id ON alarms(tag,id);
            CREATE TABLE IF NOT EXISTS tag_history(id INTEGER PRIMARY KEY, tag TEXT NOT NULL, time_ms INTEGER NOT NULL, kind TEXT NOT NULL, value REAL, quality TEXT NOT NULL, alarm_state TEXT);
            CREATE INDEX IF NOT EXISTS tag_history_tag_time ON tag_history(tag,time_ms,id);")?;
        let ops = Arc::new(Self {
            run_id: random_hex()?,
            archive_error: Mutex::new(None),
            public_read_only,
            db: Mutex::new(db),
            attempts: Mutex::new(VecDeque::new()),
            dummy_hash: hash_password(&random_hex()?)?,
            project: Mutex::new(project),
            config_path,
            scanners: tokio::sync::Mutex::new(None),
            writes_enabled: writes_enabled && !public_read_only,
            login_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            write_slots: Arc::new(tokio::sync::Semaphore::new(16)),
            write_lock: tokio::sync::Mutex::new(()),
        });
        ops.verify_audit()?;
        Ok(ops)
    }

    /// 현재 활성 프로젝트.
    pub fn project(&self) -> Arc<Project> {
        self.project.lock().expect("프로젝트 잠금").clone()
    }

    /// 스캐너 핸들을 등록한다(런타임 설정 변경 시 재시작에 사용).
    pub async fn attach_scanners(&self, scanners: crate::Scanners) {
        *self.scanners.lock().await = Some(scanners);
    }

    /// 스캐너를 멈추고 회수한다.
    pub async fn stop_scanners(&self) {
        if let Some(scanners) = self.scanners.lock().await.take() {
            scanners.stop().await;
        }
    }

    /// 프로젝트를 검증·저장·교체하고 감사 기록을 남긴다.
    pub fn save_project(
        &self,
        next: Arc<Project>,
        actor: &str,
        source: &str,
    ) -> Result<Arc<Project>> {
        let toml_text = next.to_toml();
        if let Some(cfg) = &self.config_path {
            let parent = cfg.parent().unwrap_or_else(|| Path::new("."));
            let stem = cfg
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project.toml");
            let tmp = parent.join(format!(".{stem}.{}.tmp", random_hex()?));
            let backup = cfg.with_extension("toml.bak");
            if cfg.exists() {
                let _ = std::fs::copy(cfg, &backup);
            }
            let mut file = std::fs::File::create(&tmp)?;
            use std::io::Write;
            file.write_all(toml_text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            if let Err(e) = std::fs::rename(&tmp, cfg) {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("설정 파일 교체 실패: {e}").into());
            }
        }
        let prev = {
            let mut guard = self.project.lock().map_err(|_| "프로젝트 잠금 오류")?;
            std::mem::replace(&mut *guard, Arc::clone(&next))
        };
        let detail = json!({
            "previous_devices": prev.devices.len(),
            "previous_tags": prev.tags.len(),
            "devices": next.devices.len(),
            "tags": next.tags.len(),
            "persisted": self.config_path.is_some(),
        });
        self.audit(Event {
            actor,
            source,
            action: "project.update",
            target: "project",
            detail,
        })?;
        Ok(prev)
    }

    /// 관리자 부트스트랩이 필요한지.
    pub fn needs_setup(&self) -> Result<bool> {
        let conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        Ok(conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))? == 0)
    }

    /// 계정을 만든다. bootstrap이면 계정이 없는 DB에서만 관리자를 만들 수 있다.
    pub fn create_user(
        &self,
        name: &str,
        password: &str,
        role: Role,
        bootstrap: bool,
        actor: &str,
        source: &str,
    ) -> Result<()> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        {
            return Err("계정 이름은 영문·숫자·_-. 1~64자여야 한다".into());
        }
        if !(12..=128).contains(&password.len()) {
            return Err("비밀번호는 UTF-8 기준 12~128바이트여야 한다".into());
        }
        let hash = hash_password(password)?;
        let mut conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let tx = conn.transaction()?;
        let empty = tx.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))? == 0;
        if bootstrap && (!empty || role != Role::Admin) {
            return Err("최초 관리자 설정은 비어 있는 DB에서 한 번만 가능하다".into());
        }
        tx.execute(
            "INSERT INTO users(name,password,role) VALUES (?1,?2,?3)",
            params![name, hash, role.name()],
        )?;
        append(
            &tx,
            Event {
                actor,
                source,
                action: "user.create",
                target: name,
                detail: json!({"role":role}),
            },
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 로그인은 서비스 전체에서 60초에 10회까지만 시도한다. 실패도 감사 기록을 남긴다.
    pub fn login(&self, name: &str, password: &str, source: &str) -> Result<Option<String>> {
        {
            let mut attempts = self.attempts.lock().map_err(|_| "로그인 잠금 오류")?;
            while attempts
                .front()
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(60))
            {
                attempts.pop_front();
            }
            if attempts.len() >= 10 {
                return Err("로그인 시도 제한".into());
            }
            attempts.push_back(Instant::now());
        }
        let hash: Option<String> = self
            .db
            .lock()
            .map_err(|_| "DB 잠금 오류")?
            .query_row("SELECT password FROM users WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .optional()?;
        let parsed = PasswordHash::new(hash.as_deref().unwrap_or(&self.dummy_hash))
            .map_err(|e| e.to_string())?;
        let valid = Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
            && hash.is_some();
        let mut conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let tx = conn.transaction()?;
        append(
            &tx,
            Event {
                actor: name,
                source,
                action: "login",
                target: name,
                detail: json!({"success":valid}),
            },
        )?;
        if !valid {
            tx.commit()?;
            return Ok(None);
        }
        let token = random_hex()?;
        let now = i64::try_from(now_ms())?;
        tx.execute("DELETE FROM sessions WHERE expires <= ?1", [now])?;
        // 사용자당 최신 세션 하나만 유지한다.
        tx.execute("DELETE FROM sessions WHERE name=?1", [name])?;
        tx.execute(
            "INSERT INTO sessions(token,name,expires) VALUES (?1,?2,?3)",
            params![digest(&token), name, now + 8 * 60 * 60 * 1000],
        )?;
        tx.commit()?;
        Ok(Some(token))
    }

    /// 토큰의 만료와 현재 계정 역할을 확인한다.
    pub fn authenticate(&self, token: &str) -> Result<Option<Principal>> {
        let conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let row: Option<(String,String,i64)> = conn.query_row(
            "SELECT u.name,u.role,s.expires FROM sessions s JOIN users u ON u.name=s.name WHERE s.token=?1 AND s.expires>?2",
            params![digest(token),i64::try_from(now_ms())?], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        row.map(|(username, role, expires_ms)| {
            Ok(Principal {
                username,
                role: serde_json::from_value(json!(role))?,
                expires_ms: u64::try_from(expires_ms)?,
            })
        })
        .transpose()
    }

    /// 로그아웃과 세션 폐기를 같은 트랜잭션에 기록한다.
    pub fn logout(&self, token: &str, actor: &str, source: &str) -> Result<()> {
        let mut conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM sessions WHERE token=?1", [digest(token)])?;
        append(
            &tx,
            Event {
                actor,
                source,
                action: "logout",
                target: actor,
                detail: json!({}),
            },
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 감사 기록을 영구 저장한다. 실패하면 호출자는 조작을 중단해야 한다.
    pub fn audit(&self, event: Event<'_>) -> Result<()> {
        let mut conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let tx = conn.transaction()?;
        append(&tx, event)?;
        tx.commit()?;
        Ok(())
    }

    /// 페이지 단위 감사 조회. 최대 200개.
    pub fn audit_page(&self, after: u64) -> Result<Vec<Value>> {
        let conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let mut query = conn.prepare(
            "SELECT seq,payload,previous,hash FROM audit WHERE seq>?1 ORDER BY seq LIMIT 200",
        )?;
        let rows = query.query_map([i64::try_from(after).unwrap_or(i64::MAX)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| { let (seq,payload,previous,hash) = row?;
            Ok(json!({"seq":seq,"event":serde_json::from_str::<Value>(&payload)?,"previous":previous,"hash":hash})) }).collect()
    }

    /// 체인 중간의 변경·순서 누락을 검사한다. DB 관리자에 의한 전체 재작성·끝부분 삭제는 별도 외부 앵커 없이는 검출하지 못한다.
    pub fn verify_audit(&self) -> Result<()> {
        let conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let mut query = conn.prepare("SELECT seq,payload,previous,hash FROM audit ORDER BY seq")?;
        let mut rows = query.query([])?;
        let mut previous = String::new();
        let mut expected = 1i64;
        while let Some(row) = rows.next()? {
            let seq: i64 = row.get(0)?;
            let payload: String = row.get(1)?;
            let prev: String = row.get(2)?;
            let hash: String = row.get(3)?;
            if seq != expected
                || prev != previous
                || hash != digest(format!("{seq}\n{prev}\n{payload}"))
            {
                return Err("감사 체인 검증 실패".into());
            }
            previous = hash;
            expected += 1;
        }
        Ok(())
    }

    /// 알람 묶음을 영구 저장한다. 같은 실행·순번은 한 번만 저장한다.
    pub fn save_alarms(&self, records: &[AlarmRecord]) -> Result<()> {
        let mut conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let tx = conn.transaction()?;
        for record in records {
            tx.execute(
                "INSERT OR IGNORE INTO alarms(run,seq,tag,data) VALUES (?1,?2,?3,?4)",
                params![
                    self.run_id,
                    i64::try_from(record.seq)?,
                    record.tag,
                    serde_json::to_string(record)?
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 알람 이력 조회. 최신 항목부터 최대 200개, before=0이면 첫 페이지다.
    pub fn alarm_page(&self, before: u64, tag: Option<String>) -> Result<Vec<Value>> {
        let conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let mut query=conn.prepare("SELECT id,data,ack_actor,ack_ms FROM alarms WHERE (?1=0 OR id<?1) AND (?2 IS NULL OR tag=?2) ORDER BY id DESC LIMIT 200")?;
        let rows = query.query_map(
            params![i64::try_from(before).unwrap_or(i64::MAX), tag],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )?;
        rows.map(|row|{let(id,data,actor,time)=row?;Ok(json!({"id":id,"data":serde_json::from_str::<Value>(&data)?,"ack_actor":actor,"ack_ms":time}))}).collect()
    }

    /// 발생 기록 한 건을 확인한다. 알람 정상 복귀와 구분하며 중복 확인은 변경하지 않는다.
    pub fn acknowledge(&self, id: u64, actor: &str, source: &str) -> Result<bool> {
        let mut conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let tx = conn.transaction()?;
        let event: Option<String> = tx
            .query_row(
                "SELECT data FROM alarms WHERE id=?1",
                [i64::try_from(id)?],
                |r| r.get(0),
            )
            .optional()?;
        let event: Value = serde_json::from_str(&event.ok_or("알람 기록이 없다")?)?;
        if matches!(event["event"].as_str(), Some("CNF" | "RETURN")) {
            return Err("발생 알람만 확인할 수 있다".into());
        }
        let changed = tx.execute(
            "UPDATE alarms SET ack_actor=?1,ack_ms=?2 WHERE id=?3 AND ack_actor IS NULL",
            params![actor, i64::try_from(now_ms())?, i64::try_from(id)?],
        )? == 1;
        if changed {
            append(
                &tx,
                Event {
                    actor,
                    source,
                    action: "alarm.ack",
                    target: &id.to_string(),
                    detail: json!({"tag":event["tag"],"event":event["event"]}),
                },
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// 변경되거나 주기 기준을 넘긴 태그 값 표본을 저장하고 태그당 최신 10,000개로 제한한다.
    pub fn save_tag_samples(&self, samples: &[TagSnapshot]) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        let mut conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let tx = conn.transaction()?;
        for s in samples {
            let val: Option<f64> = s.value.map(|v| match v {
                TagValue::Number(n) => n,
                TagValue::Bool(b) => {
                    if b {
                        1.0
                    } else {
                        0.0
                    }
                }
            });
            let quality = match s.quality {
                Quality::Good => "good",
                Quality::Bad => "bad",
            };
            let t = i64::try_from(if s.changed_ms == 0 {
                now_ms()
            } else {
                s.changed_ms
            })?;
            tx.execute(
                "INSERT INTO tag_history(tag,time_ms,kind,value,quality,alarm_state) VALUES (?1,?2,?3,?4,?5,?6)",
                params![s.name, t, s.kind, val, quality, s.alarm_state],
            )?;
            let last_id = tx.last_insert_rowid();
            if last_id % 200 == 0 {
                tx.execute(
                    "DELETE FROM tag_history WHERE tag=?1 AND id NOT IN (SELECT id FROM tag_history WHERE tag=?1 ORDER BY id DESC LIMIT 10000)",
                    [&s.name],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 태그의 시계열 이력을 조회한다(시간 오름차순, 최근 limit개).
    pub fn tag_history(
        &self,
        tag: &str,
        from_ms: Option<u64>,
        to_ms: Option<u64>,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let limit = limit.clamp(1, 2000);
        let from = i64::try_from(from_ms.unwrap_or(0)).unwrap_or(0);
        let to = i64::try_from(to_ms.unwrap_or(u64::MAX)).unwrap_or(i64::MAX);
        let conn = self.db.lock().map_err(|_| "DB 잠금 오류")?;
        let mut query = conn.prepare(
            "SELECT id,time_ms,kind,value,quality,alarm_state FROM tag_history \
             WHERE tag=?1 AND time_ms>=?2 AND time_ms<=?3 ORDER BY time_ms DESC, id DESC LIMIT ?4",
        )?;
        let rows = query.query_map(
            params![tag, from, to, i64::try_from(limit).unwrap_or(500)],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            let (id, time_ms, kind, val, quality, alarm_state) = row?;
            let value = match (kind.as_str(), val) {
                ("digital", Some(n)) => json!(n != 0.0),
                (_, Some(n)) => json!(n),
                (_, None) => Value::Null,
            };
            out.push(json!({
                "id": id,
                "time_ms": time_ms,
                "kind": kind,
                "value": value,
                "numeric": val,
                "quality": quality,
                "alarm_state": alarm_state,
            }));
        }
        out.reverse();
        Ok(out)
    }

    /// 기록 작업의 오류 상태. 한 번 발생한 손실·저장 오류는 숨기지 않는다.
    pub fn archive_status(&self) -> Value {
        let error = self
            .archive_error
            .lock()
            .map(|e| e.clone())
            .unwrap_or(Some("기록 상태 잠금 오류".into()));
        json!({"healthy":error.is_none(),"error":error})
    }

    /// 최근 알람과 태그 값 표본을 200 ms마다 저장한다. 오류·링 버퍼 손실을 진단 상태에 남긴다.
    pub fn start_archive(self: &Arc<Self>, hub: Arc<Hub>) -> AlarmArchive {
        let (shutdown, mut stop) = tokio::sync::watch::channel(false);
        let ops = self.clone();
        let task = tokio::spawn(async move {
            type SavedTagState = (u64, Option<TagValue>, Quality, Option<&'static str>);
            let mut cursor = 0;
            let mut last_tag_saved: std::collections::HashMap<String, SavedTagState> =
                std::collections::HashMap::new();
            let mut tick = tokio::time::interval(Duration::from_millis(200));
            loop {
                tokio::select! {_=tick.tick()=>{},changed=stop.changed()=>{if changed.is_err(){return;}}}
                let records: Vec<_> = hub
                    .recent_alarms()
                    .into_iter()
                    .filter(|r| r.seq > cursor)
                    .collect();
                if let Some(first) = records.first()
                    && first.seq > cursor + 1
                    && let Ok(mut error) = ops.archive_error.lock()
                {
                    *error = Some(format!(
                        "알람 기록 손실: 순번 {}~{}",
                        cursor + 1,
                        first.seq - 1
                    ));
                }
                if !records.is_empty() {
                    let last = records.last().expect("알람 묶음").seq;
                    let db = ops.clone();
                    match tokio::task::spawn_blocking(move || db.save_alarms(&records)).await {
                        Ok(Ok(())) => cursor = last,
                        result => {
                            if let Ok(mut error) = ops.archive_error.lock() {
                                *error = Some(format!("알람 저장 실패: {result:?}"));
                            }
                        }
                    }
                }
                let now = now_ms();
                let mut samples = Vec::new();
                for mut t in hub.tags() {
                    if t.changed_ms == 0 && t.value.is_none() {
                        continue;
                    }
                    let should_save = match last_tag_saved.get(&t.name) {
                        None => true,
                        Some((saved_ms, val, q, st)) => {
                            *val != t.value
                                || *q != t.quality
                                || *st != t.alarm_state
                                || now.saturating_sub(*saved_ms) >= 5_000
                        }
                    };
                    if should_save {
                        if now.saturating_sub(t.changed_ms) >= 1_000 {
                            t.changed_ms = now;
                        }
                        last_tag_saved
                            .insert(t.name.clone(), (now, t.value, t.quality, t.alarm_state));
                        samples.push(t);
                    }
                }
                if !samples.is_empty() {
                    let db = ops.clone();
                    if let result @ (Err(_) | Ok(Err(_))) =
                        tokio::task::spawn_blocking(move || db.save_tag_samples(&samples)).await
                        && let Ok(mut error) = ops.archive_error.lock()
                    {
                        *error = Some(format!("태그 이력 저장 실패: {result:?}"));
                    }
                }
                if *stop.borrow() {
                    return;
                }
            }
        });
        AlarmArchive { shutdown, task }
    }
}

/// 알람 기록 작업의 종료 핸들.
pub struct AlarmArchive {
    shutdown: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl AlarmArchive {
    /// 스캐너를 멈춘 다음 호출하면 마지막 묶음을 저장하고 종료한다.
    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
    }
}
