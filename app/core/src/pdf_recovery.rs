//! Durable, bounded pending PDF commands. The PDF remains the annotation source of truth.
//! Journal writes use a separate SQLite file so importing a large material cannot block ink ACKs.
use crate::storage;
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct JournalEntry {
    pub seq: u64,
    pub command: Value,
}
#[derive(Clone, Debug)]
pub struct Session {
    pub document: Value,
    pub path: PathBuf,
    pub hash: String,
    pub version: i64,
    pub pdf_seq: u64,
    pub journal_seq: u64,
    pub commands: Vec<JournalEntry>,
}
#[derive(Clone, Debug)]
pub struct Published {
    pub hash: String,
    pub version: i64,
    pub pdf_seq: u64,
    pub path: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Identity {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    nanos: i64,
    ctime: i64,
    cnanos: i64,
}
fn identity(p: &Path) -> Result<Identity> {
    let m = fs::symlink_metadata(p)?;
    ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "PDF는 일반 파일이어야 합니다"
    );
    Ok(Identity {
        dev: m.dev(),
        ino: m.ino(),
        len: m.len(),
        mtime: m.mtime(),
        nanos: m.mtime_nsec(),
        ctime: m.ctime(),
        cnanos: m.ctime_nsec(),
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Intent {
    id: String,
    path: PathBuf,
    temp: PathBuf,
    base_hash: String,
    target_hash: String,
    base_version: i64,
    through_seq: u64,
    base_identity: Identity,
    target_identity: Identity,
}

pub(crate) fn init_schema(c: &Connection) -> Result<()> {
    let initialized:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='pdf_publish_intents')",[],|r|r.get(0))?;
    if initialized {
        return Ok(());
    }
    c.execute_batch("CREATE TABLE IF NOT EXISTS feature_schema(feature TEXT PRIMARY KEY,version INTEGER NOT NULL);
        INSERT OR IGNORE INTO feature_schema VALUES('pdf_annotations',1);
        CREATE TABLE IF NOT EXISTS pdf_state(document_id TEXT PRIMARY KEY,pdf_seq INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS pdf_publish_intents(document_id TEXT PRIMARY KEY,payload TEXT NOT NULL);")?;
    Ok(())
}
// Connection close races with WAL open in the bundled macOS SQLite VFS.
// Serialize the short journal-only calls (including connection destruction); never
// hold this lock during PDF serialization/hash or a storage writer acquisition.
static JOURNAL_ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct JournalConnection {
    connection: Connection,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl std::ops::Deref for JournalConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.connection
    }
}
impl std::ops::DerefMut for JournalConnection {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}
fn journal(root: &Path) -> Result<JournalConnection> {
    let guard = JOURNAL_ACCESS
        .lock()
        .map_err(|_| anyhow::anyhow!("필기 저장 잠금 오류"))?;
    fs::create_dir_all(root)?;
    let c = Connection::open(root.join("pdf-journal.sqlite"))?;
    c.busy_timeout(std::time::Duration::from_secs(2))?;
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS schema_version(version INTEGER NOT NULL);
        INSERT INTO schema_version SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM schema_version);
        CREATE TABLE IF NOT EXISTS sessions(document_id TEXT PRIMARY KEY,base_hash TEXT NOT NULL,pdf_seq INTEGER NOT NULL,journal_seq INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS commands(document_id TEXT NOT NULL,seq INTEGER NOT NULL,command TEXT NOT NULL,PRIMARY KEY(document_id,seq));")?;
    Ok(JournalConnection {
        connection: c,
        _guard: guard,
    })
}
fn state(c: &Connection, id: &str) -> Result<Option<(String, u64, u64)>> {
    Ok(c.query_row(
        "SELECT base_hash,pdf_seq,journal_seq FROM sessions WHERE document_id=?1",
        [id],
        |r| {
            Ok((
                r.get(0)?,
                r.get::<_, i64>(1)? as u64,
                r.get::<_, i64>(2)? as u64,
            ))
        },
    )
    .optional()?)
}
fn pdf_seq(c: &Connection, id: &str) -> Result<u64> {
    Ok(c.query_row(
        "SELECT pdf_seq FROM pdf_state WHERE document_id=?1",
        [id],
        |r| r.get::<_, i64>(0),
    )
    .optional()?
    .unwrap_or(0) as u64)
}
// This also repairs the independent journal DB after a crash between storage commit and GC.
fn acknowledge(root: &Path, id: &str, hash: &str, through: u64) -> Result<()> {
    let mut c = journal(root)?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "DELETE FROM commands WHERE document_id=?1 AND seq<=?2",
        params![id, through as i64],
    )?;
    tx.execute("UPDATE sessions SET base_hash=?2,pdf_seq=MAX(pdf_seq,?3),journal_seq=MAX(journal_seq,?3) WHERE document_id=?1",params![id,hash,through as i64])?;
    tx.commit()?;
    Ok(())
}
/// Called before accepting an external change. Never silently rebase uncommitted annotations.
pub(crate) fn check_external_change(
    root: &Path,
    c: &Connection,
    id: &str,
    new_hash: &str,
) -> Result<()> {
    if !root.join("pdf-journal.sqlite").exists() {
        return Ok(());
    }
    if let Some((base, _, last)) = state(&*journal(root)?, id)? {
        ensure!(base==new_hash || last<=pdf_seq(c,id)?, "PDF가 외부에서 변경되었습니다. 미반영 필기를 보존했으므로 별도 PDF 저장 또는 다시 열기가 필요합니다");
    }
    Ok(())
}
pub fn open_session(root: &Path, id: &str) -> Result<Session> {
    let document = storage::handle(root, &json!({"op":"source.get","id":id}))?;
    ensure!(document["kind"] == "pdf", "PDF 자료만 필기할 수 있습니다");
    let path = PathBuf::from(
        document["absolute_path"]
            .as_str()
            .context("PDF 경로가 없습니다")?,
    );
    let hash = document["hash"]
        .as_str()
        .context("PDF 해시가 없습니다")?
        .to_owned();
    let version = document["version"]
        .as_i64()
        .context("PDF 버전이 없습니다")?;
    let seq = pdf_seq(&storage::connect(root)?, id)?;
    let mut c = journal(root)?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some((base, _, last)) = state(&tx, id)? {
        ensure!(
            base == hash || last <= seq,
            "PDF와 복구 기록의 기준 버전이 다릅니다. 미반영 필기를 보존했습니다"
        );
    }
    tx.execute("INSERT INTO sessions VALUES(?1,?2,?3,?3) ON CONFLICT(document_id) DO UPDATE SET base_hash=excluded.base_hash,pdf_seq=MAX(sessions.pdf_seq,excluded.pdf_seq),journal_seq=MAX(sessions.journal_seq,excluded.pdf_seq)",params![id,hash,seq as i64])?;
    tx.execute(
        "DELETE FROM commands WHERE document_id=?1 AND seq<=?2",
        params![id, seq as i64],
    )?;
    let last = state(&tx, id)?.unwrap().2;
    let commands = {
        let mut s = tx.prepare(
            "SELECT seq,command FROM commands WHERE document_id=?1 AND seq>?2 ORDER BY seq",
        )?;
        let rows = s
            .query_map(params![id, seq as i64], |r| {
                Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(seq, command)| {
                Ok(JournalEntry {
                    seq,
                    command: serde_json::from_str(&command)?,
                })
            })
            .collect::<Result<Vec<_>>>()?
    };
    tx.commit()?;
    Ok(Session {
        document,
        path,
        hash,
        version,
        pdf_seq: seq,
        journal_seq: last,
        commands,
    })
}
/// Read a conflicted session for explicit Save As/discard only. Never rebases or
/// clears commands; normal publish still rejects a changed working file.
pub fn rescue_session(root: &Path, id: &str) -> Result<Session> {
    let c = storage::connect(root)?;
    let d = storage::document(&c, id)?;
    ensure!(d.kind == "pdf", "PDF 자료만 복구할 수 있습니다");
    let path = storage::file_path(root, &c, &d)?;
    let document = storage::value(root, &c, &d)?;
    let done = pdf_seq(&c, id)?;
    let j = journal(root)?;
    let (hash, _, last) = state(&j, id)?.context("보존된 필기 복구 기록이 없습니다")?;
    let mut statement =
        j.prepare("SELECT seq,command FROM commands WHERE document_id=?1 AND seq>?2 ORDER BY seq")?;
    let rows = statement
        .query_map(params![id, done as i64], |r| {
            Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let commands = rows
        .into_iter()
        .map(|(seq, command)| {
            Ok(JournalEntry {
                seq,
                command: serde_json::from_str(&command)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Session {
        document,
        path,
        hash,
        version: d.version,
        pdf_seq: done,
        journal_seq: last,
        commands,
    })
}
/// Append exactly the next command. Retrying an identical committed command is safe.
pub fn append(root: &Path, id: &str, seq: u64, command: &Value) -> Result<u64> {
    ensure!(seq > 0 && seq <= i64::MAX as u64, "잘못된 필기 순서입니다");
    let encoded = serde_json::to_string(command)?;
    ensure!(
        encoded.len() <= 4 * 1024 * 1024,
        "한 필기 명령이 4 MiB 제한을 초과합니다"
    );
    let mut c = journal(root)?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (_, done, last) = state(&tx, id)?.context("필기 세션을 먼저 열어 주세요")?;
    if seq <= done {
        return Ok(last);
    }
    if seq <= last {
        let existing: String = tx.query_row(
            "SELECT command FROM commands WHERE document_id=?1 AND seq=?2",
            params![id, seq as i64],
            |r| r.get(0),
        )?;
        ensure!(existing == encoded, "같은 순서의 필기 명령이 다릅니다");
        return Ok(last);
    }
    ensure!(seq == last + 1, "필기 순서가 누락되었습니다");
    tx.execute(
        "INSERT INTO commands VALUES(?1,?2,?3)",
        params![id, seq as i64, encoded],
    )?;
    tx.execute(
        "UPDATE sessions SET journal_seq=?2 WHERE document_id=?1",
        params![id, seq as i64],
    )?;
    tx.commit()?;
    Ok(seq)
}
fn verify_doc(root: &Path, c: &Connection, intent: &Intent) -> Result<storage::Doc> {
    let d = storage::document(c, &intent.id)?;
    ensure!(
        d.kind == "pdf"
            && d.version == intent.base_version
            && d.hash == intent.base_hash
            && storage::file_path(root, c, &d)? == intent.path,
        "PDF가 이동·삭제·수정되었습니다. 필기를 보존하고 저장을 중단했습니다"
    );
    Ok(d)
}
fn finalize(c: &Connection, intent: &Intent) -> Result<()> {
    c.execute("UPDATE documents SET hash=?2,version=version+CASE WHEN hash<>?2 THEN 1 ELSE 0 END,updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND deleted=0",params![intent.id,intent.target_hash])?;
    c.execute("INSERT INTO pdf_state VALUES(?1,?2) ON CONFLICT(document_id) DO UPDATE SET pdf_seq=MAX(pdf_seq,excluded.pdf_seq)",params![intent.id,intent.through_seq as i64])?;
    c.execute(
        "DELETE FROM pdf_publish_intents WHERE document_id=?1",
        [&intent.id],
    )?;
    Ok(())
}
fn prepare(
    root: &Path,
    id: &str,
    expected_version: i64,
    base_hash: &str,
    through_seq: u64,
    temp_path: &Path,
) -> Result<Intent> {
    let c = storage::connect(root)?;
    let d = storage::document(&c, id)?;
    acknowledge(root, id, &d.hash, pdf_seq(&c, id)?)?;
    let path = storage::file_path(root, &c, &d)?;
    ensure!(
        temp_path != path && temp_path.parent() == path.parent(),
        "임시 PDF는 작업 파일과 같은 폴더에 있어야 합니다"
    );
    let base_identity = identity(&path)?;
    let actual_hash = storage::digest(&fs::read(&path)?);
    ensure!(
        actual_hash == base_hash && identity(&path)? == base_identity,
        "PDF가 외부에서 변경되었습니다. 필기를 보존했습니다"
    );
    let target_identity = identity(temp_path)?;
    ensure!(target_identity.len <= storage::MAX_FILE_BYTES,
        "PDF 저장 결과가 현재 자료 크기 제한 256 MiB를 초과합니다. 기존 PDF와 미반영 필기를 보존했습니다");
    let bytes = fs::read(temp_path)?;
    ensure!(
        bytes.starts_with(b"%PDF-") && bytes.windows(5).any(|s| s == b"%%EOF"),
        "임시 PDF 형식이 올바르지 않습니다"
    );
    let target_hash = storage::digest(&bytes);
    fs::File::open(temp_path)?.sync_all()?;
    ensure!(
        identity(temp_path)? == target_identity,
        "임시 PDF가 작성 중 변경되었습니다"
    );
    let intent = Intent {
        id: id.into(),
        path,
        temp: temp_path.to_path_buf(),
        base_hash: base_hash.into(),
        target_hash,
        base_version: expected_version,
        through_seq,
        base_identity,
        target_identity,
    };
    let mut c = storage::connect(root)?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    verify_doc(root, &tx, &intent)?;
    ensure!(
        identity(&intent.path)? == intent.base_identity,
        "PDF가 외부에서 변경되었습니다"
    );
    let (base, done, last) = state(&*journal(root)?, id)?.context("필기 세션이 없습니다")?;
    ensure!(
        base == base_hash && through_seq > done && through_seq <= last,
        "PDF에 반영할 필기가 복구 기록에 없습니다"
    );
    tx.execute(
        "INSERT INTO pdf_publish_intents VALUES(?1,?2)",
        params![id, serde_json::to_string(&intent)?],
    )?;
    tx.commit()?;
    Ok(intent)
}
/// Caller serializes and validates a private PDFDocument on its worker thread.
/// Hashing and fsync occur before the short metadata transaction.
pub fn publish(
    root: &Path,
    id: &str,
    expected_version: i64,
    base_hash: &str,
    through_seq: u64,
    temp_path: &Path,
) -> Result<Published> {
    // Reconcile a previous interrupted attempt before validating this attempt.
    storage::handle(root, &json!({"op":"source.get","id":id}))?;
    let c = storage::connect(root)?;
    if pdf_seq(&c, id)? >= through_seq {
        let d = storage::document(&c, id)?;
        return Ok(Published {
            hash: d.hash.clone(),
            version: d.version,
            pdf_seq: pdf_seq(&c, id)?,
            path: storage::file_path(root, &c, &d)?,
        });
    }
    drop(c);
    let intent = prepare(
        root,
        id,
        expected_version,
        base_hash,
        through_seq,
        temp_path,
    )?;
    let mut c = storage::connect(root)?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another process may already have recovered this durable intent.
    if pdf_seq(&tx, id)? < through_seq {
        verify_doc(root, &tx, &intent)?;
        ensure!(
            identity(&intent.path)? == intent.base_identity
                && identity(&intent.temp)? == intent.target_identity,
            "PDF 파일이 변경되어 저장을 중단했습니다"
        );
        fs::rename(&intent.temp, &intent.path)?;
        storage::sync_dir(intent.path.parent().unwrap())?;
        finalize(&tx, &intent)?;
    }
    let d = storage::document(&tx, id)?;
    let result = Published {
        hash: d.hash.clone(),
        version: d.version,
        pdf_seq: pdf_seq(&tx, id)?,
        path: storage::file_path(root, &tx, &d)?,
    };
    tx.commit()?;
    // Publication is complete even when journal GC is temporarily unavailable.
    // The next open/publish repairs GC from authoritative storage pdf_seq.
    let _ = acknowledge(root, id, &result.hash, result.pdf_seq);
    Ok(result)
}
/// Reconcile durable intent before refresh can mistake our replacement for an external edit.
/// On recovery only, full hashes may be required to identify the interrupted side of rename.
pub(crate) fn recover(root: &Path, c: &Connection) -> Result<()> {
    let payloads = {
        let mut s = c.prepare("SELECT payload FROM pdf_publish_intents")?;
        let rows = s
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for payload in payloads {
        let intent: Intent = serde_json::from_str(&payload)?;
        let d = match storage::document(c, &intent.id) {
            Ok(d) => d,
            Err(_) => {
                c.execute(
                    "DELETE FROM pdf_publish_intents WHERE document_id=?1",
                    [&intent.id],
                )?;
                let _ = fs::remove_file(&intent.temp);
                continue;
            }
        };
        let path = storage::file_path(root, c, &d)?;
        let current_hash = fs::read(&path).map(|b| storage::digest(&b)).ok();
        if current_hash.as_deref() == Some(&intent.target_hash)
            && path == intent.path
            && (d.hash == intent.base_hash || d.hash == intent.target_hash)
        {
            finalize(c, &intent)?;
            let _ = acknowledge(root, &intent.id, &intent.target_hash, intent.through_seq);
            let _ = fs::remove_file(&intent.temp);
        } else if verify_doc(root, c, &intent).is_ok()
            && current_hash.as_deref() == Some(&intent.base_hash)
        {
            if identity(&intent.temp).ok().as_ref() == Some(&intent.target_identity)
                && fs::read(&intent.temp).is_ok_and(|b| storage::digest(&b) == intent.target_hash)
            {
                fs::rename(&intent.temp, &intent.path)?;
                storage::sync_dir(intent.path.parent().unwrap())?;
                finalize(c, &intent)?;
                let _ = acknowledge(root, &intent.id, &intent.target_hash, intent.through_seq);
            } else {
                // Invalid/missing prepared file: retain ordered commands and retry serialization.
                c.execute(
                    "DELETE FROM pdf_publish_intents WHERE document_id=?1",
                    [&intent.id],
                )?;
                let _ = fs::remove_file(&intent.temp);
            }
        } else {
            // A conflicting external edit must never be overwritten. Keep the journal.
            c.execute(
                "DELETE FROM pdf_publish_intents WHERE document_id=?1",
                [&intent.id],
            )?;
            let _ = fs::remove_file(&intent.temp);
        }
    }
    Ok(())
}

/// A deleted document ID is never reused. Remove its pending commands after deletion commits.
pub(crate) fn forget(root: &Path, id: &str) -> Result<()> {
    if !root.join("pdf-journal.sqlite").exists() {
        return Ok(());
    }
    let mut c = journal(root)?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM commands WHERE document_id=?1", [id])?;
    tx.execute("DELETE FROM sessions WHERE document_id=?1", [id])?;
    tx.commit()?;
    Ok(())
}

/// Explicit user choice only, after the caller has stopped this document's save worker.
/// Does not replace the user's file, including when it was modified externally.
pub fn discard_recovery(root: &Path, id: &str) -> Result<()> {
    let mut c = storage::connect(root)?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let payload: Option<String> = tx
        .query_row(
            "SELECT payload FROM pdf_publish_intents WHERE document_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let mut temp = None;
    if let Some(payload) = payload {
        let intent: Intent = serde_json::from_str(&payload)?;
        if storage::document(&tx, id).is_ok_and(|d| {
            d.hash == intent.base_hash
                && storage::file_path(root, &tx, &d).ok().as_ref() == Some(&intent.path)
        }) && fs::read(&intent.path).is_ok_and(|b| storage::digest(&b) == intent.target_hash)
        {
            // A prior replacement already happened; reconcile its metadata only.
            finalize(&tx, &intent)?;
        }
        tx.execute("DELETE FROM pdf_publish_intents WHERE document_id=?1", [id])?;
        temp = Some(intent.temp);
    }
    tx.commit()?;
    forget(root, id)?;
    if let Some(path) = temp {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, String) {
        let t = tempfile::tempdir().unwrap();
        let w = storage::handle(t.path(), &json!({"op":"workspace.create","name":"PDF"})).unwrap();
        let p = t.path().join("input.pdf");
        fs::write(&p, b"%PDF-1.7\noriginal\n%%EOF").unwrap();
        let d = storage::handle(
            t.path(),
            &json!({"op":"material.import","workspace_id":w["id"],"path":p}),
        )
        .unwrap();
        (t, d["id"].as_str().unwrap().to_owned())
    }
    fn edited(s: &Session) -> PathBuf {
        let p = s.path.parent().unwrap().join(".test-pdf.tmp");
        fs::write(&p, b"%PDF-1.7\nannotated\n%%EOF").unwrap();
        p
    }
    #[test]
    fn ordered_idempotent_journal_and_publication() {
        let (t, id) = fixture();
        let s = open_session(t.path(), &id).unwrap();
        append(t.path(), &id, 1, &json!({"id":"uuid","after":"box"})).unwrap();
        append(t.path(), &id, 1, &json!({"id":"uuid","after":"box"})).unwrap();
        assert!(append(t.path(), &id, 1, &json!({"different":true})).is_err());
        assert!(append(t.path(), &id, 3, &json!({})).is_err());
        append(t.path(), &id, 2, &json!({"after":"move"})).unwrap();
        let r = publish(t.path(), &id, s.version, &s.hash, 1, &edited(&s)).unwrap();
        assert_eq!(r.version, s.version + 1);
        let restored = open_session(t.path(), &id).unwrap();
        assert_eq!(restored.pdf_seq, 1);
        assert_eq!(restored.journal_seq, 2);
        assert_eq!(restored.commands.len(), 1);
        assert!(!t.path().join("versions").exists());
    }
    #[test]
    fn recover_before_and_after_atomic_replacement() {
        for after_rename in [false, true] {
            let (t, id) = fixture();
            let s = open_session(t.path(), &id).unwrap();
            append(t.path(), &id, 1, &json!({"id":"uuid"})).unwrap();
            let i = prepare(t.path(), &id, s.version, &s.hash, 1, &edited(&s)).unwrap();
            if after_rename {
                fs::rename(&i.temp, &i.path).unwrap();
            }
            let restored = open_session(t.path(), &id).unwrap();
            assert_eq!(restored.pdf_seq, 1);
            assert!(restored.commands.is_empty());
            assert_eq!(restored.hash, i.target_hash);
            assert_eq!(restored.version, s.version + 1);
            assert_eq!(
                open_session(t.path(), &id).unwrap().version,
                restored.version
            );
        }
    }
    #[test]
    fn external_change_and_delete_do_not_resurrect() {
        let (t, id) = fixture();
        let s = open_session(t.path(), &id).unwrap();
        append(t.path(), &id, 1, &json!({})).unwrap();
        fs::write(&s.path, b"%PDF-1.7\nexternal\n%%EOF").unwrap();
        assert!(publish(t.path(), &id, s.version, &s.hash, 1, &edited(&s)).is_err());
        assert!(open_session(t.path(), &id).is_err());
        let rescued = rescue_session(t.path(), &id).unwrap();
        assert_eq!(rescued.commands.len(), 1);
        assert_eq!(rescued.hash, s.hash);
        assert_eq!(fs::read(&s.path).unwrap(), b"%PDF-1.7\nexternal\n%%EOF");
        storage::handle(t.path(), &json!({"op":"material.delete","id":id})).unwrap();
        assert!(publish(t.path(), &id, s.version, &s.hash, 1, &edited(&s)).is_err());
        assert!(!s.path.exists());
    }
    #[test]
    fn corrupted_prepared_file_preserves_commands() {
        let (t, id) = fixture();
        let s = open_session(t.path(), &id).unwrap();
        append(t.path(), &id, 1, &json!({})).unwrap();
        let i = prepare(t.path(), &id, s.version, &s.hash, 1, &edited(&s)).unwrap();
        fs::write(&i.temp, b"broken").unwrap();
        let restored = open_session(t.path(), &id).unwrap();
        assert_eq!(restored.hash, s.hash);
        assert_eq!(restored.commands.len(), 1);
        assert_eq!(restored.pdf_seq, 0);
    }
    #[test]
    fn journal_ack_does_not_wait_for_storage_writer() {
        let (t, id) = fixture();
        open_session(t.path(), &id).unwrap();
        let mut c = storage::connect(t.path()).unwrap();
        let _tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let started = std::time::Instant::now();
        append(t.path(), &id, 1, &json!({})).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(200));
    }
    #[test]
    fn recovery_survives_outer_request_rollback() {
        let (t, id) = fixture();
        let s = open_session(t.path(), &id).unwrap();
        append(t.path(), &id, 1, &json!({"id":"stable"})).unwrap();
        let i = prepare(t.path(), &id, s.version, &s.hash, 1, &edited(&s)).unwrap();
        fs::rename(&i.temp, &i.path).unwrap();
        assert!(
            storage::handle(t.path(), &json!({"op":"workspace.create","name":"../bad"})).is_err()
        );
        let restored = open_session(t.path(), &id).unwrap();
        assert_eq!(restored.pdf_seq, 1);
        assert_eq!(restored.hash, i.target_hash);
        assert!(restored.commands.is_empty());
    }
    #[test]
    fn publication_repairs_skipped_gc_before_next_save() {
        let (t, id) = fixture();
        let s = open_session(t.path(), &id).unwrap();
        append(t.path(), &id, 1, &json!({})).unwrap();
        let i = prepare(t.path(), &id, s.version, &s.hash, 1, &edited(&s)).unwrap();
        let mut c = storage::connect(t.path()).unwrap();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        fs::rename(&i.temp, &i.path).unwrap();
        finalize(&tx, &i).unwrap();
        tx.commit().unwrap();
        // Simulate GC failing after a successful metadata commit.
        append(t.path(), &id, 2, &json!({"next":true})).unwrap();
        let p = s.path.parent().unwrap().join(".next.tmp");
        fs::write(&p, b"%PDF-1.7\nnext edit\n%%EOF").unwrap();
        let r = publish(t.path(), &id, s.version + 1, &i.target_hash, 2, &p).unwrap();
        assert_eq!(r.pdf_seq, 2);
        assert!(open_session(t.path(), &id).unwrap().commands.is_empty());
    }
    #[test]
    fn append_during_publication_keeps_newer_commands() {
        let (t, id) = fixture();
        let s = open_session(t.path(), &id).unwrap();
        append(t.path(), &id, 1, &json!({})).unwrap();
        let root = t.path().to_path_buf();
        let other = id.clone();
        let handle = std::thread::spawn(move || {
            for seq in 2..=32 {
                append(&root, &other, seq, &json!({"seq":seq})).unwrap();
            }
        });
        publish(t.path(), &id, s.version, &s.hash, 1, &edited(&s)).unwrap();
        handle.join().unwrap();
        let restored = open_session(t.path(), &id).unwrap();
        assert_eq!(restored.pdf_seq, 1);
        assert_eq!(restored.journal_seq, 32);
        assert_eq!(restored.commands.len(), 31);
    }
    #[test]
    fn explicit_discard_preserves_external_file() {
        let (t, id) = fixture();
        let s = open_session(t.path(), &id).unwrap();
        append(t.path(), &id, 1, &json!({})).unwrap();
        let i = prepare(t.path(), &id, s.version, &s.hash, 1, &edited(&s)).unwrap();
        let external = b"%PDF-1.7\nexternal version\n%%EOF";
        fs::write(&s.path, external).unwrap();
        discard_recovery(t.path(), &id).unwrap();
        assert_eq!(fs::read(&s.path).unwrap(), external);
        assert!(!i.temp.exists());
        let restored = open_session(t.path(), &id).unwrap();
        assert!(restored.commands.is_empty());
        assert_ne!(restored.hash, s.hash);
    }
    #[test]
    fn oversized_pdf_output_preserves_reopenable_original() {
        let (t, id) = fixture();
        let s = open_session(t.path(), &id).unwrap();
        append(t.path(), &id, 1, &json!({})).unwrap();
        let p = edited(&s);
        fs::OpenOptions::new()
            .write(true)
            .open(&p)
            .unwrap()
            .set_len(storage::MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(publish(t.path(), &id, s.version, &s.hash, 1, &p)
            .unwrap_err()
            .to_string()
            .contains("256 MiB"));
        let reopened = open_session(t.path(), &id).unwrap();
        assert_eq!(reopened.hash, s.hash);
        assert_eq!(reopened.commands.len(), 1);
    }
    #[test]
    fn journal_roundtrip_preserves_pdf_coordinates_exactly() {
        let (t, id) = fixture();
        open_session(t.path(), &id).unwrap();
        let x = 193.33170656813786_f64;
        let command = json!({"annotation_id":"moved-box","after":{"bounds":[x,77.17316425120773_f64,100.0,51.00000000000001_f64]}});
        append(t.path(), &id, 1, &command).unwrap();
        let reopened = open_session(t.path(), &id).unwrap();
        assert_eq!(reopened.commands[0].command, command);
        assert_eq!(
            reopened.commands[0].command["after"]["bounds"][0]
                .as_f64()
                .unwrap()
                .to_bits(),
            x.to_bits()
        );
        // A retry through an independently decoded RPC payload is the same command.
        let serialized = serde_json::to_string(&command).unwrap();
        let replay: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(append(t.path(), &id, 1, &replay).unwrap(), 1);
    }
}
