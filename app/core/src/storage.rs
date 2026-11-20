//! Local documents: SQLite metadata, UTF-8 Markdown files and immutable input snapshots.
//! Every mutation takes an IMMEDIATE SQLite transaction; a deleted ID is never reused.
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

#[path = "../../shared/image_markdown.rs"]
#[allow(dead_code)]
mod image_markdown;
#[path = "workspace_delete.rs"]
mod workspace_delete;

pub(crate) const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Doc {
    pub(crate) id: String,
    pub(crate) workspace_id: String,
    pub(crate) title: String,
    pub(crate) path: Option<String>,
    pub(crate) kind: String,
    pub(crate) version: i64,
    pub(crate) hash: String,
    pub(crate) body: String,
}

pub(crate) fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("필수 값이 없습니다: {key}"))
}
fn optional<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub(crate) fn name(value: &str) -> Result<String> {
    let n = value.trim();
    if n.is_empty()
        || n == "."
        || n == ".."
        || n.len() > 240
        || n.chars().any(|c| c.is_control() || "/\\:".contains(c))
    {
        bail!("이름은 비어 있지 않아야 하며 /, \\, : 또는 제어 문자를 포함할 수 없습니다");
    }
    Ok(n.to_owned())
}
pub(crate) fn relative(value: &str) -> Result<PathBuf> {
    let p = Path::new(value);
    if p.components().any(|c| !matches!(c, Component::Normal(_))) && !value.is_empty() {
        bail!("폴더 경로는 과목 안의 상대 경로여야 합니다");
    }
    for c in p.components() {
        if let Component::Normal(n) = c {
            name(&n.to_string_lossy())?;
        }
    }
    Ok(p.to_path_buf())
}
pub(crate) fn connect(root: &Path) -> Result<Connection> {
    fs::create_dir_all(root)?;
    let c = Connection::open(root.join("storage.sqlite"))?;
    c.busy_timeout(std::time::Duration::from_secs(10))?;
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
      CREATE TABLE IF NOT EXISTS workspace_cleanup(path TEXT PRIMARY KEY);
      CREATE TABLE IF NOT EXISTS workspaces(id TEXT PRIMARY KEY,name TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
      CREATE TABLE IF NOT EXISTS documents(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id),title TEXT NOT NULL,path TEXT,kind TEXT NOT NULL,version INTEGER NOT NULL,hash TEXT NOT NULL,body TEXT NOT NULL DEFAULT '',deleted INTEGER NOT NULL DEFAULT 0,updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
      CREATE UNIQUE INDEX IF NOT EXISTS live_paths ON documents(workspace_id,path) WHERE deleted=0 AND path IS NOT NULL;
      CREATE TABLE IF NOT EXISTS source_versions(document_id TEXT NOT NULL,hash TEXT NOT NULL,path TEXT NOT NULL,version INTEGER NOT NULL,created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,PRIMARY KEY(document_id,hash));")?;
    let has_origin = c.prepare("PRAGMA table_info(documents)")?.query_map([], |r| r.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?.iter().any(|n| n == "origin_kind");
    if !has_origin {
        // A note made from a study-assistant conversation remembers its suggestion.
        c.execute_batch("ALTER TABLE documents ADD COLUMN origin_suggestion TEXT; ALTER TABLE documents ADD COLUMN origin_kind TEXT;")?;
    }
    crate::pdf_recovery::init_schema(&c)?;
    crate::generated_pdf::init_schema(&c)?;
    Ok(c)
}
fn workspace(c: &Connection, id: &str) -> Result<Value> {
    c.query_row("SELECT id,name FROM workspaces WHERE id=?1", [id], |r| {
        Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?}))
    })
    .optional()?
    .ok_or_else(|| anyhow!("과목을 찾을 수 없습니다"))
}
pub(crate) fn base(root: &Path, c: &Connection, id: &str) -> Result<PathBuf> {
    workspace(c, id)?;
    let p = root.join("workspaces").join(id).join("files");
    if !p.is_dir() {
        bail!("과목 저장 폴더가 없습니다: {}", p.display());
    }
    ensure_no_symlink(&root.join("workspaces"), &p)?;
    Ok(p)
}
pub(crate) fn ensure_no_symlink(base: &Path, path: &Path) -> Result<()> {
    let tail = path.strip_prefix(base).context("저장소 밖의 경로입니다")?;
    let mut current = base.to_path_buf();
    if fs::symlink_metadata(&current)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!("심볼릭 링크 경로는 사용할 수 없습니다");
    }
    for part in tail.components() {
        current.push(part.as_os_str());
        if fs::symlink_metadata(&current)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            bail!("심볼릭 링크 경로는 사용할 수 없습니다");
        }
    }
    Ok(())
}
pub(crate) fn file_path(root: &Path, c: &Connection, d: &Doc) -> Result<PathBuf> {
    let b = base(root, c, &d.workspace_id)?;
    let p = b.join(relative(
        d.path
            .as_deref()
            .ok_or_else(|| anyhow!("초안은 원본 파일이 없습니다"))?,
    )?);
    ensure_no_symlink(&b, &p)?;
    Ok(p)
}
pub(crate) fn document(c: &Connection, id: &str) -> Result<Doc> {
    c.query_row("SELECT id,workspace_id,title,path,kind,version,hash,body FROM documents WHERE id=?1 AND deleted=0",[id],|r|Ok(Doc{id:r.get(0)?,workspace_id:r.get(1)?,title:r.get(2)?,path:r.get(3)?,kind:r.get(4)?,version:r.get(5)?,hash:r.get(6)?,body:r.get(7)?})).optional()?.ok_or_else(||anyhow!("문서가 삭제되었거나 존재하지 않습니다"))
}
pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    let m = fs::metadata(path)
        .with_context(|| format!("원본 파일을 읽을 수 없습니다: {}", path.display()))?;
    if !m.is_file() {
        bail!("일반 파일만 사용할 수 있습니다");
    }
    if m.len() > MAX_FILE_BYTES {
        bail!("파일이 256 MiB 제한을 초과합니다");
    }
    Ok(fs::read(path)?)
}
pub(crate) fn sync_dir(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    persist_file(path, bytes, overwrite)?;
    sync_dir(path.parent().ok_or_else(|| anyhow!("잘못된 파일 경로"))?)
}
// Flush file contents before publishing the file. The caller syncs the parent
// when it owns that directory; macOS Trash does not grant directory read access.
fn persist_file(path: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow!("잘못된 파일 경로"))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    if overwrite {
        tmp.persist(path).map_err(|e| e.error)?;
    } else {
        tmp.persist_noclobber(path).map_err(|e| e.error)?;
    }
    Ok(())
}
fn snapshot(root: &Path, c: &Connection, d: &Doc, bytes: &[u8]) -> Result<PathBuf> {
    anyhow::ensure!(digest(bytes) == d.hash, "분석 준비 중 파일이 변경되었습니다. 다시 분석을 요청해 주세요");
    let ext = d
        .path
        .as_ref()
        .and_then(|p| Path::new(p).extension())
        .and_then(|e| e.to_str())
        .unwrap_or("md")
        .to_ascii_lowercase();
    let dir = root.join("versions");
    fs::create_dir_all(&dir)?;
    ensure_no_symlink(root, &dir)?;
    let p = dir.join(format!("{}.{}", d.hash, ext));
    ensure_no_symlink(&dir, &p)?;
    if !p.exists() {
        atomic_write(&p, bytes, false)?;
    } else if digest(&read_bytes(&p)?) != d.hash {
        bail!("불변 원본 버전의 해시가 일치하지 않습니다");
    }
    c.execute(
        "INSERT OR IGNORE INTO source_versions(document_id,hash,path,version) VALUES(?1,?2,?3,?4)",
        params![d.id, d.hash, p.to_string_lossy(), d.version],
    )?;
    Ok(p)
}
// Read graph state without creating or mutating knowledge.sqlite. Tree lookups
// use one connection for the whole workspace, avoiding a database open per row.
fn analysis_hashes(root: &Path, workspace_id: &str) -> Result<std::collections::HashMap<String, String>> {
    let path = root.join("knowledge.sqlite");
    if !path.is_file() { return Ok(Default::default()); }
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    c.busy_timeout(std::time::Duration::from_secs(2))?;
    let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sources')", [], |r| r.get(0))?;
    if !exists { return Ok(Default::default()); }
    let mut q = c.prepare("SELECT material_id,input_hash FROM sources WHERE workspace=?1 AND current=1")?;
    let rows = q.query_map([workspace_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
fn set_analysis_state(v: &mut Value, hashes: &std::collections::HashMap<String, String>) {
    v["analysis_state"] = json!(match v["id"].as_str().and_then(|id| hashes.get(id)) {
        None => "unanalysed",
        Some(hash) if v["hash"].as_str() == Some(hash.as_str()) => "analysed",
        Some(_) => "stale",
    });
}
fn metadata(root: &Path, c: &Connection, d: &Doc) -> Result<Value> {
    let absolute = d.path.as_ref().map(|_| file_path(root, c, d)).transpose()?;
    let missing = absolute.as_ref().map(|p| !p.is_file()).unwrap_or(false);
    let parent = d
        .path
        .as_ref()
        .and_then(|p| Path::new(p).parent())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(
        json!({"id":d.id,"workspace_id":d.workspace_id,"title":d.title,"path":d.path,"absolute_path":absolute.map(|p|p.to_string_lossy().into_owned()),"kind":d.kind,"version":d.version,"hash":d.hash,"body":null,"parent":parent,"missing":missing}),
    )
}
pub(crate) fn value(root: &Path, c: &Connection, d: &Doc) -> Result<Value> {
    let absolute = d.path.as_ref().map(|_| file_path(root, c, d)).transpose()?;
    let body = if d.kind == "draft" {
        Some(d.body.clone())
    } else if d.kind == "md" {
        Some(
            String::from_utf8(read_bytes(absolute.as_ref().unwrap())?)
                .context("Markdown이 올바른 UTF-8이 아닙니다")?,
        )
    } else {
        None
    };
    let parent = d
        .path
        .as_ref()
        .and_then(|p| Path::new(p).parent())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut v = json!({"id":d.id,"workspace_id":d.workspace_id,"title":d.title,"path":d.path,"absolute_path":absolute.map(|p|p.to_string_lossy().into_owned()),"kind":d.kind,"version":d.version,"hash":d.hash,"body":body,"parent":parent,"image_base":base(root,c,&d.workspace_id)?.to_string_lossy()});
    v["origin_kind"] = json!(c.query_row("SELECT origin_kind FROM documents WHERE id=?1", [&d.id], |r| r.get::<_, Option<String>>(0)).optional()?.flatten());
    set_analysis_state(&mut v, &analysis_hashes(root, &d.workspace_id)?);
    Ok(v)
}
fn expected(request: &Value, d: &Doc) -> Result<()> {
    if let Some(v) = request.get("expected_version") {
        if v.as_i64() != Some(d.version) {
            bail!(
                "저장 버전 충돌: 최신 문서를 다시 열어 주세요 (현재 {})",
                d.version
            );
        }
    }
    Ok(())
}
fn refresh(root: &Path, c: &Connection, mut d: Doc) -> Result<Doc> {
    if d.path.is_some() {
        let bytes = read_bytes(&file_path(root, c, &d)?)?;
        let h = digest(&bytes);
        if h != d.hash {
            if d.kind == "pdf" { crate::pdf_recovery::check_external_change(root,c,&d.id,&h)?; }
            if d.kind == "md" {
                std::str::from_utf8(&bytes).context("Markdown이 올바른 UTF-8이 아닙니다")?;
            }
            d.hash = h;
            d.version += 1;
            c.execute(
                "UPDATE documents SET hash=?2,version=?3,updated_at=CURRENT_TIMESTAMP WHERE id=?1",
                params![d.id, d.hash, d.version],
            )?;
        }
    }
    Ok(d)
}
fn folder_path(root: &Path, c: &Connection, w: &str, folder: &str) -> Result<PathBuf> {
    let b = base(root, c, w)?;
    let p = b.join(relative(folder)?);
    ensure_no_symlink(&b, &p)?;
    if !p.is_dir() {
        bail!("저장할 폴더가 없습니다");
    }
    Ok(p)
}
pub(crate) fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let p = dir.join(filename);
    if !p.exists() {
        return p;
    }
    let stem = Path::new(filename)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let ext = Path::new(filename)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 2.. {
        let p = dir.join(format!("{stem} ({n}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}
pub(crate) fn insert(c: &Connection, d: &Doc) -> Result<()> {
    c.execute("INSERT INTO documents(id,workspace_id,title,path,kind,version,hash,body) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![d.id,d.workspace_id,d.title,d.path,d.kind,d.version,d.hash,d.body])?;
    Ok(())
}
fn list_docs(c: &Connection, w: &str, drafts: bool) -> Result<Vec<Doc>> {
    let mut s=c.prepare("SELECT id FROM documents WHERE workspace_id=?1 AND deleted=0 AND ((?2=1 AND kind='draft') OR (?2=0 AND kind<>'draft')) ORDER BY updated_at DESC,id")?;
    let ids = s
        .query_map(params![w, drafts], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ids.iter().map(|id| document(c, id)).collect()
}
// Finder-hidden directories and dot directories are not navigation targets.
fn hidden_folder(path: &Path) -> Result<bool> {
    if path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')) {
        return Ok(true);
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        if fs::symlink_metadata(path)?.st_flags() & 0x00008000 != 0 {
            return Ok(true); // UF_HIDDEN
        }
    }
    Ok(false)
}
fn inside_hidden_folder(base: &Path, relative_path: &str) -> Result<bool> {
    let mut current = base.to_path_buf();
    if let Some(parent) = Path::new(relative_path).parent() {
        for component in parent.components() {
            current.push(component);
            if hidden_folder(&current)? { return Ok(true); }
        }
    }
    Ok(false)
}
fn folders(base: &Path, current: &Path, result: &mut Vec<Value>) -> Result<()> {
    for e in fs::read_dir(current)? {
        let e = e?;
        let ft = e.file_type()?;
        if ft.is_symlink() || hidden_folder(&e.path())? {
            continue;
        }
        if ft.is_dir() {
            let p = e.path();
            let rel = p.strip_prefix(base)?.to_string_lossy().into_owned();
            result.push(json!({"path":rel,"name":e.file_name().to_string_lossy(),"parent":p.parent().unwrap().strip_prefix(base)?.to_string_lossy()}));
            folders(base, &p, result)?;
        }
    }
    result.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(())
}
fn trash_dir(root: &Path) -> Result<PathBuf> {
    #[cfg(test)]
    let dir = root.join("test-trash");
    #[cfg(not(test))]
    let dir = {
        let _ = root;
        PathBuf::from(
            std::env::var_os("HOME")
                .ok_or_else(|| anyhow!("HOME이 없어 휴지통을 찾을 수 없습니다"))?,
        )
        .join(".Trash")
    };
    fs::create_dir_all(&dir)?;
    Ok(dir)
}
fn deletion_intent(root: &Path, c: &Connection, d: &Doc) -> Result<Value> {
    let source = if d.path.is_some() {
        Some(file_path(root, c, d)?)
    } else {
        None
    };
    let filename = source
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("초안-{}.md", d.id));
    let destination = trash_dir(root)?.join(format!("{}-{}", Uuid::new_v4(), filename));
    let intent = json!({"id":d.id,"source":source,"destination":destination,"body":if d.kind=="draft"{Some(&d.body)}else{None}});
    let dir = root.join("pending-deletions");
    fs::create_dir_all(&dir)?;
    ensure_no_symlink(root, &dir)?;
    atomic_write(
        &dir.join(format!("{}.json", d.id)),
        &serde_json::to_vec(&intent)?,
        false,
    )?;
    Ok(intent)
}
fn finish_deletion(c: &Connection, intent: &Value) -> Result<()> {
    let destination = Path::new(required(intent, "destination")?);
    if let Some(source) = intent["source"].as_str() {
        let source = Path::new(source);
        if source.exists() {
            if destination.exists() {
                bail!("휴지통 대상 충돌로 삭제를 중단했습니다");
            }
            fs::rename(source, destination)
                .context("휴지통으로 이동하지 못했습니다. 원본을 보존했습니다")?;
            sync_dir(source.parent().unwrap())?;
        } else if !destination.exists() {
            bail!("삭제할 원본과 휴지통 사본이 모두 없습니다");
        }
    } else if !destination.exists() {
        persist_file(destination, required(intent, "body")?.as_bytes(), false)?;
    }
    // macOS allows moving files into Trash without allowing the app to open
    // the Trash directory. Opening it for fsync can return EPERM *after* a
    // successful move and roll back the tombstone. Keep the durable deletion
    // journal for recovery, but never require read access to the system Trash.
    #[cfg(any(test, not(target_os = "macos")))]
    sync_dir(destination.parent().unwrap())?;
    c.execute("UPDATE documents SET deleted=1,body='',version=version+1,updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND deleted=0",[required(intent,"id")?])?;
    Ok(())
}
fn recover_deletions(root: &Path, c: &Connection) -> Result<Vec<PathBuf>> {
    let dir = root.join("pending-deletions");
    let mut completed = Vec::new();
    if dir.exists() {
        ensure_no_symlink(root, &dir)?;
        for entry in fs::read_dir(&dir)? {
            let p = entry?.path();
            ensure_no_symlink(&dir, &p)?;
            if p.extension().and_then(|e| e.to_str()) == Some("json") {
                let intent: Value = serde_json::from_slice(&fs::read(&p)?)?;
                // No physical move occurred if the destination is absent: abandon that intent.
                // If the move completed before a crash/rollback, finish only the tombstone.
                if Path::new(required(&intent, "destination")?).exists() {
                    finish_deletion(c, &intent)?;
                }
                completed.push(p);
            }
        }
    }
    Ok(completed)
}

// Keep the filesystem identity in the journal so recovery never mistakes an
// unrelated file created at the destination for a completed move.
fn relocation_intent(
    root: &Path,
    c: &Connection,
    d: &Doc,
    path: &str,
    title: &str,
) -> Result<Value> {
    let source = file_path(root, c, d)?;
    let m = fs::metadata(&source)?;
    let mut intent = json!({"id":d.id,"workspace_id":d.workspace_id,"source":d.path,
        "path":path,"title":title,"version":d.version + 1,"dev":m.dev(),"ino":m.ino(),"operation":Uuid::new_v4().to_string()});
    let dir = root.join("pending-relocations");
    fs::create_dir_all(&dir)?;
    ensure_no_symlink(root, &dir)?;
    if d.kind == "md" {
        let body = String::from_utf8(read_bytes(&source)?)?;
        let old_folder = Path::new(d.path.as_deref().unwrap()).parent().unwrap().to_string_lossy();
        let new_folder = Path::new(path).parent().unwrap().to_string_lossy();
        let rewritten = image_markdown::rebase(&body, &old_folder, &new_folder);
        if body != rewritten {
            let prepared = dir.join(format!("{}.md", required(&intent, "operation")?));
            atomic_write(&prepared, rewritten.as_bytes(), false)?;
            let m = fs::metadata(&prepared)?;
            intent["replacement"] = json!({"path":prepared,"dev":m.dev(),"ino":m.ino(),"hash":digest(rewritten.as_bytes()),"old_hash":digest(body.as_bytes())});
        }
    }
    atomic_write(
        &dir.join(format!("{}.json", required(&intent, "operation")?)),
        &serde_json::to_vec(&intent)?,
        false,
    )?;
    Ok(intent)
}
fn matches_identity(path: &Path, intent: &Value) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| {
        m.is_file()
            && Some(m.dev()) == intent["dev"].as_u64()
            && Some(m.ino()) == intent["ino"].as_u64()
    })
}
fn relocate_no_replace(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
        // Unlike std::fs::rename, RENAME_EXCL also rejects a destination created
        // between our validation and this syscall.
        if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("같은 이름의 파일이 있거나 파일을 이동할 수 없습니다");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Both paths belong to the same workspace. The journal also recovers
        // interruption between creating this non-overwriting link and unlinking.
        fs::hard_link(source, destination)
            .context("같은 이름의 파일이 있거나 파일을 이동할 수 없습니다")?;
        fs::remove_file(source)?;
    }
    sync_dir(source.parent().unwrap())?;
    sync_dir(destination.parent().unwrap())?;
    Ok(())
}
fn finish_relocation(root: &Path, c: &Connection, intent: &Value) -> Result<()> {
    let id = required(intent, "id")?;
    let d = document(c, id)?;
    let path = required(intent, "path")?;
    let version = intent["version"]
        .as_i64()
        .ok_or_else(|| anyhow!("잘못된 이동 기록입니다"))?;
    if d.path.as_deref() == Some(path) && d.version >= version {
        return Ok(());
    }
    if d.path.as_deref() != Some(required(intent, "source")?) || d.version != version - 1 {
        bail!("이동 중 문서 정보가 변경되었습니다. 복구 기록을 보존했습니다");
    }
    let mut hash = d.hash.clone();
    if let Some(replacement) = intent.get("replacement") {
        let destination = base(root,c,&d.workspace_id)?.join(relative(path)?);
        if !matches_identity(&destination,replacement) {
            if !matches_identity(&destination,intent) || digest(&read_bytes(&destination)?) != required(replacement,"old_hash")? { bail!("이동 중 노트 내용이 변경되었습니다"); }
            let prepared = Path::new(required(replacement,"path")?);
            ensure_no_symlink(&root.join("pending-relocations"),prepared)?;
            if !matches_identity(prepared,replacement) || digest(&read_bytes(prepared)?) != required(replacement,"hash")? { bail!("이미지 경로 복구 파일이 손상되었습니다"); }
            fs::rename(prepared,&destination)?;
            sync_dir(destination.parent().unwrap())?;
        }
        hash=required(replacement,"hash")?.to_owned();
    }
    c.execute(
        "UPDATE documents SET path=?2,title=?3,version=?4,hash=?5,updated_at=CURRENT_TIMESTAMP WHERE id=?1",
        params![id, path, required(intent, "title")?, version, hash],
    )?;
    Ok(())
}
fn recover_relocations(root: &Path, c: &Connection) -> Result<Vec<PathBuf>> {
    let dir = root.join("pending-relocations");
    let mut completed = Vec::new();
    if !dir.exists() {
        return Ok(completed);
    }
    ensure_no_symlink(root, &dir)?;
    for entry in fs::read_dir(&dir)? {
        let p = entry?.path();
        ensure_no_symlink(&dir, &p)?;
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let bytes = match fs::read(&p) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let intent: Value = serde_json::from_slice(&bytes)?;
        let committed_version: i64 = c.query_row(
            "SELECT version FROM documents WHERE id=?1",
            [required(&intent, "id")?],
            |r| r.get(0),
        )?;
        if committed_version
            >= intent["version"]
                .as_i64()
                .ok_or_else(|| anyhow!("잘못된 이동 기록입니다"))?
        {
            // Versions are monotonic, including tombstones. A newer path, edit,
            // or deletion supersedes this intent; never replay an old move.
            completed.push(p);
            continue;
        }
        let b = base(root, c, required(&intent, "workspace_id")?)?;
        let source = b.join(relative(required(&intent, "source")?)?);
        let destination = b.join(relative(required(&intent, "path")?)?);
        ensure_no_symlink(&b, &source)?;
        ensure_no_symlink(&b, &destination)?;
        if matches_identity(&destination, &intent) || intent.get("replacement").is_some_and(|r| matches_identity(&destination,r)) {
            // Non-macOS fallback can leave both links after an interrupted move.
            if matches_identity(&source, &intent) {
                if source.canonicalize()? == destination.canonicalize()? {
                    // On case-insensitive APFS both spellings refer to one entry.
                    // Repeating the exclusive rename also completes its spelling
                    // when the crash preceded the original filesystem syscall.
                    relocate_no_replace(&source, &destination)?;
                } else {
                    fs::remove_file(&source)?;
                    sync_dir(source.parent().unwrap())?;
                }
            }
            finish_relocation(root, c, &intent)?;
        } else if !matches_identity(&source, &intent) {
            bail!("이동 중 원본 위치가 변경되었습니다. 복구 기록을 보존했습니다");
        }
        // Source still present and destination differs: no move happened.
        completed.push(p);
    }
    Ok(completed)
}
fn relocate(root: &Path, c: &Connection, request: &Value) -> Result<(Value, Option<PathBuf>)> {
    let d = document(c, required(request, "id")?)?;
    if d.kind == "draft" || d.path.is_none() {
        bail!("저장된 자료만 이름이나 위치를 변경할 수 있습니다");
    }
    expected(request, &d)?;
    let source = file_path(root, c, &d)?;
    let bytes = read_bytes(&source)?;
    if digest(&bytes) != d.hash {
        bail!("파일이 외부에서 변경되었습니다. 다시 열어 변경 내용을 확인해 주세요");
    }
    let old = Path::new(d.path.as_deref().unwrap());
    let (path, title) = if required(request, "op")? == "note.rename" {
        let title = name(required(request, "title")?)?;
        let extension = old.extension().and_then(|e| e.to_str()).unwrap_or("");
        let suffix = format!(".{extension}");
        let title =
            if !extension.is_empty() && title.to_lowercase().ends_with(&suffix.to_lowercase()) {
                name(&title[..title.len() - suffix.len()])?
            } else {
                title
            };
        let filename = if extension.is_empty() {
            title.clone()
        } else {
            format!("{title}.{extension}")
        };
        name(&filename)?;
        (old.parent().unwrap_or(Path::new("")).join(filename), title)
    } else {
        let folder = required(request, "folder")?;
        folder_path(root, c, &d.workspace_id, folder)?;
        (
            relative(folder)?.join(old.file_name().unwrap()),
            d.title.clone(),
        )
    };
    let path = path.to_string_lossy().into_owned();
    if d.path.as_deref() == Some(&path) {
        return Ok((value(root, c, &d)?, None));
    }
    let b = base(root, c, &d.workspace_id)?;
    let destination = b.join(&path);
    ensure_no_symlink(&b, &destination)?;
    let occupied: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM documents WHERE workspace_id=?1 AND path=?2 AND deleted=0 AND id<>?3)",
        params![d.workspace_id,path,d.id], |r|r.get(0))?;
    let same_entry =
        destination.exists() && source.canonicalize()? == destination.canonicalize()?;
    if occupied || (fs::symlink_metadata(&destination).is_ok() && !same_entry) {
        bail!("같은 이름의 파일이 이미 있습니다. 다른 이름이나 폴더를 선택해 주세요");
    }
    let intent = relocation_intent(root, c, &d, &path, &title)?;
    relocate_no_replace(&source, &destination)?;
    finish_relocation(root, c, &intent)?;
    let journal = root
        .join("pending-relocations")
        .join(format!("{}.json", required(&intent, "operation")?));
    Ok((value(root, c, &document(c, &d.id)?)?, Some(journal)))
}

/// One JSON request, one serialized transaction. The dispatcher wraps errors as JSON.
/// Hold the cross-process document writer lock through graph publication. Hashing
/// is prepared before the lock; the short critical section rechecks file identity.
/// Every listed source must still be the version that was analysed.
pub(crate) fn with_current_sources<T>(
    root: &Path, workspace_id: &str, sources: &[(String, String)],
    action: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let changed = "분석 중 자료가 편집되거나 필기가 저장되었습니다. 현재 자료와 기존 지도는 보존했습니다. 다시 분석을 요청해 주세요.";
    let mut c = connect(root)?;
    let signature = |p: &Path| -> Result<_> {
        let m = fs::symlink_metadata(p)?;
        anyhow::ensure!(m.is_file(), "{changed}");
        Ok((m.dev(), m.ino(), m.size(), m.mtime(), m.mtime_nsec(), m.ctime(), m.ctime_nsec()))
    };
    let mut seen = Vec::new();
    for (id, hash) in sources {
        let before = document(&c, id).map_err(|_| anyhow!("{changed}"))?;
        anyhow::ensure!(before.workspace_id == workspace_id && &before.hash == hash, "{changed}");
        let path = file_path(root, &c, &before)?;
        let identity = signature(&path)?;
        anyhow::ensure!(&digest(&read_bytes(&path)?) == hash && signature(&path)? == identity, "{changed}");
        seen.push((before, identity));
    }
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for ((id, hash), (before, identity)) in sources.iter().zip(&seen) {
        let current = document(&tx, id).map_err(|_| anyhow!("{changed}"))?;
        anyhow::ensure!(current.workspace_id == workspace_id && &current.hash == hash
            && current.path == before.path && current.version == before.version
            && &signature(&file_path(root, &tx, &current)?)? == identity, "{changed}");
    }
    let result = action()?;
    tx.commit()?;
    Ok(result)
}

pub fn handle(root: &Path, request: &Value) -> Result<Value> {
    let mut c = connect(root)?;
    // Recover and mutate under the same SQLite writer lock. No request may
    // supersede a filesystem move without first reconciling its journal.
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    workspace_delete::cleanup(root,&tx)?;
    crate::pdf_recovery::recover(root, &tx)?;
    crate::generated_pdf::recover(root, &tx)?;
    let mut completed_relocations = recover_relocations(root, &tx)?;
    let mut completed_deletions = recover_deletions(root, &tx)?;
    let op = required(request, "op")?;
    let response = match op {
        "workspace.list" => {
            let mut s = tx.prepare("SELECT id,name FROM workspaces ORDER BY created_at,id")?;
            let a = s
                .query_map([], |r| {
                    Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?}))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            json!({"workspaces":a})
        }
        "workspace.rename" => {
            let id = required(request,"workspace_id")?;
            let n = name(required(request,"name")?)?;
            workspace(&tx,id)?;
            tx.execute("UPDATE workspaces SET name=?2 WHERE id=?1", params![id,n])?;
            json!({"id":id,"name":n})
        }
        "workspace.delete" => {
            workspace_delete::delete(root,&tx,required(request,"workspace_id")?)?;
            json!({"deleted":true})
        }
        "workspace.create" => {
            let n = name(required(request, "name")?)?;
            let id = Uuid::new_v4().to_string();
            fs::create_dir_all(root.join("workspaces").join(&id).join("files"))?;
            tx.execute(
                "INSERT INTO workspaces(id,name) VALUES(?1,?2)",
                params![id, n],
            )?;
            json!({"id":id,"name":n})
        }
        "folder.rename" => {
            let w = required(request, "workspace_id")?;
            let old = required(request, "path")?;
            let source = folder_path(root, &tx, w, old)?;
            let rel = relative(old)?;
            let parent = rel.parent().ok_or_else(|| anyhow!("과목 루트의 이름은 변경할 수 없습니다"))?;
            let n = name(required(request, "name")?)?;
            let path = parent.join(&n).to_string_lossy().into_owned();
            if path != old {
                let destination = source.with_file_name(&n);
                tx.execute("UPDATE documents SET path=?3 || substr(path,length(?2)+1) WHERE workspace_id=?1 AND deleted=0 AND substr(path,1,length(?2)+1)=?2 || '/'",
                    params![w,old,path])?;
                relocate_no_replace(&source, &destination)?;
                if let Err(error) = tx.commit() {
                    relocate_no_replace(&destination, &source)?;
                    return Err(error.into());
                }
                return Ok(json!({"path":path,"name":n}));
            }
            json!({"path":path,"name":n})
        }
        "folder.create" => {
            let w = required(request, "workspace_id")?;
            let parent = optional(request, "parent");
            let dir = folder_path(root, &tx, w, parent)?;
            let n = name(required(request, "name")?)?;
            fs::create_dir(dir.join(&n)).map_err(|error| match error.kind() {
                std::io::ErrorKind::AlreadyExists => {
                    anyhow!("같은 이름의 파일 또는 폴더가 있습니다. 다른 이름을 입력해 주세요.")
                }
                std::io::ErrorKind::PermissionDenied => {
                    anyhow!("선택한 위치에 폴더를 만들 권한이 없습니다. 다른 위치를 선택해 주세요.")
                }
                _ => anyhow!(error).context("폴더를 만들 수 없습니다"),
            })?;
            json!({"path":relative(parent)?.join(&n).to_string_lossy(),"name":n,"parent":parent})
        }
        "tree" | "draft.list" => {
            let w = required(request, "workspace_id")?;
            let b = base(root, &tx, w)?;
            let docs = list_docs(&tx, w, op == "draft.list")?;
            let mut a = Vec::new();
            for d in docs {
                if op == "tree" && d.path.as_deref().map(|p| inside_hidden_folder(&b, p)).transpose()?.unwrap_or(false) {
                    continue;
                }
                a.push(if op == "draft.list" {
                    value(root, &tx, &d)?
                } else {
                    metadata(root, &tx, &d)?
                });
            }
            if op == "draft.list" {
                json!({"drafts":a})
            } else {
                let mut f = Vec::new();
                folders(&b, &b, &mut f)?;
                let hashes = analysis_hashes(root, w)?;
                for entry in &mut a { set_analysis_state(entry, &hashes); }
                json!({"entries":a,"folders":f,"root_path":b})
            }
        }
        "note.image.import" => {
            let w = required(request, "workspace_id")?;
            let b = base(root, &tx, w)?;
            let input = Path::new(required(request, "source")?);
            let bytes = read_bytes(input)?;
            if bytes.len() > 20 * 1024 * 1024 { bail!("이미지는 20 MiB 이하로 붙여넣어 주세요"); }
            let extension = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { "png" }
                else if bytes.starts_with(&[0xff, 0xd8, 0xff]) { "jpg" }
                else { bail!("PNG 또는 JPEG 이미지가 필요합니다"); };
            let dir = b.join(".images");
            ensure_no_symlink(&b, &dir)?;
            fs::create_dir_all(&dir)?;
            let filename = format!("{}.{}", digest(&bytes), extension);
            let path = dir.join(&filename);
            ensure_no_symlink(&b, &path)?;
            if !path.exists() { atomic_write(&path, &bytes, false)?; }
            else if digest(&read_bytes(&path)?) != digest(&bytes) { bail!("보관된 이미지가 손상되었습니다"); }
            let folder = optional(request, "folder");
            relative(folder)?;
            let destination = format!("{}.images/{}", "../".repeat(Path::new(folder).components().count()),filename);
            json!({"destination":destination,"image_base":b,"path":path})
        }
        "draft.create" => {
            let w = required(request, "workspace_id")?;
            workspace(&tx, w)?;
            let body = optional(request, "body");
            if body.trim().is_empty() {
                json!({"empty":true,"id":null,"body":"","version":0,"kind":"draft"})
            } else {
                let d = Doc {
                    id: Uuid::new_v4().to_string(),
                    workspace_id: w.into(),
                    title: "제목 없음".into(),
                    path: None,
                    kind: "draft".into(),
                    version: 1,
                    hash: digest(body.as_bytes()),
                    body: body.into(),
                };
                insert(&tx, &d)?;
                value(root, &tx, &d)?
            }
        }
        "draft.rename" => {
            let mut d = document(&tx, required(request, "id")?)?;
            anyhow::ensure!(d.kind == "draft", "초안이 아닙니다");
            expected(request, &d)?;
            let title = required(request, "title")?.trim();
            anyhow::ensure!(!title.is_empty(), "제목을 입력하세요");
            d.title = title.to_owned();
            d.version += 1;
            tx.execute("UPDATE documents SET title=?2,version=?3,updated_at=CURRENT_TIMESTAMP WHERE id=?1",
                params![d.id, d.title, d.version])?;
            value(root, &tx, &d)?
        }
        "draft.update" => {
            let mut d = document(&tx, required(request, "id")?)?;
            if d.kind != "draft" {
                bail!("초안이 아닙니다");
            }
            expected(request, &d)?;
            let body = required(request, "body")?;
            if body.trim().is_empty() {
                tx.execute(
                    "UPDATE documents SET deleted=1,body='',version=version+1 WHERE id=?1",
                    [&d.id],
                )?;
                json!({"empty":true,"id":null,"deleted_id":d.id,"version":0,"kind":"draft"})
            } else {
                d.body = body.into();
                d.hash = digest(body.as_bytes());
                d.version += 1;
                tx.execute("UPDATE documents SET body=?2,title=?3,hash=?4,version=?5,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![d.id,d.body,d.title,d.hash,d.version])?;
                value(root, &tx, &d)?
            }
        }
        "draft.get" | "note.read" | "source.get" | "source.snapshot" => {
            let d = refresh(root, &tx, document(&tx, required(request, "id")?)?)?;
            let mut v = value(root, &tx, &d)?;
            if op == "source.snapshot" {
                if let Some(workspace_id) = request.get("workspace_id").and_then(Value::as_str) {
                    anyhow::ensure!(workspace_id == d.workspace_id, "다른 과목의 자료입니다");
                }
                if let Some(hash) = request.get("expected_hash").and_then(Value::as_str) {
                    anyhow::ensure!(hash == d.hash, "분석 준비 중 자료가 변경되었습니다. 저장 완료 후 다시 분석을 요청해 주세요");
                }
                // The explicit analysis boundary is the only creator of retained originals.
                let bytes = read_bytes(&file_path(root, &tx, &d)?)?;
                v["immutable_path"] = json!(snapshot(root, &tx, &d, &bytes)?.to_string_lossy());
            } else if op == "source.get" {
                let retained: Option<String> = tx.query_row(
                    "SELECT path FROM source_versions WHERE document_id=?1 AND hash=?2",
                    params![d.id, d.hash], |r| r.get(0),
                ).optional()?;
                v["immutable_path"] = json!(retained);
            }
            v
        }
        "note.create" => {
            let w = required(request, "workspace_id")?;
            let folder = optional(request, "folder");
            let dir = folder_path(root, &tx, w, folder)?;
            // Explicit creation may be empty. Reserve both on-disk and indexed
            // names, including missing documents and dangling symlinks.
            let mut number = 1;
            let (title, relative_path, path) = loop {
                let title = if number == 1 {
                    "새 노트".to_owned()
                } else {
                    format!("새 노트 {number}")
                };
                let relative_path = relative(folder)?
                    .join(format!("{title}.md"))
                    .to_string_lossy()
                    .into_owned();
                let path = dir.join(format!("{title}.md"));
                let occupied: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM documents WHERE workspace_id=?1 AND path=?2 AND deleted=0)", params![w,relative_path], |r|r.get(0))?;
                if !occupied && fs::symlink_metadata(&path).is_err() {
                    break (title, relative_path, path);
                }
                number += 1;
            };
            let d = Doc {
                id: Uuid::new_v4().to_string(),
                workspace_id: w.into(),
                title,
                path: Some(relative_path),
                kind: "md".into(),
                version: 1,
                hash: digest(b""),
                body: String::new(),
            };
            insert(&tx, &d)?;
            atomic_write(&path, b"", false)?;
            value(root, &tx, &d)?
        }
        "note.save" => {
            let mut d = document(&tx, required(request, "draft_id")?)?;
            if d.kind != "draft" {
                bail!("이미 저장된 문서입니다");
            }
            expected(request, &d)?;
            if d.body.trim().is_empty() {
                bail!("내용 없는 초안은 저장하지 않습니다");
            }
            let title = name(required(request, "title")?)?;
            let title = title.strip_suffix(".md").unwrap_or(&title);
            let title = name(title)?;
            let folder = optional(request, "folder");
            let dir = folder_path(root, &tx, &d.workspace_id, folder)?;
            let path = dir.join(format!("{title}.md"));
            if path.exists() {
                let suggestion = unique_path(&dir, &format!("{title}.md"));
                json!({"conflict":true,"suggested_title":suggestion.file_stem().unwrap().to_string_lossy(),"suggested_path":relative(folder)?.join(suggestion.file_name().unwrap()).to_string_lossy()})
            } else {
                let rebased = image_markdown::rebase(&d.body, "", folder);
                atomic_write(&path, rebased.as_bytes(), false)?;
                d.body = rebased;
                d.hash = digest(d.body.as_bytes());
                d.path = Some(
                    relative(folder)?
                        .join(path.file_name().unwrap())
                        .to_string_lossy()
                        .into_owned(),
                );
                d.kind = "md".into();
                d.title = title;
                d.version += 1;
                tx.execute("UPDATE documents SET title=?2,path=?3,kind='md',body='',version=?4,hash=?5,origin_suggestion=?6,origin_kind=?7,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![d.id,d.title,d.path,d.version,d.hash,request["origin_suggestion"].as_str(),request["origin_kind"].as_str()])?;
                value(root, &tx, &d)?
            }
        }
        "note.rename" | "note.move" => {
            let (result, journal) = relocate(root, &tx, request)?;
            completed_relocations.extend(journal);
            result
        }
        "note.update" => {
            let mut d = document(&tx, required(request, "id")?)?;
            if d.kind != "md" {
                bail!("Markdown 문서만 편집할 수 있습니다");
            }
            expected(request, &d)?;
            let p = file_path(root, &tx, &d)?;
            let old = read_bytes(&p)?;
            if digest(&old) != d.hash {
                bail!("파일이 외부에서 변경되었습니다. 다시 열어 변경 내용을 확인해 주세요");
            }
            let body = required(request, "body")?;
            if body.len() as u64 > MAX_FILE_BYTES {
                bail!("문서가 크기 제한을 초과합니다");
            }
            if digest(body.as_bytes()) != d.hash {
                atomic_write(&p, body.as_bytes(), true)?;
                d.hash = digest(body.as_bytes());
                d.version += 1;
                tx.execute("UPDATE documents SET hash=?2,version=?3,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![d.id,d.hash,d.version])?;
            }
            value(root, &tx, &d)?
        }
        "material.import" | "material.copy" => {
            let copy = required(request, "op")? == "material.copy";
            let w = required(request, "workspace_id")?;
            let folder = optional(request, "folder");
            let dir = folder_path(root, &tx, w, folder)?;
            let input = Path::new(required(request, "path")?);
            let ext = input
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !["md", "pdf", "png", "jpg", "jpeg", "heic", "pptx"].contains(&ext.as_str()) {
                bail!("지원하지 않는 형식입니다. PDF, MD, PNG/JPEG/HEIC, PPTX를 선택해 주세요");
            }
            let bytes = read_bytes(input)?;
            if ext == "md" {
                std::str::from_utf8(&bytes).context("Markdown이 올바른 UTF-8이 아닙니다")?;
            }
            let h = digest(&bytes);
            // A selected note already inside this subject is the same document even
            // when an external editor changed its bytes since our last read.
            let b = base(root, &tx, w)?;
            let canonical_input = input.canonicalize()?;
            let canonical_base = b.canonicalize()?;
            if let Some(relative_input) = canonical_input.strip_prefix(&canonical_base).ok().filter(|_| !copy) {
                let relative_input = relative_input.to_string_lossy();
                let id: Option<String> = tx
                    .query_row(
                        "SELECT id FROM documents WHERE workspace_id=?1 AND path=?2 AND deleted=0",
                        params![w, relative_input],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(id) = id {
                    let d = refresh(root, &tx, document(&tx, &id)?)?;
                    let mut v = value(root, &tx, &d)?;
                    v["reused"] = json!(true);
                    tx.commit()?;
                    return Ok(v);
                }
            }
            let existing=tx.query_row("SELECT id FROM documents WHERE workspace_id=?1 AND hash=?2 AND kind<>'draft' AND deleted=0 LIMIT 1",params![w,h],|r|r.get::<_,String>(0)).optional()?;
            if let Some(id) = existing.filter(|_| !copy) {
                let d = refresh(root, &tx, document(&tx, &id)?)?;
                if d.hash == h {
                    let mut v = value(root, &tx, &d)?;
                    v["reused"] = json!(true);
                    tx.commit()?;
                    return Ok(v);
                }
            }
            let filename = name(
                &input
                    .file_name()
                    .ok_or_else(|| anyhow!("파일 이름이 없습니다"))?
                    .to_string_lossy(),
            )?;
            let path = unique_path(&dir, &filename);
            atomic_write(&path, &bytes, false)?;
            let d = Doc {
                id: Uuid::new_v4().to_string(),
                workspace_id: w.into(),
                title: path.file_stem().unwrap().to_string_lossy().into_owned(),
                path: Some(
                    relative(folder)?
                        .join(path.file_name().unwrap())
                        .to_string_lossy()
                        .into_owned(),
                ),
                kind: if ext == "jpg" { "jpeg".into() } else { ext },
                version: 1,
                hash: h,
                body: String::new(),
            };
            insert(&tx, &d)?;
            value(root, &tx, &d)?
        }
        "draft.delete" | "note.delete" | "material.delete" => {
            let d = document(&tx, required(request, "id")?)?;
            expected(request, &d)?;
            let intent = deletion_intent(root, &tx, &d)?;
            finish_deletion(&tx, &intent)?;
            completed_deletions.push(
                root.join("pending-deletions")
                    .join(format!("{}.json", d.id)),
            );
            json!({"deleted":true,"id":d.id,"trash_path":intent["destination"]})
        }
        _ => bail!("알 수 없는 저장 명령: {op}"),
    };
    tx.commit()?;
    if op == "workspace.delete" {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        workspace_delete::cleanup(root,&tx)?;
        tx.commit()?;
    }
    for path in completed_relocations {
        let _ = fs::remove_file(path);
    }
    for p in completed_deletions {
        if let Some(id)=p.file_stem().and_then(|v|v.to_str()) { let _=crate::pdf_recovery::forget(root,id); }
        let _ = fs::remove_file(p);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(root: &Path, v: Value) -> Value {
        handle(root, &v).unwrap()
    }
    fn ws(root: &Path) -> String {
        call(root, json!({"op":"workspace.create","name":"생물학"}))["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn draft(root: &Path, w: &str, body: &str) -> Value {
        call(
            root,
            json!({"op":"draft.create","workspace_id":w,"body":body}),
        )
    }
    fn save(root: &Path, d: &Value, title: &str) -> Value {
        call(
            root,
            json!({"op":"note.save","draft_id":d["id"],"title":title}),
        )
    }
    #[test]
    fn tree_hides_hidden_folders_and_their_documents() {
        let t = tempfile::tempdir().unwrap(); let w = ws(t.path());
        for name in [".secret", "visible", "finder-hidden"] {
            call(t.path(), json!({"op":"folder.create","workspace_id":w,"name":name}));
            call(t.path(), json!({"op":"folder.create","workspace_id":w,"parent":name,"name":"nested"}));
            let d = draft(t.path(), &w, "body");
            call(t.path(), json!({"op":"note.save","draft_id":d["id"],"title":"note","folder":format!("{name}/nested")}));
        }
        #[cfg(target_os = "macos")]
        assert!(std::process::Command::new("chflags").arg("hidden")
            .arg(t.path().join("workspaces").join(&w).join("files/finder-hidden"))
            .status().unwrap().success());
        let tree = call(t.path(), json!({"op":"tree","workspace_id":w}));
        for key in ["folders", "entries"] {
            let rows = tree[key].as_array().unwrap();
            assert!(rows.iter().any(|r| r["path"].as_str().unwrap().starts_with("visible/")));
            assert!(!rows.iter().any(|r| r["path"].as_str().unwrap().starts_with(".secret")));
            #[cfg(target_os = "macos")]
            assert!(!rows.iter().any(|r| r["path"].as_str().unwrap().starts_with("finder-hidden")));
        }
    }
    #[test]
    fn body_edits_keep_default_title() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let d = draft(t.path(), &w, "# 첫 줄");
        assert_eq!(d["title"], "제목 없음");
        let d = call(t.path(), json!({"op":"draft.update","id":d["id"],"body":"다른 첫 줄","expected_version":d["version"]}));
        assert_eq!(d["title"], "제목 없음");
    }

    #[test]
    fn pasted_images_dedupe_rebase_and_survive_deletion() {
        let t=tempfile::tempdir().unwrap(); let w=ws(t.path());
        let input=t.path().join("paste.png"); fs::write(&input,b"\x89PNG\r\n\x1a\npixels").unwrap();
        let request=json!({"op":"note.image.import","workspace_id":w,"source":input});
        let image=call(t.path(),request.clone()); assert_eq!(image,call(t.path(),request));
        assert!(image["destination"].as_str().unwrap().starts_with(".images/"));
        assert!(call(t.path(),json!({"op":"tree","workspace_id":w}))["folders"].as_array().unwrap().is_empty());
        let body=format!("앞\n![그림](<{}> \"stology:width=237;x=0.700;wrap=1\")\n\n옆 필기",image["destination"].as_str().unwrap());
        let d=draft(t.path(),&w,&body);
        assert!(Path::new(d["image_base"].as_str().unwrap()).is_dir());
        call(t.path(),json!({"op":"folder.create","workspace_id":w,"name":"단원"}));
        let note=call(t.path(),json!({"op":"note.save","draft_id":d["id"],"title":"필기","folder":"단원"}));
        assert!(note["body"].as_str().unwrap().contains("../.images/"));
        assert_eq!(note["hash"],json!(digest(note["body"].as_str().unwrap().as_bytes())));
        let moved=call(t.path(),json!({"op":"note.move","id":note["id"],"folder":""}));
        assert_eq!(moved["body"],body);
        let edited=call(t.path(),json!({"op":"note.update","id":moved["id"],"body":format!("{body} 추가"),"expected_version":moved["version"]}));
        call(t.path(),json!({"op":"note.delete","id":edited["id"]}));
        assert!(Path::new(image["path"].as_str().unwrap()).exists());
    }
    #[test]
    fn image_relocation_recovers_after_replacement_before_db_commit() {
        let t=tempfile::tempdir().unwrap();let w=ws(t.path());
        let body=format!("![이미지](이미지/{}.png)","a".repeat(64));
        let n=save(t.path(),&draft(t.path(),&w,&body),"image");
        call(t.path(),json!({"op":"folder.create","workspace_id":w,"name":"nested"}));
        let mut c=connect(t.path()).unwrap();let tx=c.transaction().unwrap();
        let (moved,_)=relocate(t.path(),&tx,&json!({"op":"note.move","id":n["id"],"folder":"nested"})).unwrap();
        tx.rollback().unwrap();drop(c);
        let recovered=call(t.path(),json!({"op":"note.read","id":n["id"]}));
        assert_eq!(recovered,moved);assert!(recovered["body"].as_str().unwrap().contains("../이미지/"));
    }

    #[test]
    fn explicit_empty_note_is_persistent_and_never_a_draft() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let other = ws(t.path());
        call(
            t.path(),
            json!({"op":"folder.create","workspace_id":w,"name":"수업"}),
        );
        call(
            t.path(),
            json!({"op":"folder.create","workspace_id":w,"parent":"수업","name":"정리"}),
        );
        let old = draft(t.path(), &w, "작성 중인 내용");
        let n = call(
            t.path(),
            json!({"op":"note.create","workspace_id":w,"folder":"수업/정리"}),
        );
        assert_eq!(n["path"], "수업/정리/새 노트.md");
        assert_eq!(n["kind"], "md");
        assert_eq!(fs::read(n["absolute_path"].as_str().unwrap()).unwrap(), b"");
        assert_eq!(
            call(t.path(), json!({"op":"note.read","id":n["id"]}))["body"],
            ""
        );
        assert_eq!(
            call(t.path(), json!({"op":"draft.list","workspace_id":w}))["drafts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            call(t.path(), json!({"op":"draft.get","id":old["id"]}))["body"],
            "작성 중인 내용"
        );
        assert_eq!(
            call(t.path(), json!({"op":"tree","workspace_id":other}))["entries"],
            json!([])
        );
        let renamed = call(
            t.path(),
            json!({"op":"note.rename","id":n["id"],"title":"세포","expected_version":n["version"]}),
        );
        let edited = call(
            t.path(),
            json!({"op":"note.update","id":n["id"],"body":"한글 본문","expected_version":renamed["version"]}),
        );
        let empty = call(
            t.path(),
            json!({"op":"note.update","id":n["id"],"body":"","expected_version":edited["version"]}),
        );
        assert_eq!(empty["id"], n["id"]);
        assert_eq!(empty["body"], "");
        assert!(Path::new(empty["absolute_path"].as_str().unwrap()).is_file());
    }
    #[test]
    fn explicit_note_reserves_missing_indexed_and_external_names() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = call(t.path(), json!({"op":"note.create","workspace_id":w}));
        let path = Path::new(n["absolute_path"].as_str().unwrap());
        let dir = path.parent().unwrap();
        fs::remove_file(path).unwrap();
        fs::write(dir.join("새 노트 2.md"), "external").unwrap();
        std::os::unix::fs::symlink(dir.join("missing"), dir.join("새 노트 3.md")).unwrap();
        let next = call(t.path(), json!({"op":"note.create","workspace_id":w}));
        assert_eq!(next["title"], "새 노트 4");
        assert_eq!(
            fs::read_to_string(dir.join("새 노트 2.md")).unwrap(),
            "external"
        );
        assert!(handle(
            t.path(),
            &json!({"op":"note.rename","id":next["id"],"title":"새 노트 2"})
        )
        .is_err());
        assert_eq!(
            call(t.path(), json!({"op":"note.read","id":next["id"]}))["title"],
            "새 노트 4"
        );
    }
    #[test]
    fn explicit_note_rejects_invalid_destinations_without_creating_documents() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let outside = tempfile::tempdir().unwrap();
        let c = connect(t.path()).unwrap();
        let dir = base(t.path(), &c, &w).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.join("escape")).unwrap();
        for folder in ["absent", "../outside", "/tmp", "escape"] {
            assert!(handle(
                t.path(),
                &json!({"op":"note.create","workspace_id":w,"folder":folder})
            )
            .is_err());
        }
        assert!(handle(
            t.path(),
            &json!({"op":"note.create","workspace_id":"unknown"})
        )
        .is_err());
        assert_eq!(
            call(t.path(), json!({"op":"tree","workspace_id":w}))["entries"],
            json!([])
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
    #[test]
    fn renamed_draft_keeps_title_after_autosave() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let d = draft(root, &ws(root), "first");
        let rename = json!({"op":"draft.rename","id":d["id"],"expected_version":d["version"],"title":"custom"});
        let renamed = call(root, rename.clone());
        assert_eq!(renamed["body"], "first");
        assert!(handle(root, &rename).is_err());
        let updated = call(root, json!({"op":"draft.update","id":d["id"],"expected_version":renamed["version"],"body":"second"}));
        assert_eq!(updated["title"], "custom");
    }
    #[test]
    fn empty_drafts_never_accumulate() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        for _ in 0..10 {
            assert!(draft(t.path(), &w, " \n")["id"].is_null());
        }
        assert_eq!(
            call(t.path(), json!({"op":"draft.list","workspace_id":w}))["drafts"],
            json!([])
        );
    }
    #[test]
    fn markdown_roundtrip_and_restart() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let text="# 한글 받침\n\n**굵게** _기울임_\n- [x] 목록\n\n| 열 | 값 |\n|---|---|\n| 표 | 둘 |\n\n![그림](x.png)\n```rust\nlet x = 1;\n```";
        let d = draft(t.path(), &w, text);
        let n = save(t.path(), &d, "시험");
        assert_eq!(n["body"], text);
        assert!(
            call(t.path(), json!({"op":"draft.list","workspace_id":w}))["drafts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            call(t.path(), json!({"op":"note.read","id":n["id"]}))["body"],
            text
        );
        let src = call(t.path(), json!({"op":"source.snapshot","id":n["id"]}));
        assert_eq!(
            fs::read_to_string(src["immutable_path"].as_str().unwrap()).unwrap(),
            text
        );
    }
    #[test]
    fn collision_preserves_draft_and_existing() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let a = save(t.path(), &draft(t.path(), &w, "first"), "같은 이름");
        let d = draft(t.path(), &w, "second");
        let conflict = save(t.path(), &d, "같은 이름");
        assert_eq!(conflict["conflict"], true);
        assert_eq!(
            call(t.path(), json!({"op":"draft.get","id":d["id"]}))["body"],
            "second"
        );
        let b = save(t.path(), &d, conflict["suggested_title"].as_str().unwrap());
        assert_ne!(a["path"], b["path"]);
        assert_eq!(
            call(t.path(), json!({"op":"note.read","id":a["id"]}))["body"],
            "first"
        );
    }
    #[test]
    fn stale_save_and_deleted_save_cannot_overwrite() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "before"), "note");
        let updated = call(
            t.path(),
            json!({"op":"note.update","id":n["id"],"expected_version":n["version"],"body":"after"}),
        );
        assert!(handle(
            t.path(),
            &json!({"op":"note.update","id":n["id"],"expected_version":n["version"],"body":"stale"})
        )
        .is_err());
        let deleted = call(t.path(), json!({"op":"note.delete","id":n["id"]}));
        assert_eq!(
            fs::read_to_string(deleted["trash_path"].as_str().unwrap()).unwrap(),
            "after"
        );
        assert!(handle(t.path(),&json!({"op":"note.update","id":n["id"],"expected_version":updated["version"],"body":"revive"})).is_err());
        assert!(!Path::new(n["absolute_path"].as_str().unwrap()).exists());
    }
    #[test]
    fn paths_and_symlinks_cannot_escape() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let d = draft(t.path(), &w, "safe");
        assert!(handle(
            t.path(),
            &json!({"op":"note.save","draft_id":d["id"],"title":"../outside"})
        )
        .is_err());
        assert!(handle(
            t.path(),
            &json!({"op":"folder.create","workspace_id":w,"parent":"../","name":"bad"})
        )
        .is_err());
        #[cfg(unix)]
        {
            let b = base(t.path(), &connect(t.path()).unwrap(), &w).unwrap();
            std::os::unix::fs::symlink(t.path(), b.join("escape")).unwrap();
            assert!(handle(
                t.path(),
                &json!({"op":"note.save","draft_id":d["id"],"title":"bad","folder":"escape"})
            )
            .is_err());
        }
    }
    #[test]
    fn saved_md_import_reuses_id_and_versions_survive_edits() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "original"), "note");
        let source = call(t.path(), json!({"op":"source.snapshot","id":n["id"]}));
        let imported = call(
            t.path(),
            json!({"op":"material.import","workspace_id":w,"path":n["absolute_path"]}),
        );
        assert_eq!(imported["id"], n["id"]);
        call(
            t.path(),
            json!({"op":"note.update","id":n["id"],"expected_version":n["version"],"body":"changed"}),
        );
        assert_eq!(
            fs::read_to_string(source["immutable_path"].as_str().unwrap()).unwrap(),
            "original"
        );
    }
    #[test]
    fn external_copy_preserves_source_and_uses_requested_folder_without_deduplication() {
        let t = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let input = external.path().join("한글 자료.md");
        fs::write(&input, "original").unwrap();
        let w = ws(t.path());
        call(t.path(), json!({"op":"folder.create","workspace_id":w,"name":"수업"}));
        call(t.path(), json!({"op":"folder.create","workspace_id":w,"parent":"수업","name":"정리"}));
        let original = call(t.path(), json!({"op":"material.import","workspace_id":w,"path":input}));
        let first = call(t.path(), json!({"op":"material.copy","workspace_id":w,"path":input,"folder":"수업/정리"}));
        assert_eq!(first["path"], "수업/정리/한글 자료.md");
        assert_ne!(first["id"], original["id"]);
        let second = call(t.path(), json!({"op":"material.copy","workspace_id":w,"path":input,"folder":"수업/정리"}));
        assert_eq!(second["path"], "수업/정리/한글 자료 (2).md");
        assert_ne!(second["id"], first["id"]);
        let root_copy = call(t.path(), json!({"op":"material.copy","workspace_id":w,"path":first["absolute_path"]}));
        assert_eq!(root_copy["path"], "한글 자료 (2).md");
        assert_ne!(root_copy["id"], first["id"]);
        for document in [&original, &first, &second, &root_copy] {
            assert_eq!(fs::read_to_string(document["absolute_path"].as_str().unwrap()).unwrap(), "original");
        }
        assert_eq!(fs::read_to_string(&input).unwrap(), "original");
        for folder in ["../escape", "missing"] {
            assert!(handle(t.path(), &json!({"op":"material.copy","workspace_id":w,"path":input,"folder":folder})).is_err());
        }
        let invalid = external.path().join("invalid.md");
        fs::write(&invalid, [0xff]).unwrap();
        assert!(handle(t.path(), &json!({"op":"material.copy","workspace_id":w,"path":invalid})).is_err());
    }
    #[test]
    fn workspaces_remain_isolated() {
        let t = tempfile::tempdir().unwrap();
        let a = ws(t.path());
        let b = ws(t.path());
        draft(t.path(), &a, "a");
        save(t.path(), &draft(t.path(), &b, "b"), "b");
        assert_eq!(
            call(t.path(), json!({"op":"tree","workspace_id":a}))["entries"],
            json!([])
        );
        assert_eq!(
            call(t.path(), json!({"op":"draft.list","workspace_id":b}))["drafts"],
            json!([])
        );
    }
    #[test]
    fn external_edit_conflict_preserves_disk() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "original"), "note");
        let p = n["absolute_path"].as_str().unwrap();
        fs::write(p, "external").unwrap();
        assert!(handle(
            t.path(),
            &json!({"op":"note.update","id":n["id"],"expected_version":n["version"],"body":"stale"})
        )
        .is_err());
        let current = call(t.path(), json!({"op":"note.read","id":n["id"]}));
        assert_eq!(current["body"], "external");
        assert!(current["version"].as_i64() > n["version"].as_i64());
    }
    #[test]
    fn deleted_drafts_are_recoverable_markdown() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let d = draft(t.path(), &w, "# 복구 가능한 초안");
        let deleted = call(t.path(), json!({"op":"draft.delete","id":d["id"]}));
        assert_eq!(
            fs::read_to_string(deleted["trash_path"].as_str().unwrap()).unwrap(),
            "# 복구 가능한 초안"
        );
        assert!(handle(
            t.path(),
            &json!({"op":"draft.update","id":d["id"],"body":"late"})
        )
        .is_err());
    }
    #[test]
    fn deletion_recovers_after_file_move_before_sqlite_commit() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "crash safe"), "note");
        let mut c = connect(t.path()).unwrap();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let d = document(&tx, n["id"].as_str().unwrap()).unwrap();
        let intent = deletion_intent(t.path(), &tx, &d).unwrap();
        finish_deletion(&tx, &intent).unwrap();
        tx.rollback().unwrap();
        drop(c);
        let tree = call(t.path(), json!({"op":"tree","workspace_id":w}));
        assert_eq!(tree["entries"], json!([]));
        assert_eq!(
            fs::read_to_string(intent["destination"].as_str().unwrap()).unwrap(),
            "crash safe"
        );
        assert!(handle(
            t.path(),
            &json!({"op":"note.update","id":n["id"],"body":"late"})
        )
        .is_err());
    }
    #[test]
    fn missing_external_file_does_not_block_tree() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "missing"), "note");
        fs::remove_file(n["absolute_path"].as_str().unwrap()).unwrap();
        let tree = call(t.path(), json!({"op":"tree","workspace_id":w}));
        assert_eq!(tree["entries"][0]["missing"], true);
    }
    #[test]
    fn concurrent_delete_and_save_never_resurrect() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "before"), "note");
        let path = t.path().to_owned();
        let id = n["id"].clone();
        let version = n["version"].clone();
        let thread = std::thread::spawn(move || {
            handle(
                &path,
                &json!({"op":"note.update","id":id,"expected_version":version,"body":"concurrent"}),
            )
        });
        call(t.path(), json!({"op":"note.delete","id":n["id"]}));
        let _ = thread.join().unwrap();
        assert!(!Path::new(n["absolute_path"].as_str().unwrap()).exists());
        assert_eq!(
            call(t.path(), json!({"op":"tree","workspace_id":w}))["entries"],
            json!([])
        );
    }

    #[test]
    fn import_existing_md_after_external_edit_reuses_identity() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "original"), "note");
        fs::write(n["absolute_path"].as_str().unwrap(), "external edit").unwrap();
        let imported = call(
            t.path(),
            json!({"op":"material.import","workspace_id":w,"path":n["absolute_path"]}),
        );
        assert_eq!(imported["id"], n["id"]);
        assert_eq!(imported["body"], "external edit");
        assert_eq!(
            call(t.path(), json!({"op":"tree","workspace_id":w}))["entries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn rename_move_preserves_identity_evidence_and_future_edits() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "original"), "old");
        let source = call(t.path(), json!({"op":"source.snapshot","id":n["id"]}));
        call(
            t.path(),
            json!({"op":"folder.create","workspace_id":w,"name":"자료"}),
        );
        call(
            t.path(),
            json!({"op":"folder.create","workspace_id":w,"parent":"자료","name":"세포"}),
        );
        let renamed = call(
            t.path(),
            json!({"op":"note.rename","id":n["id"],"title":"새 제목.md","expected_version":n["version"]}),
        );
        assert_eq!(renamed["path"], "새 제목.md");
        assert_eq!(renamed["title"], "새 제목");
        assert!(!Path::new(n["absolute_path"].as_str().unwrap()).exists());
        let moved = call(
            t.path(),
            json!({"op":"note.move","id":n["id"],"folder":"자료/세포","expected_version":renamed["version"]}),
        );
        assert_eq!(moved["path"], "자료/세포/새 제목.md");
        assert_eq!(moved["parent"], "자료/세포");
        assert_eq!(moved["hash"], n["hash"]);
        assert_eq!(moved["id"], n["id"]);
        assert_eq!(
            moved["version"].as_i64().unwrap(),
            n["version"].as_i64().unwrap() + 2
        );
        let current = call(t.path(), json!({"op":"source.get","id":n["id"]}));
        assert_eq!(current["immutable_path"], source["immutable_path"]);
        let edited = call(
            t.path(),
            json!({"op":"note.update","id":n["id"],"expected_version":moved["version"],"body":"changed"}),
        );
        assert_eq!(edited["body"], "changed");
        assert_eq!(
            fs::read_to_string(source["immutable_path"].as_str().unwrap()).unwrap(),
            "original"
        );
        let root = call(
            t.path(),
            json!({"op":"note.move","id":n["id"],"folder":"","expected_version":edited["version"]}),
        );
        assert_eq!(root["path"], "새 제목.md");
        let noop = call(
            t.path(),
            json!({"op":"note.move","id":n["id"],"folder":"","expected_version":root["version"]}),
        );
        assert_eq!(noop, root);
        assert_eq!(
            call(
                t.path(),
                json!({"op":"note.rename","id":n["id"],"title":"새 제목"})
            ),
            root
        );
    }
    #[test]
    fn relocation_rejects_conflicts_stale_versions_and_bad_paths() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "first"), "first");
        let other = save(t.path(), &draft(t.path(), &w, "second"), "second");
        call(
            t.path(),
            json!({"op":"folder.create","workspace_id":w,"name":"destination"}),
        );
        let base = Path::new(n["absolute_path"].as_str().unwrap())
            .parent()
            .unwrap();
        fs::write(base.join("destination/first.md"), "external collision").unwrap();
        for request in [
            json!({"op":"note.rename","id":n["id"],"title":"second"}),
            json!({"op":"note.move","id":n["id"],"folder":"destination"}),
            json!({"op":"note.rename","id":n["id"],"title":"renamed","expected_version":0}),
            json!({"op":"note.move","id":n["id"],"folder":"","expected_version":0}),
            json!({"op":"note.rename","id":n["id"],"title":"../outside"}),
            json!({"op":"note.rename","id":n["id"],"title":".md"}),
            json!({"op":"note.rename","id":n["id"],"title":" "}),
            json!({"op":"note.move","id":n["id"],"folder":"../"}),
            json!({"op":"note.move","id":n["id"],"folder":"/tmp"}),
            json!({"op":"note.move","id":n["id"],"folder":"missing"}),
        ] {
            assert!(handle(t.path(), &request).is_err(), "{request}");
        }
        assert_eq!(call(t.path(), json!({"op":"note.read","id":n["id"]})), n);
        assert_eq!(
            call(t.path(), json!({"op":"note.read","id":other["id"]})),
            other
        );
        assert_eq!(
            fs::read_to_string(base.join("destination/first.md")).unwrap(),
            "external collision"
        );
        // A missing live document still owns its metadata path.
        fs::remove_file(other["absolute_path"].as_str().unwrap()).unwrap();
        assert!(handle(
            t.path(),
            &json!({"op":"note.rename","id":n["id"],"title":"second"})
        )
        .is_err());
    }
    #[test]
    fn relocation_rejects_drafts_missing_external_edits_and_symlinks() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let d = draft(t.path(), &w, "draft");
        for op in ["note.rename", "note.move"] {
            assert!(handle(
                t.path(),
                &json!({"op":op,"id":d["id"],"title":"new","folder":""})
            )
            .is_err());
        }
        let n = save(t.path(), &d, "note");
        let p = Path::new(n["absolute_path"].as_str().unwrap());
        let base = p.parent().unwrap();
        std::os::unix::fs::symlink(t.path(), base.join("escape")).unwrap();
        assert!(handle(
            t.path(),
            &json!({"op":"note.move","id":n["id"],"folder":"escape"})
        )
        .is_err());
        std::os::unix::fs::symlink(t.path().join("missing"), base.join("linked.md")).unwrap();
        assert!(handle(
            t.path(),
            &json!({"op":"note.rename","id":n["id"],"title":"linked"})
        )
        .is_err());
        fs::write(p, "external").unwrap();
        assert!(handle(
            t.path(),
            &json!({"op":"note.rename","id":n["id"],"title":"renamed"})
        )
        .is_err());
        assert_eq!(fs::read_to_string(p).unwrap(), "external");
        fs::remove_file(p).unwrap();
        assert!(handle(
            t.path(),
            &json!({"op":"note.move","id":n["id"],"folder":""})
        )
        .is_err());
        fs::write(t.path().join("outside.md"), "outside").unwrap();
        std::os::unix::fs::symlink(t.path().join("outside.md"), p).unwrap();
        assert!(handle(
            t.path(),
            &json!({"op":"note.rename","id":n["id"],"title":"renamed"})
        )
        .is_err());
    }
    #[test]
    fn relocation_preserves_imported_file_extension_and_bytes() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let input = t.path().join("image.PNG");
        let bytes = [0, 255, 1, 2, 3];
        fs::write(&input, bytes).unwrap();
        let n = call(
            t.path(),
            json!({"op":"material.import","workspace_id":w,"path":input}),
        );
        let renamed = call(
            t.path(),
            json!({"op":"note.rename","id":n["id"],"title":"새 이미지.png"}),
        );
        assert_eq!(renamed["title"], "새 이미지");
        assert_eq!(renamed["path"], "새 이미지.PNG");
        assert_eq!(renamed["kind"], "png");
        assert_eq!(
            fs::read(renamed["absolute_path"].as_str().unwrap()).unwrap(),
            bytes
        );
        assert_eq!(renamed["hash"], n["hash"]);
    }
    #[test]
    fn relocation_recovers_file_move_before_commit_and_only_once() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "crash safe"), "note");
        let mut c = connect(t.path()).unwrap();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let renamed = relocate(
            t.path(),
            &tx,
            &json!({"op":"note.rename","id":n["id"],"title":"recovered"}),
        )
        .unwrap()
        .0;
        tx.rollback().unwrap();
        drop(c);
        assert!(!Path::new(n["absolute_path"].as_str().unwrap()).exists());
        assert_eq!(
            call(t.path(), json!({"op":"note.read","id":n["id"]})),
            renamed
        );
        assert_eq!(
            call(t.path(), json!({"op":"note.read","id":n["id"]})),
            renamed
        );
        assert_eq!(
            fs::read_dir(t.path().join("pending-relocations"))
                .unwrap()
                .count(),
            0
        );
    }
    #[test]
    fn relocation_abandons_unstarted_journal_without_touching_collision() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "original"), "note");
        let c = connect(t.path()).unwrap();
        let d = document(&c, n["id"].as_str().unwrap()).unwrap();
        relocation_intent(t.path(), &c, &d, "other.md", "other").unwrap();
        let destination =
            Path::new(n["absolute_path"].as_str().unwrap()).with_file_name("other.md");
        fs::write(&destination, "unrelated").unwrap();
        assert_eq!(call(t.path(), json!({"op":"note.read","id":n["id"]})), n);
        assert_eq!(fs::read_to_string(destination).unwrap(), "unrelated");
        assert_eq!(
            fs::read_dir(t.path().join("pending-relocations"))
                .unwrap()
                .count(),
            0
        );
    }
    #[test]
    fn case_only_rename_and_unstarted_case_journal_preserve_file() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "case safe"), "note");
        let renamed = call(
            t.path(),
            json!({"op":"note.rename","id":n["id"],"title":"Note"}),
        );
        assert_eq!(renamed["path"], "Note.md");
        assert_eq!(renamed["body"], "case safe");
        assert_eq!(
            fs::read_to_string(renamed["absolute_path"].as_str().unwrap()).unwrap(),
            "case safe"
        );
        let c = connect(t.path()).unwrap();
        let d = document(&c, n["id"].as_str().unwrap()).unwrap();
        relocation_intent(t.path(), &c, &d, "NOTE.md", "NOTE").unwrap();
        let current = call(t.path(), json!({"op":"note.read","id":n["id"]}));
        assert_eq!(current["body"], "case safe");
        assert!(Path::new(current["absolute_path"].as_str().unwrap()).is_file());
    }
    #[test]
    fn committed_relocation_recovers_after_external_atomic_save() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "original"), "note");
        let mut c = connect(t.path()).unwrap();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let (renamed, _) = relocate(
            t.path(),
            &tx,
            &json!({"op":"note.rename","id":n["id"],"title":"renamed"}),
        )
        .unwrap();
        tx.commit().unwrap();
        // Simulate crash before cleanup, then an external editor's atomic save.
        atomic_write(
            Path::new(renamed["absolute_path"].as_str().unwrap()),
            b"external",
            true,
        )
        .unwrap();
        let current = call(t.path(), json!({"op":"note.read","id":n["id"]}));
        assert_eq!(current["body"], "external");
        assert_eq!(current["path"], "renamed.md");
        assert_eq!(
            fs::read_dir(t.path().join("pending-relocations"))
                .unwrap()
                .count(),
            0
        );
    }
    #[test]
    fn filesystem_move_never_clobbers_a_late_collision() {
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("source");
        let destination = t.path().join("destination");
        fs::write(&source, "source bytes").unwrap();
        fs::write(&destination, "collision bytes").unwrap();
        assert!(relocate_no_replace(&source, &destination).is_err());
        assert_eq!(fs::read_to_string(source).unwrap(), "source bytes");
        assert_eq!(fs::read_to_string(destination).unwrap(), "collision bytes");
    }
    #[test]
    fn superseded_committed_journals_do_not_replay_old_paths_or_revive_deletions() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let n = save(t.path(), &draft(t.path(), &w, "original"), "note");
        let mut c = connect(t.path()).unwrap();
        for title in ["second", "third"] {
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            relocate(
                t.path(),
                &tx,
                &json!({"op":"note.rename","id":n["id"],"title":title}),
            )
            .unwrap();
            tx.commit().unwrap();
            // Simulate cleanup interrupted for both committed operations.
        }
        let current = call(t.path(), json!({"op":"note.read","id":n["id"]}));
        assert_eq!(current["path"], "third.md");
        assert_eq!(
            fs::read_dir(t.path().join("pending-relocations"))
                .unwrap()
                .count(),
            0
        );
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        relocate(
            t.path(),
            &tx,
            &json!({"op":"note.rename","id":n["id"],"title":"fourth"}),
        )
        .unwrap();
        tx.commit().unwrap();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let d = document(&tx, n["id"].as_str().unwrap()).unwrap();
        let intent = deletion_intent(t.path(), &tx, &d).unwrap();
        finish_deletion(&tx, &intent).unwrap();
        tx.commit().unwrap();
        let tree = call(t.path(), json!({"op":"tree","workspace_id":w}));
        assert_eq!(tree["entries"], json!([]));
        assert_eq!(
            fs::read_dir(t.path().join("pending-relocations"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            fs::read_to_string(intent["destination"].as_str().unwrap()).unwrap(),
            "original"
        );
    }
    #[test]
    fn retained_originals_only_follow_explicit_analysis_snapshot_requests() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let source_file = t.path().join("import.pdf");
        fs::write(&source_file, b"%PDF fixture").unwrap();
        let pdf = call(t.path(), json!({"op":"material.import","workspace_id":w,"path":source_file}));
        let fresh = call(t.path(), json!({"op":"note.create","workspace_id":w}));
        let saved = save(t.path(), &draft(t.path(), &w, "first"), "saved");
        let edited = call(t.path(), json!({"op":"note.update","id":saved["id"],"body":"second","expected_version":saved["version"]}));
        for document in [&pdf, &fresh, &edited] {
            call(t.path(), json!({"op":"note.read","id":document["id"]}));
            let current = call(t.path(), json!({"op":"source.get","id":document["id"]}));
            assert!(current["immutable_path"].is_null());
            assert_eq!(current["analysis_state"], "unanalysed");
        }
        // Refreshing an externally edited file must not archive that edit.
        fs::write(edited["absolute_path"].as_str().unwrap(), "external").unwrap();
        let current = call(t.path(), json!({"op":"source.get","id":edited["id"]}));
        call(t.path(), json!({"op":"tree","workspace_id":w}));
        assert!(!t.path().join("versions").exists());
        assert!(handle(t.path(), &json!({"op":"source.snapshot","id":current["id"],"expected_hash":edited["hash"]})).is_err());
        assert!(!t.path().join("versions").exists());
        let snapshot = call(t.path(), json!({"op":"source.snapshot","id":current["id"],"expected_hash":current["hash"],"workspace_id":w}));
        call(t.path(), json!({"op":"source.snapshot","id":current["id"]}));
        assert_eq!(fs::read_dir(t.path().join("versions")).unwrap().count(), 1);
        assert_eq!(fs::read_to_string(snapshot["immutable_path"].as_str().unwrap()).unwrap(), "external");
        call(t.path(), json!({"op":"note.update","id":current["id"],"body":"third","expected_version":current["version"]}));
        assert_eq!(fs::read_dir(t.path().join("versions")).unwrap().count(), 1);
        let c = connect(t.path()).unwrap();
        assert_eq!(c.query_row("SELECT count(*) FROM source_versions", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn analysis_publication_lock_excludes_other_document_writers() {
        let t = tempfile::tempdir().unwrap();
        let w = ws(t.path());
        let note = save(t.path(), &draft(t.path(), &w, "stable"), "note");
        let sources = [(note["id"].as_str().unwrap().to_owned(), note["hash"].as_str().unwrap().to_owned())];
        with_current_sources(t.path(), &w, &sources, || {
            let second = connect(t.path())?;
            second.busy_timeout(std::time::Duration::from_millis(1))?;
            assert!(second.execute_batch("BEGIN IMMEDIATE").is_err());
            Ok(())
        }).unwrap();
        fs::write(note["absolute_path"].as_str().unwrap(), "external").unwrap();
        let mut published = false;
        assert!(with_current_sources(t.path(), &w, &sources, || {
            published = true;
            Ok(())
        }).is_err());
        assert!(!published);
    }

}
