//! A run: the steps of 과목 정리 (or one material's 분석) carried out by one worker.
//! The plan and every step are recorded on disk; a finished step's result is kept
//! under (input, skill fingerprint, provider, model) so a later run reuses it.
//! Nothing reaches the subject's map until the last step publishes it at once.
use crate::{
    knowledge::{self, Analysis, Checked, Extraction},
    ontology,
    providers::{self, Call},
    skills::{self, Skill},
};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const STOPPED: &str = "정리를 멈췄습니다";

fn now() -> String {
    chrono::Local::now().to_rfc3339()
}
fn run_dir(root: &Path, id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id).context("잘못된 실행 ID")?;
    Ok(root.join("ai/runs").join(id))
}
fn load(root: &Path, id: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(run_dir(root, id)?.join("run.json")).context("실행 기록을 찾을 수 없습니다")?)?)
}
fn save(root: &Path, run: &mut Value) -> Result<()> {
    run["updated_at"] = json!(now());
    skills::write_json(&run_dir(root, run["id"].as_str().unwrap_or(""))?.join("run.json"), run)
}
fn pointer(root: &Path, w: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(w).context("잘못된 과목 ID")?;
    Ok(root.join("ai/runs").join(format!("latest-{w}.json")))
}
fn latest(root: &Path, w: &str) -> Result<Option<Value>> {
    let Ok(bytes) = fs::read(pointer(root, w)?) else { return Ok(None) };
    let id = serde_json::from_slice::<Value>(&bytes)?["run_id"].as_str().unwrap_or("").to_owned();
    let mut run = match load(root, &id) {
        Ok(run) => run,
        Err(_) => return Ok(None),
    };
    // A worker that died (app quit, crash) leaves a run that says it is running. Whether the
    // worker lives is read from the job it runs as, never from what the record says about it.
    if run["state"] == "running" {
        let job = providers::handle(root, &json!({"op":"analysis.active"})).ok().map(|v| v["active"].clone()).filter(|job| job["run_id"] == run["id"]);
        match job {
            Some(job) => run["job_id"] = job["id"].clone(),
            None => {
                // The worker writes its last record before its job ends, so this read is final.
                run = load(root, &id)?;
                if run["state"] == "running" {
                    run["state"] = json!("paused");
                    if run["error"].is_null() {
                        run["error"] = json!("정리가 끝나기 전에 멈췄어요. 읽은 부분은 보관해 두었어요.");
                    }
                    save(root, &mut run)?;
                }
            }
        }
    }
    Ok(Some(run))
}

// ── Splitting a material ─────────────────────────────────────────────────
/// Consecutive, evenly sized groups: at most `max_pages` page images and about
/// `max_chars` characters each. 27 pages become 14 + 13, not 24 + 3.
pub(crate) fn split(units: &[(bool, usize)], max_pages: usize, max_chars: usize) -> Vec<std::ops::Range<usize>> {
    if units.is_empty() {
        return Vec::new();
    }
    let pages = units.iter().filter(|u| u.0).count();
    let chars: usize = units.iter().map(|u| u.1).sum();
    let groups = pages.div_ceil(max_pages.max(1)).max(chars.div_ceil(max_chars.max(1))).clamp(1, units.len());
    let (base, extra) = (units.len() / groups, units.len() % groups);
    let mut out = Vec::with_capacity(groups);
    let mut start = 0;
    for i in 0..groups {
        let len = base + usize::from(i < extra);
        out.push(start..start + len);
        start += len;
    }
    out
}
fn chunk_limits(routing: &Value) -> (usize, usize) {
    (routing["chunk"]["max_pages"].as_u64().unwrap_or(24) as usize, routing["chunk"]["max_chars"].as_u64().unwrap_or(60000) as usize)
}
/// Pages and chunks a material will take, without extracting it.
fn estimate(path: &Path, kind: &str, routing: &Value) -> Result<(usize, usize)> {
    let (max_pages, max_chars) = chunk_limits(routing);
    Ok(match kind {
        "pdf" => {
            let out = Command::new(knowledge::helper_path()?).arg("--pages").arg(path).output().context("PDF 쪽 수를 셀 수 없습니다")?;
            anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim().to_owned());
            let pages: usize = String::from_utf8_lossy(&out.stdout).trim().parse().context("PDF 쪽 수를 셀 수 없습니다")?;
            anyhow::ensure!(pages <= 100, "PDF는 100쪽 이하만 읽을 수 있어요 ({pages}쪽)");
            (pages, pages.div_ceil(max_pages).max(1))
        }
        "md" => {
            let chars = fs::read_to_string(path)?.chars().count();
            (1, chars.div_ceil(max_chars).max(1))
        }
        "pptx" => {
            let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
            let slides = (0..zip.len()).filter(|i| zip.by_index(*i).is_ok_and(|f| f.name().starts_with("ppt/slides/slide") && f.name().ends_with(".xml"))).count();
            (slides, slides.div_ceil(max_pages).max(1))
        }
        _ => (1, 1),
    })
}

// ── Planning ─────────────────────────────────────────────────────────────
/// The provider a run uses. 과목 정리 remembers its own last choice; one
/// material's 분석 follows the provider chosen for the app.
fn provider_for(root: &Path, req: &Value) -> Result<String> {
    let saved = skills::settings(root)["organize_provider"].as_str().map(str::to_owned);
    let selected = fs::read(root.join("provider.json")).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["provider"].as_str().map(str::to_owned));
    let remembered = if req["single"] == true { selected.or(saved) } else { saved.or(selected) };
    let p = req["provider"].as_str().map(str::to_owned).or(remembered).context("먼저 AI 공급자를 연결해 주세요")?;
    anyhow::ensure!(["codex", "claude", "antigravity"].contains(&p.as_str()), "지원하지 않는 공급자");
    Ok(skills::helper_provider(&skills::settings(root), "extract", &p).to_owned())
}
fn usage_label(calls: usize) -> &'static str {
    match calls {
        0..=5 => "적음",
        6..=20 => "보통",
        _ => "많음",
    }
}
/// What a run would do: which materials it reads, how many calls it expects, and
/// which tier and model each stage uses.
fn plan(root: &Path, req: &Value) -> Result<Value> {
    let w = req["workspace_id"].as_str().context("workspace_id가 필요합니다")?;
    let settings = skills::settings(root);
    let single = req["single"] == true;
    let reread = req["reread"] == true;
    let mode = req["mode"].as_str().or(settings["organize_mode"].as_str()).unwrap_or("auto").to_owned();
    let provider = provider_for(root, req)?;
    let routing = skills::routing(root)?;
    let c = knowledge::db(root)?;
    let chosen: Option<Vec<&str>> = req["materials"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect());
    let mut materials = ontology::material_states(&c, root, w)?;
    let (mut pages, mut extract_calls, mut targets) = (0, 0, 0);
    for m in &mut materials {
        let target = match &chosen {
            Some(ids) => ids.contains(&m["id"].as_str().unwrap_or("")),
            None => m["state"] != "organised",
        };
        m["target"] = json!(target);
        if !target {
            continue;
        }
        match estimate(Path::new(m["absolute_path"].as_str().unwrap_or("")), m["kind"].as_str().unwrap_or(""), &routing) {
            Ok((p, chunks)) => {
                m["pages"] = json!(p);
                m["chunks"] = json!(chunks);
                pages += p;
                extract_calls += chunks;
                targets += 1;
            }
            Err(e) => {
                m["target"] = json!(false);
                m["unreadable"] = json!(format!("{e:#}"));
            }
        }
    }
    if let Some(ids) = &chosen {
        anyhow::ensure!(targets == ids.len(), "{}", materials.iter().find_map(|m| m["unreadable"].as_str()).unwrap_or("선택한 자료를 찾을 수 없습니다"));
    }
    let catalog = crate::chat::handle(root, &json!({"op":"chat.models","provider":provider}))?["models"].as_array().cloned().unwrap_or_default();
    let mut stages = skills::stage_plan_for(root, &provider, &mode, &catalog, if single { &["extract"] } else { &["extract", "align", "connect", "review"] })?;
    if reread && stages["stages"]["extract"]["tier"] != "custom" {
        if let Some((tier, model)) = skills::highest(&stages["tiers"]) {
            stages["stages"]["extract"] = json!({"provider":provider,"tiers":stages["tiers"],"requested":tier,"tier":tier,"model":model["model"],"effort":model["effort"],"note":"이 자료만 가장 높은 등급으로 다시 읽어요","below_min":false});
        }
    }
    let structure = !single;
    let review = structure && !stages["stages"]["review"].is_null();
    let calls = json!({
        "extract":extract_calls,"align":usize::from(structure),"connect":usize::from(structure),"review":usize::from(review),
        "total":extract_calls + if structure { 2 + usize::from(review) } else { 0 },
    });
    let organised = materials.iter().filter(|m| m["state"] == "organised").count();
    let mut notes: Vec<String> = stages["stages"].as_object().into_iter().flatten().filter(|(k, _)| structure || *k == "extract")
        .filter_map(|(_, s)| s["note"].as_str().filter(|n| !n.is_empty()).map(str::to_owned)).collect();
    notes.dedup();
    if structure && ["align", "connect"].iter().any(|s| stages["stages"][*s]["below_min"] == true) {
        notes.push("이 계정의 모델로는 개념 맞추기와 점선 연결을 만들기 어려워, 단원 묶기까지만 해요".into());
    }
    let customized = skills::customized(root);
    if !customized.is_empty() {
        notes.push("고친 스킬을 사용해요".into());
    }
    let total = calls["total"].as_u64().unwrap_or(0) as usize;
    Ok(json!({
        "workspace_id":w,"provider":provider,"mode":mode,"single":single,"reread":reread,"materials":materials,
        "targets":targets,"pages":pages,"organised":organised,"calls":calls,"usage":usage_label(total),
        "stages":stages["stages"],"tiers":stages["tiers"],"notes":notes,"customized":customized,
        "guidance":ontology::guidance(&c, w)?,
        "resumable":latest(root, w)?.filter(|r| matches!(r["state"].as_str(), Some("paused" | "limit" | "failed"))).map(|r| json!({"id":r["id"],"state":r["state"],"progress":r["progress"],"error":r["error"]})),
    }))
}
fn spawn(root: &Path, run: &Value) -> Result<Value> {
    // A waiting chat-note job gives way to what the user asked for.
    if let Ok(active) = providers::handle(root, &json!({"op":"analysis.active"})) {
        if active["active"]["kind"] == "distill" {
            providers::handle(root, &json!({"op":"analysis.cancel","id":active["active"]["id"]}))?;
        }
    }
    providers::start(root, &json!({"provider":run["provider"],"workspace_id":run["workspace_id"],"run_id":run["id"],"lane":"front"}), "run")
}
pub(crate) fn start(root: &Path, req: &Value) -> Result<Value> {
    let plan = plan(root, req)?;
    let w = plan["workspace_id"].as_str().unwrap_or("");
    let single = plan["single"] == true;
    anyhow::ensure!(
        plan["targets"].as_u64().unwrap_or(0) > 0 || (!single && plan["organised"].as_u64().unwrap_or(0) > 0),
        "정리할 자료가 없습니다. 과목에 PDF·노트·이미지·PPTX 자료를 먼저 넣어 주세요."
    );
    let provider = plan["provider"].as_str().unwrap_or("");
    anyhow::ensure!(providers::bin(root, provider).is_some(), "{provider} 공식 CLI가 없습니다. 설정에서 먼저 연결해 주세요.");
    let mut materials = Vec::new();
    for m in plan["materials"].as_array().into_iter().flatten().filter(|m| m["target"] == true) {
        let expected = if single && req["expected_hash"].is_string() { req["expected_hash"].clone() } else { m["hash"].clone() };
        let source = crate::storage::handle(root, &json!({"op":"source.snapshot","id":m["id"],"expected_hash":expected,"workspace_id":w}))?;
        materials.push(json!({
            "id":m["id"],"title":m["title"],"kind":m["kind"],"hash":source["hash"],"immutable_path":source["immutable_path"],
            "state":"pending","chunks":[],
        }));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let mut run = json!({
        "id":id,"workspace_id":w,"provider":provider,"mode":plan["mode"],"single":single,"reread":plan["reread"],
        "state":"running","created_at":now(),"stages":plan["stages"],"tiers":plan["tiers"],"notes":plan["notes"],
        "materials":materials,"progress":{"label":"준비하는 중","done":0,"total":plan["calls"]["total"]},
        "expected_calls":plan["calls"],"error":null,"result":null,
    });
    fs::create_dir_all(run_dir(root, &id)?)?;
    // From here on only the worker writes the record; a second write here could undo its progress.
    save(root, &mut run)?;
    let job = match spawn(root, &run) {
        Ok(job) => job,
        Err(e) => {
            let _ = fs::remove_dir_all(run_dir(root, &id)?);
            return Err(e);
        }
    };
    skills::write_json(&pointer(root, w)?, &json!({"run_id":id}))?;
    if !single && req["provider"].is_string() {
        skills::handle(root, &json!({"op":"ai.settings.set","organize_provider":provider}))?;
    }
    let mut job = job;
    job["run_id"] = json!(id);
    Ok(job)
}
fn resume(root: &Path, req: &Value) -> Result<Value> {
    let w = req["workspace_id"].as_str().context("workspace_id가 필요합니다")?;
    let mut run = latest(root, w)?.context("이어서 할 정리가 없습니다")?;
    anyhow::ensure!(matches!(run["state"].as_str(), Some("paused" | "limit" | "failed")), "이어서 할 정리가 없습니다");
    for m in run["materials"].as_array_mut().into_iter().flatten() {
        if m["state"] != "done" {
            m["state"] = json!("pending");
            m["reason"] = Value::Null;
        }
    }
    run["state"] = json!("running");
    run["error"] = Value::Null;
    save(root, &mut run)?;
    let job = spawn(root, &run)?;
    let mut job = job;
    job["run_id"] = run["id"].clone();
    Ok(job)
}
/// Stop the step in progress. Finished steps stay in the cache for `이어서 정리`.
fn pause(root: &Path, req: &Value) -> Result<Value> {
    let w = req["workspace_id"].as_str().context("workspace_id가 필요합니다")?;
    let mut run = latest(root, w)?.context("진행 중인 정리가 없습니다")?;
    if run["state"] == "running" {
        if let Some(job) = run["job_id"].as_str() {
            providers::handle(root, &json!({"op":"analysis.cancel","id":job}))?;
        }
        run = load(root, run["id"].as_str().unwrap_or(""))?;
        run["state"] = json!("paused");
        run["error"] = json!("멈췄어요. 읽은 부분은 보관해 두었어요.");
        save(root, &mut run)?;
    }
    Ok(json!({"run":described(root, run)?}))
}
pub fn handle(root: &Path, req: &Value) -> Result<Value> {
    match req["op"].as_str().unwrap_or("") {
        "organize.plan" => plan(root, req),
        "organize.start" => start(root, req),
        "organize.resume" => resume(root, req),
        "organize.pause" => pause(root, req),
        "organize.status" => {
            let w = req["workspace_id"].as_str().context("workspace_id가 필요합니다")?;
            Ok(json!({"run":latest(root, w)?.map(|run| described(root, run)).transpose()?}))
        }
        "organize.dismiss" => {
            let w = req["workspace_id"].as_str().context("workspace_id가 필요합니다")?;
            if let Some(mut run) = latest(root, w)? {
                if run["state"] != "running" {
                    run["seen"] = json!(true);
                    save(root, &mut run)?;
                }
            }
            Ok(json!({"dismissed":true}))
        }
        op => bail!("지원하지 않는 과목 정리 명령: {op}"),
    }
}

// ── Reporting ────────────────────────────────────────────────────────────
fn stage_label(stage: &str) -> &'static str {
    match stage {
        "extract" => "자료 읽기",
        "align" => "개념 맞추기",
        "connect" => "개념 잇기",
        "review" => "연결 검수",
        _ => "대화 정리",
    }
}
fn provider_label(p: &str) -> &'static str {
    match p {
        "codex" => "Codex",
        "claude" => "Claude",
        _ => "Antigravity",
    }
}
fn mode_label(mode: &str) -> &'static str {
    match mode {
        "fast" => "빠르게",
        "thorough" => "꼼꼼하게",
        _ => "알아서",
    }
}
fn attempts(run: &Value) -> Vec<&Value> {
    let mut all: Vec<&Value> = run["materials"].as_array().into_iter().flatten()
        .flat_map(|m| m["chunks"].as_array().into_iter().flatten())
        .flat_map(|c| c["attempts"].as_array().into_iter().flatten()).collect();
    for stage in ["align", "connect", "review"] {
        all.extend(run[stage]["calls"].as_array().into_iter().flatten());
    }
    all
}
/// `어떻게 정리했나요`: calls per stage, tiers asked for and models that answered,
/// why something was read again, and whether edited skills were used.
fn how(root: &Path, run: &Value) -> Vec<String> {
    let mut lines = vec![format!("{} · {} 방식", provider_label(run["provider"].as_str().unwrap_or("")), mode_label(run["mode"].as_str().unwrap_or("")))];
    let all = attempts(run);
    for stage in ["extract", "align", "connect", "review"] {
        let of: Vec<&&Value> = all.iter().filter(|a| a["stage"] == stage).collect();
        if of.is_empty() {
            if let Some(reason) = run[stage]["reason"].as_str().filter(|r| !r.is_empty()) {
                lines.push(format!("{}: 하지 않음 — {reason}", stage_label(stage)));
            }
            continue;
        }
        let called = of.iter().filter(|a| a["cache"] != true).count();
        let reused = of.len() - called;
        let mut models: Vec<String> = Vec::new();
        for a in &of {
            let requested = a["requested"].as_str().unwrap_or("");
            let tier = a["tier"].as_str().unwrap_or("");
            let asked = if requested == tier || requested.is_empty() { skills::tier_label(tier).to_owned() } else { format!("요청 {} → {}", skills::tier_label(requested), skills::tier_label(tier)) };
            let effort = a["effort"].as_str().filter(|e| !e.is_empty()).map(|e| format!(" · {e}")).unwrap_or_default();
            let entry = format!("{asked} 등급 {}{effort}", a["model"].as_str().unwrap_or("?"));
            if !models.contains(&entry) {
                models.push(entry);
            }
        }
        let sum = |key: &str| -> Option<u64> {
            let values: Vec<u64> = of.iter().filter(|a| a["cache"] != true).filter_map(|a| a["usage"][key].as_u64()).collect();
            (!values.is_empty()).then(|| values.iter().sum())
        };
        let tokens = match (sum("input"), sum("output")) {
            (Some(i), Some(o)) => format!(" · 토큰 입력 {i} / 출력 {o}"),
            _ => " · 토큰 수는 공급자가 알려 주지 않았어요".to_owned(),
        };
        let reuse = if reused > 0 { format!(" (보관한 결과 {reused}번 재사용)") } else { String::new() };
        lines.push(format!("{}: AI 호출 {called}번{reuse} · {}{}", stage_label(stage), models.join(", "), if called > 0 { tokens } else { String::new() }));
    }
    for m in run["materials"].as_array().into_iter().flatten() {
        let title = m["title"].as_str().unwrap_or("자료");
        let chunks = m["chunks"].as_array().map(Vec::len).unwrap_or(0);
        for (k, chunk) in m["chunks"].as_array().into_iter().flatten().enumerate() {
            for a in chunk["attempts"].as_array().into_iter().flatten().filter(|a| a["why"].as_str().is_some_and(|w| !w.is_empty())) {
                lines.push(format!("다시 읽음: {title} ({chunks}묶음 중 {}) — {}", k + 1, a["why"].as_str().unwrap_or("")));
            }
        }
        match m["state"].as_str() {
            Some("failed") => lines.push(format!("건너뜀: {title} — {}", m["reason"].as_str().unwrap_or("읽지 못했어요"))),
            Some("changed") => lines.push(format!("건너뜀: {title} — 정리하는 동안 자료가 바뀌었어요. 다음 정리에서 다시 읽어요")),
            _ => {}
        }
        if m["quality"] == "low" {
            lines.push(format!("정리 품질 낮음: {title} — 다시 읽어도 근거가 많이 맞지 않았어요"));
        }
    }
    for stage in ["align", "connect", "review"] {
        for a in run[stage]["calls"].as_array().into_iter().flatten().filter(|a| a["why"].as_str().is_some_and(|w| !w.is_empty())) {
            lines.push(format!("{} 다시 함 — {}", stage_label(stage), a["why"].as_str().unwrap_or("")));
        }
    }
    if let Some(note) = run["connect"]["note"].as_str().filter(|n| !n.is_empty()) {
        lines.push(note.to_owned());
    }
    let customized: Vec<&str> = run["skills"].as_object().into_iter().flatten().filter(|(_, s)| s["customized"] == true).map(|(k, _)| k.as_str()).collect();
    lines.push(if customized.is_empty() { "앱에 든 스킬을 그대로 사용".to_owned() } else { format!("고친 스킬 사용: {}", customized.join(", ")) });
    // Materials of one subject may have been read by different providers.
    if let (Some(w), Ok(c)) = (run["workspace_id"].as_str(), knowledge::db(root)) {
        let rows: Vec<(String, String)> = c.prepare("SELECT s.title,m.provider FROM ont_material_state m JOIN sources s ON s.workspace=m.workspace AND s.material_id=m.material_id AND s.current=1 WHERE m.workspace=?1 ORDER BY s.title")
            .and_then(|mut q| q.query_map([w], |r| Ok((r.get(0)?, r.get(1)?))).and_then(|rows| rows.collect())).unwrap_or_default();
        let mut providers: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
        providers.sort_unstable();
        providers.dedup();
        if providers.len() > 1 {
            lines.push(format!("자료별 공급자: {}", rows.iter().map(|(t, p)| format!("{t} — {}", provider_label(p))).collect::<Vec<_>>().join(", ")));
        }
    }
    lines
}
fn described(root: &Path, mut run: Value) -> Result<Value> {
    run["how"] = json!(how(root, &run));
    Ok(run)
}

// ── Worker ───────────────────────────────────────────────────────────────
struct Ctx<'a> {
    root: &'a Path,
    job: &'a Path,
    dir: PathBuf,
    provider: String,
    run_id: String,
    routing: Value,
}
fn stopped(job: &Path) -> bool {
    job.join("cancelled").exists()
}
/// A failure that ends the run instead of one step: the user stopped it, or the
/// subscription limit was reached.
fn ends_run(job: &Path, message: &str) -> bool {
    stopped(job) || providers::is_limit(message)
}
pub(crate) fn yield_request(root: &Path) -> PathBuf {
    root.join("ai/yield-request")
}
/// Take the execution lock for one step. A chat turn that asked to go first is
/// let through before the next step starts.
fn wait_turn(ctx: &Ctx) -> Result<crate::chat::ExecutionLock> {
    loop {
        anyhow::ensure!(!stopped(ctx.job), STOPPED);
        let asked = fs::metadata(yield_request(ctx.root)).ok().and_then(|m| m.modified().ok())
            .is_some_and(|at| at.elapsed().map(|d| d < Duration::from_secs(600)).unwrap_or(false));
        if !asked {
            if let Ok(lock) = crate::chat::ExecutionLock::acquire(ctx.root) {
                return Ok(lock);
            }
        }
        thread::sleep(Duration::from_millis(300));
    }
}
#[derive(Clone)]
struct Choice {
    provider: String,
    tiers: Value,
    requested: String,
    tier: String,
    model: String,
    effort: String,
}
fn choice(stage: &Value) -> Choice {
    let text = |k: &str| stage[k].as_str().unwrap_or("").to_owned();
    Choice { provider: text("provider"), tiers: stage["tiers"].clone(), requested: text("requested"), tier: text("tier"), model: text("model"), effort: text("effort") }
}
/// The next tier up for a second try, when the account has one.
fn step_up(run: &Value, current: &Choice) -> Option<Choice> {
    if current.tier == "custom" {
        return None;
    }
    skills::tier_above(if current.tiers.is_object() { &current.tiers } else { &run["tiers"] }, &current.tier, &current.model).map(|(tier, m)| Choice {
        provider: current.provider.clone(), tiers: current.tiers.clone(),
        requested: current.requested.clone(),
        tier: tier.to_owned(),
        model: m["model"].as_str().unwrap_or("").to_owned(),
        effort: m["effort"].as_str().unwrap_or("").to_owned(),
    })
}
fn top(run: &Value, current: &Choice) -> Option<Choice> {
    if current.tier == "custom" {
        return None;
    }
    skills::highest(if current.tiers.is_object() { &current.tiers } else { &run["tiers"] }).filter(|(_, m)| m["model"] != current.model.as_str()).map(|(tier, m)| Choice {
        provider: current.provider.clone(), tiers: current.tiers.clone(),
        requested: current.requested.clone(),
        tier: tier.to_owned(),
        model: m["model"].as_str().unwrap_or("").to_owned(),
        effort: m["effort"].as_str().unwrap_or("").to_owned(),
    })
}
/// One skill call, or its kept result. Returns the outcome and the step record.
#[allow(clippy::too_many_arguments)]
fn call(ctx: &Ctx, skill: &Skill, stage: &str, c: &Choice, input: &Value, images: &[(PathBuf, String)], attempt: usize, why: &str) -> (Result<Value>, Value) {
    let provider = if c.provider.is_empty() { ctx.provider.as_str() } else { c.provider.as_str() };
    let text = input.to_string();
    let key = skills::digest(format!("{}|{}|{}|{}|{}|{attempt}", skill.fingerprint, provider, c.model, c.effort, skills::digest(text.as_bytes())).as_bytes());
    let kept = ctx.root.join("ai/cache").join(format!("{key}.json"));
    let mut record = json!({"provider":provider,"stage":stage,"skill":skill.name,"requested":c.requested,"tier":c.tier,"model":c.model,"effort":c.effort,"why":why,"key":key});
    if let Ok(saved) = fs::read(&kept).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice::<Value>(&b)?)) {
        // A result this run asked for before it stopped is still this run's call, not a reuse.
        if saved["run"] == ctx.run_id.as_str() {
            record["seconds"] = saved["seconds"].clone();
        } else {
            record["cache"] = json!(true);
        }
        record["model"] = saved["model"].clone();
        record["usage"] = saved["usage"].clone();
        return (Ok(saved["result"].clone()), record);
    }
    let started = Instant::now();
    let outcome = (|| {
        let _turn = wait_turn(ctx)?;
        let prompt = format!("{}\n\n## INPUT\n\n```json\n{text}\n```\n", skill.text);
        let timeout = Duration::from_secs(ctx.routing["timeouts_sec"][stage].as_u64().unwrap_or(600));
        let out = ctx.dir.join("calls").join(&key);
        // Every attempt uses the subscription, also one that is stopped or fails halfway.
        let _ = skills::log_usage(ctx.root, "organize", &skill.name, provider, &c.model, &ctx.run_id);
        providers::invoke(ctx.root, ctx.job, &out, &Call {
            provider: provider,
            model: (!c.model.is_empty()).then_some(c.model.as_str()),
            effort: &c.effort,
            prompt: &prompt,
            schema: &skill.schema,
            images,
            timeout,
        })
    })();
    record["seconds"] = json!(started.elapsed().as_secs());
    match outcome {
        Ok(reply) => {
            record["model"] = json!(reply.model);
            record["usage"] = reply.usage.clone();
            let _ = fs::create_dir_all(ctx.root.join("ai/cache"));
            let _ = skills::write_json(&kept, &json!({"result":reply.result,"model":reply.model,"usage":reply.usage,"at":now(),"skill":skill.name,"version":skill.version,"run":ctx.run_id,"seconds":record["seconds"]}));
            let _ = providers::mark_success(ctx.root, provider, ctx.job.file_name().and_then(|n| n.to_str()).unwrap_or(""));
            (Ok(reply.result), record)
        }
        Err(e) => {
            record["error"] = json!(format!("{e:#}"));
            (Err(e), record)
        }
    }
}
fn set_progress(root: &Path, run: &mut Value, label: String) -> Result<()> {
    let done = attempts(run).len();
    run["progress"]["label"] = json!(label);
    run["progress"]["done"] = json!(done);
    save(root, run)
}
fn prefixed(mut a: Analysis, prefix: &str) -> Analysis {
    for c in &mut a.concepts {
        c.id = format!("{prefix}{}", c.id);
    }
    for r in &mut a.relations {
        r.id = format!("{prefix}{}", r.id);
    }
    for claim in &mut a.assertions {
        claim.subject = format!("{prefix}{}", claim.subject);
        claim.object = format!("{prefix}{}", claim.object);
        claim.relation = format!("{prefix}{}", claim.relation);
    }
    a
}
/// Join the chunks of one material. A concept named the same in two chunks is
/// one concept; within a chunk the provider's own split of homonyms is kept.
fn merge_chunks(chunks: Vec<Analysis>) -> Analysis {
    let mut out = Analysis::default();
    let mut concept_names: Vec<(Vec<String>, String, usize)> = Vec::new();
    let mut relation_keys: Vec<(String, String, usize)> = Vec::new();
    for (k, chunk) in chunks.into_iter().enumerate() {
        let chunk = prefixed(chunk, &format!("k{k}_"));
        let mut renamed = std::collections::HashMap::new();
        for concept in chunk.concepts {
            let names: Vec<String> = std::iter::once(&concept.name).chain(&concept.aliases).map(|n| knowledge::normalized(n)).collect();
            let earlier = concept_names.iter().position(|(known, _, from)| *from != k && known.iter().any(|n| names.contains(n)));
            match earlier {
                Some(at) => {
                    let id = concept_names[at].1.clone();
                    let target = out.concepts.iter_mut().find(|c| c.id == id).unwrap();
                    for alias in std::iter::once(&concept.name).chain(&concept.aliases) {
                        if alias != &target.name && !target.aliases.contains(alias) {
                            target.aliases.push(alias.clone());
                        }
                    }
                    concept_names[at].0.extend(names);
                    renamed.insert(concept.id, id);
                }
                None => {
                    concept_names.push((names, concept.id.clone(), k));
                    out.concepts.push(concept);
                }
            }
        }
        for relation in chunk.relations {
            let key = format!("{}|{}|{}", knowledge::normalized(&relation.label), relation.kind.as_deref().unwrap_or(""), relation.directed);
            match relation_keys.iter().find(|(known, _, from)| *from != k && *known == key) {
                Some((_, id, _)) => {
                    renamed.insert(relation.id, id.clone());
                }
                None => {
                    relation_keys.push((key, relation.id.clone(), k));
                    out.relations.push(relation);
                }
            }
        }
        for mut claim in chunk.assertions {
            for id in [&mut claim.subject, &mut claim.object, &mut claim.relation] {
                if let Some(to) = renamed.get(id.as_str()) {
                    *id = to.clone();
                }
            }
            // The two halves of a merged concept may state a link to themselves.
            if claim.subject != claim.object {
                out.assertions.push(claim);
            }
        }
    }
    out
}
struct Read {
    analysis: Analysis,
    low: bool,
    total: usize,
    dropped: usize,
}
fn drop_rate(checked: &Checked) -> f64 {
    if checked.total == 0 { 0.0 } else { checked.dropped as f64 / checked.total as f64 }
}
/// Read one chunk: check the evidence, and read once more a tier up when too
/// much of it fails, the format is wrong, or nothing comes back.
fn read_chunk(ctx: &Ctx, run: &mut Value, skill: &Skill, mi: usize, ki: usize, extraction: &Extraction, range: std::ops::Range<usize>) -> Result<(Checked, bool)> {
    let units = &extraction.units[range];
    let limit = ctx.routing["extract"]["max_concepts_per_chunk"].as_u64().unwrap_or(24);
    let input = json!({
        "material":run["materials"][mi]["title"],
        "units":units.iter().map(|u| json!({"locator":u.locator,"text":u.text,"images":u.images.iter().map(|i| &i.path).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "limits":{"max_concepts":limit},
    });
    let images: Vec<(PathBuf, String)> = units.iter().flat_map(|u| &u.images).map(|i| (PathBuf::from(&i.path), i.mime.clone())).collect();
    let part = Extraction { input_hash: extraction.input_hash.clone(), kind: extraction.kind.clone(), units: units.to_vec(), warnings: Vec::new() };
    let threshold = ctx.routing["checks"]["evidence_drop_rate"].as_f64().unwrap_or(0.3);
    let mut current = choice(&run["stages"]["extract"]);
    let mut best: Option<Checked> = None;
    let mut why = String::new();
    for attempt in 0..2 {
        let (outcome, record) = call(ctx, skill, "extract", &current, &input, &images, attempt, &why);
        run["materials"][mi]["chunks"][ki]["attempts"].as_array_mut().context("묶음 기록 없음")?.push(record);
        save(ctx.root, run)?;
        let mut retry_same = false;
        let problem = match outcome {
            Err(e) => {
                let message = format!("{e:#}");
                if ends_run(ctx.job, &message) {
                    return Err(e);
                }
                retry_same = true;
                format!("호출 실패: {}", message.chars().take(200).collect::<String>())
            }
            Ok(result) => match serde_json::from_value::<Analysis>(result).map_err(anyhow::Error::from).and_then(|a| knowledge::check(a, &part)) {
                Err(_) => {
                    retry_same = true;
                    "형식이 맞지 않는 답".to_owned()
                }
                Ok(checked) => {
                    let (rate, total) = (drop_rate(&checked), checked.total);
                    let last = run["materials"][mi]["chunks"][ki]["attempts"].as_array_mut().and_then(|a| a.last_mut()).context("묶음 기록 없음")?;
                    last["claims"] = json!(total);
                    last["dropped"] = json!(checked.dropped);
                    if best.as_ref().is_none_or(|b| (b.total == 0 && total > 0) || (total > 0 && rate < drop_rate(b))) {
                        best = Some(checked);
                    }
                    if total > 0 && rate <= threshold {
                        return Ok((best.unwrap(), false));
                    }
                    if total == 0 { "빈 결과".to_owned() } else { format!("근거가 맞지 않아 버린 주장 {:.0}%", rate * 100.0) }
                }
            },
        };
        if attempt == 1 {
            break;
        }
        match step_up(run, &current) {
            Some(next) => {
                why = format!("{problem} → {} 등급으로 다시 읽음", skills::tier_label(&next.tier));
                current = next;
            }
            // Already the highest tier: a failed call is tried once more, a weak result is marked as it is.
            None if retry_same => why = format!("{problem} → 같은 모델로 다시 읽음"),
            None => break,
        }
    }
    match best {
        Some(checked) => {
            let low = checked.total > 0 && drop_rate(&checked) > threshold;
            Ok((checked, low))
        }
        None => bail!("{}묶음 중 {}번째 묶음을 다시 읽어도 읽지 못했어요", run["materials"][mi]["chunks"].as_array().map(Vec::len).unwrap_or(0), ki + 1),
    }
}
fn read_material(ctx: &Ctx, run: &mut Value, skill: &Skill, mi: usize, of: usize) -> Result<Read> {
    let path = run["materials"][mi]["immutable_path"].as_str().context("고정한 원본이 없습니다")?.to_owned();
    let extraction = knowledge::extract(ctx.root, Path::new(&path))?;
    let (max_pages, max_chars) = chunk_limits(&ctx.routing);
    let weights: Vec<(bool, usize)> = extraction.units.iter().map(|u| (!u.images.is_empty(), u.text.chars().count())).collect();
    let ranges = split(&weights, max_pages, max_chars);
    anyhow::ensure!(!ranges.is_empty(), "읽을 내용이 없는 자료예요");
    let planned: Vec<Value> = ranges.iter().map(|r| json!({"from":extraction.units[r.start].locator,"to":extraction.units[r.end - 1].locator,"units":r.len(),"attempts":[]})).collect();
    run["materials"][mi]["pages"] = json!(extraction.units.len());
    run["materials"][mi]["chunks"] = json!(planned);
    let title = run["materials"][mi]["title"].as_str().unwrap_or("자료").to_owned();
    let mut parts = Vec::new();
    let (mut low, mut total, mut dropped) = (false, 0, 0);
    for (ki, range) in ranges.iter().enumerate() {
        let label = if ranges.len() > 1 { format!("자료 읽는 중 {}/{of} ({}묶음 중 {}) · {title}", mi + 1, ranges.len(), ki + 1) } else { format!("자료 읽는 중 {}/{of} · {title}", mi + 1) };
        set_progress(ctx.root, run, label)?;
        let (checked, weak) = read_chunk(ctx, run, skill, mi, ki, &extraction, range.clone())?;
        low |= weak;
        total += checked.total;
        dropped += checked.dropped;
        parts.push(checked.analysis);
    }
    let merged = knowledge::check(merge_chunks(parts), &extraction)?;
    anyhow::ensure!(!merged.analysis.assertions.is_empty(), "근거가 있는 연결을 찾지 못했어요. 빈 결과는 반영하지 않아요.");
    Ok(Read { analysis: merged.analysis, low, total, dropped })
}
/// A step outside reading: call, and call once more a tier up (or again) when
/// the call fails or `accept` rejects the answer.
fn step(ctx: &Ctx, run: &mut Value, skill: &Skill, stage: &str, input: &Value, first: &Choice, base_attempt: usize, accept: &dyn Fn(&Value) -> Result<()>) -> Result<Value> {
    let mut current = first.clone();
    let mut why = String::new();
    for attempt in 0..2 {
        let (outcome, record) = call(ctx, skill, stage, &current, input, &[], base_attempt + attempt, &why);
        run[stage]["calls"].as_array_mut().context("단계 기록 없음")?.push(record);
        save(ctx.root, run)?;
        let problem = match outcome {
            Err(e) => {
                let message = format!("{e:#}");
                if ends_run(ctx.job, &message) || attempt == 1 {
                    return Err(e.context(format!("{} 단계가 실패했습니다", stage_label(stage))));
                }
                format!("호출 실패: {}", message.chars().take(200).collect::<String>())
            }
            Ok(result) => match accept(&result) {
                Ok(()) => return Ok(result),
                Err(e) if attempt == 1 => return Err(e.context(format!("{} 단계가 실패했습니다", stage_label(stage)))),
                Err(e) => format!("{e:#}"),
            },
        };
        match step_up(run, &current) {
            Some(next) => {
                why = format!("{problem} → {} 등급으로 다시 함", skills::tier_label(&next.tier));
                current = next;
            }
            None => why = format!("{problem} → 같은 모델로 다시 함"),
        }
    }
    unreachable!("the second attempt returns")
}
fn align(ctx: &Ctx, run: &mut Value, scratch: &rusqlite::Connection, skill: &Skill, revision: i64) -> Result<usize> {
    let w = run["workspace_id"].as_str().unwrap_or("").to_owned();
    let pairs = ontology::align_candidates(scratch, ctx.root, &w)?;
    run["align"] = json!({"pairs":pairs.len(),"calls":[],"merged":0});
    if pairs.is_empty() {
        run["align"]["reason"] = json!("이름이 겹치는 개념이 없었어요");
        return Ok(0);
    }
    if run["stages"]["align"]["below_min"] == true {
        run["align"]["reason"] = json!("이 계정의 모델 등급으로는 같은 개념인지 판단하기 어려워 합치지 않았어요");
        return Ok(0);
    }
    let first = choice(&run["stages"]["align"]);
    let size = ctx.routing["batch"]["align_pairs"].as_u64().unwrap_or(60).max(1) as usize;
    let mut same = Vec::new();
    let batches = pairs.chunks(size).count();
    for (n, batch) in pairs.chunks(size).enumerate() {
        set_progress(ctx.root, run, if batches > 1 { format!("같은 개념 맞추는 중 ({batches}묶음 중 {})", n + 1) } else { "같은 개념 맞추는 중".to_owned() })?;
        let strip = |v: &Value| json!({"name":v["name"],"aliases":v["aliases"],"definition":v["definition"],"evidence":v["evidence"]});
        let input = json!({"pairs":batch.iter().map(|p| json!({"pair":p["pair"],"a":strip(&p["a"]),"b":strip(&p["b"])})).collect::<Vec<_>>()});
        let result = step(ctx, run, skill, "align", &input, &first, 0, &|r| {
            anyhow::ensure!(r["decisions"].as_array().is_some_and(|d| !d.is_empty()), "빈 결과");
            Ok(())
        })?;
        for decision in result["decisions"].as_array().into_iter().flatten().filter(|d| d["verdict"] == "same") {
            if let Some(pair) = batch.iter().find(|p| p["pair"] == decision["pair"]) {
                same.push((
                    pair["a"]["id"].as_str().unwrap_or("").to_owned(),
                    pair["b"]["id"].as_str().unwrap_or("").to_owned(),
                    decision["name"].as_str().unwrap_or("").to_owned(),
                    decision["definition"].as_str().unwrap_or("").to_owned(),
                ));
            }
        }
    }
    let merged = ontology::apply_alignment(scratch, &w, &same, revision)?;
    run["align"]["merged"] = json!(merged);
    Ok(merged)
}
/// One pass of connect over every batch, with ids shortened for the provider.
fn connect_pass(ctx: &Ctx, run: &mut Value, skill: &Skill, view: &Value, c: &Choice, make_links: bool, base_attempt: usize) -> Result<Vec<Value>> {
    let concepts: Vec<&Value> = view["concepts"].as_array().into_iter().flatten().collect();
    let short: std::collections::HashMap<&str, String> = concepts.iter().enumerate().map(|(i, c)| (c["id"].as_str().unwrap_or(""), format!("c{}", i + 1))).collect();
    let size = ctx.routing["batch"]["connect_concepts"].as_u64().unwrap_or(300).max(1) as usize;
    let limits = json!({"related_per_concept":ctx.routing["checks"]["related_per_concept"],"links_per_concept":ctx.routing["checks"]["inferred_links_per_concept"]});
    let mut units: Vec<Value> = Vec::new();
    let mut outputs = Vec::new();
    let batches = concepts.chunks(size).count();
    for (n, batch) in concepts.chunks(size).enumerate() {
        set_progress(ctx.root, run, if batches > 1 { format!("개념 잇는 중 ({batches}묶음 중 {})", n + 1) } else { "개념 잇는 중".to_owned() })?;
        let inside: std::collections::HashMap<&str, &str> = batch.iter().map(|c| (short[c["id"].as_str().unwrap_or("")].as_str(), c["id"].as_str().unwrap_or(""))).collect();
        let known = |id: &Value| id.as_str().and_then(|id| short.get(id)).filter(|s| inside.contains_key(s.as_str()));
        let input = json!({
            "concepts":batch.iter().map(|c| json!({"id":short[c["id"].as_str().unwrap_or("")],"name":c["name"],"definition":c["definition"],"type":c["kind"]})).collect::<Vec<_>>(),
            "links":view["assertions"].as_array().into_iter().flatten().filter(|l| l["basis"] == "written")
                .filter_map(|l| Some(json!({"subject":known(&l["subject"])?,"object":known(&l["object"])?,"type":l["relation"]}))).collect::<Vec<_>>(),
            "units":units,"make_links":make_links,"limits":limits,
        });
        let result = step(ctx, run, skill, "connect", &input, c, base_attempt, &|r| {
            anyhow::ensure!(r["units"].as_array().is_some_and(|u| u.iter().any(|x| x["concepts"].as_array().is_some_and(|c| !c.is_empty()))), "단원이 없는 빈 결과");
            Ok(())
        })?;
        // Back to the subject's own ids; an id the provider made up stays unknown and is dropped by the checks.
        let long = |id: &Value| json!(id.as_str().and_then(|s| inside.get(s)).copied().unwrap_or(""));
        let mut mapped = json!({"units":[],"links":[]});
        for unit in result["units"].as_array().into_iter().flatten() {
            if !units.iter().any(|u| u["name"] == unit["name"]) {
                units.push(json!({"name":unit["name"],"description":unit["description"]}));
            }
            mapped["units"].as_array_mut().unwrap().push(json!({"name":unit["name"],"description":unit["description"],"concepts":unit["concepts"].as_array().into_iter().flatten().map(long).collect::<Vec<_>>()}));
        }
        if make_links {
            for link in result["links"].as_array().into_iter().flatten() {
                let mut link = link.clone();
                link["subject"] = long(&link["subject"]);
                link["object"] = long(&link["object"]);
                mapped["links"].as_array_mut().unwrap().push(link);
            }
        }
        outputs.push(mapped);
    }
    Ok(outputs)
}
fn connect(ctx: &Ctx, run: &mut Value, scratch: &rusqlite::Connection, skill: &Skill, review: Option<&Skill>) -> Result<()> {
    let w = run["workspace_id"].as_str().unwrap_or("").to_owned();
    let view = ontology::view(scratch, ctx.root, &w)?;
    run["connect"] = json!({"concepts":view["concepts"].as_array().map(Vec::len).unwrap_or(0),"calls":[]});
    let make_links = run["stages"]["connect"]["below_min"] != true;
    if !make_links {
        run["connect"]["note"] = json!("이 계정의 모델 등급으로는 점선 연결을 만들기 어려워 단원 묶기까지만 했어요");
    }
    let first = choice(&run["stages"]["connect"]);
    let related = ctx.routing["checks"]["related_per_concept"].as_u64().unwrap_or(3) as usize;
    let cap = ctx.routing["checks"]["inferred_links_per_concept"].as_u64().unwrap_or(8) as usize;
    let threshold = ctx.routing["checks"]["link_drop_rate"].as_f64().unwrap_or(0.3);
    let rate = |s: &ontology::Structure| if s.proposed == 0 { 0.0 } else { s.dropped as f64 / s.proposed as f64 };
    let mut structure = ontology::settle(&view, &connect_pass(ctx, run, skill, &view, &first, make_links, 0)?, related, cap);
    let mut keep = make_links;
    if make_links && rate(&structure) > threshold {
        let problem = format!("검증에서 버린 연결 {:.0}%", rate(&structure) * 100.0);
        match top(run, &first) {
            Some(best) => {
                run["connect"]["redo"] = json!(format!("{problem} → 가장 높은 등급으로 다시 이음"));
                let again = ontology::settle(&view, &connect_pass(ctx, run, skill, &view, &best, true, 10)?, related, cap);
                if rate(&again) <= rate(&structure) {
                    structure = again;
                }
            }
            None => run["connect"]["redo"] = json!(format!("{problem} · 이미 가장 높은 등급")),
        }
        if rate(&structure) > threshold {
            keep = false;
            run["connect"]["note"] = json!(format!("개념 잇기 결과에서 검증에 걸린 연결이 {:.0}%여서 이번 개정에는 점선 연결을 넣지 않았어요", rate(&structure) * 100.0));
        }
    }
    run["connect"]["proposed"] = json!(structure.proposed);
    run["connect"]["dropped"] = json!(structure.dropped);
    run["connect"]["units"] = json!(structure.units.len());
    if keep {
        if let Some(skill) = review {
            let removed = review_links(ctx, run, skill, &view, &mut structure)?;
            run["review"]["removed"] = json!(removed);
        }
    }
    run["connect"]["links"] = json!(if keep { structure.links.len() } else { 0 });
    ontology::write_structure(scratch, &w, &view, &structure, keep)
}
/// `꼼꼼하게`: drop the inferred links the definitions and evidence do not support.
fn review_links(ctx: &Ctx, run: &mut Value, skill: &Skill, view: &Value, structure: &mut ontology::Structure) -> Result<usize> {
    run["review"] = json!({"links":structure.links.len(),"calls":[],"removed":0});
    if structure.links.is_empty() {
        run["review"]["reason"] = json!("검수할 점선 연결이 없었어요");
        return Ok(0);
    }
    let first = choice(&run["stages"]["review"]);
    let describe = |id: &str| {
        let c = view["concepts"].as_array().into_iter().flatten().find(|c| c["id"] == id);
        let evidence = c.and_then(|c| c["learned"].as_array()).into_iter().flatten().filter_map(|e| e["quote"].as_str()).find(|q| !q.is_empty()).unwrap_or("");
        json!({"name":c.map(|c| c["name"].clone()),"definition":c.map(|c| c["definition"].clone()),"evidence":evidence.chars().take(300).collect::<String>()})
    };
    let size = ctx.routing["batch"]["review_links"].as_u64().unwrap_or(80).max(1) as usize;
    let mut unsupported = std::collections::HashSet::new();
    let indexed: Vec<(usize, &(String, String, String, String, String, bool))> = structure.links.iter().enumerate().collect();
    let batches = indexed.chunks(size).count();
    for (n, batch) in indexed.chunks(size).enumerate() {
        set_progress(ctx.root, run, if batches > 1 { format!("이은 연결 검수하는 중 ({batches}묶음 중 {})", n + 1) } else { "이은 연결 검수하는 중".to_owned() })?;
        let input = json!({"links":batch.iter().map(|(i, l)| json!({"link":format!("l{}", i + 1),"type":l.2,"reason":l.3,"subject":describe(&l.0),"object":describe(&l.1)})).collect::<Vec<_>>()});
        let result = step(ctx, run, skill, "review", &input, &first, 0, &|r| {
            anyhow::ensure!(r["reviews"].as_array().is_some_and(|d| !d.is_empty()), "빈 결과");
            Ok(())
        })?;
        for verdict in result["reviews"].as_array().into_iter().flatten().filter(|v| v["verdict"] == "unsupported") {
            if let Some(i) = verdict["link"].as_str().and_then(|l| l.strip_prefix('l')).and_then(|n| n.parse::<usize>().ok()) {
                unsupported.insert(i.wrapping_sub(1));
            }
        }
    }
    let before = structure.links.len();
    let mut index = 0;
    structure.links.retain(|_| {
        index += 1;
        !unsupported.contains(&(index - 1))
    });
    Ok(before - structure.links.len())
}
fn comparable(view: &Value) -> String {
    json!([view["concepts"], view["assertions"], view["units"], view["hidden"], view["sources"]]).to_string()
}
fn counts(view: &Value) -> (i64, i64) {
    (view["concepts"].as_array().map(Vec::len).unwrap_or(0) as i64, view["assertions"].as_array().map(Vec::len).unwrap_or(0) as i64)
}
fn pipeline(ctx: &Ctx, run: &mut Value) -> Result<Value> {
    let root = ctx.root;
    let w = run["workspace_id"].as_str().context("과목 없음")?.to_owned();
    let single = run["single"] == true;
    // A private copy of the map: the subject's own map is untouched until publish.
    let scratch_path = ctx.dir.join("scratch.sqlite");
    let (before, generation, guidance) = loop {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(ctx.dir.join(format!("scratch.sqlite{suffix}")));
        }
        let main = knowledge::db(root)?;
        let generation = knowledge::generation(&main, &w)?;
        main.execute("VACUUM INTO ?1", [scratch_path.to_string_lossy()])?;
        if knowledge::generation(&main, &w)? == generation {
            break (ontology::graph(&main, root, &w)?, generation, ontology::guidance(&main, &w)?);
        }
    };
    let scratch = knowledge::open(&scratch_path)?;
    let extract = skills::load(root, "extract-material", &guidance)?;
    let align_skill = skills::load(root, "align-concepts", &guidance)?;
    let connect_skill = skills::load(root, "connect-concepts", &guidance)?;
    let review_skill = skills::load(root, "review-links", "")?;
    let used: Vec<&Skill> = if single { vec![&extract] } else { vec![&extract, &align_skill, &connect_skill, &review_skill] };
    run["bundle_version"] = json!(skills::bundle_version()?);
    run["skills"] = Value::Object(used.iter().map(|s| (s.name.clone(), json!({"version":s.version,"fingerprint":s.fingerprint,"customized":s.customized}))).collect());
    run["guidance"] = json!(guidance);
    for stage in ["align", "connect", "review"] {
        run[stage] = Value::Null;
    }
    let rules = format!("bundle {} · extract-material {} · {}", run["bundle_version"].as_str().unwrap_or(""), extract.version, &extract.fingerprint[..12]);
    let of = run["materials"].as_array().map(Vec::len).unwrap_or(0);
    let mut read = Vec::new();
    for mi in 0..of {
        anyhow::ensure!(!stopped(ctx.job), STOPPED);
        let (id, hash) = (run["materials"][mi]["id"].as_str().unwrap_or("").to_owned(), run["materials"][mi]["hash"].as_str().unwrap_or("").to_owned());
        let current = crate::storage::handle(root, &json!({"op":"source.get","id":id})).ok();
        if current.as_ref().and_then(|c| c["hash"].as_str()) != Some(hash.as_str()) {
            run["materials"][mi]["state"] = json!("changed");
            save(root, run)?;
            continue;
        }
        run["materials"][mi]["state"] = json!("reading");
        match read_material(ctx, run, &extract, mi, of) {
            Ok(result) => {
                let quality = if result.low { "low" } else { "ok" };
                let model = run["materials"][mi]["chunks"].as_array().into_iter().flatten()
                    .filter_map(|c| c["attempts"].as_array().and_then(|a| a.last())).filter_map(|a| a["model"].as_str()).next_back().unwrap_or("").to_owned();
                let provider = run["stages"]["extract"]["provider"].as_str().unwrap_or(&ctx.provider);
                knowledge::apply_checked(&scratch, &knowledge::Material {
                    workspace: &w, id: &id, hash: &hash,
                    immutable_path: run["materials"][mi]["immutable_path"].as_str().unwrap_or(""),
                    title: run["materials"][mi]["title"].as_str().unwrap_or(""),
                    provider, model: &model, rules: &rules,
                }, &result.analysis)?;
                scratch.execute("INSERT OR REPLACE INTO ont_material_state VALUES(?1,?2,?3,?4,?5,?6,?7)", params![w, id, hash, quality, provider, model, ctx.run_id])?;
                let m = &mut run["materials"][mi];
                m["state"] = json!("done");
                m["quality"] = json!(quality);
                m["claims"] = json!(result.total);
                m["dropped"] = json!(result.dropped);
                m["kept"] = json!(result.analysis.assertions.len());
                read.push((id, hash));
            }
            Err(e) => {
                let message = format!("{e:#}");
                if ends_run(ctx.job, &message) {
                    return Err(e);
                }
                run["materials"][mi]["state"] = json!("failed");
                run["materials"][mi]["reason"] = json!(message);
            }
        }
        save(root, run)?;
    }
    let failed: Vec<Value> = run["materials"].as_array().into_iter().flatten().filter(|m| matches!(m["state"].as_str(), Some("failed" | "changed")))
        .map(|m| json!({"title":m["title"],"reason":if m["state"] == "changed" { json!("정리하는 동안 자료가 바뀌었어요") } else { m["reason"].clone() }})).collect();
    let low: Vec<Value> = run["materials"].as_array().into_iter().flatten().filter(|m| m["quality"] == "low").map(|m| m["title"].clone()).collect();
    if single {
        let reason = failed.first().and_then(|f| f["reason"].as_str()).unwrap_or("자료를 읽지 못했습니다").to_owned();
        anyhow::ensure!(!read.is_empty(), "{reason}");
        set_progress(root, run, "지도에 반영하는 중".to_owned())?;
        drop(scratch);
        ontology::publish(root, &scratch_path, &w, &read, generation)?;
        let kept = run["materials"][0]["kept"].as_u64().unwrap_or(0);
        return Ok(json!({"changed":true,"single":true,"materials_read":1,"low_quality":low,"failed":failed,"assertions":kept,"text":format!("연결 {kept}개를 지도에 반영했어요")}));
    }
    anyhow::ensure!(of == 0 || !read.is_empty(), "읽을 수 있는 자료가 없었어요. {}", failed.first().and_then(|f| f["reason"].as_str()).unwrap_or(""));
    let revision = ontology::latest_revision(&scratch, &w)?.unwrap_or(0) + 1;
    let merged = align(ctx, run, &scratch, &align_skill, revision)?;
    let review = (!run["stages"]["review"].is_null() && run["stages"]["review"]["below_min"] != true).then_some(&review_skill);
    connect(ctx, run, &scratch, &connect_skill, review)?;
    set_progress(root, run, "지도에 반영하는 중".to_owned())?;
    let after = ontology::view(&scratch, root, &w)?;
    if before["layer"] == "subject" && read.is_empty() && comparable(&before) == comparable(&after) {
        return Ok(json!({"changed":false,"materials_read":0,"failed":failed,"low_quality":low,"text":"바뀐 것이 없어요. 지도는 그대로예요."}));
    }
    let (concepts, links) = counts(&after);
    let (old_concepts, old_links) = counts(&before);
    let inferred = after["assertions"].as_array().into_iter().flatten().filter(|l| l["basis"] == "inferred").count();
    let units = after["units"].as_array().map(Vec::len).unwrap_or(0);
    let (more_concepts, more_links) = (concepts - old_concepts, links - old_links);
    let text = if more_concepts >= 0 && more_links >= 0 {
        format!("개념 {more_concepts}개와 연결 {more_links}개가 늘었고 {merged}개를 하나로 합쳤어요")
    } else {
        format!("개념 {concepts}개, 연결 {links}개로 정리했고 {merged}개를 하나로 합쳤어요")
    };
    let summary = json!({
        "concepts":concepts,"links":links,"inferred":inferred,"units":units,"merged":merged,"added_concepts":more_concepts,"added_links":more_links,
        "provider":ctx.provider,"mode":run["mode"],"bundle_version":run["bundle_version"],"skills":run["skills"],"stages":run["stages"],"text":text,
    });
    scratch.execute("INSERT INTO ont_revisions VALUES(?1,?2,?3,?4,?5)", params![w, revision, now(), ctx.run_id, summary.to_string()])?;
    drop(scratch);
    ontology::publish(root, &scratch_path, &w, &read, generation)?;
    Ok(json!({
        "changed":true,"revision":revision,"concepts":concepts,"links":links,"inferred":inferred,"units":units,"merged":merged,
        "added_concepts":more_concepts,"added_links":more_links,"materials_read":read.len(),"failed":failed,"low_quality":low,"text":text,
    }))
}
pub(crate) fn work(root: &Path, job: &Path, req: &Value) -> Result<Value> {
    let id = req["run_id"].as_str().context("실행 ID 없음")?;
    let mut run = load(root, id)?;
    let ctx = Ctx {
        root,
        job,
        dir: run_dir(root, id)?,
        provider: run["provider"].as_str().context("공급자 없음")?.to_owned(),
        run_id: id.to_owned(),
        routing: skills::routing(root)?,
    };
    run["state"] = json!("running");
    run["job_id"] = json!(job.file_name().and_then(|name| name.to_str()).unwrap_or(""));
    run["error"] = Value::Null;
    run["result"] = Value::Null;
    run["seen"] = json!(false);
    save(root, &mut run)?;
    let outcome = pipeline(&ctx, &mut run);
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(ctx.dir.join(format!("scratch.sqlite{suffix}")));
    }
    run["finished_at"] = json!(now());
    match outcome {
        Ok(result) => {
            run["state"] = json!("succeeded");
            run["result"] = result.clone();
            run["progress"]["label"] = json!("정리 완료");
            save(root, &mut run)?;
            Ok(result)
        }
        Err(e) => {
            let message = format!("{e:#}");
            run["state"] = json!(if stopped(job) { "paused" } else if providers::is_limit(&message) { "limit" } else { "failed" });
            run["error"] = json!(if stopped(job) { "멈췄어요. 읽은 부분은 보관해 두었어요.".to_owned() } else { message });
            save(root, &mut run)?;
            Err(anyhow!("{e:#}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(pages: usize) -> Vec<usize> {
        split(&vec![(true, 100); pages], 24, 60000).iter().map(|r| r.len()).collect()
    }
    #[test]
    fn pages_split_evenly_under_the_limit() {
        assert_eq!(sizes(15), [15]);
        assert_eq!(sizes(24), [24]);
        assert_eq!(sizes(27), [14, 13]);
        assert_eq!(sizes(47), [24, 23]);
        assert_eq!(sizes(51), [17, 17, 17]);
        assert!(split(&[], 24, 60000).is_empty());
        // Text without page images is limited by length only.
        assert_eq!(split(&vec![(false, 100); 200], 24, 60000).len(), 1);
        assert_eq!(split(&vec![(false, 1000); 200], 24, 60000).len(), 4);
    }
    #[test]
    fn chunks_of_one_material_join_by_name() {
        use knowledge::{Assertion, Concept, Evidence, Locator, Relation};
        let concept = |id: &str, name: &str, aliases: &[&str]| Concept { id: id.into(), name: name.into(), aliases: aliases.iter().map(|a| a.to_string()).collect(), definition: format!("{name} 정의"), kind: None };
        let relation = |id: &str| Relation { id: id.into(), label: "포함".into(), definition: "포함한다".into(), directed: true, aliases: vec![], kind: Some("part_of".into()) };
        let claim = |s: &str, o: &str| Assertion { subject: s.into(), relation: "r1".into(), object: o.into(), condition: String::new(), negated: false, evidence: vec![Evidence { locator: Locator { kind: "page".into(), index: 1 }, quote: "q".into(), image_path: None, region: None }] };
        let first = Analysis { concepts: vec![concept("c1", "JWT", &["JSON Web Token"]), concept("c2", "토큰", &[])], relations: vec![relation("r1")], assertions: vec![claim("c1", "c2")] };
        let second = Analysis { concepts: vec![concept("c1", "json web token", &[]), concept("c2", "서명", &[])], relations: vec![relation("r1")], assertions: vec![claim("c2", "c1")] };
        let merged = merge_chunks(vec![first, second]);
        assert_eq!(merged.concepts.len(), 3);
        assert_eq!(merged.relations.len(), 1);
        assert_eq!(merged.assertions.len(), 2);
        assert_eq!(merged.assertions[1].object, merged.assertions[0].subject);
    }
}
