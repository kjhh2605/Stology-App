//! Text-only, durable provider sessions. No document contents enter this module.
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{io::AsRawFd, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// One provider execution at a time. A chat turn and each step of a run hold it.
pub struct ExecutionLock(fs::File);
impl ExecutionLock {
    fn lock(root: &Path, name: &str, busy: &str) -> Result<Self> {
        fs::create_dir_all(root)?;
        let f = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(name))?;
        anyhow::ensure!(unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0, "{busy}");
        Ok(Self(f))
    }
    pub fn acquire(root: &Path) -> Result<Self> {
        Self::lock(root, "ai-execution.lock", "다른 AI 작업이 진행 중입니다. 원래 과목에서 완료하거나 중지하세요.")
    }
    /// Held for a whole chat turn, including while it waits behind a run's step.
    /// Saved turns may only be rewritten when no turn is running.
    fn chat_turn(root: &Path) -> Result<Self> {
        Self::lock(root, "chat-turn.lock", "다른 대화 답변이 진행 중입니다. 원래 과목에서 완료하거나 중지하세요.")
    }
}
impl Drop for ExecutionLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
/// Asks a run to let this chat turn go first once its current step ends.
struct YieldRequest(std::path::PathBuf);
impl Drop for YieldRequest {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
/// The execution lock for a chat turn. A chat-note job in the back lane is
/// cancelled and queued again; a run in progress finishes its step first.
fn take_turn(root: &Path, control: &mpsc::Receiver<String>, out: &mut impl Write) -> Result<ExecutionLock> {
    if let Ok(lock) = ExecutionLock::acquire(root) {
        let active = super::providers::handle(root, &json!({"op":"analysis.active"}))?["active"].clone();
        match active["kind"].as_str() {
            None | Some("run") => return Ok(lock),
            Some("distill") => {
                super::providers::handle(root, &json!({"op":"analysis.cancel","id":active["id"]}))?;
                return Ok(lock);
            }
            Some(_) => bail!("다른 AI 작업이 진행 중입니다. 원래 과목에서 완료하거나 중지하세요."),
        }
    }
    let active = super::providers::handle(root, &json!({"op":"analysis.active"}))?["active"].clone();
    match active["kind"].as_str() {
        Some("distill") => {
            super::providers::handle(root, &json!({"op":"analysis.cancel","id":active["id"]}))?;
        }
        Some("run") => {
            writeln!(out, "{}", json!({"event":"waiting","workspace_id":active["workspace_id"],"message":"과목 정리의 지금 단계가 끝나면 바로 답변해요"}))?;
            out.flush()?;
        }
        _ => bail!("다른 AI 작업이 진행 중입니다. 원래 과목에서 완료하거나 중지하세요."),
    }
    let path = crate::organize::yield_request(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, b"chat")?;
    let _request = YieldRequest(path.clone());
    let mut touched = Instant::now();
    loop {
        if let Ok(lock) = ExecutionLock::acquire(root) {
            return Ok(lock);
        }
        match control.try_recv() {
            Ok(_) | Err(mpsc::TryRecvError::Disconnected) => bail!("질문을 취소했습니다"),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        // Keep the request fresh: a run ignores one left behind by a dead process.
        if touched.elapsed() > Duration::from_secs(60) {
            fs::write(&path, b"chat")?;
            touched = Instant::now();
        }
        thread::sleep(Duration::from_millis(200));
    }
}
pub(crate) fn db(root: &Path) -> Result<Connection> {
    fs::create_dir_all(root)?;
    let mut c = Connection::open(root.join("chat.sqlite3"))?;
    c.busy_timeout(Duration::from_secs(2))?;
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
CREATE TABLE IF NOT EXISTS conversations(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,provider TEXT NOT NULL,session TEXT,draft TEXT NOT NULL DEFAULT '',needs_new INTEGER NOT NULL DEFAULT 0,created TEXT NOT NULL,updated TEXT NOT NULL,UNIQUE(workspace,provider));
CREATE TABLE IF NOT EXISTS turns(id TEXT PRIMARY KEY,conversation TEXT NOT NULL,run TEXT NOT NULL,user TEXT NOT NULL,answer TEXT NOT NULL DEFAULT '',state TEXT NOT NULL,error TEXT NOT NULL DEFAULT '',created TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS preferences(key TEXT PRIMARY KEY,value TEXT NOT NULL);")?;
    if c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? == 0 {
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if tx.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? == 0 {
            tx.execute_batch("CREATE TABLE conversations_multi(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,
                provider TEXT NOT NULL,session TEXT,draft TEXT NOT NULL DEFAULT '',needs_new INTEGER NOT NULL DEFAULT 0,
                created TEXT NOT NULL,updated TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 1);
                INSERT INTO conversations_multi SELECT *,1 FROM conversations;
                DROP TABLE conversations; ALTER TABLE conversations_multi RENAME TO conversations;
                CREATE UNIQUE INDEX current_conversation ON conversations(workspace,provider) WHERE active=1;
                CREATE INDEX conversation_turns ON turns(conversation);
                PRAGMA user_version=1;")?;
        }
        tx.commit()?;
    }
    let has_model: bool = c.prepare("PRAGMA table_info(turns)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?.iter().any(|n| n == "model");
    if !has_model { c.execute("ALTER TABLE turns ADD COLUMN model TEXT NOT NULL DEFAULT ''", [])?; }
    let has_effort = c.prepare("PRAGMA table_info(turns)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?.iter().any(|n| n == "effort");
    if !has_effort { c.execute("ALTER TABLE turns ADD COLUMN effort TEXT NOT NULL DEFAULT ''", [])?; }
    let has_images = c.prepare("PRAGMA table_info(turns)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?.iter().any(|n| n == "images");
    if !has_images { c.execute("ALTER TABLE turns ADD COLUMN images INTEGER NOT NULL DEFAULT 0", [])?; }
    crate::distill::schema(&c)?;
    Ok(c)
}
fn key(r: &Value) -> Result<(&str, &str)> {
    let ws = r["workspace_id"]
        .as_str()
        .context("과목을 선택하면 대화를 시작할 수 있어요")?;
    uuid::Uuid::parse_str(ws).context("잘못된 과목 ID")?;
    let p = r["provider"].as_str().context("공급자를 선택하세요")?;
    anyhow::ensure!(["codex", "claude", "antigravity"].contains(&p), "지원하지 않는 공급자");
    Ok((ws, p))
}
fn conversation(c: &Connection, r: &Value) -> Result<String> {
    let (ws, p) = key(r)?;
    let now = chrono::Utc::now().to_rfc3339();
    c.execute("INSERT OR IGNORE INTO conversations(id,workspace,provider,created,updated) VALUES(?1,?2,?3,?4,?4)",params![uuid::Uuid::new_v4().to_string(),ws,p,now])?;
    Ok(c.query_row(
        "SELECT id FROM conversations WHERE workspace=?1 AND provider=?2 AND active=1",
        params![ws, p],
        |r| r.get(0),
    )?)
}
fn recover(c: &Connection) -> Result<()> {
    c.execute("UPDATE turns SET state='interrupted',error='앱 종료로 중단되었습니다. 자동 재전송하지 않았습니다.' WHERE state='running'",[])?;
    Ok(())
}
pub fn handle(root: &Path, r: &Value) -> Result<Value> {
    if r["op"] == "chat.models" { return models(root, r["provider"].as_str().unwrap_or("codex")); }
    let c = db(root)?;
    let op = r["op"].as_str().unwrap_or("");
    if op == "chat.preferences" {
        if let Some(effort) = r["effort"].as_str() { validate_effort(effort)?; }
        if let Some(model) = r["model"].as_str() {
            let provider: String = c.query_row("SELECT value FROM preferences WHERE key='provider'", [], |r| r.get(0)).unwrap_or_else(|_| "codex".into());
            validate_model(r["provider"].as_str().unwrap_or(&provider), model)?;
        }
        if let Some(mode) = r["mode"].as_str() {
            anyhow::ensure!(["materials", "chat"].contains(&mode), "잘못된 패널 모드");
            c.execute(
                "INSERT OR REPLACE INTO preferences VALUES('mode',?1)",
                [mode],
            )?;
        }
        let mode: String = c
            .query_row("SELECT value FROM preferences WHERE key='mode'", [], |r| {
                r.get(0)
            })
            .unwrap_or_else(|_| "materials".into());
        if let Some(provider) = r["provider"].as_str() {
            anyhow::ensure!(
                ["codex", "claude", "antigravity"].contains(&provider),
                "지원하지 않는 공급자"
            );
            c.execute(
                "INSERT OR REPLACE INTO preferences VALUES('provider',?1)",
                [provider],
            )?;
        }
        let provider: String = c
            .query_row(
                "SELECT value FROM preferences WHERE key='provider'",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "codex".into());
        if let Some(model) = r["model"].as_str() {
            validate_model(&provider, model)?;
            c.execute("INSERT OR REPLACE INTO preferences VALUES(?1,?2)", params![format!("model.{provider}"),model])?;
        }
        if let Some(effort) = r["effort"].as_str() {
            let model = selected_model(&c, &provider)?;
            c.execute("INSERT OR REPLACE INTO preferences VALUES(?1,?2)", params![format!("effort.{provider}.{model}"),effort])?;
        }
        let mut efforts = json!({});
        let mut query = c.prepare("SELECT substr(key,8),value FROM preferences WHERE key LIKE 'effort.%'")?;
        for row in query.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))? {
            let (key, value) = row?; efforts[key] = json!(value);
        }
        let mut models = json!({});
        for p in ["codex", "claude", "antigravity"] { models[p] = json!(selected_model(&c, p)?); }
        return Ok(json!({"mode":mode,"provider":provider,"models":models,"efforts":efforts}));
    }
    if op == "chat.search" {
        let (ws, _) = key(r)?;
        let term = r["query"].as_str().unwrap_or("").trim();
        let mut query = c.prepare("SELECT c.id,c.provider,c.updated,
            coalesce((SELECT nullif(user,'') FROM turns WHERE conversation=c.id ORDER BY rowid LIMIT 1),nullif(c.draft,''),'이미지 질문'),
            (SELECT substr(CASE WHEN instr(lower(user),lower(?2))>0 THEN user ELSE answer END,1,100)
             FROM turns WHERE conversation=c.id AND (instr(lower(user),lower(?2)) OR instr(lower(answer),lower(?2))) ORDER BY rowid LIMIT 1)
            FROM conversations c WHERE workspace=?1 AND (draft<>'' OR EXISTS(SELECT 1 FROM turns WHERE conversation=c.id))
            AND (instr(lower(draft),lower(?2)) OR EXISTS(SELECT 1 FROM turns WHERE conversation=c.id
            AND (instr(lower(user),lower(?2)) OR instr(lower(answer),lower(?2))))) ORDER BY updated DESC")?;
        let rows = query.query_map(params![ws,term], |r| Ok(json!({"id":r.get::<_,String>(0)?,
            "provider":r.get::<_,String>(1)?,"updated":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,
            "snippet":r.get::<_,Option<String>>(4)?})))?;
        return Ok(json!(rows.collect::<rusqlite::Result<Vec<_>>>()?));
    }
    let _switch_lock = if matches!(op,"chat.new"|"chat.open") { Some(ExecutionLock::chat_turn(root)?) } else { None };
    if op == "chat.open" {
        let (ws, provider) = key(r)?;
        let target = r["id"].as_str().context("대화를 선택하세요")?;
        let tx = c.unchecked_transaction()?;
        tx.execute("UPDATE conversations SET active=0 WHERE workspace=?1 AND provider=?2 AND active=1",params![ws,provider])?;
        anyhow::ensure!(tx.execute("UPDATE conversations SET active=1 WHERE id=?1 AND workspace=?2 AND provider=?3",
            params![target,ws,provider])? == 1,"이 워크스페이스에서 대화를 찾을 수 없습니다");
        tx.execute("DELETE FROM conversations WHERE workspace=?1 AND provider=?2 AND active=0 AND session IS NULL AND draft='' AND NOT EXISTS(SELECT 1 FROM turns WHERE conversation=conversations.id)",params![ws,provider])?;
        tx.execute("INSERT OR REPLACE INTO preferences VALUES('provider',?1)",[provider])?;
        tx.commit()?;
    }
    let id = conversation(&c, r)?;
    match op {
        "chat.save" => {
            let _lock = ExecutionLock::chat_turn(root)?;
            let answer = r["answer"].as_str().context("답변 없음")?;
            anyhow::ensure!(answer.len() <= 4 * 1024 * 1024, "답변 길이 제한 초과");
            let state = r["state"].as_str().context("종료 상태 없음")?;
            anyhow::ensure!(
                ["completed", "interrupted", "failed"].contains(&state),
                "잘못된 종료 상태"
            );
            let updated=c.execute("UPDATE turns SET answer=?1,state=?2,error='' WHERE id=?3 AND run=?4 AND conversation=?5",params![answer,state,r["turn_id"].as_str(),r["run_id"].as_str(),id])?;
            anyhow::ensure!(updated == 1, "저장할 대화를 찾을 수 없습니다");
        }
        "chat.draft" => {
            let draft = r["draft"].as_str().context("입력 내용 없음")?;
            anyhow::ensure!(draft.len() <= 128 * 1024, "질문은 128KB 이내로 입력하세요");
            c.execute(
                "UPDATE conversations SET draft=?1 WHERE id=?2",
                params![draft, id],
            )?;
        }
        "chat.new" => {
            recover(&c)?;
            // What was worth a note is carried over before the conversation is erased.
            let (ws, provider) = key(r)?;
            crate::distill::before_clear(root, &c, ws, provider)?;
            let tx = c.unchecked_transaction()?;
            if r["disposition"].as_str().unwrap_or("archive") == "archive" {
                tx.execute("UPDATE conversations SET active=0 WHERE id=?1",[&id])?;
            } else {
                anyhow::ensure!(r["disposition"] == "delete","잘못된 대화 처리 방식");
                tx.execute("DELETE FROM turns WHERE conversation=?1", [&id])?;
            tx.execute("DELETE FROM conversations WHERE id=?1", [&id])?;
            }
            conversation(&tx,r)?;
            tx.commit()?;
            return handle(
                root,
                &json!({"op":"chat.get","workspace_id":r["workspace_id"],"provider":r["provider"]}),
            );
        }
        "chat.get" | "chat.open" => {
            if let Ok(_lock) = ExecutionLock::chat_turn(root) {
                recover(&c)?;
            }
        }
        _ => bail!("지원하지 않는 채팅 요청"),
    }
    let mut result=c.query_row("SELECT session,draft,needs_new FROM conversations WHERE id=?1",[&id],|r|Ok(json!({"id":id,"session_id":r.get::<_,Option<String>>(0)?,"draft":r.get::<_,String>(1)?,"needs_new":r.get::<_,bool>(2)?})))?;
    result["workspace_id"] = r["workspace_id"].clone();
    result["provider"] = r["provider"].clone();
    let mut q = c.prepare(
        "SELECT id,run,user,answer,state,error,model,effort,images,mark,distill FROM turns WHERE conversation=?1 ORDER BY rowid",
    )?;
    let rows=q.query_map([&id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"run_id":r.get::<_,String>(1)?,"user":r.get::<_,String>(2)?,"answer":r.get::<_,String>(3)?,"state":r.get::<_,String>(4)?,"error":r.get::<_,String>(5)?,"model":r.get::<_,String>(6)?,"effort":r.get::<_,String>(7)?,"images":r.get::<_,i64>(8)?,"mark":r.get::<_,String>(9)?,"distill":r.get::<_,String>(10)?})))?;
    result["turns"] = json!(rows.collect::<std::result::Result<Vec<_>, _>>()?);
    Ok(result)
}

pub const MAX_IMAGES: usize = 10;
/// 첨부 이미지는 base64 그대로 공급자에게 넘긴다. 디스크에 남기지 않는다.
fn attached_images(r: &Value) -> Result<Vec<(String, String)>> {
    let mut images = Vec::new();
    for item in r["images"].as_array().into_iter().flatten() {
        let mime = item["mime"].as_str().context("이미지 형식 없음")?;
        anyhow::ensure!(["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&mime), "PNG·JPEG·GIF·WebP 이미지만 첨부할 수 있어요");
        let data = item["data"].as_str().context("이미지 내용 없음")?;
        anyhow::ensure!(!data.is_empty() && data.len() <= 7 * 1024 * 1024, "이미지는 한 장에 5MB 이하로 첨부하세요");
        anyhow::ensure!(data.len() % 4 == 0 && data.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)), "이미지 인코딩이 올바르지 않습니다");
        images.push((mime.to_owned(), data.to_owned()));
    }
    anyhow::ensure!(images.len() <= MAX_IMAGES, "이미지는 한 번에 {MAX_IMAGES}장까지 첨부할 수 있어요");
    Ok(images)
}
fn claude_content(prompt: &str, images: &[(String, String)]) -> Value {
    let mut content: Vec<Value> = images.iter()
        .map(|(mime, data)| json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}}))
        .collect();
    if !prompt.trim().is_empty() { content.push(json!({"type":"text","text":prompt})); }
    json!(content)
}
fn codex_input(prompt: &str, images: &[(String, String)]) -> Value {
    let mut input: Vec<Value> = images.iter()
        .map(|(mime, data)| json!({"type":"image","url":format!("data:{mime};base64,{data}")}))
        .collect();
    if !prompt.trim().is_empty() { input.push(json!({"type":"text","text":prompt})); }
    json!(input)
}
fn validate_model(provider: &str, model: &str) -> Result<()> {
    anyhow::ensure!(model.len() <= 128 && model.chars().all(|c| c.is_ascii_alphanumeric() || "-_.[]".contains(c)), "잘못된 모델 ID");
    anyhow::ensure!(provider == "antigravity" || provider == "codex" || (provider == "claude" && (["", "sonnet", "opus", "haiku", "fable"].contains(&model) || model.starts_with("claude-"))), "지원하지 않는 모델");
    Ok(())
}
fn validate_effort(effort: &str) -> Result<()> {
    anyhow::ensure!(["", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"].contains(&effort), "잘못된 effort 단계");
    Ok(())
}
fn selected_effort(c: &Connection, provider: &str, model: &str) -> Result<String> {
    use rusqlite::OptionalExtension;
    Ok(c.query_row("SELECT value FROM preferences WHERE key=?1", [format!("effort.{provider}.{model}")], |r| r.get(0)).optional()?.unwrap_or_default())
}
fn check_effort(item: &Value, effort: &str) -> Result<()> {
    validate_effort(effort)?;
    anyhow::ensure!(effort.is_empty() || item["efforts"].as_array().is_some_and(|a| a.iter().any(|v| v == effort)), "이 모델은 선택한 effort를 지원하지 않습니다. 단계를 다시 선택하세요.");
    Ok(())
}
fn claude_model_list(items: &[Value]) -> Result<Vec<Value>> {
    let mut list = Vec::new();
    for item in items {
        let alias = item["value"].as_str().context("Claude 모델 ID 없음")?;
        let resolved = item["resolvedModel"].as_str().unwrap_or(alias);
        let id = if alias == "default" { "" } else { resolved };
        validate_model("claude", id)?;
        let label = item["displayName"].as_str().unwrap_or(id);
        let efforts: Vec<_> = item["supportedEffortLevels"].as_array().into_iter().flatten()
            .filter_map(Value::as_str).filter(|e| !e.is_empty() && validate_effort(e).is_ok()).collect();
        let entry = json!({"id":id,"label":if alias=="default" {"기본 모델"} else {label},"resolved_model":resolved,"efforts":efforts});
        if !list.iter().any(|m: &Value| m["id"] == id) { list.push(entry.clone()); }
        // Keep the former family aliases explicitly available as automatic versions.
        if alias != "default" && alias != id && ["sonnet","opus","haiku","fable"].contains(&alias) {
            let mut automatic = entry;
            automatic["id"] = json!(alias);
            automatic["label"] = json!(format!("{label} · 자동 버전"));
            list.push(automatic);
        }
    }
    Ok(list)
}
fn selected_model(c: &Connection, provider: &str) -> Result<String> {
    use rusqlite::OptionalExtension;
    Ok(c.query_row("SELECT value FROM preferences WHERE key=?1", [format!("model.{provider}")], |r| r.get(0)).optional()?.unwrap_or_default())
}
/// Query metadata only; no thread or inference is created. The caller runs off the UI thread.
fn models(root: &Path, provider: &str) -> Result<Value> {
    anyhow::ensure!(["codex", "claude", "antigravity"].contains(&provider), "지원하지 않는 공급자");
    if provider == "antigravity" { return super::antigravity::models(root); }
    let work = root.join("model-discovery");
    fs::create_dir_all(&work)?;
    let mut cmd = command(root, provider, &work, None, "", "")?;
    if provider == "claude" { cmd.arg("--no-session-persistence"); }
    let mut child = Process(cmd.spawn()?);
    let mut input = child.0.stdin.take().unwrap();
    let output = child.0.stdout.take().unwrap();
    let err = child.0.stderr.take().unwrap();
    thread::spawn(move || { let _ = std::io::copy(&mut BufReader::new(err), &mut std::io::sink()); });
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || { for line in BufReader::new(output).lines() { if tx.send(line).is_err() { break; } } });
    if provider == "claude" {
        writeln!(input, "{}", json!({"type":"control_request","request_id":"models","request":{"subtype":"initialize"}}))?;
        input.flush()?;
    } else {
        write_request(&mut input, 1, "initialize", json!({"clientInfo":{"name":"stology_models","version":"0.1.0"}}))?;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut list = Vec::new();
    loop {
        let line = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).context("모델 목록을 가져오지 못했습니다. 목록을 다시 불러오세요.")??;
        let v: Value = serde_json::from_str(&line)?;
        anyhow::ensure!(v["error"].is_null(), "모델 목록 조회 실패: {}", v["error"]);
        if provider == "claude" {
            if v["type"] == "control_response" && v["response"]["request_id"] == "models" {
                anyhow::ensure!(v["response"]["subtype"] == "success", "Claude 모델 목록 조회 실패");
                list = claude_model_list(v["response"]["response"]["models"].as_array().context("Claude 모델 목록 응답 없음")?)?;
                break;
            }
            continue;
        }
        if v["id"] == 1 {
            writeln!(input, "{}", json!({"method":"initialized"}))?;
            write_request(&mut input, 2, "model/list", json!({"limit":100}))?;
        } else if v["id"] == 2 {
            for item in v["result"]["data"].as_array().context("모델 목록 응답 없음")? {
                if item["hidden"] == true { continue; }
                if let Some(id) = item["model"].as_str() {
                    validate_model(provider, id)?;
                    list.push(json!({"id":id,"label":item["displayName"].as_str().unwrap_or(id),"default":item["isDefault"]==true,"default_effort":item["defaultReasoningEffort"],"efforts":item["supportedReasoningEfforts"].as_array().into_iter().flatten().filter_map(|e|e["reasoningEffort"].as_str()).collect::<Vec<_>>()}));
                }
            }
            if let Some(cursor) = v["result"]["nextCursor"].as_str().filter(|s| !s.is_empty()) {
                write_request(&mut input, 2, "model/list", json!({"limit":100,"cursor":cursor}))?;
            } else { break; }
        }
    }
    anyhow::ensure!(!list.is_empty(), "선택 가능한 모델이 없습니다");
    Ok(json!({"models":list}))
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.wait();
        }
    }
}
fn write_request(w: &mut impl Write, id: i64, method: &str, params: Value) -> Result<()> {
    writeln!(w, "{}", json!({"id":id,"method":method,"params":params}))?;
    w.flush()?;
    Ok(())
}
const INSTRUCTIONS:&str="You are Stology's study assistant. Answer the user's text in their language. Only the conversation, including any images the user attached, is available. Do not claim access to files, documents or tools. Do not execute tools.";
fn command(root: &Path, p: &str, work: &Path, session: Option<&str>, model: &str, effort: &str) -> Result<Command> {
    let binary =
        super::providers::bin(root, p).context("공식 CLI가 없습니다. 연결에서 설치하세요.")?;
    let mut cmd = super::providers::clean_command(&binary);
    cmd.current_dir(work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if p == "codex" {
        cmd.args(["app-server", "--stdio"]);
        for flag in [
            "hooks",
            "plugins",
            "apps",
            "shell_tool",
            "unified_exec",
            "browser_use",
            "computer_use",
            "image_generation",
            "multi_agent",
            "multi_agent_v2",
            "memories",
            "skill_search",
            "code_mode",
            "js_repl",
            "view_image",
            "sleep_tool",
            "tool_suggest",
            "request_permissions_tool",
            "external_agent_memory_import",
            "agent_message_board",
            "agent_message_board_mcp",
        ] {
            cmd.args(["--disable", flag]);
        }
        for conf in [
            "web_search=\"disabled\"",
            "project_doc_max_bytes=0",
            "approval_policy=\"never\"",
            "forced_login_method=\"chatgpt\"",
            "model_provider=\"openai\"",
            "mcp_servers={}",
            "default_permissions=\"stology_chat\"",
            "permissions.stology_chat.filesystem={\":minimal\"=\"read\",\":workspace_roots\"=\"read\",\":tmpdir\"=\"deny\",\":slash_tmp\"=\"deny\"}",
            "permissions.stology_chat.network.enabled=false",
        ] {
            cmd.args(["-c", conf]);
        }
        cmd.args(["--enable", "skip_host_skill_discovery"]);
    } else {
        cmd.args([
            "--print",
            "--safe-mode",
            "--restricted",
            "--tools",
            "",
            "--disable-slash-commands",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--setting-sources",
            "",
            "--no-chrome",
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--system-prompt",
            INSTRUCTIONS,
        ]);
        cmd.args(["--model", if model.is_empty() { "default" } else { model }]);
        if !effort.is_empty() { cmd.args(["--effort", effort]); }
        if let Some(s) = session {
            cmd.args(["--resume", s]);
        }
    }
    Ok(cmd)
}

fn friendly_error(message: &str, diagnostic: &str) -> String {
    let all = format!("{message} {diagnostic}").to_lowercase();
    if ["rate limit", "usage limit", "quota", "hit your limit"]
        .iter()
        .any(|s| all.contains(s))
    {
        return "구독 사용 한도에 도달했습니다. 공급자에서 한도를 확인한 뒤 다시 시도하세요."
            .into();
    }
    if [
        "unauthorized",
        "authentication",
        "token expired",
        "not logged",
    ]
    .iter()
    .any(|s| all.contains(s))
    {
        return "인증이 만료되었거나 유효하지 않습니다. 연결에서 다시 로그인하세요.".into();
    }
    if [
        "unexpected argument",
        "unknown option",
        "unrecognized option",
    ]
    .iter()
    .any(|s| all.contains(s))
    {
        return "설치된 CLI 버전이 채팅 옵션을 지원하지 않습니다. 공식 CLI를 업데이트하세요."
            .into();
    }
    if [
        "connection refused",
        "connection reset",
        "network error",
        "dns error",
    ]
    .iter()
    .any(|s| all.contains(s))
    {
        return "네트워크 연결이 끊겼습니다. 연결을 확인한 뒤 직접 다시 시도하세요.".into();
    }
    message.into()
}

pub fn stream(root: &Path) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    let first = rx.recv().context("대화 시작 요청 없음")?;
    let request: Value = serde_json::from_str(&first)?;
    let mut out = std::io::stdout();
    if let Err(e) = run(root, &request, &rx, &mut out) {
        writeln!(
            out,
            "{}",
            json!({"event":"failed","error":format!("{e:#}"),"accepted":false})
        )?;
        out.flush()?;
    }
    Ok(())
}
fn run(
    root: &Path,
    r: &Value,
    control: &mpsc::Receiver<String>,
    out: &mut impl Write,
) -> Result<()> {
    let _turn = ExecutionLock::chat_turn(root)?;
    let _lock = take_turn(root, control, out)?;
    let (_, p) = key(r)?;
    let images = attached_images(r)?;
    let prompt = r["text"]
        .as_str()
        .filter(|s| !s.trim().is_empty() || !images.is_empty())
        .context("질문을 입력하세요")?;
    anyhow::ensure!(prompt.len() <= 128 * 1024, "질문은 128KB 이내로 입력하세요");
    let c = db(root)?;
    recover(&c)?;
    let mut model = r["model"].as_str().map(str::to_owned).unwrap_or(selected_model(&c, p)?);
    validate_model(p, &model)?;
    let effort = r["effort"].as_str().map(str::to_owned).unwrap_or(selected_effort(&c, p, &model)?);
    validate_effort(&effort)?;
    // Resolve versions and verify effort against current CLI capabilities before inference.
    let catalog = models(root, p)?;
    let items = catalog["models"].as_array().context("모델 목록 없음")?;
    let item = items.iter().find(|m| if p == "codex" && model.is_empty() { m["default"] == true } else { m["id"] == model })
        .context("선택한 모델이 현재 CLI 목록에 없습니다. 모델을 다시 선택하세요.")?;
    check_effort(item, &effort)?;
    model = item["resolved_model"].as_str().or_else(||item["id"].as_str()).context("모델 ID 없음")?.into();
    let id = conversation(&c, r)?;
    let (session, needs_new): (Option<String>, bool) = c.query_row(
        "SELECT session,needs_new FROM conversations WHERE id=?1",
        [&id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    anyhow::ensure!(
        !needs_new,
        "이전 CLI 세션을 이어갈 수 없습니다. 새 대화를 시작하세요."
    );
    let status = super::providers::status(root, p);
    anyhow::ensure!(
        status["installed"] == true,
        "공식 CLI가 없습니다. 연결에서 설치하세요."
    );
    anyhow::ensure!(
        status["logged_in"] == true,
        "공식 CLI에 구독 계정으로 로그인하세요. 연결에서 상태를 확인할 수 있어요."
    );
    let work = root.join("chat-work").join(&id);
    fs::create_dir_all(&work)?;
    if p == "antigravity" {
        return run_antigravity(root, r, control, out, &c, &id, session.as_deref(), &work, &model, &effort, prompt, &images);
    }
    let mut child = Process(
        command(root, p, &work, session.as_deref(), &model, &effort)?
            .spawn()
            .context("CLI 실행 실패")?,
    );
    let mut input = child.0.stdin.take().unwrap();
    let output = child.0.stdout.take().unwrap();
    let err = child.0.stderr.take().unwrap();
    let stderr = thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = err.take(64 * 1024).read_to_string(&mut s);
        s
    });
    let (tx, rx) = mpsc::sync_channel(256);
    thread::spawn(move || {
        let mut reader = BufReader::new(output);
        loop {
            let mut line = String::new();
            use std::io::Read;
            let n = reader.by_ref().take(1024 * 1024).read_line(&mut line);
            if !matches!(n,Ok(n) if n>0) {
                break;
            }
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let turn = uuid::Uuid::new_v4().to_string();
    let run = uuid::Uuid::new_v4().to_string();
    let tx = c.unchecked_transaction()?;
    tx.execute("INSERT INTO turns(id,conversation,run,user,state,created,model,effort,images) VALUES(?1,?2,?3,?4,'running',?5,?6,?7,?8)",params![turn,id,run,prompt,chrono::Utc::now().to_rfc3339(),model,effort,images.len() as i64])?;
    tx.execute("UPDATE conversations SET draft='' WHERE id=?1", [&id])?;
    tx.commit()?;
    let mut seq = 0u64;
    let mut emit = |event: &str, data: Value| -> Result<()> {
        seq += 1;
        let mut v = json!({"event":event,"conversation_id":id,"workspace_id":r["workspace_id"],"provider":p,"turn_id":turn,"run_id":run,"sequence":seq,"model":model,"effort":effort});
        for (k, val) in data.as_object().into_iter().flatten() {
            v[k] = val.clone();
        }
        writeln!(out, "{v}")?;
        out.flush()?;
        Ok(())
    };
    let mut answer = String::new();
    let mut thread_id = session.unwrap_or_default();
    let mut provider_turn = String::new();
    let mut cancelled = None;
    let mut last_save = Instant::now();
    let mut session_missing = false;
    let result = (|| -> Result<&str> {
        emit("accepted", json!({"text":prompt,"images":images.len()}))?;
        if p == "codex" {
            write_request(
                &mut input,
                1,
                "initialize",
                json!({"clientInfo":{"name":"stology_local_chat","version":"0.1.0"},"capabilities":{"experimentalApi":true}}),
            )?;
        } else {
            writeln!(
                input,
                "{}",
                json!({"type":"user","message":{"role":"user","content":claude_content(prompt, &images)}})
            )?;
            input.flush()?;
        }
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            if cancelled.is_none() {
                match control.try_recv() {
                    Ok(_) | Err(mpsc::TryRecvError::Disconnected) => {
                        cancelled = Some(Instant::now());
                        if p == "codex" && !provider_turn.is_empty() {
                            write_request(
                                &mut input,
                                9,
                                "turn/interrupt",
                                json!({"threadId":thread_id,"turnId":provider_turn}),
                            )?;
                        } else {
                            unsafe {
                                libc::kill(-(child.0.id() as i32), libc::SIGINT);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if cancelled.is_some_and(|t| t.elapsed() > Duration::from_secs(2)) {
                return Ok("interrupted");
            }
            if Instant::now() > deadline {
                bail!("응답 시간이 초과되었습니다. 자동 재전송하지 않았습니다.")
            }
            if last_save.elapsed() > Duration::from_millis(250) {
                c.execute(
                    "UPDATE turns SET answer=?1 WHERE id=?2",
                    params![answer, turn],
                )?;
                last_save = Instant::now();
            }
            let line = match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(v) => v,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => {
                    if cancelled.is_some() {
                        return Ok("interrupted");
                    }
                    bail!("CLI 연결이 종료되었습니다. 전송 결과를 확인할 수 없어요.")
                }
            };
            let v: Value =
                serde_json::from_str(&line).context("CLI 응답 형식이 호환되지 않습니다")?;
            if p == "codex" {
                if !v["error"].is_null() {
                    let message = v["error"].to_string().to_lowercase();
                    session_missing = v["id"] == 2
                        && !thread_id.is_empty()
                        && [
                            "not found",
                            "no rollout",
                            "does not exist",
                            "unable to find",
                        ]
                        .iter()
                        .any(|s| message.contains(s));
                    bail!("CLI 요청 실패: {}", v["error"])
                }
                if v["id"] == 1 {
                    write_request(&mut input, 4, "config/read", json!({"includeLayers":false}))?;
                }
                if v["id"] == 4 {
                    let mut config = json!({"model_provider":"openai","web_search":"disabled","project_doc_max_bytes":0});
                    if let Some(servers) = v["result"]["config"]["mcp_servers"].as_object() {
                        for name in servers.keys() {
                            anyhow::ensure!(name.chars().all(|c|c.is_ascii_alphanumeric() || c=='_' || c=='-'),"CLI 설정의 MCP 이름을 안전하게 제한할 수 없습니다. 지원되지 않는 설정입니다.");
                            config[format!("mcp_servers.{name}.enabled")] = json!(false);
                        }
                    }
                    let mut params = json!({"model":model,"cwd":work,"approvalPolicy":"never","permissions":"stology_chat","baseInstructions":INSTRUCTIONS,"developerInstructions":"Only text conversation. No tools or external context.","config":config});
                    let effective_effort = if effort.is_empty() { item["default_effort"].clone() } else { json!(effort) };
                    if !effective_effort.is_null() { params["config"]["model_reasoning_effort"] = effective_effort; }
                    let method = if thread_id.is_empty() {
                        params["ephemeral"] = json!(false);
                        params["environments"] = json!([]);
                        "thread/start"
                    } else {
                        params["threadId"] = json!(thread_id);
                        "thread/resume"
                    };
                    write_request(&mut input, 2, method, params)?;
                }
                if v["id"] == 2 {
                    thread_id = v["result"]["thread"]["id"]
                        .as_str()
                        .context("CLI 세션 ID 없음")?
                        .into();
                    c.execute(
                        "UPDATE conversations SET session=?1 WHERE id=?2",
                        params![thread_id, id],
                    )?;
                    if cancelled.is_some() {
                        return Ok("interrupted");
                    }
                    write_request(
                        &mut input,
                        3,
                        "turn/start",
                        json!({"effort":if effort.is_empty() {item["default_effort"].clone()}else{json!(effort)},"model":model,"threadId":thread_id,"input":codex_input(prompt, &images),"environments":[]}),
                    )?;
                }
                if v["id"] == 3 {
                    provider_turn = v["result"]["turn"]["id"].as_str().unwrap_or("").into();
                }
                if v["method"] == "turn/started" {
                    provider_turn = v["params"]["turn"]["id"].as_str().unwrap_or("").into();
                }
                if v["method"] == "item/agentMessage/delta" {
                    if let Some(delta) = v["params"]["delta"].as_str() {
                        answer.push_str(delta);
                        emit("text_delta", json!({"text":delta}))?;
                    }
                }
                if v["method"] == "turn/completed" {
                    return match v["params"]["turn"]["status"].as_str() {
                        Some("completed") => Ok(if cancelled.is_some() {
                            "interrupted"
                        } else {
                            "completed"
                        }),
                        Some("interrupted") => Ok("interrupted"),
                        _ => bail!("응답 실패: {}", v["params"]["turn"]["error"]),
                    };
                }
                // Never approve server-initiated tools, permissions, or user prompts.
                if v["method"].is_string() && !v["id"].is_null() {
                    writeln!(
                        input,
                        "{}",
                        json!({"id":v["id"],"error":{"code":-32601,"message":"Tools disabled in Stology chat"}})
                    )?;
                    input.flush()?;
                }
            } else {
                if v["type"] == "system" && v["subtype"] == "init" {
                    anyhow::ensure!(
                        v["tools"].as_array().is_some_and(|a| a.is_empty()),
                        "CLI 도구 제한을 확인하지 못했습니다"
                    );
                    thread_id = v["session_id"].as_str().context("CLI 세션 ID 없음")?.into();
                    c.execute(
                        "UPDATE conversations SET session=?1 WHERE id=?2",
                        params![thread_id, id],
                    )?;
                }
                if v["type"] == "stream_event" && v["event"]["delta"]["type"] == "text_delta" {
                    if let Some(delta) = v["event"]["delta"]["text"].as_str() {
                        answer.push_str(delta);
                        emit("text_delta", json!({"text":delta}))?;
                    }
                }
                if v["type"] == "result" {
                    if v["is_error"] == true {
                        let message = format!("{} {}", v["result"], v["errors"]);
                        session_missing = message.to_lowercase().contains("no conversation found");
                        bail!("응답 실패: {message}")
                    }
                    return Ok(if cancelled.is_some() {
                        "interrupted"
                    } else {
                        "completed"
                    });
                }
            }
            anyhow::ensure!(
                answer.len() <= 4 * 1024 * 1024,
                "답변 길이 제한을 초과했습니다"
            );
        }
    })();
    drop(child);
    let diagnostic = stderr.join().unwrap_or_default();
    let (state, error) = match result {
        Ok(s) => (s, String::new()),
        Err(e) => {
            let message = if cancelled.is_some() {
                String::new()
            } else {
                friendly_error(&format!("{e:#}"), &diagnostic)
            };
            (
                if cancelled.is_some() {
                    "interrupted"
                } else {
                    "failed"
                },
                message,
            )
        }
    };
    if session_missing {
        c.execute("UPDATE conversations SET needs_new=1 WHERE id=?1", [&id])?;
    }
    if let Err(e) = c.execute(
        "UPDATE turns SET answer=?1,state=?2,error=?3 WHERE id=?4",
        params![answer, state, error, turn],
    ) {
        emit(
            "failed",
            json!({"error":format!("저장 실패 · 복사 후 다시 저장하세요: {e}"),"answer":answer,"save_failed":true,"state":state}),
        )?;
        return Ok(());
    }
    c.execute(
        "UPDATE conversations SET updated=?1 WHERE id=?2",
        params![chrono::Utc::now().to_rfc3339(), id],
    )?;
    emit(
        state,
        json!({"error":error,"answer":answer,"session_id":thread_id}),
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_antigravity(root: &Path, r: &Value, control: &mpsc::Receiver<String>, out: &mut impl Write,
    c: &Connection, conversation: &str, existing: Option<&str>, work: &Path,
    model: &str, effort: &str, prompt: &str, images: &[(String,String)]) -> Result<()> {
    let mut client = super::antigravity::Client::spawn(root, work)?;
    client.initialize()?;
    let data = match client.session(work, existing) {
        Ok(v) => v,
        Err(e) => {
            let message=e.to_string().to_lowercase();
            if existing.is_some() && ["not found","does not exist","unknown session","invalid session"].iter().any(|s|message.contains(s)) {
                c.execute("UPDATE conversations SET needs_new=1 WHERE id=?1", [conversation])?;
            }
            return Err(e.context("Antigravity 대화를 열지 못했습니다. 기존 기록은 보존됩니다."));
        }
    };
    let session = data["sessionId"].as_str().or(existing).context("Antigravity 세션 ID 없음")?;
    client.configure(session, &data, model, effort)?;
    let turn = uuid::Uuid::new_v4().to_string();
    let run = uuid::Uuid::new_v4().to_string();
    let tx = c.unchecked_transaction()?;
    tx.execute("INSERT INTO turns(id,conversation,run,user,state,created,model,effort,images) VALUES(?1,?2,?3,?4,'running',?5,?6,?7,?8)",params![turn,conversation,run,prompt,chrono::Utc::now().to_rfc3339(),model,effort,images.len() as i64])?;
    tx.execute("UPDATE conversations SET draft='',session=?1 WHERE id=?2", params![session,conversation])?;
    tx.commit()?;
    let mut sequence = 0;
    let mut emit = |event:&str, fields:Value| -> Result<()> {
        sequence += 1;
        let mut value = json!({"event":event,"conversation_id":conversation,"workspace_id":r["workspace_id"],"provider":"antigravity","turn_id":turn,"run_id":run,"sequence":sequence,"model":model,"effort":effort});
        for (key,val) in fields.as_object().into_iter().flatten() {value[key]=val.clone();}
        writeln!(out,"{value}")?; out.flush()?; Ok(())
    };
    let mut answer=String::new();
    let mut cancelled=None;
    let result=(|| -> Result<&str> {
        emit("accepted",json!({"text":prompt,"images":images.len()}))?;
        let request=client.prompt(session,prompt,images)?;
        let deadline=Instant::now()+Duration::from_secs(600);
        let mut last_save=Instant::now();
        loop {
            if cancelled.is_none() && !matches!(control.try_recv(),Err(mpsc::TryRecvError::Empty)) {
                cancelled=Some(Instant::now()); let _=client.cancel(session);
            }
            if cancelled.is_some_and(|t|t.elapsed()>Duration::from_secs(2)) {return Ok("interrupted");}
            anyhow::ensure!(Instant::now()<deadline,"Antigravity 응답 시간 초과");
            if let Some(v)=client.next(Duration::from_millis(50))? {
                if let Some(delta)=super::antigravity::text_delta(&v,session) {
                    answer.push_str(delta);
                    anyhow::ensure!(answer.len()<=4*1024*1024,"답변 길이 제한 초과");
                    emit("text_delta",json!({"text":delta}))?;
                }
                if let Some(state)=super::antigravity::stop(&v,request)? {return Ok(if cancelled.is_some(){"interrupted"}else{state});}
            }
            if last_save.elapsed()>Duration::from_millis(250) {
                c.execute("UPDATE turns SET answer=?1 WHERE id=?2",params![answer,turn])?; last_save=Instant::now();
            }
        }
    })();
    drop(client);
    let (state,error)=match result {
        Ok(state)=>(state,String::new()),
        Err(_) if cancelled.is_some()=>("interrupted",String::new()),
        Err(e)=>("failed",friendly_error(&format!("{e:#}"),"")),
    };
    if let Err(e)=c.execute("UPDATE turns SET answer=?1,state=?2,error=?3 WHERE id=?4",params![answer,state,error,turn]) {
        emit("failed",json!({"error":format!("저장 실패 · 복사 후 다시 저장하세요: {e}"),"answer":answer,"save_failed":true,"state":state}))?;
        return Ok(());
    }
    c.execute("UPDATE conversations SET updated=?1 WHERE id=?2",params![chrono::Utc::now().to_rfc3339(),conversation])?;
    emit(state,json!({"error":error,"answer":answer,"session_id":session}))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn req(ws: &str, p: &str, op: &str) -> Value {
        json!({"workspace_id":ws,"provider":p,"op":op})
    }
    #[test]
    fn claude_model_is_passed_on_new_and_resumed_sessions() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("runtime/bin");
        fs::create_dir_all(&bin).unwrap();
        let path = bin.join("claude");
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        for session in [None, Some("existing-session")] {
            let c = command(d.path(), "claude", d.path(), session, "claude-opus-4-6", "high").unwrap();
            let args: Vec<_> = c.get_args().map(|s| s.to_str().unwrap()).collect();
            assert!(args.windows(2).any(|a| a == ["--model", "claude-opus-4-6"]));
            assert!(args.windows(2).any(|a| a == ["--effort", "high"]));
            assert_eq!(args.contains(&"--resume"), session.is_some());
        }
        let c = command(d.path(), "claude", d.path(), Some("existing-session"), "", "").unwrap();
        let args: Vec<_> = c.get_args().map(|s| s.to_str().unwrap()).collect();
        assert!(args.windows(2).any(|a| a == ["--model", "default"]));
        assert!(!args.contains(&"--effort"));
    }
    #[test]
    fn attached_images_are_validated_and_sent_before_text() {
        let r = json!({"images":[{"mime":"image/png","data":"iVBORw0KGgo="}]});
        let images = attached_images(&r).unwrap();
        let claude = claude_content("이게 뭐야?", &images);
        assert_eq!(claude[0]["source"]["media_type"], "image/png");
        assert_eq!(claude[1]["text"], "이게 뭐야?");
        let codex = codex_input("", &images);
        assert_eq!(codex.as_array().unwrap().len(), 1);
        assert_eq!(codex[0]["url"], "data:image/png;base64,iVBORw0KGgo=");
        assert!(attached_images(&json!({"images":[{"mime":"image/svg+xml","data":"AAAA"}]})).is_err());
        assert!(attached_images(&json!({"images":[{"mime":"image/png","data":"not base64!"}]})).is_err());
        let many: Vec<_> = (0..=MAX_IMAGES).map(|_| json!({"mime":"image/png","data":"AAAA"})).collect();
        assert!(attached_images(&json!({"images":many})).is_err());
    }
    #[test]
    fn provider_errors_offer_specific_next_steps() {
        assert!(friendly_error("failed", "rate limit exceeded").contains("사용 한도"));
        assert!(friendly_error("failed", "unknown option --safe-mode").contains("업데이트"));
        assert!(friendly_error("token expired", "").contains("로그인"));
        assert!(friendly_error("connection reset", "").contains("네트워크"));
    }
    #[test]
    fn failed_save_can_be_retried_without_provider_call() {
        let d = tempfile::tempdir().unwrap();
        let ws = uuid::Uuid::new_v4().to_string();
        let r = req(&ws, "codex", "chat.get");
        let v = handle(d.path(), &r).unwrap();
        let c = db(d.path()).unwrap();
        c.execute(
            "INSERT INTO turns(id,conversation,run,user,answer,state,error,created) VALUES('t',?1,'r','q','','interrupted','','now')",
            [v["id"].as_str().unwrap()],
        )
        .unwrap();
        let saved=handle(d.path(),&json!({"op":"chat.save","workspace_id":ws,"provider":"codex","turn_id":"t","run_id":"r","answer":"recovered","state":"completed"})).unwrap();
        assert_eq!(saved["turns"][0]["answer"], "recovered");
        assert_eq!(saved["turns"][0]["state"], "completed");
        assert!(handle(d.path(),&json!({"op":"chat.save","workspace_id":ws,"provider":"codex","turn_id":"t","run_id":"wrong","answer":"bad","state":"completed"})).is_err());
    }
    #[test]
    fn conversations_and_drafts_are_isolated_and_reset() {
        let d = tempfile::tempdir().unwrap();
        let ws = uuid::Uuid::new_v4().to_string();
        let mut r = req(&ws, "codex", "chat.draft");
        r["draft"] = json!("초안");
        let a = handle(d.path(), &r).unwrap();
        let b = handle(d.path(), &req(&ws, "claude", "chat.get")).unwrap();
        assert_ne!(a["id"], b["id"]);
        assert_eq!(b["draft"], "");
        assert_eq!(
            handle(d.path(), &req(&ws, "codex", "chat.get")).unwrap()["draft"],
            "초안"
        );
        let n = handle(d.path(), &req(&ws, "codex", "chat.new")).unwrap();
        assert_ne!(a["id"], n["id"]);
        assert_eq!(n["draft"], "");
    }
    #[test]
    fn legacy_session_search_reopen_and_delete_stay_in_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let legacy = Connection::open(root.join("chat.sqlite3")).unwrap();
        legacy.execute_batch("CREATE TABLE conversations(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,provider TEXT NOT NULL,session TEXT,draft TEXT NOT NULL DEFAULT '',needs_new INTEGER NOT NULL DEFAULT 0,created TEXT NOT NULL,updated TEXT NOT NULL,UNIQUE(workspace,provider));").unwrap();
        legacy.execute("INSERT INTO conversations VALUES('old',?1,'codex','cli-session','초안',0,'now','now')",[&ws]).unwrap();
        drop(legacy);
        let c = db(root).unwrap();
        c.execute("INSERT INTO turns(id,conversation,run,user,answer,state,created) VALUES('turn','old','run','질문','검색할 본문','completed','now')",[]).unwrap();
        handle(root,&req(&ws,"codex","chat.new")).unwrap();
        let search = |ws:&str| handle(root,&json!({"op":"chat.search","workspace_id":ws,"provider":"claude","query":"본문"})).unwrap();
        assert_eq!(search(&ws)[0]["id"],"old");
        assert_eq!(search(&other),json!([]));
        let open = |ws:&str| handle(root,&json!({"op":"chat.open","workspace_id":ws,"provider":"codex","id":"old"}));
        assert!(open(&other).is_err());
        let restored=open(&ws).unwrap();
        assert_eq!(restored["draft"],"초안");
        assert_eq!(restored["session_id"],"cli-session");
        assert_eq!(restored["turns"][0]["answer"],"검색할 본문");
        handle(root,&json!({"op":"chat.new","workspace_id":ws,"provider":"codex","disposition":"delete"})).unwrap();
        assert_eq!(search(&ws),json!([]));
        assert!(open(&ws).is_err());
    }
    #[test]
    fn a_running_turn_excludes_reset_but_a_run_step_does_not() {
        let d = tempfile::tempdir().unwrap();
        let ws = uuid::Uuid::new_v4().to_string();
        let turn = ExecutionLock::chat_turn(d.path()).unwrap();
        assert!(ExecutionLock::chat_turn(d.path()).is_err());
        assert!(handle(d.path(), &req(&ws, "codex", "chat.new")).is_err());
        drop(turn);
        // A run's step holds the execution lock; saved turns stay editable meanwhile.
        let step = ExecutionLock::acquire(d.path()).unwrap();
        assert!(ExecutionLock::acquire(d.path()).is_err());
        assert!(handle(d.path(), &req(&ws, "codex", "chat.new")).is_ok());
        drop(step);
        assert!(ExecutionLock::acquire(d.path()).is_ok());
    }
    #[test]
    fn unfinished_turn_recovers_without_new_request() {
        let d = tempfile::tempdir().unwrap();
        let r = req(&uuid::Uuid::new_v4().to_string(), "codex", "chat.get");
        let v = handle(d.path(), &r).unwrap();
        let c = db(d.path()).unwrap();
        c.execute(
            "INSERT INTO turns(id,conversation,run,user,answer,state,error,created) VALUES('t',?1,'r','q','partial','running','','now')",
            [v["id"].as_str().unwrap()],
        )
        .unwrap();
        let v = handle(d.path(), &r).unwrap();
        assert_eq!(v["turns"][0]["state"], "interrupted");
        assert_eq!(v["turns"][0]["answer"], "partial");
    }
}
