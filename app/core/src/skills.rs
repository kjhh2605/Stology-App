//! App-owned skill bundle: bundled defaults, an optional user copy, fingerprints,
//! routing values, tier→model rules, AI settings and the usage log.
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const SKILLS: [&str; 5] = [
    "extract-material",
    "align-concepts",
    "connect-concepts",
    "review-links",
    "distill-chat",
];
/// Lowest to highest.
pub const TIERS: [&str; 3] = ["fast", "balanced", "precise"];
pub const STAGES: [&str; 5] = ["extract", "align", "connect", "review", "distill"];
pub const MODES: [&str; 3] = ["auto", "fast", "thorough"];

pub fn stage_skill(stage: &str) -> &'static str {
    match stage {
        "extract" => "extract-material",
        "align" => "align-concepts",
        "connect" => "connect-concepts",
        "review" => "review-links",
        _ => "distill-chat",
    }
}
pub fn tier_rank(tier: &str) -> usize {
    TIERS.iter().position(|t| *t == tier).unwrap_or(0)
}

/// Bundled defaults: the app's `Resources/ai`, or the source tree in development.
fn bundle_dir() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(p) = std::env::var_os("STOLOGY_AI_DIR") {
        candidates.push(PathBuf::from(p));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join("../Resources/ai"));
    }
    candidates.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/ai")));
    candidates
        .into_iter()
        .find(|p| p.join("manifest.json").is_file())
        .context("앱에 내장된 AI 정리 스킬을 찾을 수 없습니다. 앱을 다시 설치해 주세요.")
}
pub fn user_dir(root: &Path) -> PathBuf {
    root.join("ai/skills")
}
fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("{}을(를) 읽을 수 없습니다", path.display()))?)
        .with_context(|| format!("{}의 JSON 형식이 올바르지 않습니다", path.display()))
}
pub(crate) fn write_json(path: &Path, v: &Value) -> Result<()> {
    fs::create_dir_all(path.parent().context("잘못된 경로")?)?;
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn manifest() -> Result<Value> {
    read_json(&bundle_dir()?.join("manifest.json"))
}
pub fn bundle_version() -> Result<String> {
    Ok(manifest()?["bundle_version"].as_str().context("묶음 버전 없음")?.to_owned())
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub version: String,
    /// Instruction text the provider receives: the user copy when present.
    pub text: String,
    pub schema: Value,
    pub fingerprint: String,
    pub customized: bool,
    pub min_tier: String,
}
/// `guidance` is the subject's own instruction appended after the skill body.
pub fn load(root: &Path, name: &str, guidance: &str) -> Result<Skill> {
    anyhow::ensure!(SKILLS.contains(&name), "알 수 없는 스킬: {name}");
    let dir = bundle_dir()?;
    let entry = manifest()?["skills"][name].clone();
    let version = entry["version"].as_str().context("스킬 버전 없음")?.to_owned();
    let schema = read_json(&dir.join(entry["schema"].as_str().context("스킬 스키마 경로 없음")?))?;
    let bundled = fs::read_to_string(dir.join("skills").join(name).join("SKILL.md"))
        .with_context(|| format!("내장 스킬 {name}을(를) 읽을 수 없습니다"))?;
    let copy = user_dir(root).join(name).join("SKILL.md");
    let (mut text, customized) = match fs::read_to_string(&copy) {
        Ok(user) if user.trim().is_empty() => bail!("고친 스킬 {name}이(가) 비어 있습니다. 내용을 채우거나 기본값으로 되돌려 주세요."),
        Ok(user) => {
            let changed = user != bundled;
            (user, changed)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (bundled, false),
        Err(e) => return Err(anyhow!(e).context(format!("고친 스킬 {name}을(를) 읽을 수 없습니다"))),
    };
    if !guidance.trim().is_empty() && ["extract-material", "align-concepts", "connect-concepts"].contains(&name) {
        text.push_str("\n\n## 이 과목의 정리 지침 (사용자가 적음)\n\n");
        text.push_str(guidance.trim());
        text.push('\n');
    }
    let fingerprint = digest(format!("{name}\n{version}\n{text}\n{schema}").as_bytes());
    Ok(Skill {
        name: name.to_owned(),
        version,
        text,
        schema,
        fingerprint,
        customized,
        min_tier: entry["min_tier"].as_str().unwrap_or("fast").to_owned(),
    })
}
fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                merge(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, o) => *b = o.clone(),
    }
}
/// Bundled routing values with the user's copy laid over them.
pub fn routing(root: &Path) -> Result<Value> {
    let mut v = read_json(&bundle_dir()?.join("routing.json"))?;
    let copy = user_dir(root).join("routing.json");
    if copy.is_file() {
        let user = read_json(&copy).context("고친 routing.json을 읽을 수 없습니다. 값을 고치거나 기본값으로 되돌려 주세요.")?;
        anyhow::ensure!(user.is_object(), "고친 routing.json은 JSON 객체여야 합니다");
        merge(&mut v, &user);
    }
    let pages = v["chunk"]["max_pages"].as_u64().unwrap_or(0);
    anyhow::ensure!((1..=32).contains(&pages), "routing.json의 chunk.max_pages는 1~32여야 합니다 (호출당 이미지 상한 32장)");
    Ok(v)
}
fn routing_customized(root: &Path) -> bool {
    let copy = user_dir(root).join("routing.json");
    match (read_json(&copy), bundle_dir().and_then(|d| read_json(&d.join("routing.json")))) {
        (Ok(user), Ok(bundled)) => user != bundled,
        (Err(_), _) => copy.is_file(),
        _ => false,
    }
}
/// Names of skills whose user copy differs from the bundled text, plus `routing` when it differs.
pub fn customized(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = SKILLS
        .iter()
        .filter(|name| load(root, name, "").map(|s| s.customized).unwrap_or(true) )
        .map(|s| s.to_string())
        .collect();
    if routing_customized(root) {
        names.push("routing".into());
    }
    names
}

// ── Settings ─────────────────────────────────────────────────────────────
fn settings_path(root: &Path) -> PathBuf {
    root.join("ai/settings.json")
}
pub fn settings(root: &Path) -> Value {
    let mut v = json!({"organize_mode":"auto","distill_mode":null,"background":"normal","organize_provider":null,"stage_models":{}});
    if let Ok(saved) = read_json(&settings_path(root)) {
        merge(&mut v, &saved);
    }
    v
}
fn set_settings(root: &Path, req: &Value) -> Result<Value> {
    let mut v = settings(root);
    if let Some(mode) = req.get("organize_mode") {
        anyhow::ensure!(mode.as_str().is_some_and(|m| MODES.contains(&m)), "잘못된 과목 정리 방식");
        v["organize_mode"] = mode.clone();
    }
    if let Some(mode) = req.get("distill_mode") {
        anyhow::ensure!(mode.is_null() || mode.as_str().is_some_and(|m| ["off", "ask", "auto"].contains(&m)), "잘못된 대화 정리 방식");
        v["distill_mode"] = mode.clone();
    }
    if let Some(amount) = req.get("background") {
        anyhow::ensure!(amount.as_str().is_some_and(|m| ["low", "normal", "high"].contains(&m)), "잘못된 뒤에서 쓰는 양");
        v["background"] = amount.clone();
    }
    if let Some(provider) = req.get("organize_provider") {
        anyhow::ensure!(provider.as_str().is_some_and(|p| ["codex", "claude", "antigravity"].contains(&p)), "지원하지 않는 공급자");
        v["organize_provider"] = provider.clone();
    }
    // {"provider":"codex","stage":"extract","model":"gpt-…","effort":"high"}; an empty model clears the override.
    if let Some(choice) = req.get("stage_model") {
        let provider = choice["provider"].as_str().filter(|p| ["codex", "claude", "antigravity"].contains(p)).context("공급자 없음")?;
        let stage = choice["stage"].as_str().filter(|s| STAGES.contains(s)).context("잘못된 단계")?;
        let model = choice["model"].as_str().unwrap_or("");
        anyhow::ensure!(model.len() <= 128 && model.chars().all(|c| c.is_ascii_alphanumeric() || "-_.[]".contains(c)), "잘못된 모델 ID");
        if !v["stage_models"][provider].is_object() {
            v["stage_models"][provider] = json!({});
        }
        let stages = v["stage_models"][provider].as_object_mut().unwrap();
        if model.is_empty() {
            stages.remove(stage);
        } else {
            stages.insert(stage.into(), json!({"model":model,"effort":choice["effort"].as_str().unwrap_or("")}));
        }
    }
    write_json(&settings_path(root), &v)?;
    Ok(v)
}

// ── Tier → model ─────────────────────────────────────────────────────────
fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}
/// Version numbers in a model ID, ignoring date stamps, so newer versions sort higher.
fn version_key(id: &str) -> Vec<u64> {
    tokens(id)
        .iter()
        .flat_map(|t| t.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(str::to_owned).collect::<Vec<_>>())
        .filter_map(|t| t.parse::<u64>().ok())
        .filter(|n| *n < 10_000)
        .collect()
}
/// Catalog entries a tier may pick: exact versions only, never the provider's moving aliases.
fn selectable(catalog: &[Value]) -> Vec<&Value> {
    catalog
        .iter()
        .filter(|m| {
            let id = m["id"].as_str().unwrap_or("");
            !id.is_empty() && m["resolved_model"].as_str().is_none_or(|resolved| resolved == id)
        })
        .collect()
}
fn pick<'a>(rule: &Value, catalog: &[&'a Value]) -> Option<&'a Value> {
    for prefer in rule["prefer"].as_array().into_iter().flatten() {
        let mut matches: Vec<&Value> = catalog
            .iter()
            .copied()
            .filter(|m| {
                if prefer["default"] == true {
                    return m["default"] == true;
                }
                let Some(family) = prefer["family"].as_str() else { return false };
                tokens(m["id"].as_str().unwrap_or("")).iter().chain(tokens(m["label"].as_str().unwrap_or("")).iter()).any(|t| t == family)
            })
            .collect();
        // Stable: among equal versions the catalog order wins.
        matches.sort_by(|a, b| version_key(b["id"].as_str().unwrap_or("")).cmp(&version_key(a["id"].as_str().unwrap_or(""))));
        if let Some(found) = matches.first() {
            return Some(found);
        }
    }
    None
}
fn effort_for(rule: &Value, model: &Value) -> String {
    let supported: Vec<&str> = model["efforts"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    rule["effort"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|e| supported.contains(e))
        .unwrap_or("")
        .to_owned()
}
/// `{tier: {"model","effort","label"} | null}` for the models this account lists.
pub fn tier_models(provider: &str, catalog: &[Value]) -> Result<Value> {
    anyhow::ensure!(["codex", "claude", "antigravity"].contains(&provider), "지원하지 않는 공급자");
    let rules = read_json(&bundle_dir()?.join("providers").join(format!("{provider}.json")))?;
    let list = selectable(catalog);
    let mut out = Map::new();
    for tier in TIERS {
        let rule = &rules["tiers"][tier];
        out.insert(
            tier.into(),
            pick(rule, &list)
                .map(|m| json!({"model":m["id"],"effort":effort_for(rule, m),"label":m["label"]}))
                .unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(out))
}
/// The tier itself, or the nearest lower tier the account can fill.
fn at_or_below<'a>(tiers: &'a Value, tier: &str) -> Option<(&'static str, &'a Value)> {
    TIERS[..=tier_rank(tier)].iter().rev().find_map(|t| (!tiers[*t].is_null()).then(|| (*t, &tiers[*t])))
}
/// One tier above `tier` with a different model, if the account has one.
pub fn tier_above(tiers: &Value, tier: &str, model: &str) -> Option<(&'static str, Value)> {
    TIERS[tier_rank(tier) + 1..]
        .iter()
        .find_map(|t| (!tiers[*t].is_null() && tiers[*t]["model"] != model).then(|| (*t, tiers[*t].clone())))
}
pub fn highest(tiers: &Value) -> Option<(&'static str, Value)> {
    TIERS.iter().rev().find_map(|t| (!tiers[*t].is_null()).then(|| (*t, tiers[*t].clone())))
}
/// What each stage of a run will use: requested tier, the tier and model actually
/// available, and a note when the account forced a lower tier.
pub fn stage_plan(root: &Path, provider: &str, mode: &str, catalog: &[Value]) -> Result<Value> {
    anyhow::ensure!(MODES.contains(&mode), "잘못된 정리 방식");
    let routing = routing(root)?;
    let tiers = tier_models(provider, catalog)?;
    let overrides = settings(root)["stage_models"][provider].clone();
    let manifest = manifest()?;
    let mut stages = Map::new();
    for stage in STAGES {
        let requested = routing["modes"][mode][stage].as_str();
        let Some(requested) = requested else {
            stages.insert(stage.into(), Value::Null);
            continue;
        };
        anyhow::ensure!(TIERS.contains(&requested), "routing.json의 modes.{mode}.{stage} 값이 올바르지 않습니다");
        let min = manifest["skills"][stage_skill(stage)]["min_tier"].as_str().unwrap_or("fast");
        let chosen = overrides[stage]["model"].as_str().filter(|m| !m.is_empty());
        let entry = if let Some(model) = chosen {
            anyhow::ensure!(catalog.iter().any(|m| m["id"] == model), "직접 고른 {stage} 모델 {model}이(가) 현재 계정의 목록에 없습니다. AI 정리 설정에서 다시 골라 주세요.");
            json!({"requested":requested,"tier":"custom","model":model,"effort":overrides[stage]["effort"].as_str().unwrap_or(""),"note":"직접 고른 모델","below_min":false})
        } else {
            let (tier, model) = at_or_below(&tiers, requested)
                .or_else(|| highest(&tiers).map(|(t, _)| (t, &tiers[t])))
                .with_context(|| format!("{provider} 계정의 모델 목록에서 쓸 수 있는 모델을 찾지 못했습니다"))?;
            let note = if tier == requested { String::new() } else { format!("계정에 {} 등급 모델이 없어 {} 등급으로 진행해요", tier_label(requested), tier_label(tier)) };
            json!({"requested":requested,"tier":tier,"model":model["model"],"effort":model["effort"],"note":note,"below_min":tier_rank(tier) < tier_rank(min)})
        };
        stages.insert(stage.into(), entry);
    }
    Ok(json!({"provider":provider,"mode":mode,"tiers":tiers,"stages":stages}))
}
pub fn tier_label(tier: &str) -> &'static str {
    match tier {
        "fast" => "빠름",
        "balanced" => "균형",
        "precise" => "정밀",
        _ => "직접 선택",
    }
}

// ── Usage log ────────────────────────────────────────────────────────────
/// One line per provider call. `lane` is `organize`, `background` or `manual`.
pub fn log_usage(root: &Path, lane: &str, skill: &str, provider: &str, model: &str, run: &str) -> Result<()> {
    fs::create_dir_all(root.join("ai"))?;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(root.join("ai/usage.jsonl"))?;
    writeln!(f, "{}", json!({"at":chrono::Local::now().to_rfc3339(),"lane":lane,"skill":skill,"provider":provider,"model":model,"run":run}))?;
    Ok(())
}
fn usage_lines(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("ai/usage.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
/// Background calls made today (local time).
pub fn background_today(root: &Path) -> usize {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    usage_lines(root)
        .iter()
        .filter(|v| v["lane"] == "background" && v["at"].as_str().is_some_and(|at| at.starts_with(&today)))
        .count()
}
pub fn background_cap(root: &Path) -> Result<usize> {
    let amount = settings(root)["background"].as_str().unwrap_or("normal").to_owned();
    Ok(routing(root)?["background_daily"][amount.as_str()].as_u64().unwrap_or(20) as usize)
}
fn usage_summary(root: &Path) -> Value {
    let since = chrono::Local::now() - chrono::Duration::days(7);
    let recent: Vec<Value> = usage_lines(root)
        .into_iter()
        .filter(|v| v["at"].as_str().and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok()).is_some_and(|at| at >= since))
        .collect();
    let runs: std::collections::HashSet<&str> = recent.iter().filter(|v| v["lane"] == "organize").filter_map(|v| v["run"].as_str()).collect();
    json!({
        "organize_runs": runs.len(),
        "organize_calls": recent.iter().filter(|v| v["lane"] == "organize").count(),
        "background_calls": recent.iter().filter(|v| v["lane"] == "background").count(),
        "background_today": background_today(root),
    })
}

// ── User copy ────────────────────────────────────────────────────────────
/// Copy the bundled skills beside any existing user edits and return the folder to open.
fn open_copy(root: &Path) -> Result<Value> {
    let bundle = bundle_dir()?;
    let dir = user_dir(root);
    for name in SKILLS {
        let target = dir.join(name).join("SKILL.md");
        if !target.exists() {
            fs::create_dir_all(target.parent().unwrap())?;
            fs::copy(bundle.join("skills").join(name).join("SKILL.md"), &target)?;
        }
    }
    let routing = dir.join("routing.json");
    if !routing.exists() {
        fs::copy(bundle.join("routing.json"), routing)?;
    }
    Ok(json!({"path":dir}))
}
/// Return to the bundled skills. The user's edits move aside instead of being deleted.
fn reset_copy(root: &Path) -> Result<Value> {
    let dir = user_dir(root);
    if !dir.exists() {
        return Ok(json!({"reset":false}));
    }
    let backup = root.join("ai").join(format!("skills-이전-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    fs::rename(&dir, &backup)?;
    Ok(json!({"reset":true,"backup":backup}))
}
fn status(root: &Path) -> Result<Value> {
    let mut skills = Vec::new();
    for name in SKILLS {
        skills.push(match load(root, name, "") {
            Ok(s) => json!({"name":name,"version":s.version,"fingerprint":s.fingerprint,"customized":s.customized,"min_tier":s.min_tier}),
            Err(e) => json!({"name":name,"error":format!("{e:#}"),"customized":true}),
        });
    }
    Ok(json!({
        "bundle_version": bundle_version()?,
        "bundle_path": bundle_dir()?,
        "user_path": user_dir(root),
        "user_copy": user_dir(root).exists(),
        "customized": customized(root),
        "routing_error": routing(root).err().map(|e| format!("{e:#}")),
        "skills": skills,
    }))
}
pub fn handle(root: &Path, req: &Value) -> Result<Value> {
    match req["op"].as_str().unwrap_or("") {
        "ai.skills.status" => status(root),
        "ai.skills.open" => open_copy(root),
        "ai.skills.reset" => reset_copy(root),
        "ai.settings.get" => Ok(json!({"settings":settings(root),"usage":usage_summary(root),"routing":routing(root).unwrap_or(Value::Null)})),
        "ai.settings.set" => Ok(json!({"settings":set_settings(root, req)?})),
        "ai.tiers" => {
            let provider = req["provider"].as_str().context("공급자 없음")?;
            let catalog = crate::chat::handle(root, &json!({"op":"chat.models","provider":provider}))?;
            let list = catalog["models"].as_array().cloned().unwrap_or_default();
            let mode = req["mode"].as_str().map(str::to_owned).unwrap_or_else(|| settings(root)["organize_mode"].as_str().unwrap_or("auto").to_owned());
            let mut plan = stage_plan(root, provider, &mode, &list)?;
            plan["models"] = json!(list);
            Ok(plan)
        }
        op => bail!("지원하지 않는 AI 정리 명령: {op}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn codex_catalog() -> Vec<Value> {
        vec![
            json!({"id":"gpt-6.1-sol","label":"GPT-6.1-Sol","default":true,"efforts":["low","medium","high"]}),
            json!({"id":"gpt-6.1-terra","label":"GPT-6.1 Terra","efforts":["low","medium","high"]}),
            json!({"id":"gpt-6-sol","label":"GPT-6-Sol","efforts":["low","medium","high"]}),
            json!({"id":"gpt-6-luna","label":"GPT-6-Luna","efforts":["low","medium"]}),
            json!({"id":"gpt-5.6-luna","label":"GPT-5.6-Luna","efforts":["low","medium"]}),
        ]
    }
    #[test]
    fn user_copy_replaces_bundled_skill_and_reset_restores_it() {
        let d = tempfile::tempdir().unwrap();
        let bundled = load(d.path(), "extract-material", "").unwrap();
        assert!(!bundled.customized);
        let dir = open_copy(d.path()).unwrap()["path"].as_str().unwrap().to_owned();
        // An untouched copy is still the bundled skill.
        assert_eq!(load(d.path(), "extract-material", "").unwrap().fingerprint, bundled.fingerprint);
        assert!(customized(d.path()).is_empty());
        fs::write(Path::new(&dir).join("extract-material/SKILL.md"), "영어 용어를 대표 이름으로 쓴다.").unwrap();
        let edited = load(d.path(), "extract-material", "").unwrap();
        assert!(edited.customized);
        assert_eq!(edited.text, "영어 용어를 대표 이름으로 쓴다.");
        assert_ne!(edited.fingerprint, bundled.fingerprint);
        assert_eq!(customized(d.path()), vec!["extract-material"]);
        assert_eq!(reset_copy(d.path()).unwrap()["reset"], true);
        let restored = load(d.path(), "extract-material", "").unwrap();
        assert_eq!(restored.fingerprint, bundled.fingerprint);
        assert!(!restored.customized);
    }
    #[test]
    fn subject_guidance_changes_the_fingerprint_only_where_it_applies() {
        let d = tempfile::tempdir().unwrap();
        let plain = load(d.path(), "connect-concepts", "").unwrap();
        let guided = load(d.path(), "connect-concepts", "영어 용어를 대표 이름으로").unwrap();
        assert!(guided.text.ends_with("영어 용어를 대표 이름으로\n"));
        assert_ne!(plain.fingerprint, guided.fingerprint);
        assert_eq!(load(d.path(), "distill-chat", "").unwrap().fingerprint, load(d.path(), "distill-chat", "지침").unwrap().fingerprint);
    }
    #[test]
    fn broken_user_routing_fails_instead_of_using_wrong_values() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(routing(d.path()).unwrap()["chunk"]["max_pages"], 24);
        fs::create_dir_all(user_dir(d.path())).unwrap();
        fs::write(user_dir(d.path()).join("routing.json"), r#"{"chunk":{"max_pages":12}}"#).unwrap();
        let v = routing(d.path()).unwrap();
        assert_eq!(v["chunk"]["max_pages"], 12);
        assert_eq!(v["chunk"]["max_chars"], 60000);
        fs::write(user_dir(d.path()).join("routing.json"), r#"{"chunk":{"max_pages":40}}"#).unwrap();
        assert!(routing(d.path()).is_err());
        fs::write(user_dir(d.path()).join("routing.json"), "{").unwrap();
        assert!(routing(d.path()).is_err());
    }
    #[test]
    fn tiers_pick_newest_family_member_and_fall_to_the_nearest_lower_tier() {
        let d = tempfile::tempdir().unwrap();
        let tiers = tier_models("codex", &codex_catalog()).unwrap();
        assert_eq!(tiers["precise"]["model"], "gpt-6.1-sol");
        assert_eq!(tiers["precise"]["effort"], "high");
        assert_eq!(tiers["balanced"]["model"], "gpt-6.1-terra");
        assert_eq!(tiers["fast"]["model"], "gpt-6-luna");
        let auto = stage_plan(d.path(), "codex", "auto", &codex_catalog()).unwrap();
        assert_eq!(auto["stages"]["extract"]["tier"], "balanced");
        assert_eq!(auto["stages"]["connect"]["model"], "gpt-6.1-sol");
        assert!(auto["stages"]["review"].is_null());
        let thorough = stage_plan(d.path(), "codex", "thorough", &codex_catalog()).unwrap();
        assert_eq!(thorough["stages"]["extract"]["tier"], "precise");
        assert_eq!(thorough["stages"]["review"]["tier"], "precise");
        // An account with only small Claude models: every stage drops, and aligning is below its floor.
        let haiku = vec![json!({"id":"haiku","label":"Haiku · 자동 버전","resolved_model":"claude-haiku-5-5","efforts":["low"]}),
            json!({"id":"claude-haiku-5-5","label":"Haiku 5.5","resolved_model":"claude-haiku-5-5","efforts":["low"]}),
            json!({"id":"claude-haiku-4-5-20251001","label":"Haiku 4.5","resolved_model":"claude-haiku-4-5-20251001","efforts":[]})];
        let plan = stage_plan(d.path(), "claude", "auto", &haiku).unwrap();
        assert_eq!(plan["stages"]["align"]["model"], "claude-haiku-5-5");
        assert_eq!(plan["stages"]["align"]["tier"], "fast");
        assert_eq!(plan["stages"]["align"]["below_min"], true);
        assert!(plan["stages"]["align"]["note"].as_str().unwrap().contains("정밀"));
        assert_eq!(plan["stages"]["extract"]["below_min"], false);
        assert!(tier_above(&plan["tiers"], "fast", "claude-haiku-5-5").is_none());
        assert_eq!(tier_above(&tiers, "balanced", "gpt-6.1-terra").unwrap().0, "precise");
    }
    #[test]
    fn chosen_stage_model_overrides_the_tier() {
        let d = tempfile::tempdir().unwrap();
        set_settings(d.path(), &json!({"stage_model":{"provider":"codex","stage":"extract","model":"gpt-6-sol","effort":"low"}})).unwrap();
        let plan = stage_plan(d.path(), "codex", "auto", &codex_catalog()).unwrap();
        assert_eq!(plan["stages"]["extract"]["model"], "gpt-6-sol");
        assert_eq!(plan["stages"]["extract"]["tier"], "custom");
        set_settings(d.path(), &json!({"stage_model":{"provider":"codex","stage":"extract","model":""}})).unwrap();
        assert_eq!(stage_plan(d.path(), "codex", "auto", &codex_catalog()).unwrap()["stages"]["extract"]["model"], "gpt-6.1-terra");
        assert!(set_settings(d.path(), &json!({"organize_mode":"everything"})).is_err());
    }
    #[test]
    fn background_calls_count_against_todays_cap() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(background_cap(d.path()).unwrap(), 20);
        set_settings(d.path(), &json!({"background":"low"})).unwrap();
        assert_eq!(background_cap(d.path()).unwrap(), 5);
        log_usage(d.path(), "background", "distill-chat", "codex", "m", "r1").unwrap();
        log_usage(d.path(), "organize", "extract-material", "codex", "m", "r2").unwrap();
        log_usage(d.path(), "organize", "connect-concepts", "codex", "m", "r2").unwrap();
        assert_eq!(background_today(d.path()), 1);
        let usage = usage_summary(d.path());
        assert_eq!(usage["organize_runs"], 1);
        assert_eq!(usage["organize_calls"], 2);
    }
}
