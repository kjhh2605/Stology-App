//! Official subscription CLI adapters. Only job-private files and provider preferences are written here.
use super::skills::write_json;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{io::AsRawFd, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const MAX_OUTPUT: u64 = 16 * 1024 * 1024;
fn string<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("필수 항목: {k}"))
}
fn provider(v: &Value) -> Result<&str> {
    let p = string(v, "provider")?;
    if !["codex", "claude", "antigravity"].contains(&p) {
        bail!("지원하지 않는 공급자")
    }
    Ok(p)
}
fn json_read(p: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(p)?)?)
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}
pub(crate) fn bin(root: &Path, p: &str) -> Option<PathBuf> {
    let p = if p == "antigravity" { "agy" } else { p };
    let mut paths = vec![
        root.join("runtime/bin").join(p),
        home().join(".local/bin").join(p),
        PathBuf::from("/opt/homebrew/bin").join(p),
        PathBuf::from("/usr/local/bin").join(p),
    ];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path).map(|d| d.join(p)));
    }
    if let Ok(entries) = fs::read_dir(home().join(".nvm/versions/node")) {
        paths.extend(entries.flatten().map(|d| d.path().join("bin").join(p)));
    }
    paths.into_iter().find(|p| p.is_file())
}
pub(crate) fn clean_command(binary: &Path) -> Command {
    let mut c = Command::new(binary);
    // Preserve only the system environment required for official OAuth/keychain and native decoding.
    // In particular, never inherit API credentials, provider overrides, plugin injection, hooks or MCP settings.
    c.env_clear();
    for key in [
        "HOME", "USER", "LOGNAME", "TMPDIR", "LANG", "LC_ALL", "SHELL",
    ] {
        if let Some(value) = std::env::var_os(key) {
            c.env(key, value);
        }
    }
    c.env("CLAUDE_CODE_DISABLE_BUNDLED_SKILLS", "1");
    let parent = binary.parent().unwrap_or(Path::new("/usr/bin"));
    c.env(
        "PATH",
        format!(
            "{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
            parent.display()
        ),
    );
    c
}
fn output_timeout(mut command: Command, timeout: Duration) -> Result<std::process::Output> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn()?;
    let pid = child.id();
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let a = thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out.take(MAX_OUTPUT).read_to_end(&mut v);
        v
    });
    let b = thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.take(MAX_OUTPUT).read_to_end(&mut v);
        v
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > timeout {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            bail!("공식 도구 응답 시간 초과");
        }
        thread::sleep(Duration::from_millis(50));
    };
    Ok(std::process::Output {
        status,
        stdout: a.join().unwrap_or_default(),
        stderr: b.join().unwrap_or_default(),
    })
}
fn version_of(path: &Path) -> Option<String> {
    let mut c = clean_command(path);
    c.arg("--version");
    output_timeout(c, Duration::from_secs(5))
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}
/// The installed CLI's version number alone, for failure messages.
pub(crate) fn version(root: &Path, p: &str) -> Option<String> {
    let line = version_of(&bin(root, p)?)?;
    line.split_whitespace().find(|w| w.starts_with(|c: char| c.is_ascii_digit())).map(str::to_owned)
}
pub(crate) fn status(root: &Path, p: &str) -> Value {
    let Some(path) = bin(root, p) else {
        return json!({"provider":p,"installed":false,"logged_in":false,"analysis_succeeded":false});
    };
    if p == "antigravity" {
        let probe = super::antigravity::probe(root);
        let verified = json_read(&root.join("provider-antigravity-success.json")).ok();
        return json!({"provider":p,"installed":true,"path":path,"logged_in":probe.is_ok(),"auth_kind":"Google 계정 · Stology 전용 로그인","analysis_succeeded":verified.is_some(),"last_analysis":verified,"error":probe.err().map(|e|e.to_string())});
    }
    let mut c = clean_command(&path);
    if p == "codex" {
        c.args(["login", "status"]);
    } else {
        c.args(["auth", "status"]);
    }
    let login = output_timeout(c, Duration::from_secs(12));
    let (logged, kind, error) = match login {
        Ok(o) => {
            let raw = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            if p == "codex" {
                (
                    o.status.success() && raw.contains("ChatGPT"),
                    "ChatGPT".to_string(),
                    None,
                )
            } else {
                let v: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
                (
                    v["loggedIn"] == true && v["authMethod"] == "claude.ai",
                    "Claude 구독".to_string(),
                    None,
                )
            }
        }
        Err(e) => (false, "unknown".into(), Some(e.to_string())),
    };
    let version = version_of(&path);
    let verified = json_read(&root.join(format!("provider-{p}-success.json"))).ok();
    json!({"provider":p,"installed":true,"path":path,"version":version,"logged_in":logged,"auth_kind":kind,"analysis_succeeded":verified.is_some(),"last_analysis":verified,"error":error})
}
fn job_dir(root: &Path, id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id).context("잘못된 작업 ID")?;
    Ok(root.join("jobs").join(id))
}
fn process_identity(pid: i32) -> Option<Value> {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        let got = libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if got != size {
            return None;
        }
        return Some(
            json!({"pid":info.pbi_pid,"pgid":info.pbi_pgid,"start_sec":info.pbi_start_tvsec,"start_usec":info.pbi_start_tvusec}),
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}
fn identity_matches(v: &Value) -> bool {
    v["pid"]
        .as_i64()
        .and_then(|p| process_identity(p as i32))
        .as_ref()
        == Some(v)
}
fn signal_identity(v: &Value, signal: i32) {
    if identity_matches(v) && v["pid"] == v["pgid"] {
        if let Some(pid) = v["pid"].as_i64() {
            unsafe {
                libc::kill(-(pid as i32), signal);
            }
        }
    }
}
fn job_state(root: &Path, id: &str) -> Result<Value> {
    let d = job_dir(root, id)?;
    let mut v = json_read(&d.join("state.json"))?;
    if matches!(
        v["state"].as_str(),
        Some("running" | "queued" | "waiting_login")
    ) {
        if v["pid"].is_number() && !identity_matches(&v["worker_identity"]) {
            if let Ok(identity) = json_read(&d.join("child.identity.json")) {
                signal_identity(&identity, libc::SIGKILL);
            }
            v["state"] = json!("failed");
            v["error"]=json!("작업 프로세스가 종료되었거나 실행 식별자가 바뀌었습니다. 원본과 기존 지도는 보존됩니다.");
            write_json(&d.join("state.json"), &v)?;
        }
    }
    Ok(v)
}
struct UnlockOnDrop(i32);
impl Drop for UnlockOnDrop {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0, libc::LOCK_UN);
        }
    }
}
fn release_lock(_root: &Path, _id: &str) {} // Persistent lock inode; terminal job state permits next owner.
/// Worker jobs are exclusive. Login and install also refuse to start during a
/// chat turn; a run waits for the execution lock itself, one step at a time.
pub(crate) fn start(root: &Path, req: &Value, kind: &str) -> Result<Value> {
    let _ai_lock = if kind == "run" { None } else { Some(crate::chat::ExecutionLock::acquire(root)?) };
    fs::create_dir_all(root)?;
    let id = uuid::Uuid::new_v4().to_string();
    let lock = root.join("analysis.lock");
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("다른 작업을 시작 중입니다. 잠시 후 다시 시도하세요.")
    }
    let _unlock = UnlockOnDrop(f.as_raw_fd());
    let mut old = String::new();
    f.read_to_string(&mut old)?;
    if !old.is_empty() {
        let v = job_state(root, &old)
            .context("이전 작업 상태를 확인할 수 없어 새 분석을 시작하지 않았습니다")?;
        if matches!(
            v["state"].as_str(),
            Some("queued" | "running" | "waiting_login")
        ) {
            bail!("다른 작업이 진행 중입니다. 완료하거나 취소하세요.")
        }
    }
    let d = job_dir(root, &id)?;
    fs::create_dir_all(d.join("work"))?;
    let mut request = req.clone();
    request["kind"] = json!(kind);
    write_json(&d.join("request.json"), &request)?;
    let mut state = json!({"id":id,"provider":req["provider"],"kind":kind,"state":"queued","created_at":now(),"workspace_id":req["workspace_id"],"run_id":req["run_id"],"lane":req["lane"],"events":[]});
    write_json(&d.join("state.json"), &state)?;
    f.set_len(0)?;
    std::io::Seek::rewind(&mut f)?;
    f.write_all(id.as_bytes())?;
    f.sync_all()?;
    let log = fs::File::create(d.join("worker.log"))?;
    let child = Command::new(std::env::current_exe()?)
        .args(["--root", &root.to_string_lossy(), "--worker", &id])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .process_group(0)
        .spawn();
    match child {
        Ok(c) => {
            state["pid"] = json!(c.id());
            state["worker_identity"] = process_identity(c.id() as i32)
                .context("작업 프로세스 식별 정보를 확인하지 못했습니다")?;
            write_json(&d.join("state.json"), &state)?;
            Ok(state)
        }
        Err(e) => {
            state["state"] = json!("failed");
            state["error"] = json!(e.to_string());
            write_json(&d.join("state.json"), &state)?;
            Err(e.into())
        }
    }
}
pub fn handle(root: &Path, req: &Value) -> Result<Value> {
    match string(req, "op")? {
        "provider.status" => {
            let selected = json_read(&root.join("provider.json"))
                .ok()
                .and_then(|v| v["provider"].as_str().map(str::to_owned));
            Ok(
                json!({"selected":selected,"providers":[status(root,"codex"),status(root,"claude"),status(root,"antigravity")],"disclosure":"자료의 텍스트와 이미지가 선택한 구독 공급자에게 분석용으로 전송됩니다. API 과금으로 전환하지 않습니다."}),
            )
        }
        "provider.set" => {
            let p = provider(req)?;
            write_json(&root.join("provider.json"), &json!({"provider":p}))?;
            Ok(json!({"provider":p}))
        }
        "provider.login" => {
            provider(req)?;
            start(root, req, "login")
        }
        "provider.install" => {
            provider(req)?;
            start(root, req, "install")
        }
        "analysis.active" | "analysis.cancel_active" => {
            let lock = root.join("analysis.lock");
            if !lock.exists() {
                return Ok(json!({"active":null}));
            }
            let mut f = OpenOptions::new().read(true).write(true).open(lock)?;
            if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
                bail!("작업 잠금 확인 실패")
            };
            let _unlock = UnlockOnDrop(f.as_raw_fd());
            let mut id = String::new();
            f.read_to_string(&mut id)?;
            if id.is_empty() {
                return Ok(json!({"active":null}));
            }
            let s = job_state(root, &id)?;
            if !matches!(
                s["state"].as_str(),
                Some("queued" | "running" | "waiting_login")
            ) {
                return Ok(json!({"active":null}));
            }
            if req["op"] == "analysis.cancel_active" {
                return handle(root, &json!({"op":"analysis.cancel","id":id}));
            }
            Ok(json!({"active":s}))
        }
        "analysis.status" | "provider.job.status" => job_state(root, string(req, "id")?),
        "analysis.cancel" | "provider.job.cancel" => {
            let id = string(req, "id")?;
            let d = job_dir(root, id)?;
            let mut s = job_state(root, id)?;
            if matches!(
                s["state"].as_str(),
                Some("running" | "queued" | "waiting_login")
            ) {
                fs::write(d.join("cancelled"), b"cancelled")?;
                thread::sleep(Duration::from_millis(100));
                if let Ok(identity) = json_read(&d.join("child.identity.json")) {
                    signal_identity(&identity, libc::SIGKILL);
                }
                signal_identity(&s["worker_identity"], libc::SIGTERM);
                thread::sleep(Duration::from_millis(150));
                signal_identity(&s["worker_identity"], libc::SIGKILL);
                s["state"] = json!("cancelled");
                s["finished_at"] = json!(now());
                write_json(&d.join("state.json"), &s)?;
            }
            release_lock(root, id);
            Ok(s)
        }
        _ => bail!("지원하지 않는 공급자 명령"),
    }
}
fn event(d: &Path, kind: &str, text: &str) -> Result<()> {
    let mut s = json_read(&d.join("state.json"))?;
    if let Some(a) = s["events"].as_array_mut() {
        a.push(json!({"at":now(),"kind":kind,"message":text}));
        if a.len() > 100 {
            a.remove(0);
        }
    }
    write_json(&d.join("state.json"), &s)
}
pub fn run_worker(root: &Path, id: &str) -> Result<()> {
    let d = job_dir(root, id)?;
    let r = json_read(&d.join("request.json"))?;
    // Parent publishes PID before handing state ownership to this worker.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let ready = json_read(&d.join("state.json"))?;
        if ready["pid"].as_u64() == Some(std::process::id() as u64) {
            break;
        }
        if Instant::now() > deadline {
            bail!("작업 시작 동기화 실패")
        };
        thread::sleep(Duration::from_millis(10));
    }
    let mut s = json_read(&d.join("state.json"))?;
    s["pid"] = json!(std::process::id());
    s["state"] = json!("running");
    write_json(&d.join("state.json"), &s)?;
    if d.join("cancelled").exists() {
        bail!("작업이 취소되었습니다")
    };
    let result = match r["kind"].as_str() {
        Some("login") => login(root, &d, provider(&r)?),
        Some("install") => install(root, &d, provider(&r)?),
        Some("run") => crate::organize::work(root, &d, &r),
        Some("distill") => crate::distill::work(root, &d, &r),
        _ => Err(anyhow!("알 수 없는 작업 종류")),
    };
    let mut s = json_read(&d.join("state.json"))?;
    s["finished_at"] = json!(now());
    if let Some(o) = s.as_object_mut() {
        o.remove("auth_url");
    }
    match result {
        Ok(v) => {
            s["state"] = json!("succeeded");
            s["result"] = v;
        }
        Err(e) => {
            s["state"] = json!("failed");
            s["error"] = json!(format!("{e:#}"));
        }
    }
    write_json(&d.join("state.json"), &s)?;
    release_lock(root, id);
    Ok(())
}
/// Reaps the entire official CLI group on every return path, including I/O errors.
struct ChildGroup(std::process::Child);
impl std::ops::Deref for ChildGroup {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ChildGroup {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl ChildGroup {
    fn kill_group(&self) {
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
    }
}
impl Drop for ChildGroup {
    fn drop(&mut self) {
        self.kill_group();
        let _ = self.0.wait();
    }
}
fn register_child(d: &Path, child: &std::process::Child) -> Result<()> {
    fs::write(d.join("child.pid"), child.id().to_string())?;
    let identity =
        process_identity(child.id() as i32).context("공식 도구 프로세스 식별 정보 없음")?;
    write_json(&d.join("child.identity.json"), &identity)?;
    let mut s = json_read(&d.join("state.json"))?;
    s["child_pid"] = json!(child.id());
    write_json(&d.join("state.json"), &s)?;
    if d.join("cancelled").exists() {
        bail!("작업이 취소되었습니다")
    };
    Ok(())
}
/// `job` owns the cancel flag and child identity; `out` receives this call's output files.
fn run_capture(
    mut c: Command,
    job: &Path,
    d: &Path,
    input: &[u8],
    timeout: Duration,
) -> Result<(String, String)> {
    fs::create_dir_all(d)?;
    let stdout = fs::File::create(d.join("stdout.jsonl"))?;
    let stderr = fs::File::create(d.join("stderr.txt"))?;
    c.stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .process_group(0);
    let mut child = ChildGroup(c.spawn()?);
    register_child(job, &child)?;

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(input) {
            child.kill_group();
            let _ = child.wait();
            return Err(e.into());
        }
    }
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > timeout {
            child.kill_group();
            let _ = child.wait();
            bail!("분석 시간 제한을 초과했습니다. 원본은 보존됩니다.")
        };
        if fs::metadata(d.join("stdout.jsonl"))
            .map(|m| m.len() > MAX_OUTPUT)
            .unwrap_or(false)
            || fs::metadata(d.join("stderr.txt"))
                .map(|m| m.len() > MAX_OUTPUT)
                .unwrap_or(false)
        {
            child.kill_group();
            let _ = child.wait();
            bail!("공식 도구 출력 크기 제한을 초과했습니다")
        };
        thread::sleep(Duration::from_millis(100));
    };
    child.kill_group();
    let out = fs::read_to_string(d.join("stdout.jsonl")).unwrap_or_default();
    let err = fs::read_to_string(d.join("stderr.txt")).unwrap_or_default();
    if !status.success() {
        let mut detail = truncate(&err, 1200);
        for line in out.lines().rev() {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                if v["type"] == "error" || v["type"] == "turn.failed" || v["is_error"] == true {
                    detail = v["message"]
                        .as_str()
                        .or(v["error"]["message"].as_str())
                        .or(v["result"].as_str())
                        .map(|s| truncate(s, 1200))
                        .unwrap_or(detail);
                    break;
                }
            }
        }
        bail!("{} ({}): {}", classify_failure(&detail), status, detail)
    }
    Ok((out, err))
}
/// How every Codex app-server run starts, before the caller's own settings.
pub(crate) const CODEX_APP_SERVER: [&str; 10] = [
    "app-server",
    "--stdio",
    "-c",
    "features.hooks=false",
    "-c",
    "features.plugins=false",
    "-c",
    "features.apps=false",
    "-c",
    "mcp_servers={}",
];
/// A child's stdout, read line by line on its own thread.
pub(crate) fn stdout_lines(
    stdout: std::process::ChildStdout,
) -> mpsc::Receiver<std::io::Result<String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}
fn classify_failure(message: &str) -> &'static str {
    let m = message.to_ascii_lowercase();
    if ["unknown option", "unknown feature flag", "unexpected argument", "unrecognized option"]
        .iter()
        .any(|s| m.contains(s))
    {
        "설치된 공식 CLI가 이 작업에 필요한 옵션을 지원하지 않습니다. 공식 CLI를 업데이트하세요"
    } else if [
        "usage limit",
        "quota",
        "rate limit",
        "rate_limit",
        "capacity",
    ]
    .iter()
    .any(|s| m.contains(s))
    {
        "구독 사용 한도에 도달했습니다. 공급자의 사용량을 확인한 뒤 다시 시도하세요"
    } else if [
        "unauthorized",
        "authentication",
        "login",
        "token expired",
        "401",
    ]
    .iter()
    .any(|s| m.contains(s))
    {
        "구독 인증을 확인할 수 없습니다. 공식 로그인에서 다시 연결하세요"
    } else if ["network", "connection", "dns", "timed out", "timeout"]
        .iter()
        .any(|s| m.contains(s))
    {
        "네트워크 연결이 실패했습니다. 원본과 기존 지도는 보존됩니다"
    } else {
        "공식 도구 실행에 실패했습니다. 원본과 기존 지도는 보존됩니다"
    }
}
fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
fn login(root: &Path, d: &Path, p: &str) -> Result<Value> {
    let path = bin(root, p).context("공식 CLI가 설치되어 있지 않습니다")?;
    event(d, "login", "공식 브라우저 로그인 대기")?;
    if p == "antigravity" {
        let mut client = super::antigravity::Client::spawn(root, &d.join("work"))?;
        register_child(d, client.child())?;
        client.initialize()?;
        client.rpc("authenticate", json!({"methodId":"oauth-personal"}), Duration::from_secs(600))?;
        client.session(&d.join("work"), None)?;
        return Ok(json!({"logged_in":true}));
    }
    if p == "claude" {
        let mut c = clean_command(&path);
        c.args(["auth", "login", "--claudeai"])
            .current_dir(d.join("work"));
        run_capture(c, d, d, &[], Duration::from_secs(600))?;
        return Ok(json!({"logged_in":status(root,p)["logged_in"]}));
    }
    let mut c = clean_command(&path);
    c.args(CODEX_APP_SERVER)
    .current_dir(d.join("work"))
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::from(fs::File::create(d.join("stderr.txt"))?))
    .process_group(0);
    let mut child = ChildGroup(c.spawn()?);
    register_child(d, &child)?;
    let mut stdin = child.stdin.take().unwrap();
    let rx = stdout_lines(child.stdout.take().unwrap());
    writeln!(
        stdin,
        "{}",
        json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"stology_local","title":"Stology Local","version":"0.1.0"}}})
    )?;
    stdin.flush()?;
    let result = (|| -> Result<Value> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(600) {
            let line = rx.recv_timeout(Duration::from_secs(1));
            let Ok(Ok(line)) = line else { continue };
            let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            if v["id"] == 1 {
                if !v["error"].is_null() {
                    bail!("로그인 서버 초기화 실패")
                };
                writeln!(stdin, "{}", json!({"method":"initialized","params":{}}))?;
                writeln!(
                    stdin,
                    "{}",
                    json!({"id":2,"method":"account/login/start","params":{"type":"chatgpt"}})
                )?;
                stdin.flush()?;
            }
            if v["id"] == 2 {
                if let Some(url) = v["result"]["authUrl"].as_str() {
                    let mut s = json_read(&d.join("state.json"))?;
                    s["state"] = json!("waiting_login");
                    s["auth_url"] = json!(url);
                    write_json(&d.join("state.json"), &s)?;
                    Command::new("/usr/bin/open").arg(url).spawn()?;
                } else {
                    bail!("공식 로그인 URL을 받지 못했습니다")
                }
            }
            if v["method"] == "account/login/completed" {
                if v["params"]["success"] == true {
                    return Ok(json!({"logged_in":true}));
                }
                bail!("공식 계정 로그인 실패")
            }
        }
        bail!("로그인 시간이 만료되었습니다")
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}
fn install(root: &Path, d: &Path, p: &str) -> Result<Value> {
    if bin(root, p).is_some() {
        return Ok(json!({"installed":true,"existing":true}));
    }
    event(d, "install", "선택한 공급자의 공식 CLI 설치 중")?;
    if p == "antigravity" {
        let installer = d.join("antigravity-install.sh");
        let mut download = Command::new("/usr/bin/curl");
        download.args(["--fail","--location","--proto","=https","--tlsv1.2","https://antigravity.google/cli/install.sh","--output"]).arg(&installer);
        run_capture(download, d, d, &[], Duration::from_secs(120))?;
        // The official installer edits shell profiles: confine that setup to the job home.
        let install_home = d.join("installer-home");
        fs::create_dir_all(&install_home)?;
        let mut c = clean_command(Path::new("/bin/bash"));
        c.env("HOME", install_home).arg(installer).arg("--dir").arg(root.join("runtime/bin")).current_dir(d.join("work"));
        run_capture(c, d, d, &[], Duration::from_secs(600))?;
    } else if p == "codex" {
        let npm=bin(root,"npm").context("Node.js/npm이 필요합니다. https://developers.openai.com/codex/quickstart 를 확인하세요")?;
        let mut c = clean_command(&npm);
        c.args(["install", "--global", "--prefix"])
            .arg(root.join("runtime"))
            .arg("@openai/codex")
            .current_dir(d.join("work"));
        run_capture(c, d, d, &[], Duration::from_secs(600))?;
    } else {
        let installer = d.join("claude-install.sh");
        let mut c = Command::new("/usr/bin/curl");
        c.args([
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "https://claude.ai/install.sh",
            "--output",
        ])
        .arg(&installer);
        if !c.status()?.success() {
            bail!("Claude 공식 설치 스크립트 다운로드 실패")
        };
        let mut c = clean_command(Path::new("/bin/bash"));
        c.arg(installer).arg("stable").current_dir(d.join("work"));
        run_capture(c, d, d, &[], Duration::from_secs(600))?;
    }
    Ok(json!({"installed":bin(root,p).is_some()}))
}
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for b in bytes.chunks(3) {
        let n = ((b[0] as u32) << 16)
            | ((b.get(1).copied().unwrap_or(0) as u32) << 8)
            | (b.get(2).copied().unwrap_or(0) as u32);
        for shift in [18, 12] {
            s.push(T[((n >> shift) & 63) as usize] as char);
        }
        s.push(if b.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        s.push(if b.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}
fn parse_result(out: &str, p: &str) -> Result<Value> {
    if p == "codex" {
        return serde_json::from_str(out).context("Codex 최종 응답이 올바른 JSON이 아닙니다");
    }
    for line in out.lines().rev() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if v["type"] == "result" {
                if v["is_error"] == true {
                    bail!("Claude 분석 실패: {}", v["result"])
                }
                if v["structured_output"].is_object() {
                    return Ok(v["structured_output"].clone());
                }
                if let Some(s) = v["result"].as_str() {
                    return serde_json::from_str(s)
                        .context("Claude 최종 응답이 올바른 JSON이 아닙니다");
                }
            }
        }
    }
    bail!("Claude 구조화된 최종 응답이 없습니다")
}
fn codex_default_model(path: &Path, d: &Path) -> Result<String> {
    let mut c = clean_command(path);
    c.args(CODEX_APP_SERVER)
    .current_dir(d.join("work"))
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .process_group(0);
    let mut child = ChildGroup(c.spawn()?);
    register_child(d, &child)?;
    let mut input = child.stdin.take().unwrap();
    let rx = stdout_lines(child.stdout.take().unwrap());
    let result = (|| -> Result<String> {
        writeln!(
            input,
            "{}",
            json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"stology_local","version":"0.1.0"},"capabilities":null}})
        )?;
        input.flush()?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let Ok(Ok(line)) = rx.recv_timeout(Duration::from_secs(1)) else {
                continue;
            };
            let v: Value = serde_json::from_str(&line)?;
            if v["id"] == 1 {
                writeln!(input, "{}", json!({"method":"initialized","params":{}}))?;
                writeln!(
                    input,
                    "{}",
                    json!({"id":2,"method":"model/list","params":{}})
                )?;
                input.flush()?;
            }
            if v["id"] == 2 {
                let list = v["result"]["data"]
                    .as_array()
                    .context("모델 목록 응답 없음")?;
                return list
                    .iter()
                    .find(|m| m["isDefault"] == true)
                    .and_then(|m| m["model"].as_str())
                    .map(str::to_owned)
                    .context("기본 분석 모델을 확인하지 못했습니다");
            }
        }
        bail!("공식 기본 모델 확인 시간 초과")
    })();
    child.kill_group();
    let _ = child.wait();
    result
}
/// One structured provider call. The prompt already carries the skill text and input.
pub(crate) struct Call<'a> {
    pub provider: &'a str,
    pub model: Option<&'a str>,
    pub effort: &'a str,
    pub prompt: &'a str,
    pub schema: &'a Value,
    pub images: &'a [(PathBuf, String)],
    pub timeout: Duration,
}
pub(crate) struct Reply {
    pub result: Value,
    /// Model the provider reported, or the one requested when it reports none.
    pub model: String,
    /// `{"input","output","cached"}` token counts when the provider returns them, else null.
    pub usage: Value,
}
pub(crate) fn is_limit(message: &str) -> bool {
    message.contains("구독 사용 한도")
}
pub(crate) fn mark_success(root: &Path, p: &str, job_id: &str) -> Result<()> {
    write_json(&root.join(format!("provider-{p}-success.json")), &json!({"at":now(),"job_id":job_id}))
}
/// Token counts a provider reported. Claude lists the cached parts of the prompt
/// apart from `input_tokens`; Codex counts them inside it.
fn usage_of(log: &str) -> Value {
    for line in log.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let u = if v["type"] == "result" || v["type"] == "turn.completed" { &v["usage"] } else { continue };
        if u.is_object() {
            let (read, created) = (u["cache_read_input_tokens"].as_u64(), u["cache_creation_input_tokens"].as_u64());
            if read.is_some() || created.is_some() {
                let input = u["input_tokens"].as_u64().unwrap_or(0) + read.unwrap_or(0) + created.unwrap_or(0);
                return json!({"input":input,"output":u["output_tokens"],"cached":read});
            }
            return json!({"input":u["input_tokens"],"output":u["output_tokens"],"cached":u["cached_input_tokens"].as_u64()});
        }
    }
    Value::Null
}
/// `job` is the worker's job directory (cancel flag, child identity, isolated cwd);
/// `out` keeps this call's schema and raw output.
pub(crate) fn invoke(root: &Path, job: &Path, out: &Path, call: &Call) -> Result<Reply> {
    let p = call.provider;
    anyhow::ensure!(["codex", "claude", "antigravity"].contains(&p), "지원하지 않는 공급자");
    let path = bin(root, p).context("공식 CLI가 없습니다")?;
    fs::create_dir_all(out)?;
    let work = job.join("work");
    fs::create_dir_all(&work)?;
    for (image, mime) in call.images {
        if !["image/png", "image/jpeg"].contains(&mime.as_str()) {
            bail!("공급자 이미지 형식은 PNG/JPEG여야 합니다")
        };
        if fs::metadata(image)?.len() > 20 * 1024 * 1024 {
            bail!("분석 이미지가 20MB를 초과합니다")
        };
    }
    if call.images.len() > 32 {
        bail!("한 번에 최대 32개 이미지를 보낼 수 있습니다")
    }
    let model = if p == "codex" {
        Some(match call.model {
            Some(m) => m.to_owned(),
            None => codex_default_model(&path, job)?,
        })
    } else {
        call.model.map(str::to_owned)
    };
    let schema_path = out.join("schema.json");
    write_json(&schema_path, call.schema)?;
    let prompt = call.prompt;
    let mut reported = model.clone().unwrap_or_default();
    let mut c = clean_command(&path);
    c.current_dir(&work);
    let result = if p == "antigravity" {
        let mut client = super::antigravity::Client::spawn(root, &work)?;
        register_child(job, client.child())?;
        client.initialize()?;
        let data = client.session(&work, None)?;
        let session = data["sessionId"].as_str().context("Antigravity 세션 ID 없음")?;
        let catalog = super::antigravity::model_list(&data)?;
        let choice = match model.as_deref() {
            Some(m) => catalog.iter().find(|v| v["id"] == m),
            None => catalog.iter().find(|v| v["default"] == true).or_else(|| catalog.first()),
        }
        .context("Antigravity 모델을 확인할 수 없습니다")?;
        let selected = choice["id"].as_str().context("모델 ID 없음")?;
        client.configure(session, &data, selected, call.effort)?;
        reported = selected.to_owned();
        let input_images = call.images.iter().map(|(path, mime)| Ok((mime.clone(), base64(&fs::read(path)?)))).collect::<Result<Vec<_>>>()?;
        let image_paths = call.images.iter().enumerate().map(|(i, (p, _))| format!("Image #{} source path: {}", i + 1, p.display())).collect::<Vec<_>>().join("\n");
        let id = client.prompt(session, &format!("{prompt}\n{image_paths}"), &input_images)?;
        let deadline = Instant::now() + call.timeout;
        let mut answer = String::new();
        loop {
            if job.join("cancelled").exists() {
                let _ = client.cancel(session);
                bail!("작업이 취소되었습니다")
            }
            if Instant::now() > deadline {
                bail!("Antigravity 응답 시간 초과")
            }
            if let Some(v) = client.next(Duration::from_millis(100))? {
                if let Some(text) = super::antigravity::text_delta(&v, session) {
                    answer.push_str(text);
                }
                if answer.len() > MAX_OUTPUT as usize {
                    bail!("Antigravity 응답 크기 제한 초과")
                }
                if let Some(state) = super::antigravity::stop(&v, id)? {
                    if state != "completed" {
                        bail!("응답이 중단되었습니다")
                    }
                    break;
                }
            }
        }
        fs::write(out.join("answer.txt"), &answer)?;
        let trimmed = answer.trim();
        let raw = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .and_then(|s| s.trim_end().strip_suffix("```"))
            .unwrap_or(trimmed)
            .trim();
        serde_json::from_str(raw).context("Antigravity 응답이 올바른 JSON이 아닙니다")?
    } else if p == "codex" {
        let final_path = out.join("final.json");
        c.args([
            "exec",
            "--ignore-user-config",
            "--ignore-rules",
            "--skip-git-repo-check",
            "--ephemeral",
            "--json",
            "--color",
            "never",
            "--output-schema",
        ])
        .arg(&schema_path)
        .arg("--output-last-message")
        .arg(&final_path);
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
        ] {
            c.args(["--disable", flag]);
        }
        for conf in [
            "web_search=\"disabled\"",
            "default_permissions=\"stology\"",
            "permissions.stology.filesystem={\":minimal\"=\"read\",\":workspace_roots\"=\"read\",\":tmpdir\"=\"deny\",\":slash_tmp\"=\"deny\"}",
            "permissions.stology.network.enabled=false",
            "project_doc_max_bytes=0",
            "approval_policy=\"never\"",
            "mcp_servers={}",
            "forced_login_method=\"chatgpt\"",
        ] {
            c.args(["-c", conf]);
        }
        if let Some(model) = &model {
            c.args(["--model", model]);
        }
        if !call.effort.is_empty() {
            c.args(["-c", &format!("model_reasoning_effort=\"{}\"", call.effort)]);
        }
        c.args(["--enable", "skip_host_skill_discovery"]);
        for (im, _) in call.images {
            c.arg("--image").arg(im);
        }
        c.arg("-");
        run_capture(c, job, out, prompt.as_bytes(), call.timeout)?;
        parse_result(
            &fs::read_to_string(final_path).context("Codex 최종 응답 파일 없음")?,
            p,
        )?
    } else {
        c.args([
            "--print",
            "--safe-mode",
            "--restricted",
            "--tools",
            "",
            "--allowedTools",
            "StructuredOutput",
            "--disable-slash-commands",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--setting-sources",
            "",
            "--no-chrome",
            "--no-session-persistence",
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--json-schema",
        ])
        .arg(call.schema.to_string());
        if let Some(model) = &model {
            c.args(["--model", model]);
        }
        if !call.effort.is_empty() {
            c.args(["--effort", call.effort]);
        }
        let mut content = vec![json!({"type":"text","text":prompt})];
        for (im, mime) in call.images {
            content.push(json!({"type":"text","text":format!("Following image source path: {}",im.display())}));
            content.push(json!({"type":"image","source":{"type":"base64","media_type":mime,"data":base64(&fs::read(im)?)}}));
        }
        let input = format!(
            "{}\n",
            json!({"type":"user","message":{"role":"user","content":content}})
        );
        let (log, _) = run_capture(c, job, out, input.as_bytes(), call.timeout)?;
        parse_result(&log, p)?
    };
    let log = fs::read_to_string(out.join("stdout.jsonl")).unwrap_or_default();
    for line in log.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if let Some(model) = v["model"].as_str() {
                reported = model.to_owned();
            }
        }
    }
    Ok(Reply { result, model: reported, usage: usage_of(&log) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn b64_known() {
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"abc"), "YWJj");
    }
    #[test]
    fn rejects_bad_job_path() {
        assert!(job_dir(Path::new("/tmp"), "../secrets").is_err());
    }
    #[test]
    fn input_tokens_include_what_the_provider_cached() {
        let claude = usage_of(r#"{"type":"result","usage":{"input_tokens":2,"cache_creation_input_tokens":57280,"cache_read_input_tokens":100,"output_tokens":11129}}"#);
        assert_eq!(claude, json!({"input":57382,"output":11129,"cached":100}));
        let codex = usage_of(r#"{"type":"turn.completed","usage":{"input_tokens":63323,"cached_input_tokens":16896,"output_tokens":2657}}"#);
        assert_eq!(codex, json!({"input":63323,"output":2657,"cached":16896}));
        assert!(usage_of("no usage here").is_null());
    }
    #[test]
    fn structured_claude_and_errors() {
        assert!(parse_result("{}", "claude").is_err());
        let v = parse_result(
            "{\"type\":\"result\",\"structured_output\":{\"concepts\":[]}}",
            "claude",
        )
        .unwrap();
        assert!(v["concepts"].is_array());
        assert!(parse_result(
            "{\"type\":\"result\",\"is_error\":true,\"result\":\"quota\"}",
            "claude"
        )
        .is_err());
    }
    #[test]
    fn selection_persisted_no_graph_writes() {
        let d = tempfile::tempdir().unwrap();
        handle(d.path(), &json!({"op":"provider.set","provider":"claude"})).unwrap();
        assert_eq!(
            json_read(&d.path().join("provider.json")).unwrap()["provider"],
            "claude"
        );
        assert!(!d.path().join("storage.sqlite").exists());
    }
    #[test]
    fn active_job_and_held_lock_block_new_worker() {
        let d = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let jd = job_dir(d.path(), &id).unwrap();
        fs::create_dir_all(&jd).unwrap();
        write_json(
            &jd.join("state.json"),
            &json!({"state":"running","pid":std::process::id(),"worker_identity":process_identity(std::process::id() as i32)}),
        )
        .unwrap();
        fs::write(d.path().join("analysis.lock"), &id).unwrap();
        assert!(start(d.path(), &json!({"provider":"codex"}), "install").is_err());
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(d.path().join("analysis.lock"))
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        assert!(start(d.path(), &json!({"provider":"codex"}), "install").is_err());
        assert_eq!(
            fs::read_to_string(d.path().join("analysis.lock")).unwrap(),
            id
        );
    }
    #[test]
    fn timeout_reaps_child_and_grandchild_group() {
        let d = tempfile::tempdir().unwrap();
        write_json(&d.path().join("state.json"), &json!({"state":"running"})).unwrap();
        let mut c = Command::new("/bin/sh");
        c.arg("-c")
            .arg("sleep 30 & echo $! > grandchild.pid; wait")
            .current_dir(d.path());
        assert!(run_capture(c, d.path(), d.path(), b"", Duration::from_millis(200)).is_err());
        let pid = fs::read_to_string(d.path().join("grandchild.pid"))
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "grandchild must be reaped"
        );
    }
    #[test]
    fn actionable_failures() {
        assert!(classify_failure("usage limit exceeded").contains("한도"));
        assert!(classify_failure("401 unauthorized").contains("인증"));
        assert!(classify_failure("network timeout").contains("네트워크"));
        // An older CLI that refuses an option is not mistaken for a login problem.
        assert!(classify_failure("error: unknown option '--safe-mode' (login)").contains("업데이트"));
        assert!(classify_failure("Error: Unknown feature flag: skip_host_skill_discovery").contains("업데이트"));
    }
    #[test]
    fn stale_process_identity_does_not_signal_reused_pid() {
        let pid = std::process::id() as i32;
        let mut identity = process_identity(pid).unwrap();
        assert!(identity_matches(&identity));
        identity["start_usec"] = json!(identity["start_usec"].as_u64().unwrap() + 1);
        assert!(!identity_matches(&identity));
        signal_identity(&identity, libc::SIGKILL);
        assert!(process_identity(pid).is_some());
    }
}
