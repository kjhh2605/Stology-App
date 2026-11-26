//! Workspace-owned rows are removed together. File cleanup survives interruption
//! and runs under the storage writer lock before another snapshot can reuse a path.
use super::*;
use std::collections::BTreeSet;

fn strings(c: &Connection, sql: &str, id: &str) -> Result<Vec<String>> {
    Ok(c.prepare(sql)?
        .query_map([id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

pub(super) fn cleanup(root: &Path, c: &Connection) -> Result<()> {
    let paths = c
        .prepare("SELECT path FROM workspace_cleanup")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for path in paths {
        let path = PathBuf::from(path);
        let parent = path.parent().context("삭제 경로 오류")?;
        ensure_no_symlink(root, parent)?;
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => fs::remove_dir_all(&path)?,
            Ok(_) => fs::remove_file(&path)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        c.execute(
            "DELETE FROM workspace_cleanup WHERE path=?1",
            [path.to_string_lossy()],
        )?;
    }
    Ok(())
}

fn records(root: &Path, directory: &str, record: &str) -> Result<Vec<(PathBuf, Value)>> {
    let dir = root.join(directory);
    ensure_no_symlink(root, &dir)?;
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut rows = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let file = if record.is_empty() {
            path.clone()
        } else {
            path.join(record)
        };
        if file.is_file() {
            ensure_no_symlink(root, &file)?;
            rows.push((path, serde_json::from_slice(&fs::read(file)?)?));
        }
    }
    Ok(rows)
}

pub(super) fn delete(root: &Path, c: &Connection, id: &str) -> Result<()> {
    workspace(c, id)?;
    Uuid::parse_str(id)?;
    let mut paths = vec![
        root.join("workspaces").join(id),
        root.join("ai/runs").join(format!("latest-{id}.json")),
        crate::screen::screen_path(root, id)?,
    ];
    use std::io::Read;
    use std::os::fd::AsRawFd;
    let mut job_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("analysis.lock"))?;
    anyhow::ensure!(
        unsafe { libc::flock(job_lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "작업이 시작되는 중입니다. 잠시 후 다시 시도하세요."
    );
    let mut current_job = String::new();
    job_lock.read_to_string(&mut current_job)?;
    for (path, job) in records(root, "jobs", "state.json")? {
        if job["workspace_id"] == id {
            anyhow::ensure!(
                !matches!(
                    job["state"].as_str(),
                    Some("queued" | "running" | "waiting_login")
                ),
                "진행 중인 작업이 끝난 후 삭제하세요."
            );
            if job["id"] == current_job.as_str() {
                job_lock.set_len(0)?;
            }
            paths.push(path);
        }
    }
    let mut runs = BTreeSet::new();
    for (path, run) in records(root, "ai/runs", "run.json")? {
        if run["workspace_id"] == id {
            runs.insert(required(&run, "id")?.to_owned());
            paths.push(path);
        }
    }
    if !runs.is_empty() {
        for (path, cache) in records(root, "ai/cache", "")? {
            if cache["run"].as_str().is_some_and(|run| runs.contains(run)) {
                paths.push(path);
            }
        }
    }
    let docs = "SELECT id FROM documents WHERE workspace_id=?1";
    let versions = strings(
        c,
        &format!("SELECT DISTINCT path FROM source_versions WHERE document_id IN ({docs})"),
        id,
    )?;
    let mut hashes = strings(c,&format!("SELECT hash FROM documents WHERE workspace_id=?1 UNION SELECT hash FROM source_versions WHERE document_id IN ({docs})"),id)?;
    for (db, file) in [
        ("knowledge", "knowledge.sqlite"),
        ("backup", "knowledge.before-subject-layer.sqlite"),
        ("chat", "chat.sqlite3"),
        ("journal", "pdf-journal.sqlite"),
    ] {
        let path = root.join(file);
        if !path.exists() {
            continue;
        }
        c.execute(
            &format!("ATTACH DATABASE ?1 AS {db}"),
            [path.to_string_lossy()],
        )?;
        if db == "journal" {
            for table in ["sessions", "commands"] {
                c.execute(
                    &format!("DELETE FROM journal.{table} WHERE document_id IN ({docs})"),
                    [id],
                )?;
            }
            continue;
        }
        if db == "chat" {
            for convo in strings(
                c,
                "SELECT id FROM chat.conversations WHERE workspace=?1",
                id,
            )? {
                anyhow::ensure!(relative(&convo)?.components().count() == 1);
                paths.push(root.join("chat-work").join(convo));
            }
            c.execute("DELETE FROM chat.turns WHERE conversation IN (SELECT id FROM chat.conversations WHERE workspace=?1)",[id])?;
        }
        // Older graph databases may have only the evidence layer. Delete children first.
        let sql = format!(
            "SELECT m.name FROM {db}.sqlite_master m WHERE m.type='table' AND EXISTS
            (SELECT 1 FROM pragma_table_info(m.name,'{db}') WHERE name='workspace')
            ORDER BY CASE m.name WHEN 'supports' THEN 0 WHEN 'assertions' THEN 1 ELSE 2 END"
        );
        let tables = c
            .prepare(&sql)?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for table in tables {
            if table == "sources" {
                hashes.extend(strings(
                    c,
                    &format!("SELECT input_hash FROM {db}.sources WHERE workspace=?1"),
                    id,
                )?);
            }
            anyhow::ensure!(table
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'));
            c.execute(
                &format!("DELETE FROM {db}.{table} WHERE workspace=?1"),
                [id],
            )?;
        }
    }
    for (table, col) in [
        ("source_versions", "document_id"),
        ("generated_pdfs", "source_id"),
        ("pdf_state", "document_id"),
        ("pdf_publish_intents", "document_id"),
    ] {
        c.execute(
            &format!("DELETE FROM {table} WHERE {col} IN ({docs})"),
            [id],
        )?;
    }
    c.execute("DELETE FROM documents WHERE workspace_id=?1", [id])?;
    c.execute("DELETE FROM workspaces WHERE id=?1", [id])?;
    for path in versions {
        if !c.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_versions WHERE path=?1)",
            [&path],
            |r| r.get::<_, bool>(0),
        )? {
            let path = PathBuf::from(path);
            anyhow::ensure!(path.parent() == Some(root.join("versions").as_path()));
            paths.push(path);
        }
    }
    let mut unused_hashes = BTreeSet::new();
    for hash in hashes.into_iter().collect::<BTreeSet<_>>() {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let mut used = c.query_row("SELECT EXISTS(SELECT 1 FROM documents WHERE hash=?1 UNION ALL SELECT 1 FROM source_versions WHERE hash=?1)",[&hash],|r|r.get::<_,bool>(0))?;
        for (db, file) in [
            ("knowledge", "knowledge.sqlite"),
            ("backup", "knowledge.before-subject-layer.sqlite"),
        ] {
            if root.join(file).exists()
                && c.query_row(
                    &format!(
                        "SELECT EXISTS(SELECT 1 FROM {db}.sqlite_master WHERE name='sources')"
                    ),
                    [],
                    |r| r.get::<_, bool>(0),
                )?
            {
                used |= c.query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM {db}.sources WHERE input_hash=?1)"),
                    [&hash],
                    |r| r.get::<_, bool>(0),
                )?;
            }
        }
        if !used {
            paths.push(root.join("extractions").join(&hash));
            unused_hashes.insert(hash);
        }
    }
    let previews = root.join("preview-cache");
    ensure_no_symlink(root, &previews)?;
    if previews.is_dir() {
        for entry in fs::read_dir(previews)? {
            let entry = entry?;
            let name = entry.file_name();
            if name
                .to_str()
                .and_then(|n| {
                    n.strip_suffix(".pdf")?
                        .strip_prefix("pptx-")?
                        .split_once('-')
                })
                .is_some_and(|(hash, _)| unused_hashes.contains(hash))
            {
                paths.push(entry.path());
            }
        }
    }
    for path in paths {
        c.execute(
            "INSERT OR IGNORE INTO workspace_cleanup VALUES(?1)",
            [path.to_string_lossy()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_and_delete_only_the_target_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let call = |r: Value| handle(root, &r).unwrap();
        let mut subjects = Vec::new();
        for n in ["삭제할 과목", "유지할 과목"] {
            let w = call(json!({"op":"workspace.create","name":n}))["id"].clone();
            let d = call(json!({"op":"draft.create","workspace_id":w,"body":"공유 자료"}));
            let note =
                call(json!({"op":"note.save","draft_id":d["id"],"title":"노트","folder":""}));
            let snap = call(json!({"op":"source.snapshot","id":note["id"]}));
            let c = crate::chat::db(root).unwrap();
            c.execute("INSERT INTO conversations(id,workspace,provider,created,updated) VALUES(?1,?1,'codex','','')",[w.as_str().unwrap()]).unwrap();
            crate::knowledge::db(root)
                .unwrap()
                .execute(
                    "INSERT INTO ont_guidance VALUES(?1,'지침')",
                    [w.as_str().unwrap()],
                )
                .unwrap();
            subjects.push((w, snap));
        }
        let (w, snap) = &subjects[0];
        assert_eq!(
            call(json!({"op":"workspace.rename","workspace_id":w,"name":"새 이름"}))["id"],
            *w
        );
        for (subject, _) in &subjects {
            crate::screen::handle(root, &json!({"op":"screen.save","workspace_id":subject,"screen":{"map_open":true}})).unwrap();
        }
        call(json!({"op":"workspace.delete","workspace_id":w}));
        assert!(!crate::screen::screen_path(root, w.as_str().unwrap()).unwrap().exists());
        assert!(crate::screen::screen_path(root, subjects[1].0.as_str().unwrap()).unwrap().exists());
        assert!(Path::new(snap["immutable_path"].as_str().unwrap()).is_file());
        assert!(!root.join("workspaces").join(w.as_str().unwrap()).exists());
        assert_eq!(
            call(json!({"op":"workspace.list"}))["workspaces"][0]["id"],
            subjects[1].0
        );
        for (db, table) in [
            (crate::chat::db(root).unwrap(), "conversations"),
            (crate::knowledge::db(root).unwrap(), "ont_guidance"),
        ] {
            assert_eq!(
                db.query_row(&format!("SELECT workspace FROM {table}"), [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                subjects[1].0.as_str().unwrap()
            );
        }
        call(json!({"op":"workspace.delete","workspace_id":subjects[1].0}));
        assert!(!Path::new(snap["immutable_path"].as_str().unwrap()).exists());
    }
}
