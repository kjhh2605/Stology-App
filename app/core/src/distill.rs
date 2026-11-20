//! Notes drawn from study-assistant conversations. The app's own records decide which
//! turns deserve a note; one batched call drafts them; nothing enters the materials
//! until the user registers a suggestion.
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use crate::{chat, providers, skills};

/// Registered notes live here, inside the subject.
pub const FOLDER: &str = "학습도우미 정리";

pub(crate) fn schema(c: &Connection) -> Result<()> {
    c.execute_batch("
CREATE TABLE IF NOT EXISTS question_groups(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,key TEXT NOT NULL,question TEXT NOT NULL,count INTEGER NOT NULL DEFAULT 0,settled INTEGER NOT NULL DEFAULT 0,updated TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS suggestions(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,provider TEXT NOT NULL,model TEXT NOT NULL DEFAULT '',skill TEXT NOT NULL DEFAULT '',state TEXT NOT NULL,title TEXT NOT NULL,body TEXT NOT NULL DEFAULT '',kind TEXT NOT NULL DEFAULT '',concepts TEXT NOT NULL DEFAULT '[]',turns TEXT NOT NULL DEFAULT '[]',snapshot TEXT NOT NULL DEFAULT '[]',question_group TEXT,repeats INTEGER NOT NULL DEFAULT 1,signal TEXT NOT NULL DEFAULT '',automatic INTEGER NOT NULL DEFAULT 0,seen INTEGER NOT NULL DEFAULT 0,document_id TEXT,created TEXT NOT NULL,updated TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS distill_batches(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,provider TEXT NOT NULL,state TEXT NOT NULL,automatic INTEGER NOT NULL,snapshot TEXT NOT NULL,job TEXT,error TEXT NOT NULL DEFAULT '',created TEXT NOT NULL,updated TEXT NOT NULL);")?;
    let columns: Vec<String> = c.prepare("PRAGMA table_info(turns)")?.query_map([], |r| r.get(1))?.collect::<rusqlite::Result<_>>()?;
    for (column, definition) in [("mark", "TEXT NOT NULL DEFAULT ''"), ("distill", "TEXT NOT NULL DEFAULT ''"), ("copied", "INTEGER NOT NULL DEFAULT 0"), ("question_group", "TEXT")] {
        if !columns.iter().any(|name| name == column) {
            c.execute(&format!("ALTER TABLE turns ADD COLUMN {column} {definition}"), [])?;
        }
    }
    Ok(())
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
/// Letters and digits only, lower-cased: what two questions are compared by.
fn key(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).take(300).collect()
}
/// How alike two question keys are, letter by letter: 1.0 is identical.
pub(crate) fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0.;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut left = i + 1;
        let mut diagonal = i;
        for (j, y) in b.iter().enumerate() {
            let cost = if x == y { diagonal } else { diagonal.min(left).min(row[j + 1]) + 1 };
            diagonal = row[j + 1];
            row[j + 1] = cost;
            left = cost;
        }
        row[0] = i + 1;
    }
    1. - row[b.len()] as f64 / longest as f64
}
fn mode(root: &Path) -> Option<String> {
    skills::settings(root)["distill_mode"].as_str().map(str::to_owned)
}
/// Names of the subject's concepts as comparison keys with the name to show.
fn concept_names(root: &Path, w: &str) -> Vec<(String, String)> {
    let graph = crate::knowledge::db(root).and_then(|c| crate::ontology::graph(&c, root, w)).unwrap_or(Value::Null);
    let mut names = Vec::new();
    for concept in graph["concepts"].as_array().into_iter().flatten() {
        let name = concept["name"].as_str().unwrap_or("");
        for text in std::iter::once(name).chain(concept["aliases"].as_array().into_iter().flatten().filter_map(Value::as_str)) {
            let k = key(text);
            if k.chars().count() >= 2 {
                names.push((k, name.to_owned()));
            }
        }
    }
    names
}
struct Turn {
    id: String,
    question: String,
    answer: String,
    completed: bool,
    mark: String,
    distill: String,
    copied: bool,
    group: Option<String>,
}
fn turns(c: &Connection, conversation: &str) -> Result<Vec<Turn>> {
    let mut q = c.prepare("SELECT id,user,answer,state,mark,distill,copied,question_group FROM turns WHERE conversation=?1 ORDER BY rowid")?;
    let rows = q.query_map([conversation], |r| {
        Ok(Turn {
            id: r.get(0)?,
            question: r.get(1)?,
            answer: r.get(2)?,
            completed: r.get::<_, String>(3)? == "completed",
            mark: r.get(4)?,
            distill: r.get(5)?,
            copied: r.get(6)?,
            group: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
/// The group of questions this one repeats, counted once per turn.
fn join_group(c: &Connection, w: &str, question: &str, threshold: f64) -> Result<String> {
    let k = key(question);
    let mut q = c.prepare("SELECT id,key FROM question_groups WHERE workspace=?1")?;
    let groups: Vec<(String, String)> = q.query_map([w], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let found = groups.into_iter().map(|(id, other)| (similarity(&k, &other), id)).filter(|(s, _)| *s >= threshold).max_by(|a, b| a.0.total_cmp(&b.0));
    let id = match found {
        Some((_, id)) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            c.execute("INSERT INTO question_groups VALUES(?1,?2,?3,?4,0,0,?5)", params![id, w, k, question.chars().take(200).collect::<String>(), now()])?;
            id
        }
    };
    c.execute("UPDATE question_groups SET count=count+1,updated=?2 WHERE id=?1", params![id, now()])?;
    Ok(id)
}
/// Automatic drafts the user mostly did not keep: only repeats and marks stay automatic.
fn narrowed(c: &Connection, w: &str, cfg: &Value) -> Result<bool> {
    let window = cfg["narrow_window"].as_u64().unwrap_or(10) as usize;
    let mut q = c.prepare("SELECT state FROM suggestions WHERE workspace=?1 AND automatic=1 AND state IN ('registered','dismissed') ORDER BY updated DESC LIMIT ?2")?;
    let states: Vec<String> = q.query_map(params![w, window as i64], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(states.len() >= window && states.iter().filter(|s| *s == "registered").count() < cfg["narrow_min_registered"].as_u64().unwrap_or(3) as usize)
}
fn entry(turn: &Turn, group: Option<&str>, repeats: i64, signal: &str) -> Value {
    json!({"turn":turn.id,"question":turn.question,"answer":turn.answer,"group":group,"repeats":repeats,"signal":signal})
}
/// One line in the suggestion box for a question that might deserve a note. No model is called.
fn candidate(c: &Connection, w: &str, provider: &str, group: &str, repeats: i64, signal: &str, strong: bool, entries: &[Value]) -> Result<()> {
    let existing: Option<(String, String, String)> = c
        .query_row("SELECT id,state,snapshot FROM suggestions WHERE workspace=?1 AND question_group=?2 AND state IN ('candidate','draft') ORDER BY created DESC LIMIT 1", params![w, group], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let turns: Vec<&Value> = entries.iter().map(|e| &e["turn"]).collect();
    match existing {
        // A draft already covers this question; another ask does not make a second line.
        Some((_, state, _)) if state == "draft" => {}
        Some((id, _, snapshot)) => {
            let mut all: Vec<Value> = serde_json::from_str(&snapshot).unwrap_or_default();
            for e in entries {
                if !all.iter().any(|x| x["turn"] == e["turn"]) {
                    all.push(e.clone());
                }
            }
            let ids: Vec<&Value> = all.iter().map(|e| &e["turn"]).collect();
            c.execute(
                "UPDATE suggestions SET snapshot=?2,turns=?3,repeats=?4,signal=CASE WHEN ?5 THEN ?6 ELSE signal END,seen=0,updated=?7 WHERE id=?1",
                params![id, json!(all).to_string(), json!(ids).to_string(), repeats, strong, signal, now()],
            )?;
        }
        None => {
            let title: String = entries.last().and_then(|e| e["question"].as_str()).unwrap_or("").lines().next().unwrap_or("").chars().take(80).collect();
            c.execute(
                "INSERT INTO suggestions(id,workspace,provider,state,title,turns,snapshot,question_group,repeats,signal,created,updated) VALUES(?1,?2,?3,'candidate',?4,?5,?6,?7,?8,?9,?10,?10)",
                params![uuid::Uuid::new_v4().to_string(), w, provider, title, json!(turns).to_string(), json!(entries).to_string(), group, repeats, signal, now()],
            )?;
        }
    }
    Ok(())
}
/// Judge the turns not judged yet. Only the app's records are read; no model is called.
fn scan(root: &Path, c: &Connection, w: &str, provider: &str) -> Result<()> {
    let Some(conversation) = c.query_row("SELECT id FROM conversations WHERE workspace=?1 AND provider=?2 AND active=1", params![w, provider], |r| r.get::<_, String>(0)).optional()? else {
        return Ok(());
    };
    let mut all = turns(c, &conversation)?;
    if !all.iter().any(|t| t.completed && t.distill.is_empty()) {
        return Ok(());
    }
    let routing = skills::routing(root)?;
    let cfg = &routing["distill"];
    let mode = mode(root);
    let names = concept_names(root, w);
    let min_chars = cfg["min_answer_chars"].as_u64().unwrap_or(400) as usize;
    let run = cfg["topic_run"].as_u64().unwrap_or(3).max(2) as usize;
    let narrowed = narrowed(c, w, cfg)?;
    if narrowed {
        c.execute("INSERT OR IGNORE INTO preferences VALUES('distill.narrow_notice','pending')", [])?;
    }
    let mentions: Vec<HashSet<&str>> = all
        .iter()
        .map(|t| {
            let k = key(&t.question);
            names.iter().filter(|(name, _)| k.contains(name.as_str())).map(|(_, shown)| shown.as_str()).collect()
        })
        .collect();
    for i in 0..all.len() {
        if !all[i].completed || !all[i].distill.is_empty() {
            continue;
        }
        let group = match all[i].group.clone() {
            Some(group) => group,
            None => {
                let group = join_group(c, w, &all[i].question, cfg["repeat_similarity"].as_f64().unwrap_or(0.8))?;
                c.execute("UPDATE turns SET question_group=?2 WHERE id=?1", params![all[i].id, group])?;
                all[i].group = Some(group.clone());
                group
            }
        };
        let (count, settled): (i64, i64) = c.query_row("SELECT count,settled FROM question_groups WHERE id=?1", [&group], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let long = all[i].answer.chars().count() >= min_chars;
        let marked = all[i].mark == "later";
        // A registered or discarded note covers the question until it is asked again.
        let open = count > settled;
        let repeated = long && open && count >= 2;
        let window: Vec<usize> = (0..=i).rev().filter(|k| all[*k].completed).take(run).collect();
        let shared = window.len() == run && window.iter().map(|k| &mentions[*k]).fold(None::<HashSet<&str>>, |acc, set| Some(match acc {
            None => set.clone(),
            Some(acc) => acc.intersection(set).copied().collect(),
        })).is_some_and(|common| !common.is_empty());
        let topic = long && open && shared;
        let weak = long && open && (!mentions[i].is_empty() || all[i].copied);
        let signal = if marked { "marked" } else if repeated { "repeated" } else if topic { "topic" } else if all[i].copied { "copied" } else { "concept" };
        let strong = marked || repeated || topic;
        let automatic = strong && mode.as_deref() == Some("auto") && (signal != "topic" || !narrowed);
        let related: Vec<usize> = match signal {
            "repeated" => (0..all.len()).filter(|k| all[*k].completed && all[*k].group.as_deref() == Some(group.as_str())).collect(),
            "topic" => window.iter().rev().copied().collect(),
            _ => vec![i],
        };
        let state = if automatic {
            for k in &related {
                if *k != i && matches!(all[*k].distill.as_str(), "none" | "candidate") {
                    c.execute("UPDATE turns SET distill='target' WHERE id=?1", [&all[*k].id])?;
                    all[*k].distill = "target".into();
                }
            }
            "target"
        } else if (strong && (mode.as_deref() != Some("off") || marked)) || (weak && mode.as_deref() != Some("off")) {
            let entries: Vec<Value> = related.iter().map(|k| entry(&all[*k], Some(&group), count, signal)).collect();
            candidate(c, w, provider, &group, count, signal, strong, &entries)?;
            "candidate"
        } else {
            "none"
        };
        c.execute("UPDATE turns SET distill=?2 WHERE id=?1", params![all[i].id, state])?;
        all[i].distill = state.into();
    }
    Ok(())
}
fn batch(c: &Connection, w: &str, provider: &str, automatic: bool, entries: &[Value], candidates: &[String]) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO distill_batches(id,workspace,provider,state,automatic,snapshot,created,updated) VALUES(?1,?2,?3,'queued',?4,?5,?6,?6)",
        params![id, w, provider, automatic, json!({"entries":entries,"candidates":candidates}).to_string(), now()],
    )?;
    for e in entries {
        c.execute("UPDATE turns SET distill='queued' WHERE id=?1", [e["turn"].as_str()])?;
    }
    Ok(id)
}
/// Gather the conversation's automatic targets into one batch: one call, at most a few notes.
fn gather(c: &Connection, w: &str, provider: &str) -> Result<()> {
    let Some(conversation) = c.query_row("SELECT id FROM conversations WHERE workspace=?1 AND provider=?2 AND active=1", params![w, provider], |r| r.get::<_, String>(0)).optional()? else {
        return Ok(());
    };
    let mut entries = Vec::new();
    for t in turns(c, &conversation)?.iter().filter(|t| t.distill == "target") {
        let (count, signal) = match &t.group {
            Some(group) => (c.query_row("SELECT count FROM question_groups WHERE id=?1", [group], |r| r.get::<_, i64>(0)).unwrap_or(1), if t.mark == "later" { "marked" } else { "repeated" }),
            None => (1, "marked"),
        };
        entries.push(entry(t, t.group.as_deref(), count, signal));
    }
    if !entries.is_empty() {
        batch(c, w, provider, true, &entries, &[])?;
    }
    Ok(())
}
/// Before `새 대화` erases the conversation: what was worth a note is carried over.
pub(crate) fn before_clear(root: &Path, c: &Connection, w: &str, provider: &str) -> Result<()> {
    scan(root, c, w, provider)?;
    gather(c, w, provider)
}
/// A batch whose worker is gone waits again; then the oldest waiting batch starts if nothing else runs.
fn kick(root: &Path, c: &Connection) -> Result<Option<String>> {
    let active = providers::handle(root, &json!({"op":"analysis.active"}))?["active"].clone();
    let running = if active["kind"] == "distill" { active["id"].as_str().unwrap_or("").to_owned() } else { String::new() };
    c.execute("UPDATE distill_batches SET state='queued',updated=?2 WHERE state='running' AND COALESCE(job,'')<>?1", params![running, now()])?;
    if !active.is_null() {
        return Ok(None);
    }
    // A batch that failed or met the limit is tried again later, not in a loop.
    let retry = (chrono::Utc::now() - chrono::Duration::minutes(30)).to_rfc3339();
    let next: Option<(String, String, String, bool)> = c
        .query_row(
            "SELECT id,workspace,provider,automatic FROM distill_batches WHERE state='queued' OR (automatic=1 AND state IN ('failed','limit') AND updated<?1) ORDER BY automatic,created LIMIT 1",
            [retry],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((id, w, provider, automatic)) = next else {
        return Ok(None);
    };
    if automatic && skills::background_today(root) >= skills::background_cap(root)? {
        return Ok(Some("오늘 뒤에서 정리할 수 있는 양을 다 썼어요. 남은 정리는 내일 이어서 해요.".into()));
    }
    // Busy with a chat turn or another job: the batch simply keeps waiting.
    let _ = providers::start(root, &json!({"provider":provider,"workspace_id":w,"batch_id":id,"lane":if automatic { "back" } else { "front" }}), "distill");
    Ok(None)
}
fn clean_title(title: &str) -> String {
    let cleaned: String = title.lines().next().unwrap_or("").chars().map(|c| if c.is_control() || "/\\:".contains(c) { ' ' } else { c }).collect();
    let cleaned: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
    if cleaned.is_empty() { "학습도우미 정리".into() } else { cleaned }
}
fn settle(c: &Connection, group: Option<&str>) -> Result<()> {
    if let Some(group) = group {
        c.execute("UPDATE question_groups SET settled=count WHERE id=?1", [group])?;
    }
    Ok(())
}
/// The back lane's job: one call for the whole batch, then drafts in the suggestion box.
pub(crate) fn work(root: &Path, job: &Path, req: &Value) -> Result<Value> {
    let id = req["batch_id"].as_str().context("정리 묶음 ID 없음")?;
    let c = chat::db(root)?;
    let (w, provider, automatic, snapshot): (String, String, bool, String) =
        c.query_row("SELECT workspace,provider,automatic,snapshot FROM distill_batches WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    let job_id = job.file_name().and_then(|name| name.to_str()).unwrap_or("");
    c.execute("UPDATE distill_batches SET state='running',job=?2,error='',updated=?3 WHERE id=?1", params![id, job_id, now()])?;
    let snapshot: Value = serde_json::from_str(&snapshot)?;
    match draft(root, job, &c, id, &w, &provider, automatic, &snapshot) {
        Ok(result) => {
            c.execute("UPDATE distill_batches SET state='done',error=?2,updated=?3 WHERE id=?1", params![id, result["skipped_reason"].as_str().unwrap_or(""), now()])?;
            Ok(result)
        }
        Err(e) => {
            let message = format!("{e:#}");
            let state = if job.join("cancelled").exists() { "queued" } else if providers::is_limit(&message) { "limit" } else { "failed" };
            c.execute("UPDATE distill_batches SET state=?2,error=?3,updated=?4 WHERE id=?1", params![id, state, message, now()])?;
            // A draft the user asked for can be asked for again; an automatic batch is retried by the app.
            if !automatic && state != "queued" {
                for e in snapshot["entries"].as_array().into_iter().flatten() {
                    c.execute("UPDATE turns SET distill='none' WHERE id=?1 AND distill='queued'", [e["turn"].as_str()])?;
                }
            }
            Err(e)
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn draft(root: &Path, job: &Path, c: &Connection, id: &str, w: &str, provider: &str, automatic: bool, snapshot: &Value) -> Result<Value> {
    let entries = snapshot["entries"].as_array().cloned().unwrap_or_default();
    anyhow::ensure!(!entries.is_empty(), "정리할 대화가 없습니다");
    let routing = skills::routing(root)?;
    let max_notes = routing["distill"]["max_notes"].as_u64().unwrap_or(3) as usize;
    let skill = skills::load(root, "distill-chat", "")?;
    let helper_settings = skills::settings(root);
    let provider = skills::helper_provider(&helper_settings, "distill", provider);
    let catalog = chat::handle(root, &json!({"op":"chat.models","provider":provider}))?["models"].as_array().cloned().unwrap_or_default();
    let settings = skills::settings(root);
    let stages = skills::stage_plan_for(root, provider, settings["organize_mode"].as_str().unwrap_or("auto"), &catalog, &["distill"])?;
    let stage = &stages["stages"]["distill"];
    anyhow::ensure!(stage["model"].is_string(), "대화 정리에 쓸 모델을 찾지 못했습니다");
    let (model, effort) = (stage["model"].as_str().unwrap_or("").to_owned(), stage["effort"].as_str().unwrap_or("").to_owned());
    let names = concept_names(root, w);
    let mut concepts: Vec<&str> = Vec::new();
    for (_, shown) in &names {
        if !concepts.contains(&shown.as_str()) && concepts.len() < 300 {
            concepts.push(shown);
        }
    }
    let mut q = c.prepare("SELECT title FROM suggestions WHERE workspace=?1 AND state IN ('draft','registered')")?;
    let titles: Vec<String> = q.query_map([w], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let input = json!({
        "turns": entries.iter().enumerate().map(|(i, e)| json!({"turn":format!("t{}", i + 1),"question":e["question"],"answer":e["answer"]})).collect::<Vec<_>>(),
        "concepts": concepts,
        "existing_titles": titles,
        "max_notes": max_notes,
    });
    let prompt = format!("{}\n\n## INPUT\n\n```json\n{input}\n```\n", skill.text);
    // A chat turn always goes first; this call waits for the lane to clear.
    let waited = Instant::now();
    let lock = loop {
        match chat::ExecutionLock::acquire(root) {
            Ok(lock) => break lock,
            Err(_) if waited.elapsed() < Duration::from_secs(60) => thread::sleep(Duration::from_millis(300)),
            Err(_) => bail!("대화가 진행 중이어서 조금 뒤에 정리해요"),
        }
    };
    let _ = skills::log_usage(root, if automatic { "background" } else { "manual" }, &skill.name, provider, &model, id);
    let reply = providers::invoke(root, job, &job.join("call"), &providers::Call {
        provider,
        model: (!model.is_empty()).then_some(model.as_str()),
        effort: &effort,
        prompt: &prompt,
        schema: &skill.schema,
        images: &[],
        timeout: Duration::from_secs(routing["timeouts_sec"]["distill"].as_u64().unwrap_or(300)),
    });
    drop(lock);
    let reply = reply?;
    let notes: Vec<&Value> = reply.result["notes"].as_array().into_iter().flatten().filter(|n| n["body"].as_str().is_some_and(|b| !b.trim().is_empty())).take(max_notes).collect();
    let candidates: Vec<String> = snapshot["candidates"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let tx = c.unchecked_transaction()?;
    let turn_ids: Vec<&str> = entries.iter().filter_map(|e| e["turn"].as_str()).collect();
    for turn in &turn_ids {
        tx.execute("UPDATE turns SET distill=?2 WHERE id=?1", params![turn, if notes.is_empty() { "skipped" } else { "done" }])?;
    }
    if notes.is_empty() {
        // Judged not worth a note: the line leaves the box and is not offered again for the same asks.
        for candidate in &candidates {
            let group: Option<String> = tx.query_row("SELECT question_group FROM suggestions WHERE id=?1", [candidate], |r| r.get(0)).optional()?.flatten();
            tx.execute("UPDATE suggestions SET state='dismissed',snapshot='[]',updated=?2 WHERE id=?1", params![candidate, now()])?;
            settle(&tx, group.as_deref())?;
        }
        tx.commit()?;
        return Ok(json!({"drafts":0,"skipped_reason":reply.result["skipped_reason"].as_str().filter(|r| !r.is_empty()).unwrap_or("남길 내용이 없다고 판단했어요")}));
    }
    for note in &notes {
        let picked: Vec<&Value> = note["turns"].as_array().into_iter().flatten()
            .filter_map(|t| t.as_str().and_then(|t| t.strip_prefix('t')).and_then(|n| n.parse::<usize>().ok()).and_then(|n| entries.get(n.wrapping_sub(1))))
            .collect();
        let source: Vec<&Value> = if picked.is_empty() { entries.iter().collect() } else { picked };
        let known: Vec<&Value> = note["concepts"].as_array().into_iter().flatten().filter(|n| n.as_str().is_some_and(|n| concepts.contains(&n))).collect();
        // The draft says why it was made: the strongest reason among the exchanges it covers.
        let strength = |e: &&&Value| (matches!(e["signal"].as_str(), Some("marked" | "repeated" | "topic" | "asked")), e["repeats"].as_i64().unwrap_or(1));
        let first = *source.iter().max_by_key(strength).unwrap_or(&source[0]);
        tx.execute(
            "INSERT INTO suggestions(id,workspace,provider,model,skill,state,title,body,kind,concepts,turns,snapshot,question_group,repeats,signal,automatic,created,updated) VALUES(?1,?2,?3,?4,?5,'draft',?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?16)",
            params![
                uuid::Uuid::new_v4().to_string(), w, provider, reply.model, format!("{}@{}", skill.version, skill.fingerprint.chars().take(12).collect::<String>()),
                clean_title(note["title"].as_str().unwrap_or("")), note["body"].as_str().unwrap_or("").trim(), note["kind"].as_str().unwrap_or("concept"),
                json!(known).to_string(), json!(source.iter().map(|e| &e["turn"]).collect::<Vec<_>>()).to_string(),
                json!(source.iter().map(|e| json!({"turn":e["turn"],"question":e["question"],"answer":e["answer"]})).collect::<Vec<_>>()).to_string(),
                first["group"].as_str(), first["repeats"].as_i64().unwrap_or(1), first["signal"].as_str().unwrap_or(""), automatic, now(),
            ],
        )?;
    }
    // The drafts replace the lines they were made from.
    for candidate in &candidates {
        tx.execute("DELETE FROM suggestions WHERE id=?1 AND state='candidate'", [candidate])?;
    }
    let mut q = tx.prepare("SELECT id,turns FROM suggestions WHERE workspace=?1 AND state='candidate'")?;
    let lines: Vec<(String, String)> = q.query_map([w], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    drop(q);
    for (line, turns) in lines {
        let turns: Vec<String> = serde_json::from_str(&turns).unwrap_or_default();
        if !turns.is_empty() && turns.iter().all(|t| turn_ids.contains(&t.as_str())) {
            tx.execute("DELETE FROM suggestions WHERE id=?1", [&line])?;
        }
    }
    tx.commit()?;
    let _ = providers::mark_success(root, provider, job.file_name().and_then(|n| n.to_str()).unwrap_or(""));
    Ok(json!({"drafts":notes.len(),"model":reply.model,"usage":reply.usage,"skipped_reason":""}))
}
/// Save a note in the subject's `학습도우미 정리` folder and remember where it came from.
fn save_note(root: &Path, w: &str, title: &str, body: &str, suggestion: &str, kind: &str) -> Result<Value> {
    let storage = |req: Value| crate::storage::handle(root, &req);
    let folders = storage(json!({"op":"tree","workspace_id":w}))?["folders"].as_array().cloned().unwrap_or_default();
    if !folders.iter().any(|f| f["path"] == FOLDER) {
        storage(json!({"op":"folder.create","workspace_id":w,"parent":"","name":FOLDER}))?;
    }
    let draft = storage(json!({"op":"draft.create","workspace_id":w,"body":body}))?;
    let mut title = clean_title(title);
    for _ in 0..3 {
        let saved = storage(json!({"op":"note.save","draft_id":draft["id"],"title":title,"folder":FOLDER,"origin_suggestion":suggestion,"origin_kind":kind}))?;
        match saved["suggested_title"].as_str() {
            Some(other) if saved["conflict"] == true => title = other.to_owned(),
            _ => return Ok(saved),
        }
    }
    bail!("같은 이름의 노트가 있어 저장하지 못했어요. 제목을 바꿔 다시 등록해 주세요.")
}
fn status(root: &Path, c: &Connection, w: &str) -> Result<Value> {
    let notice = kick(root, c)?;
    let mode = mode(root);
    let mut q = c.prepare("SELECT id,state,title,body,kind,concepts,snapshot,repeats,signal,automatic,seen,provider,model,created FROM suggestions WHERE workspace=?1 AND state IN ('candidate','draft') ORDER BY state='candidate',updated DESC")?;
    let suggestions: Vec<Value> = q
        .query_map([w], |r| {
            let snapshot: Vec<Value> = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default();
            Ok(json!({
                "id":r.get::<_, String>(0)?,"state":r.get::<_, String>(1)?,"title":r.get::<_, String>(2)?,"body":r.get::<_, String>(3)?,"kind":r.get::<_, String>(4)?,
                "concepts":serde_json::from_str::<Value>(&r.get::<_, String>(5)?).unwrap_or(json!([])),
                "questions":snapshot.iter().filter_map(|e| e["question"].as_str()).collect::<Vec<_>>(),
                "repeats":r.get::<_, i64>(7)?,"signal":r.get::<_, String>(8)?,"automatic":r.get::<_, bool>(9)?,"seen":r.get::<_, bool>(10)?,
                "provider":r.get::<_, String>(11)?,"model":r.get::<_, String>(12)?,"created":r.get::<_, String>(13)?,
            }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let (working, running): (i64, i64) = c.query_row(
        "SELECT COUNT(*), COUNT(CASE WHEN state='running' THEN 1 END) FROM distill_batches WHERE workspace=?1 AND state IN ('queued','running')",
        [w], |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let recent = (chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
    let failed: Option<String> = c.query_row("SELECT error FROM distill_batches WHERE workspace=?1 AND state IN ('failed','limit') AND updated>?2 ORDER BY updated DESC LIMIT 1", params![w, recent], |r| r.get(0)).optional()?;
    let strong = suggestions.iter().any(|s| s["state"] == "candidate" && matches!(s["signal"].as_str(), Some("marked" | "repeated" | "topic")));
    let narrow: Option<String> = c.query_row("SELECT value FROM preferences WHERE key='distill.narrow_notice'", [], |r| r.get(0)).optional()?;
    let notice = notice.or_else(|| (narrow.as_deref() == Some("pending")).then(|| "자동으로 만든 초안을 거의 쓰지 않으셔서, 앞으로는 반복해 물은 질문과 표시한 답만 알아서 정리해요.".to_owned()));
    // 정리 제안 창을 처음 열 때 한 번 보이는 안내. `알겠어요`를 누르면 다시 보이지 않는다.
    let intro_seen = c.query_row("SELECT 1 FROM preferences WHERE key='distill.intro'", [], |_| Ok(())).optional()?.is_some();
    Ok(json!({
        "mode":mode,"consent_needed":mode.is_none() && strong,"suggestions":suggestions,"working":working,"running":running,
        "unseen":suggestions.iter().any(|s| s["seen"] == false),"notice":notice,"error":failed,"intro_seen":intro_seen,
    }))
}
fn suggestion(c: &Connection, id: &str) -> Result<(String, String, String, String, String, Option<String>, String)> {
    c.query_row("SELECT workspace,provider,state,title,body,question_group,snapshot FROM suggestions WHERE id=?1", [id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
    })
    .optional()?
    .context("정리 제안을 찾을 수 없습니다")
}
fn turn(c: &Connection, id: &str) -> Result<(String, String, Turn)> {
    let (conversation, w, provider): (String, String, String) = c
        .query_row("SELECT t.conversation,c.workspace,c.provider FROM turns t JOIN conversations c ON c.id=t.conversation WHERE t.id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?
        .context("대화를 찾을 수 없습니다")?;
    let found = turns(c, &conversation)?.into_iter().find(|t| t.id == id).context("대화를 찾을 수 없습니다")?;
    Ok((w, provider, found))
}
pub fn handle(root: &Path, req: &Value) -> Result<Value> {
    let c = chat::db(root)?;
    let text = |name: &str| req[name].as_str().with_context(|| format!("{name}이 필요합니다"));
    match req["op"].as_str().unwrap_or("") {
        "distill.status" => status(root, &c, text("workspace_id")?),
        "distill.scan" => {
            scan(root, &c, text("workspace_id")?, text("provider")?)?;
            status(root, &c, text("workspace_id")?)
        }
        // The conversation went quiet, the subject changed, or a new conversation begins.
        "distill.flush" => {
            let (w, provider) = (text("workspace_id")?, text("provider")?);
            scan(root, &c, w, provider)?;
            gather(&c, w, provider)?;
            status(root, &c, w)
        }
        "distill.mark" => {
            let mark = req["mark"].as_str().unwrap_or("later");
            anyhow::ensure!(["later", ""].contains(&mark), "잘못된 표시");
            // Judged again at the next scan: a mark turns a passed-over answer into a target.
            let turn_id = text("turn_id")?;
            c.execute("UPDATE turns SET mark=?2,distill=CASE WHEN distill IN ('none','candidate','target') THEN '' ELSE distill END WHERE id=?1", params![turn_id, mark])?;
            if mark.is_empty() {
                // Taking the mark back also takes back the line it made.
                c.execute("DELETE FROM suggestions WHERE state='candidate' AND signal='marked' AND turns=?1", [json!([turn_id]).to_string()])?;
            }
            Ok(json!({"mark":mark}))
        }
        "distill.copied" => {
            c.execute("UPDATE turns SET copied=1,distill=CASE WHEN distill='none' THEN '' ELSE distill END WHERE id=?1", [text("turn_id")?])?;
            Ok(json!({"copied":true}))
        }
        // `노트로 정리`: a draft now, with one call.
        "distill.now" => {
            let (w, provider, t) = turn(&c, text("turn_id")?)?;
            anyhow::ensure!(t.completed && !t.answer.trim().is_empty(), "끝난 답변만 정리할 수 있어요");
            anyhow::ensure!(!matches!(t.distill.as_str(), "queued"), "이미 정리하는 중이에요");
            let id = batch(&c, &w, &provider, false, &[entry(&t, t.group.as_deref(), 1, "asked")], &[])?;
            kick(root, &c)?;
            Ok(json!({"batch_id":id}))
        }
        // `그대로 저장`: the question and answer as they are; no model is called.
        "distill.keep" => {
            let (w, provider, t) = turn(&c, text("turn_id")?)?;
            anyhow::ensure!(!t.answer.trim().is_empty(), "저장할 답변이 없어요");
            let id = uuid::Uuid::new_v4().to_string();
            let title = clean_title(&t.question);
            let body = format!("## 질문\n\n{}\n\n## 답\n\n{}\n", t.question.trim(), t.answer.trim());
            let document = save_note(root, &w, &title, &body, &id, "verbatim")?;
            c.execute(
                "INSERT INTO suggestions(id,workspace,provider,state,title,body,kind,turns,question_group,signal,seen,document_id,created,updated) VALUES(?1,?2,?3,'registered',?4,?5,'qa',?6,?7,'kept',1,?8,?9,?9)",
                params![id, w, provider, document["title"].as_str().unwrap_or(&title), body, json!([t.id]).to_string(), t.group, document["id"].as_str(), now()],
            )?;
            c.execute("UPDATE turns SET distill='kept' WHERE id=?1", [&t.id])?;
            settle(&c, t.group.as_deref())?;
            Ok(json!({"document":document}))
        }
        // The first strong signal asks once how conversations should be organised.
        "distill.consent" => {
            let (w, provider) = (text("workspace_id")?, text("provider")?);
            let choice = text("choice")?;
            anyhow::ensure!(["once", "auto", "no"].contains(&choice), "잘못된 선택");
            skills::handle(root, &json!({"op":"ai.settings.set","distill_mode":if choice == "auto" { "auto" } else { "ask" }}))?;
            if choice != "no" {
                let mut q = c.prepare("SELECT id,snapshot FROM suggestions WHERE workspace=?1 AND provider=?2 AND state='candidate' AND signal IN ('marked','repeated','topic')")?;
                let lines: Vec<(String, String)> = q.query_map(params![w, provider], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
                let mut entries: Vec<Value> = Vec::new();
                for (_, snapshot) in &lines {
                    for e in serde_json::from_str::<Vec<Value>>(snapshot).unwrap_or_default() {
                        if !entries.iter().any(|x| x["turn"] == e["turn"]) {
                            entries.push(e);
                        }
                    }
                }
                if !entries.is_empty() {
                    batch(&c, w, provider, true, &entries, &lines.into_iter().map(|(id, _)| id).collect::<Vec<_>>())?;
                }
            }
            status(root, &c, w)
        }
        // A line's `정리`: the draft is made now.
        "distill.draft" => {
            let id = text("id")?;
            let (w, provider, state, _, _, _, snapshot) = suggestion(&c, id)?;
            anyhow::ensure!(state == "candidate", "이미 정리한 제안이에요");
            let entries: Vec<Value> = serde_json::from_str(&snapshot).unwrap_or_default();
            anyhow::ensure!(!entries.is_empty(), "정리할 대화가 남아 있지 않아요");
            let batch_id = batch(&c, &w, &provider, false, &entries, &[id.to_owned()])?;
            kick(root, &c)?;
            Ok(json!({"batch_id":batch_id}))
        }
        "distill.update" => {
            let id = text("id")?;
            anyhow::ensure!(suggestion(&c, id)?.2 == "draft", "초안만 고칠 수 있어요");
            let (title, body) = (clean_title(text("title")?), text("body")?);
            anyhow::ensure!(!body.trim().is_empty(), "내용이 비어 있어요");
            c.execute("UPDATE suggestions SET title=?2,body=?3,updated=?4 WHERE id=?1", params![id, title, body, now()])?;
            Ok(json!({"id":id,"title":title}))
        }
        "distill.register" => {
            let id = text("id")?;
            let (w, _, state, title, body, group, _) = suggestion(&c, id)?;
            anyhow::ensure!(state == "draft", "먼저 초안으로 정리해 주세요");
            let document = save_note(root, &w, &title, &body, id, "distilled")?;
            c.execute("UPDATE suggestions SET state='registered',seen=1,document_id=?2,title=?3,updated=?4 WHERE id=?1", params![id, document["id"].as_str(), document["title"].as_str().unwrap_or(&title), now()])?;
            settle(&c, group.as_deref())?;
            Ok(json!({"document":document}))
        }
        // Discarding also drops the kept copy of the conversation.
        "distill.dismiss" => {
            let id = text("id")?;
            let (_, _, state, _, _, group, snapshot) = suggestion(&c, id)?;
            anyhow::ensure!(matches!(state.as_str(), "candidate" | "draft"), "이미 처리한 제안이에요");
            for e in serde_json::from_str::<Vec<Value>>(&snapshot).unwrap_or_default() {
                c.execute("UPDATE turns SET distill='skipped' WHERE id=?1", [e["turn"].as_str()])?;
            }
            c.execute("UPDATE suggestions SET state='dismissed',body='',snapshot='[]',seen=1,updated=?2 WHERE id=?1", params![id, now()])?;
            settle(&c, group.as_deref())?;
            Ok(json!({"dismissed":id}))
        }
        "distill.seen" => {
            c.execute("UPDATE suggestions SET seen=1 WHERE workspace=?1 AND state IN ('candidate','draft')", [text("workspace_id")?])?;
            c.execute("UPDATE preferences SET value='shown' WHERE key='distill.narrow_notice'", [])?;
            Ok(json!({"seen":true}))
        }
        "distill.intro" => {
            c.execute("INSERT OR REPLACE INTO preferences VALUES('distill.intro','seen')", [])?;
            Ok(json!({"intro_seen":true}))
        }
        op => bail!("지원하지 않는 대화 정리 명령: {op}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_are_compared_letter_by_letter() {
        let same = similarity(&key("JWT 토큰이 뭐야?"), &key("jwt 토큰이 뭐야"));
        assert_eq!(same, 1.);
        assert!(similarity(&key("스프링 시큐리티에서 JWT 토큰은 어떻게 검증해?"), &key("스프링 시큐리티에서 JWT 토큰은 어떻게 검증하나요")) >= 0.8);
        assert!(similarity(&key("JWT 토큰이 뭐야"), &key("세션과 쿠키의 차이")) < 0.5);
        assert_eq!(clean_title("a/b: c\n둘째 줄"), "a b c");
    }
}
