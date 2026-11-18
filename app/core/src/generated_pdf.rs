//! User-visible PDF material generated from PPTX. The converted PDF is an
//! independent document; the link is only a shortcut for resuming annotation.
use crate::storage::{self, Doc};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) fn init_schema(c: &Connection) -> Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS generated_pdfs(source_id TEXT PRIMARY KEY,pdf_id TEXT NOT NULL UNIQUE,source_hash TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS generated_pdf_intents(id TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS generated_pdf_staging(path TEXT PRIMARY KEY,created_at INTEGER NOT NULL DEFAULT (unixepoch()),checked_at INTEGER NOT NULL DEFAULT 0,dev INTEGER,ino INTEGER);
        CREATE INDEX IF NOT EXISTS generated_pdf_staging_gc ON generated_pdf_staging(checked_at,created_at);")?;
    Ok(())
}

// Register the private filename before creating it. A killed process therefore
// cannot leave an undiscoverable stage before its publication intent exists.
// flock is held across writing and publication, including conversions lasting
// longer than the GC grace period. Process death releases the lock naturally.
struct StagedFile {
    file: fs::File,
    path: PathBuf,
    identity: (u64, u64),
    durable: bool,
}
impl StagedFile {
    fn create(c: &Connection, parent: &Path) -> Result<Self> {
        let path = parent.join(format!(".stology-generated-{}.pdf", Uuid::new_v4()));
        c.execute(
            "INSERT INTO generated_pdf_staging(path) VALUES(?1)",
            [path.to_string_lossy().as_ref()],
        )?;
        let file = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) => {
                c.execute(
                    "DELETE FROM generated_pdf_staging WHERE path=?1",
                    [path.to_string_lossy().as_ref()],
                )?;
                return Err(error.into());
            }
        };
        let m = file.metadata()?;
        let stage = Self {
            file,
            path,
            identity: (m.dev(), m.ino()),
            durable: false,
        };
        anyhow::ensure!(
            unsafe { libc::flock(stage.file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "PDF 임시 파일 잠금을 얻지 못했습니다"
        );
        let updated = c.execute(
            "UPDATE generated_pdf_staging SET dev=?2,ino=?3 WHERE path=?1",
            params![stage.path.to_string_lossy(), m.dev() as i64, m.ino() as i64],
        )?;
        anyhow::ensure!(
            updated == 1,
            "PDF 임시 파일 예약이 만료되었습니다. 다시 시도해 주세요"
        );
        Ok(stage)
    }
}
impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.durable
            && fs::symlink_metadata(&self.path)
                .is_ok_and(|m| m.is_file() && (m.dev(), m.ino()) == self.identity)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

// At most eight expired registered stages per request. No directory traversal
// or full-file reads run under the storage writer lock. Busy writers are
// rescheduled so one long conversion cannot starve other expired stages.
fn collect_orphan_stages(root: &Path, c: &Connection) -> Result<()> {
    let rows = {
        let mut q = c.prepare("SELECT path,dev,ino FROM generated_pdf_staging s WHERE created_at <= unixepoch()-86400 AND checked_at <= unixepoch()-600 AND NOT EXISTS (SELECT 1 FROM generated_pdf_intents i WHERE json_extract(i.payload,'$.temp')=s.path) ORDER BY checked_at,created_at LIMIT 8")?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (path, dev, ino) in rows {
        c.execute(
            "UPDATE generated_pdf_staging SET checked_at=unixepoch() WHERE path=?1",
            [&path],
        )?;
        let path = Path::new(&path);
        // The registry is internal, but validate its scope before any deletion.
        if !path.starts_with(root.join("workspaces"))
            || !path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".stology-generated-"))
            || storage::ensure_no_symlink(root, path).is_err()
        {
            continue;
        }
        let opened = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path);
        let file = match opened {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                c.execute(
                    "DELETE FROM generated_pdf_staging WHERE path=?1",
                    [path.to_string_lossy().as_ref()],
                )?;
                continue;
            }
            Err(_) => continue,
        };
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            continue;
        }
        let m = file.metadata()?;
        if !m.is_file()
            || dev.is_some_and(|v| v != m.dev() as i64)
            || ino.is_some_and(|v| v != m.ino() as i64)
        {
            continue;
        }
        // A crash between create_new and recording the inode still has the
        // precommitted private UUID reservation; age and flock protect writers.
        if !fs::symlink_metadata(path)
            .is_ok_and(|now| now.is_file() && (now.dev(), now.ino()) == (m.dev(), m.ino()))
        {
            continue;
        }
        if fs::remove_file(path).is_ok() {
            storage::sync_dir(path.parent().unwrap())?;
            c.execute(
                "DELETE FROM generated_pdf_staging WHERE path=?1",
                [path.to_string_lossy().as_ref()],
            )?;
        }
    }
    Ok(())
}

fn fingerprint(path: &Path) -> Result<Value> {
    let m = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "일반 파일만 사용할 수 있습니다"
    );
    Ok(json!([
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec()
    ]))
}
fn matches(path: &Path, expected: &Value) -> bool {
    fingerprint(path).is_ok_and(|v| v == *expected)
}
fn linked(root: &Path, c: &Connection, source_id: &str) -> Result<Option<Value>> {
    let id: Option<String> = c.query_row("SELECT p.pdf_id FROM generated_pdfs p JOIN documents d ON d.id=p.pdf_id WHERE p.source_id=?1 AND d.deleted=0", [source_id], |r|r.get(0)).optional()?;
    id.map(|id| storage::value(root, c, &storage::document(c, &id)?))
        .transpose()
}
fn intent_doc(v: &Value) -> Result<Doc> {
    Ok(Doc {
        id: storage::required(v, "id")?.into(),
        workspace_id: storage::required(v, "workspace_id")?.into(),
        title: storage::required(v, "title")?.into(),
        path: Some(storage::required(v, "path")?.into()),
        kind: "pdf".into(),
        version: 1,
        hash: storage::required(v, "hash")?.into(),
        body: String::new(),
    })
}
fn remove_intent(c: &Connection, intent: &Value) -> Result<()> {
    c.execute(
        "DELETE FROM generated_pdf_intents WHERE id=?1",
        [storage::required(intent, "id")?],
    )?;
    if let Some(temp) = intent["temp"].as_str() {
        c.execute("DELETE FROM generated_pdf_staging WHERE path=?1", [temp])?;
    }
    Ok(())
}
fn discard_staging(intent: &Value) {
    if let Some(path) = intent["temp"].as_str() {
        if matches(Path::new(path), &intent["prepared_identity"]) {
            let _ = fs::remove_file(path);
        }
    }
}

// Returns None when an uncommitted publication was safely cancelled. Recovery
// never overwrites an unrelated target, recreates a deleted source, or changes
// an existing PDF. The stable staged-file identity makes post-rename replay
// distinguishable from a filename collision without hashing under the DB lock.
fn finish(root: &Path, c: &Connection, intent: &Value) -> Result<Option<Value>> {
    let d = intent_doc(intent)?;
    if let Ok(existing) = storage::document(c, &d.id) {
        remove_intent(c, intent)?;
        discard_staging(intent);
        return Ok(Some(storage::value(root, c, &existing)?));
    }
    let target = storage::file_path(root, c, &d)?;
    let temp = Path::new(storage::required(intent, "temp")?);
    anyhow::ensure!(
        temp.parent() == target.parent(),
        "PDF 임시 파일 위치가 잘못되었습니다"
    );
    storage::ensure_no_symlink(&storage::base(root, c, &d.workspace_id)?, temp)?;
    let already_published = matches(&target, &intent["prepared_identity"]);
    if !already_published {
        let source = storage::document(c, storage::required(intent, "source_id")?);
        let source_valid = source.as_ref().is_ok_and(|s| {
            s.kind == "pptx"
                && s.hash == intent["source_hash"].as_str().unwrap_or("")
                && s.version == intent["source_version"].as_i64().unwrap_or(-1)
                && s.path.as_deref() == intent["source_path"].as_str()
                && storage::file_path(root, c, s)
                    .is_ok_and(|p| matches(&p, &intent["source_identity"]))
        });
        if !source_valid
            || !matches(temp, &intent["prepared_identity"])
            || fs::symlink_metadata(&target).is_ok()
        {
            remove_intent(c, intent)?;
            discard_staging(intent);
            return Ok(None);
        }
        // link(2) publishes without replacing even when another application
        // creates the destination after the existence check. Both names refer
        // to the same prepared inode until the journal transaction commits.
        if let Err(error) = fs::hard_link(temp, &target) {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                remove_intent(c, intent)?;
                discard_staging(intent);
                return Ok(None);
            }
            return Err(error).context("변환 PDF를 게시하지 못했습니다");
        }
        storage::sync_dir(target.parent().unwrap())?;
    }
    storage::insert(c, &d)?;
    c.execute("INSERT INTO generated_pdfs(source_id,pdf_id,source_hash) VALUES(?1,?2,?3) ON CONFLICT(source_id) DO UPDATE SET pdf_id=excluded.pdf_id,source_hash=excluded.source_hash",
        params![storage::required(intent,"source_id")?,d.id,storage::required(intent,"source_hash")?])?;
    remove_intent(c, intent)?;
    // Keeping the staging hardlink until metadata commits would only be a
    // temporary leak. Removing it here is safe because recovery identifies the
    // already-published target by inode after a transaction rollback.
    discard_staging(intent);
    Ok(Some(storage::value(root, c, &d)?))
}

pub(crate) fn recover(root: &Path, c: &Connection) -> Result<()> {
    let pending = {
        let mut q = c.prepare("SELECT payload FROM generated_pdf_intents ORDER BY id")?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for serialized in pending {
        finish(root, c, &serde_json::from_str::<Value>(&serialized)?)?;
    }
    // Deleted PDFs no longer provide a resume target. Deleting the PPTX does
    // not delete or alter its independently registered PDF.
    c.execute(
        "DELETE FROM generated_pdfs WHERE pdf_id IN (SELECT id FROM documents WHERE deleted=1)",
        [],
    )?;
    collect_orphan_stages(root, c)?;
    Ok(())
}

fn candidate(root: &Path, c: &Connection, source: &Doc) -> Result<(PathBuf, String, String)> {
    let source_path = storage::file_path(root, c, source)?;
    let dir = source_path.parent().context("PPTX 폴더가 없습니다")?;
    let stem = source_path
        .file_stem()
        .context("PPTX 이름이 없습니다")?
        .to_string_lossy();
    let parent = Path::new(source.path.as_deref().unwrap())
        .parent()
        .unwrap_or(Path::new(""));
    for index in 1.. {
        let title = if index == 1 {
            stem.to_string()
        } else {
            format!("{stem} ({index})")
        };
        let filename = format!("{title}.pdf");
        let relative = parent.join(&filename).to_string_lossy().into_owned();
        let target = dir.join(&filename);
        let occupied: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM documents WHERE workspace_id=?1 AND path=?2 AND deleted=0)",params![source.workspace_id,relative],|r|r.get(0))?;
        if !occupied && fs::symlink_metadata(&target).is_err() {
            return Ok((target, relative, title));
        }
    }
    unreachable!()
}

pub fn handle(root: &Path, request: &Value) -> Result<Value> {
    let op = storage::required(request, "op")?;
    let source_id = storage::required(request, "source_id")?;
    // Reconcile filesystem transactions before inspecting either side of the
    // link. This also updates the PPTX hash after an external modification.
    let source_value = storage::handle(root, &json!({"op":"source.get","id":source_id}))?;
    anyhow::ensure!(
        source_value["kind"] == "pptx",
        "PPTX 자료에서만 필기 PDF를 만들 수 있습니다"
    );
    let mut c = storage::connect(root)?;
    if op == "pdf.generated.get" {
        return Ok(json!({"document":linked(root,&c,source_id)?}));
    }
    anyhow::ensure!(
        op == "pdf.generated.register",
        "지원하지 않는 PDF 생성 명령"
    );
    let force_new = request["force_new"].as_bool().unwrap_or(false);
    if !force_new {
        if let Some(existing) = linked(root, &c, source_id)? {
            return Ok(existing);
        }
    }
    let expected_hash = storage::required(request, "source_hash")?;
    let source = storage::document(&c, source_id)?;
    anyhow::ensure!(
        source.hash == expected_hash,
        "PPTX가 변환 중 변경되었습니다. 다시 필기를 시작해 주세요"
    );
    let original = storage::file_path(root, &c, &source)?;
    let source_identity = fingerprint(&original)?;
    // Capture/hash files before the writer lock; the final fingerprint check
    // detects rename, replacement, truncation, and ordinary external edits.
    anyhow::ensure!(
        storage::digest(&storage::read_bytes(&original)?) == expected_hash,
        "PPTX가 변환 중 변경되었습니다"
    );
    anyhow::ensure!(
        matches(&original, &source_identity),
        "PPTX가 변환 중 변경되었습니다"
    );
    let input = Path::new(storage::required(request, "pdf_path")?);
    let bytes = storage::read_bytes(input)?;
    anyhow::ensure!(
        bytes.starts_with(b"%PDF-"),
        "변환 결과가 PDF 파일이 아닙니다"
    );
    let hash = storage::digest(&bytes);
    let parent = original.parent().unwrap();
    for _ in 0..16 {
        let mut prepared = StagedFile::create(&c, parent)?;
        prepared.file.write_all(&bytes)?;
        prepared.file.sync_all()?;
        let prepared_identity = fingerprint(&prepared.path)?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        recover(root, &tx)?;
        if !force_new {
            if let Some(existing) = linked(root, &tx, source_id)? {
                tx.commit()?;
                return Ok(existing);
            }
        }
        let latest = storage::document(&tx, source_id)?;
        anyhow::ensure!(
            latest.version == source.version
                && latest.path == source.path
                && latest.hash == source.hash
                && matches(&original, &source_identity),
            "PPTX가 변환 중 이동·수정·삭제되었습니다. 다시 열어 주세요"
        );
        let (_target, relative, title) = candidate(root, &tx, &latest)?;
        let id = Uuid::new_v4().to_string();
        let intent = json!({"id":id,"workspace_id":source.workspace_id,"title":title,"path":relative,"hash":hash,
            "source_id":source_id,"source_hash":expected_hash,"source_version":source.version,"source_path":source.path,
            "source_identity":source_identity,"temp":prepared.path,"prepared_identity":prepared_identity});
        tx.execute(
            "INSERT INTO generated_pdf_intents(id,payload) VALUES(?1,?2)",
            params![id, serde_json::to_string(&intent)?],
        )?;
        tx.commit()?;
        // An ordinary error after the durable intent must preserve its file.
        prepared.durable = true;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Another core request may have completed this durable intent already.
        let result = if let Ok(existing) = storage::document(&tx, &id) {
            Some(storage::value(root, &tx, &existing)?)
        } else {
            finish(root, &tx, &intent)?
        };
        tx.commit()?;
        if let Some(doc) = result {
            return Ok(doc);
        }
        // Final name collisions retry with the next suffix. Source mutations
        // fail the next validation, preserving both originals and old PDFs.
    }
    bail!("PDF 이름 충돌이 반복되었습니다. 다시 필기를 시작해 주세요")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(root: &Path) -> (Value, PathBuf) {
        let w = storage::handle(root, &json!({"op":"workspace.create","name":"강의"})).unwrap();
        let input = root.join("강의자료.pptx");
        fs::write(&input, b"pptx conversion fixture").unwrap();
        let source = storage::handle(
            root,
            &json!({"op":"material.import","workspace_id":w["id"],"path":input}),
        )
        .unwrap();
        let pdf = root.join("converted.pdf");
        fs::write(&pdf, b"%PDF-1.7\nfixture for registration boundary\n%%EOF").unwrap();
        (source, pdf)
    }
    fn register(root: &Path, s: &Value, p: &Path) -> Value {
        handle(root,&json!({"op":"pdf.generated.register","source_id":s["id"],"source_hash":s["hash"],"pdf_path":p})).unwrap()
    }
    fn pending(root: &Path, source: &Value, pdf: &Path) -> Value {
        let c = storage::connect(root).unwrap();
        let source = storage::document(&c, source["id"].as_str().unwrap()).unwrap();
        let original = storage::file_path(root, &c, &source).unwrap();
        let (_target, path, title) = candidate(root, &c, &source).unwrap();
        let mut tmp = tempfile::NamedTempFile::new_in(original.parent().unwrap()).unwrap();
        let bytes = fs::read(pdf).unwrap();
        tmp.write_all(&bytes).unwrap();
        tmp.as_file().sync_all().unwrap();
        let identity = fingerprint(tmp.path()).unwrap();
        let (_, temp) = tmp.keep().unwrap();
        let intent = json!({"id":Uuid::new_v4().to_string(),"workspace_id":source.workspace_id,
            "title":title,"path":path,"hash":storage::digest(&bytes),"source_id":source.id,
            "source_hash":source.hash,"source_version":source.version,"source_path":source.path,
            "source_identity":fingerprint(&original).unwrap(),"temp":temp,"prepared_identity":identity});
        c.execute(
            "INSERT INTO generated_pdf_intents(id,payload) VALUES(?1,?2)",
            params![intent["id"].as_str().unwrap(), intent.to_string()],
        )
        .unwrap();
        intent
    }
    #[test]
    fn durable_intent_recovers_before_and_after_file_publication() {
        for already_published in [false, true] {
            let t = tempfile::tempdir().unwrap();
            let (source, pdf) = setup(t.path());
            let intent = pending(t.path(), &source, &pdf);
            let c = storage::connect(t.path()).unwrap();
            let target = storage::file_path(t.path(), &c, &intent_doc(&intent).unwrap()).unwrap();
            if already_published {
                fs::hard_link(intent["temp"].as_str().unwrap(), &target).unwrap();
            }
            storage::handle(t.path(), &json!({"op":"source.get","id":source["id"]})).unwrap();
            let document = handle(
                t.path(),
                &json!({"op":"pdf.generated.get","source_id":source["id"]}),
            )
            .unwrap();
            assert_eq!(document["document"]["id"], intent["id"]);
            assert_eq!(fs::read(&target).unwrap(), fs::read(&pdf).unwrap());
            assert!(!Path::new(intent["temp"].as_str().unwrap()).exists());
            let count: i64 = c
                .query_row("SELECT COUNT(*) FROM generated_pdf_intents", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
            // Re-running recovery cannot add a second document or change bytes.
            let tx = c.unchecked_transaction().unwrap();
            recover(t.path(), &tx).unwrap();
            tx.commit().unwrap();
            assert_eq!(
                handle(
                    t.path(),
                    &json!({"op":"pdf.generated.get","source_id":source["id"]})
                )
                .unwrap()["document"]["id"],
                intent["id"]
            );
        }
    }
    #[test]
    fn recovery_preserves_target_collision_and_cancels_changed_source() {
        for collide in [false, true] {
            let t = tempfile::tempdir().unwrap();
            let (source, pdf) = setup(t.path());
            let intent = pending(t.path(), &source, &pdf);
            let c = storage::connect(t.path()).unwrap();
            let target = storage::file_path(t.path(), &c, &intent_doc(&intent).unwrap()).unwrap();
            if collide {
                fs::write(&target, b"external file").unwrap();
            } else {
                fs::write(source["absolute_path"].as_str().unwrap(), b"changed source").unwrap();
            }
            let tx = c.unchecked_transaction().unwrap();
            recover(t.path(), &tx).unwrap();
            tx.commit().unwrap();
            assert!(storage::document(&c, intent["id"].as_str().unwrap()).is_err());
            assert!(!Path::new(intent["temp"].as_str().unwrap()).exists());
            if collide {
                assert_eq!(fs::read(&target).unwrap(), b"external file");
            } else {
                assert!(!target.exists());
            }
        }
    }
    #[test]
    fn orphan_gc_keeps_active_stage_then_removes_it_after_writer_death() {
        let t = tempfile::tempdir().unwrap();
        let (source, _) = setup(t.path());
        let c = storage::connect(t.path()).unwrap();
        let parent = Path::new(source["absolute_path"].as_str().unwrap())
            .parent()
            .unwrap();
        let mut stage = StagedFile::create(&c, parent).unwrap();
        stage.file.write_all(b"partially converted PDF").unwrap();
        let path = stage.path.clone();
        c.execute(
            "UPDATE generated_pdf_staging SET created_at=unixepoch()-90000",
            [],
        )
        .unwrap();
        collect_orphan_stages(t.path(), &c).unwrap();
        assert!(
            path.exists(),
            "an old but active writer must remain protected by flock"
        );
        stage.durable = true;
        drop(stage); // model a killed process: file stays, lock is released
        c.execute("UPDATE generated_pdf_staging SET checked_at=0", [])
            .unwrap();
        collect_orphan_stages(t.path(), &c).unwrap();
        assert!(!path.exists());
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM generated_pdf_staging", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn orphan_gc_is_bounded_and_keeps_foreign_identity_and_published_file() {
        let t = tempfile::tempdir().unwrap();
        let (source, _) = setup(t.path());
        let c = storage::connect(t.path()).unwrap();
        let parent = Path::new(source["absolute_path"].as_str().unwrap())
            .parent()
            .unwrap();
        let mut orphan = StagedFile::create(&c, parent).unwrap();
        orphan.file.write_all(b"published PDF").unwrap();
        let published = parent.join("published.pdf");
        fs::hard_link(&orphan.path, &published).unwrap();
        let private = orphan.path.clone();
        orphan.durable = true;
        drop(orphan);
        let mut foreign = StagedFile::create(&c, parent).unwrap();
        let foreign_path = foreign.path.clone();
        foreign.durable = true;
        drop(foreign);
        let replacement = parent.join("replacement.tmp");
        fs::write(&replacement, b"foreign file").unwrap();
        fs::rename(&replacement, &foreign_path).unwrap();
        for _ in 0..10 {
            let mut stage = StagedFile::create(&c, parent).unwrap();
            stage.durable = true;
        }
        c.execute(
            "UPDATE generated_pdf_staging SET created_at=unixepoch()-90000",
            [],
        )
        .unwrap();
        collect_orphan_stages(t.path(), &c).unwrap();
        assert!(!private.exists());
        assert_eq!(fs::read(published).unwrap(), b"published PDF");
        assert_eq!(fs::read(foreign_path).unwrap(), b"foreign file");
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM generated_pdf_staging WHERE checked_at>0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM generated_pdf_staging WHERE checked_at=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            4
        );
    }
    #[test]
    fn collision_preserved_and_reentry_reuses_link_not_bytes() {
        let t = tempfile::tempdir().unwrap();
        let (s, p) = setup(t.path());
        let original = Path::new(s["absolute_path"].as_str().unwrap());
        let collision = original.with_extension("pdf");
        fs::write(&collision, b"existing independent pdf").unwrap();
        let d = register(t.path(), &s, &p);
        assert_eq!(d["title"], "강의자료 (2)");
        assert_ne!(d["id"], s["id"]);
        assert_eq!(fs::read(&collision).unwrap(), b"existing independent pdf");
        assert_eq!(register(t.path(), &s, &p)["id"], d["id"]);
        assert!(!t.path().join("versions").exists());
        let forced = handle(t.path(),&json!({"op":"pdf.generated.register","source_id":s["id"],"source_hash":s["hash"],"pdf_path":p,"force_new":true})).unwrap();
        assert_ne!(forced["id"], d["id"]);
        assert_eq!(forced["hash"], d["hash"]);
    }
    #[test]
    fn independent_pdf_survives_source_delete_and_rename_keeps_link() {
        let t = tempfile::tempdir().unwrap();
        let (s, p) = setup(t.path());
        let d = register(t.path(), &s, &p);
        let renamed = storage::handle(t.path(),&json!({"op":"note.rename","id":d["id"],"expected_version":d["version"],"title":"필기 노트"})).unwrap();
        let linked = handle(
            t.path(),
            &json!({"op":"pdf.generated.get","source_id":s["id"]}),
        )
        .unwrap();
        assert_eq!(
            linked["document"]["absolute_path"],
            renamed["absolute_path"]
        );
        storage::handle(t.path(), &json!({"op":"material.delete","id":s["id"]})).unwrap();
        assert!(Path::new(renamed["absolute_path"].as_str().unwrap()).is_file());
        assert!(storage::handle(t.path(), &json!({"op":"note.read","id":d["id"]})).is_ok());
    }
    #[test]
    fn deleting_pdf_unlinks_and_source_change_does_not_replace_annotations() {
        let t = tempfile::tempdir().unwrap();
        let (s, p) = setup(t.path());
        let d = register(t.path(), &s, &p);
        fs::write(s["absolute_path"].as_str().unwrap(), b"new pptx content").unwrap();
        assert_eq!(
            handle(
                t.path(),
                &json!({"op":"pdf.generated.get","source_id":s["id"]})
            )
            .unwrap()["document"]["id"],
            d["id"]
        );
        assert_eq!(register(t.path(), &s, &p)["id"], d["id"]);
        storage::handle(t.path(), &json!({"op":"material.delete","id":d["id"]})).unwrap();
        assert!(handle(
            t.path(),
            &json!({"op":"pdf.generated.get","source_id":s["id"]})
        )
        .unwrap()["document"]
            .is_null());
        assert!(handle(t.path(),&json!({"op":"pdf.generated.register","source_id":s["id"],"source_hash":s["hash"],"pdf_path":p})).is_err());
    }
}
