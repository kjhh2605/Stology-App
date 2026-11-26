//! Immutable extraction snapshots and evidence-validated, transactional knowledge graph.
use crate::storage::digest;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

const MAX_INPUT: u64 = 100 * 1024 * 1024;
const MAX_CACHE: u64 = 512 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    pub kind: String,
    pub index: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceImage {
    pub path: String,
    pub mime: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub locator: Locator,
    pub text: String,
    pub images: Vec<SourceImage>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extraction {
    pub input_hash: String,
    pub kind: String,
    pub units: Vec<Unit>,
    #[serde(default)]
    pub warnings: Vec<String>,
}
pub(crate) const CONCEPT_KINDS: [&str; 6] = ["concept", "principle", "formula", "procedure", "example", "person_event"];
pub(crate) const LINK_KINDS: [&str; 8] = ["kind_of", "part_of", "prerequisite", "cause", "contrast", "example", "applies", "related"];
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Concept {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) definition: String,
    /// One of `CONCEPT_KINDS`. Absent in results made before the skill bundle.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Relation {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) definition: String,
    pub(crate) directed: bool,
    pub(crate) aliases: Vec<String>,
    /// One of `LINK_KINDS`.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Evidence {
    pub(crate) locator: Locator,
    pub(crate) quote: String,
    pub(crate) image_path: Option<String>,
    pub(crate) region: Option<Vec<f64>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Assertion {
    pub(crate) subject: String,
    pub(crate) relation: String,
    pub(crate) object: String,
    pub(crate) condition: String,
    pub(crate) negated: bool,
    pub(crate) evidence: Vec<Evidence>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Analysis {
    pub(crate) concepts: Vec<Concept>,
    pub(crate) relations: Vec<Relation>,
    pub(crate) assertions: Vec<Assertion>,
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("필수 항목이 없습니다: {key}"))
}
pub(crate) fn normalized(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn cache_path(root: &Path, hash: &str) -> Result<PathBuf> {
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        bail!("잘못된 원본 해시");
    }
    Ok(root.join("extractions").join(hash))
}
// PDF rendering/text extraction now includes annotations. Keep earlier image
// paths intact: existing graph evidence may still refer to the legacy cache.
const PDF_EXTRACTION_VERSION: &str = "pdf-annotations-v2";
pub(crate) fn extraction_cache_path(root: &Path, hash: &str, kind: &str) -> Result<PathBuf> {
    let base = cache_path(root, hash)?;
    Ok(if kind == "pdf" { base.join(PDF_EXTRACTION_VERSION) } else { base })
}

// Lower layer: what each material says, with evidence. The `ont_*` tables are the
// subject layer derived from it; `ont_hidden`, `ont_corrections` and `ont_guidance`
// hold the user's own corrections and are never replaced by a run.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS concepts(id TEXT PRIMARY KEY, workspace TEXT NOT NULL, name TEXT NOT NULL, aliases TEXT NOT NULL, definition TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS relations(id TEXT PRIMARY KEY, workspace TEXT NOT NULL, label TEXT NOT NULL, aliases TEXT NOT NULL, definition TEXT NOT NULL, directed INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS assertions(id TEXT PRIMARY KEY, workspace TEXT NOT NULL, subject TEXT NOT NULL REFERENCES concepts(id), relation TEXT NOT NULL REFERENCES relations(id), object TEXT NOT NULL REFERENCES concepts(id), condition TEXT NOT NULL, negated INTEGER NOT NULL, UNIQUE(workspace,subject,relation,object,condition,negated));
CREATE TABLE IF NOT EXISTS sources(workspace TEXT NOT NULL, material_id TEXT NOT NULL, input_hash TEXT NOT NULL, path TEXT NOT NULL, title TEXT NOT NULL, current INTEGER NOT NULL, PRIMARY KEY(workspace,material_id,input_hash));
CREATE TABLE IF NOT EXISTS supports(assertion_id TEXT NOT NULL REFERENCES assertions(id), workspace TEXT NOT NULL, material_id TEXT NOT NULL, input_hash TEXT NOT NULL, locator TEXT NOT NULL, quote TEXT NOT NULL, image_path TEXT, region TEXT, UNIQUE(assertion_id,material_id,input_hash,locator,quote));
CREATE TABLE IF NOT EXISTS analyses(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,material_id TEXT NOT NULL,input_hash TEXT NOT NULL,provider TEXT NOT NULL,model TEXT NOT NULL,rules_version TEXT NOT NULL,status TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
CREATE TABLE IF NOT EXISTS concept_kinds(concept_id TEXT PRIMARY KEY, workspace TEXT NOT NULL, kind TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS relation_kinds(relation_id TEXT PRIMARY KEY, workspace TEXT NOT NULL, kind TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS ont_generation(workspace TEXT PRIMARY KEY, n INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS ont_revisions(workspace TEXT NOT NULL, revision INTEGER NOT NULL, created_at TEXT NOT NULL, run_id TEXT NOT NULL, summary TEXT NOT NULL, PRIMARY KEY(workspace,revision));
CREATE TABLE IF NOT EXISTS ont_concepts(workspace TEXT NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL, definition TEXT NOT NULL, unit_id TEXT, PRIMARY KEY(workspace,id));
CREATE TABLE IF NOT EXISTS ont_members(workspace TEXT NOT NULL, member_id TEXT NOT NULL, concept_id TEXT NOT NULL, member_name TEXT NOT NULL, revision INTEGER NOT NULL, PRIMARY KEY(workspace,member_id));
CREATE TABLE IF NOT EXISTS ont_units(workspace TEXT NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL, description TEXT NOT NULL, position INTEGER NOT NULL, PRIMARY KEY(workspace,id));
CREATE TABLE IF NOT EXISTS ont_kept_concepts(workspace TEXT NOT NULL, id TEXT NOT NULL, PRIMARY KEY(workspace,id));
CREATE TABLE IF NOT EXISTS ont_deleted_nodes(workspace TEXT NOT NULL,id TEXT NOT NULL,PRIMARY KEY(workspace,id));
CREATE TABLE IF NOT EXISTS ont_deleted_links(workspace TEXT NOT NULL, fingerprint TEXT NOT NULL, PRIMARY KEY(workspace,fingerprint));
CREATE TABLE IF NOT EXISTS ont_links(workspace TEXT NOT NULL, id TEXT NOT NULL, subject TEXT NOT NULL, object TEXT NOT NULL, kind TEXT NOT NULL, reason TEXT NOT NULL, condition TEXT NOT NULL, negated INTEGER NOT NULL, PRIMARY KEY(workspace,id));
CREATE TABLE IF NOT EXISTS ont_material_state(workspace TEXT NOT NULL, material_id TEXT NOT NULL, input_hash TEXT NOT NULL, quality TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL, run_id TEXT NOT NULL, PRIMARY KEY(workspace,material_id));
CREATE TABLE IF NOT EXISTS ont_hidden(workspace TEXT NOT NULL, subject TEXT NOT NULL, object TEXT NOT NULL, kind TEXT NOT NULL, PRIMARY KEY(workspace,subject,object,kind));
CREATE TABLE IF NOT EXISTS ont_corrections(workspace TEXT NOT NULL, a TEXT NOT NULL, b TEXT NOT NULL, PRIMARY KEY(workspace,a,b));
CREATE TABLE IF NOT EXISTS ont_guidance(workspace TEXT PRIMARY KEY, text TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS ont_notes(workspace TEXT NOT NULL, concept_id TEXT NOT NULL, definition TEXT NOT NULL, PRIMARY KEY(workspace,concept_id));";
pub(crate) fn open(path: &Path) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(std::time::Duration::from_secs(10))?;
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")?;
    c.execute_batch(SCHEMA)?;
    Ok(c)
}
fn has_table(c: &Connection, name: &str) -> Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [name], |r| r.get(0))?)
}
pub(crate) fn db(root: &Path) -> Result<Connection> {
    fs::create_dir_all(root)?;
    let path = root.join("knowledge.sqlite");
    // Keep one copy of a database from before the subject layer, made the first
    // time the new tables are added to it.
    let backup = root.join("knowledge.before-subject-layer.sqlite");
    if path.is_file() && !backup.exists() {
        let c = Connection::open(&path)?;
        c.busy_timeout(std::time::Duration::from_secs(10))?;
        if has_table(&c, "concepts")? && !has_table(&c, "ont_revisions")? {
            c.execute("VACUUM INTO ?1", [backup.to_string_lossy()])?;
        }
    }
    open(&path)
}
pub fn handle(root: &Path, request: &Value) -> Result<Value> {
    match required(request, "op")? {
        "extract" => Ok(serde_json::to_value(extract(
            root,
            Path::new(required(request, "path")?),
        )?)?),
        "graph.apply" => apply(root, request),
        "graph.get" | "graph.status" => get_graph(root, required(request, "workspace_id")?),
        "graph.remove_source" => remove_source(root, required(request, "material_id")?),
        _ => bail!("알 수 없는 지식 명령"),
    }
}
pub(crate) fn helper_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("STOLOGY_EXTRACTOR") {
        return Ok(p.into());
    }
    let exe = std::env::current_exe()?;
    for p in [
        exe.parent().unwrap().join("stology-extract"),
        exe.parent().unwrap().join("../Resources/stology-extract"),
    ] {
        if p.exists() {
            return Ok(p);
        }
    }
    bail!("시스템 PDF·이미지 추출 도구를 찾을 수 없습니다. 앱을 다시 설치하거나 STOLOGY_EXTRACTOR를 지정하세요.")
}
fn enforce_cache_limit(root: &Path, active: &Path) -> Result<()> {
    let base = root.join("extractions");
    let mut dirs = Vec::new();
    let mut total = 0;
    if !base.exists() {
        return Ok(());
    }
    for ent in fs::read_dir(&base)? {
        let ent = ent?;
        if !ent.file_type()?.is_dir() {
            continue;
        }
        let size = dir_size(&ent.path())?;
        total += size;
        dirs.push((ent.metadata()?.modified()?, ent.path(), size));
    }
    dirs.sort_by_key(|x| x.0);
    for (_, p, size) in dirs {
        if total <= MAX_CACHE {
            break;
        }
        if p != active {
            fs::remove_dir_all(p)?;
            total -= size;
        }
    }
    if total > MAX_CACHE {
        bail!("자료 추출 캐시가 512 MiB 제한을 초과했습니다.");
    }
    Ok(())
}
fn dir_size(path: &Path) -> Result<u64> {
    let mut n = 0;
    for e in fs::read_dir(path)? {
        let e = e?;
        let m = e.metadata()?;
        n += if m.is_dir() {
            dir_size(&e.path())?
        } else {
            m.len()
        };
    }
    Ok(n)
}
pub fn extract(root: &Path, path: &Path) -> Result<Extraction> {
    extract_with_helper(root, path, None)
}
fn extract_with_helper(root: &Path, path: &Path, native_helper: Option<&Path>) -> Result<Extraction> {
    let meta = fs::metadata(path)
        .with_context(|| format!("원본을 읽을 수 없습니다: {}", path.display()))?;
    if !meta.is_file() || meta.len() == 0 {
        bail!("비어 있거나 일반 파일이 아닌 자료입니다.");
    }
    if meta.len() > MAX_INPUT {
        bail!("원본 파일은 100 MiB 이하여야 합니다.");
    }
    let bytes = fs::read(path)?;
    let hash = digest(&bytes);
    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = match ext.as_str() {
        "md" | "markdown" => "md",
        "pdf" => "pdf",
        "png" | "jpg" | "jpeg" | "heic" => "image",
        "pptx" => "pptx",
        _ => bail!("지원하지 않는 형식: {ext}. PDF, MD, PNG/JPEG/HEIC, PPTX를 사용하세요."),
    };
    let cache_root = cache_path(root, &hash)?;
    let cache = extraction_cache_path(root, &hash, kind)?;
    fs::create_dir_all(&cache)?;
    // Share the immutable original across extractor versions, but never reuse
    // old PDF text/images or replace image paths held by existing evidence.
    let snapshot = cache_root.join(format!("original.{ext}"));
    if !snapshot.exists() {
        fs::write(&snapshot, &bytes)?;
    } else if digest(&fs::read(&snapshot)?) != hash {
        bail!("고정된 추출 원본이 손상되었습니다. 분석을 중단했습니다.");
    }
    let cached = cache.join("extraction.json");
    if cached.exists() {
        let e: Extraction = serde_json::from_slice(&fs::read(&cached)?)?;
        if e.input_hash == hash && e.kind == kind && e.units
            .iter()
            .flat_map(|u| &u.images)
            .all(|i| Path::new(&i.path).exists())
        {
            enforce_cache_limit(root, &cache_root)?;
            return Ok(e);
        }
    }
    let mut result = match kind {
        "md" => {
            let text = String::from_utf8(bytes).context("Markdown은 UTF-8 형식이어야 합니다.")?;
            let text = text.replace("\r\n", "\n");
            let units = text
                .split("\n\n")
                .enumerate()
                .filter(|(_, s)| !s.trim().is_empty())
                .map(|(i, s)| Unit {
                    locator: Locator {
                        kind: "paragraph".into(),
                        index: i as u32 + 1,
                    },
                    text: s.to_string(),
                    images: vec![],
                })
                .collect::<Vec<_>>();
            Extraction {
                input_hash: hash.clone(),
                kind: kind.into(),
                units,
                warnings: vec![],
            }
        }
        "pptx" => extract_pptx(&snapshot, &cache, &hash)?,
        _ => {
            let helper = match native_helper {
                Some(path) => path.to_owned(),
                None => helper_path()?,
            };
            let output = Command::new(helper)
                .arg(&snapshot)
                .arg(&cache)
                .output()
                .context("시스템 자료 추출기를 시작할 수 없습니다.")?;
            if !output.status.success() {
                bail!(
                    "자료 추출 실패: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let mut v: Value = serde_json::from_slice(&output.stdout)
                .context("시스템 추출기의 응답이 올바르지 않습니다.")?;
            v["input_hash"] = json!(hash);
            serde_json::from_value(v)?
        }
    };
    if result.units.is_empty() {
        bail!("분석 가능한 페이지·문단·이미지가 없습니다.");
    }
    if result.units.len() > 1000 {
        bail!("자료 단위가 1,000개를 초과했습니다. 자료를 나누어 주세요.");
    }
    result.input_hash = hash;
    let tmp = cache.join("extraction.json.tmp");
    fs::write(&tmp, serde_json::to_vec(&result)?)?;
    fs::rename(tmp, cached)?;
    enforce_cache_limit(root, &cache_root)?;
    Ok(result)
}
fn zip_text<R: Read + std::io::Seek>(
    z: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<String>> {
    match z.by_name(name) {
        Ok(mut f) => {
            if f.size() > 10 * 1024 * 1024 {
                bail!("PPTX XML 항목이 제한을 초과했습니다.");
            }
            let mut s = String::new();
            f.read_to_string(&mut s)
                .context("PPTX XML이 손상되었습니다.")?;
            Ok(Some(s))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn xml_text(xml: &str) -> Result<String> {
    use quick_xml::{events::Event, Reader};
    let mut r = Reader::from_str(xml);
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match r.read_event()? {
            Event::Start(e) => {
                if e.local_name().as_ref() == b"t" {
                    in_text = true;
                }
            }
            Event::End(e) => {
                if e.local_name().as_ref() == b"t" {
                    in_text = false;
                    out.push(' ');
                }
                if e.local_name().as_ref() == b"p" {
                    out.push('\n');
                }
            }
            Event::Text(e) if in_text => {
                out.push_str(&e.unescape()?);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out.trim().into())
}
fn relationships(xml: &str) -> Result<HashMap<String, (String, String)>> {
    use quick_xml::{events::Event, Reader};
    let mut r = Reader::from_str(xml);
    let mut out = HashMap::new();
    loop {
        match r.read_event()? {
            Event::Empty(e) | Event::Start(e) if e.local_name().as_ref() == b"Relationship" => {
                let mut id = String::new();
                let mut target = String::new();
                let mut typ = String::new();
                let mut external = false;
                for a in e.attributes() {
                    let a = a?;
                    let v = a.decode_and_unescape_value(r.decoder())?.to_string();
                    match a.key.as_ref() {
                        b"Id" => id = v,
                        b"Target" => target = v,
                        b"Type" => typ = v,
                        b"TargetMode" => external = v == "External",
                        _ => {}
                    }
                }
                if !external {
                    out.insert(id, (target, typ));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}
fn zip_resolve(base: &str, target: &str) -> Result<String> {
    let target = target.replace('\\', "/");
    let combined = if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else {
        format!("{base}/{target}")
    };
    let mut parts = Vec::new();
    for p in Path::new(&combined).components() {
        match p {
            Component::Normal(x) => parts.push(x.to_string_lossy().to_string()),
            Component::ParentDir => {
                if parts.pop().is_none() {
                    bail!("PPTX 내부 경로가 잘못되었습니다.");
                }
            }
            Component::CurDir => {}
            _ => bail!("PPTX 내부 경로가 잘못되었습니다."),
        }
    }
    Ok(parts.join("/"))
}
fn extract_pptx(path: &Path, cache: &Path, hash: &str) -> Result<Extraction> {
    use quick_xml::{events::Event, Reader};
    let mut z = zip::ZipArchive::new(fs::File::open(path)?)
        .context("PPTX ZIP 구조가 손상되었거나 암호화되었습니다.")?;
    if z.len() > 10000 {
        bail!("PPTX 항목 수가 제한을 초과했습니다.");
    }
    let mut total = 0u64;
    for i in 0..z.len() {
        let f = z.by_index(i)?;
        total = total
            .checked_add(f.size())
            .ok_or_else(|| anyhow!("PPTX 크기 오류"))?;
        if f.size() > MAX_INPUT || total > 256 * 1024 * 1024 {
            bail!("PPTX 압축 해제 크기가 제한을 초과했습니다.");
        }
    }
    let presentation = zip_text(&mut z, "ppt/presentation.xml")?
        .ok_or_else(|| anyhow!("유효한 PPTX 프레젠테이션이 아닙니다."))?;
    let rels = relationships(
        &zip_text(&mut z, "ppt/_rels/presentation.xml.rels")?
            .ok_or_else(|| anyhow!("PPTX 슬라이드 연결 정보가 없습니다."))?,
    )?;
    let mut r = Reader::from_str(&presentation);
    let mut slides = Vec::new();
    loop {
        match r.read_event()? {
            Event::Empty(e) | Event::Start(e) if e.local_name().as_ref() == b"sldId" => {
                for a in e.attributes() {
                    let a = a?;
                    if a.key.as_ref() == b"r:id" {
                        let id = a.decode_and_unescape_value(r.decoder())?;
                        let (target, _) = rels
                            .get(id.as_ref())
                            .ok_or_else(|| anyhow!("PPTX 슬라이드 연결이 손상되었습니다."))?;
                        slides.push(zip_resolve("ppt", target)?);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if slides.len() > 100 {
        bail!("PPTX는 최대 100개 슬라이드까지 지원합니다.");
    }
    let mut units = Vec::new();
    let mut warnings=vec!["SmartArt 및 복잡한 도형 배치는 완전히 해석하지 않습니다. 시각 구조가 중요하면 PDF로 가져오세요.".into()];
    for (index, slide) in slides.iter().enumerate() {
        let xml = zip_text(&mut z, slide)?
            .ok_or_else(|| anyhow!("PPTX 슬라이드 파일이 없습니다: {slide}"))?;
        let mut text = xml_text(&xml)?;
        let (base, file) = slide
            .rsplit_once('/')
            .ok_or_else(|| anyhow!("잘못된 슬라이드 경로"))?;
        let rs = relationships(
            &zip_text(&mut z, &format!("{base}/_rels/{file}.rels"))?.unwrap_or_default(),
        )?;
        let mut images = Vec::new();
        let mut sorted = rs.into_iter().collect::<Vec<_>>();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for (rid, (target, typ)) in sorted {
            let entry = zip_resolve(base, &target)?;
            if typ.ends_with("/notesSlide") {
                if let Some(notes) = zip_text(&mut z, &entry)? {
                    let notes = xml_text(&notes)?;
                    if !notes.is_empty() {
                        text.push_str("\n발표자 노트:\n");
                        text.push_str(&notes);
                    }
                }
            }
            if typ.ends_with("/image") {
                let ext = Path::new(&entry)
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let mime = match ext.as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    _ => {
                        warnings.push(format!("슬라이드 {}의 {ext} 내장 이미지는 분석하지 못했습니다. PDF 변환을 사용하세요.",index+1));
                        continue;
                    }
                };
                let mut f = z
                    .by_name(&entry)
                    .with_context(|| format!("PPTX 내장 이미지가 없습니다: {entry}"))?;
                let mut bytes = Vec::new();
                f.read_to_end(&mut bytes)?;
                let safe_rid = rid
                    .chars()
                    .filter(|x| x.is_ascii_alphanumeric())
                    .collect::<String>();
                let output = cache.join(format!("slide-{:04}-{safe_rid}.{ext}", index + 1));
                fs::write(&output, bytes)?;
                images.push(SourceImage {
                    path: output.to_string_lossy().into(),
                    mime: mime.into(),
                });
            }
        }
        units.push(Unit {
            locator: Locator {
                kind: "slide".into(),
                index: index as u32 + 1,
            },
            text,
            images,
        });
    }
    Ok(Extraction {
        input_hash: hash.into(),
        kind: "pptx".into(),
        units,
        warnings,
    })
}
/// Whitespace-insensitive form used to find a quote in OCR text.
fn squashed(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub(crate) struct Checked {
    pub(crate) analysis: Analysis,
    /// Claims the provider returned, and how many failed the checks.
    pub(crate) total: usize,
    pub(crate) dropped: usize,
    /// Kept claims that named a concept or relation instead of using its id.
    pub(crate) repaired: usize,
}
/// What a claim may write in place of an id: names and aliases for concepts,
/// labels, aliases and the type for relations. Only a name that points to one
/// id is used; anything ambiguous stays unresolved and the claim is dropped.
struct Names(HashMap<String, Vec<String>>);
impl Names {
    fn new() -> Self {
        Names(HashMap::new())
    }
    fn add(&mut self, name: &str, id: &str) {
        let ids = self.0.entry(normalized(name)).or_default();
        if !ids.iter().any(|known| known == id) {
            ids.push(id.to_owned());
        }
    }
    fn one(&self, name: &str) -> Option<&str> {
        self.0.get(&normalized(name)).filter(|ids| ids.len() == 1).map(|ids| ids[0].as_str())
    }
}
/// Keep only claims whose concepts, relation and evidence really exist in the
/// source. A wrong claim is dropped on its own; the caller judges the ratio.
pub(crate) fn check(a: Analysis, e: &Extraction) -> Result<Checked> {
    if a.concepts.len() > 2000 || a.relations.len() > 500 || a.assertions.len() > 5000 {
        bail!("분석 결과가 크기 제한을 초과했습니다.");
    }
    let total = a.assertions.len();
    let mut concepts = HashMap::new();
    for mut c in a.concepts {
        let valid = !c.id.is_empty() && !c.name.trim().is_empty() && !c.definition.trim().is_empty() && c.name.len() <= 1000 && c.definition.len() <= 8000;
        if valid && !concepts.contains_key(&c.id) {
            c.kind = Some(c.kind.filter(|k| CONCEPT_KINDS.contains(&k.as_str())).unwrap_or_else(|| "concept".into()));
            concepts.insert(c.id.clone(), c);
        }
    }
    let mut relations = HashMap::new();
    for mut r in a.relations {
        let valid = !r.id.is_empty() && !r.label.trim().is_empty() && !r.definition.trim().is_empty() && r.definition.len() <= 8000;
        if valid && !relations.contains_key(&r.id) {
            r.kind = Some(r.kind.filter(|k| LINK_KINDS.contains(&k.as_str())).unwrap_or_else(|| "related".into()));
            relations.insert(r.id.clone(), r);
        }
    }
    let mut concept_names = Names::new();
    for c in concepts.values() {
        for name in std::iter::once(&c.name).chain(&c.aliases) {
            concept_names.add(name, &c.id);
        }
    }
    let (mut relation_labels, mut relation_kinds) = (Names::new(), Names::new());
    for r in relations.values() {
        for label in std::iter::once(&r.label).chain(&r.aliases) {
            relation_labels.add(label, &r.id);
        }
        relation_kinds.add(r.kind.as_deref().unwrap_or(""), &r.id);
    }
    let concept_id = |given: &str| -> Option<String> {
        if concepts.contains_key(given) { Some(given.to_owned()) } else { concept_names.one(given).map(str::to_owned) }
    };
    let relation_id = |given: &str| -> Option<String> {
        if relations.contains_key(given) {
            Some(given.to_owned())
        } else {
            relation_labels.one(given).or_else(|| relation_kinds.one(given)).map(str::to_owned)
        }
    };
    let squashed_units: HashMap<&Locator, String> = e.units.iter().map(|u| (&u.locator, squashed(&u.text))).collect();
    let mut assertions = Vec::new();
    let mut repaired = 0;
    for mut claim in a.assertions {
        let (Some(subject), Some(relation), Some(object)) = (concept_id(&claim.subject), relation_id(&claim.relation), concept_id(&claim.object)) else {
            continue;
        };
        if claim.condition.len() > 8000 {
            continue;
        }
        let renamed = subject != claim.subject || relation != claim.relation || object != claim.object;
        (claim.subject, claim.relation, claim.object) = (subject, relation, object);
        let mut evidence = Vec::new();
        for mut ev in claim.evidence.into_iter().take(100) {
            let Some(unit) = e.units.iter().find(|u| u.locator == ev.locator) else { continue };
            let quote = squashed(&ev.quote);
            let quote_ok = !quote.is_empty() && (unit.text.contains(&ev.quote) || squashed_units[&unit.locator].contains(&quote));
            let image_ok = ev.image_path.as_ref().is_some_and(|p| unit.images.iter().any(|i| &i.path == p))
                && ev.region.as_ref().is_some_and(|r| {
                    r.len() == 4 && r.iter().all(|x| x.is_finite() && *x >= 0.0 && *x <= 1.0) && r[2] > 0.0 && r[3] > 0.0 && r[0] + r[2] <= 1.000001 && r[1] + r[3] <= 1.000001
                });
            if !quote_ok {
                ev.quote.clear();
            }
            if !image_ok {
                ev.image_path = None;
                ev.region = None;
            }
            if quote_ok || image_ok {
                evidence.push(ev);
            }
        }
        if !evidence.is_empty() {
            claim.evidence = evidence;
            repaired += usize::from(renamed);
            assertions.push(claim);
        }
    }
    let used_concepts: HashSet<&str> = assertions.iter().flat_map(|c| [c.subject.as_str(), c.object.as_str()]).collect();
    let used_relations: HashSet<&str> = assertions.iter().map(|c| c.relation.as_str()).collect();
    let mut kept_concepts: Vec<Concept> = concepts.into_values().filter(|c| used_concepts.contains(c.id.as_str())).collect();
    let mut kept_relations: Vec<Relation> = relations.into_values().filter(|r| used_relations.contains(r.id.as_str())).collect();
    kept_concepts.sort_by(|a, b| a.id.cmp(&b.id));
    kept_relations.sort_by(|a, b| a.id.cmp(&b.id));
    let dropped = total - assertions.len();
    Ok(Checked { analysis: Analysis { concepts: kept_concepts, relations: kept_relations, assertions }, total, dropped, repaired })
}
fn candidate_concept(
    c: &Connection,
    w: &str,
    new: &Concept,
) -> Result<Option<(String, Vec<String>)>> {
    let mut q = c.prepare("SELECT id,name,aliases,definition FROM concepts WHERE workspace=?1")?;
    let rows = q.query_map([w], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let new_names = std::iter::once(&new.name)
        .chain(new.aliases.iter())
        .map(|s| normalized(s))
        .collect::<HashSet<_>>();
    for row in rows {
        let (id, name, aliases, definition) = row?;
        let mut aliases: Vec<String> = serde_json::from_str(&aliases)?;
        // Exact definition equality is intentionally conservative: matching spelling alone never merges homonyms.
        if normalized(&definition) == normalized(&new.definition)
            && std::iter::once(&name)
                .chain(aliases.iter())
                .any(|s| new_names.contains(&normalized(s)))
        {
            if new.name != name && !aliases.contains(&new.name) {
                aliases.push(new.name.clone());
            }
            for a in &new.aliases {
                if a != &name && !aliases.contains(a) {
                    aliases.push(a.clone());
                }
            }
            return Ok(Some((id, aliases)));
        }
    }
    Ok(None)
}
fn candidate_relation(
    c: &Connection,
    w: &str,
    new: &Relation,
) -> Result<Option<(String, Vec<String>)>> {
    let mut q =
        c.prepare("SELECT id,label,aliases,definition,directed FROM relations WHERE workspace=?1")?;
    let rows = q.query_map([w], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, bool>(4)?,
        ))
    })?;
    let names = std::iter::once(&new.label)
        .chain(new.aliases.iter())
        .map(|s| normalized(s))
        .collect::<HashSet<_>>();
    for row in rows {
        let (id, label, aliases, definition, directed) = row?;
        let mut aliases: Vec<String> = serde_json::from_str(&aliases)?;
        if directed == new.directed
            && normalized(&definition) == normalized(&new.definition)
            && std::iter::once(&label)
                .chain(aliases.iter())
                .any(|s| names.contains(&normalized(s)))
        {
            if new.label != label && !aliases.contains(&new.label) {
                aliases.push(new.label.clone());
            }
            for a in &new.aliases {
                if a != &label && !aliases.contains(a) {
                    aliases.push(a.clone());
                }
            }
            return Ok(Some((id, aliases)));
        }
    }
    Ok(None)
}
fn current_source(root: &Path, id: &str, w: &str, hash: &str) -> Result<Value> {
    let src = crate::storage::handle(root, &json!({"op":"source.get","id":id}))?;
    if src["workspace_id"].as_str() != Some(w) {
        bail!("다른 과목의 자료를 지도에 반영할 수 없습니다.");
    }
    if src["hash"].as_str() != Some(hash) {
        bail!("분석 중 자료가 편집되거나 필기가 저장되었습니다. 현재 자료와 기존 지도는 보존했습니다. 파일 메뉴에서 다시 분석을 요청해 주세요.");
    }
    Ok(src)
}
/// One material's checked result and where it came from.
pub(crate) struct Material<'a> {
    pub(crate) workspace: &'a str,
    pub(crate) id: &'a str,
    pub(crate) hash: &'a str,
    pub(crate) immutable_path: &'a str,
    pub(crate) title: &'a str,
    pub(crate) provider: &'a str,
    pub(crate) model: &'a str,
    /// Bundle and skill version with the instruction fingerprint.
    pub(crate) rules: &'a str,
}
pub(crate) fn bump_generation(c: &Connection, w: &str) -> Result<()> {
    c.execute("INSERT INTO ont_generation VALUES(?1,1) ON CONFLICT(workspace) DO UPDATE SET n=n+1", [w])?;
    Ok(())
}
pub(crate) fn generation(c: &Connection, w: &str) -> Result<i64> {
    Ok(c.query_row("SELECT n FROM ont_generation WHERE workspace=?1", [w], |r| r.get(0)).optional()?.unwrap_or(0))
}
fn apply(root: &Path, v: &Value) -> Result<Value> {
    let w = required(v, "workspace_id")?;
    let material = required(v, "material_id")?;
    let hash = required(v, "input_hash")?;
    let source = current_source(root, material, w, hash)?;
    let immutable_path = source["immutable_path"].as_str()
        .filter(|path| Path::new(path).is_file())
        .context("분석 요청 시 고정한 원본이 없습니다. 파일 메뉴에서 다시 분석을 요청해 주세요.")?;
    let extraction: Extraction = serde_json::from_slice(
        &fs::read(extraction_cache_path(root, hash, source["kind"].as_str().unwrap_or(""))?.join("extraction.json"))
            .context("고정된 추출 원본이 없습니다. 먼저 자료 추출을 실행하세요.")?,
    )?;
    if extraction.input_hash != hash {
        bail!("추출 원본 해시가 일치하지 않습니다.");
    }
    let result: Analysis = serde_json::from_value(
        v.get("result")
            .cloned()
            .ok_or_else(|| anyhow!("분석 결과가 없습니다."))?,
    )
    .context("분석 JSON 구조가 올바르지 않습니다.")?;
    let checked = check(result, &extraction)?;
    if checked.analysis.assertions.is_empty() {
        bail!("근거가 있는 관계를 추출하지 못했습니다. 빈 지도는 성공으로 반영하지 않습니다.");
    }
    let mut c = db(root)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let previous:Option<i32>=tx.query_row("SELECT 1 FROM sources WHERE workspace=?1 AND material_id=?2 AND input_hash=?3 AND current=1",params![w,material,hash],|r|r.get(0)).optional()?;
    if previous.is_some() && v["force"] != true {
        return Ok(json!({"applied":false,"duplicate":true,"input_hash":hash}));
    }
    let analysis_id = apply_checked(&tx, &Material {
        workspace: w, id: material, hash, immutable_path, title: source["title"].as_str().unwrap_or(""),
        provider: required(v, "provider")?, model: v["model"].as_str().unwrap_or("provider-default"),
        rules: v["rules"].as_str().unwrap_or("manual"),
    }, &checked.analysis)?;
    if let Some(quality) = v["quality"].as_str() {
        tx.execute("INSERT OR REPLACE INTO ont_material_state VALUES(?1,?2,?3,?4,?5,?6,?7)", params![w, material, hash, quality, required(v, "provider")?, v["model"].as_str().unwrap_or(""), v["run_id"].as_str().unwrap_or("")])?;
    }
    crate::storage::with_current_sources(root, w, &[(material.to_owned(), hash.to_owned())], || {
        tx.commit()?;
        Ok(())
    })?;
    Ok(
        json!({"applied":true,"duplicate":false,"analysis_id":analysis_id,"input_hash":hash,"assertions":checked.analysis.assertions.len(),"dropped":checked.dropped,"total":checked.total}),
    )
}
/// Replace one material's support inside the caller's transaction. Other
/// materials keep their claims and evidence.
pub(crate) fn apply_checked(tx: &Connection, m: &Material, result: &Analysis) -> Result<String> {
    let w = m.workspace;
    let mut concepts = HashMap::new();
    let mut relations = HashMap::new();
    for new in &result.concepts {
        let id = if let Some((id, aliases)) = candidate_concept(tx, w, new)? {
            tx.execute(
                "UPDATE concepts SET aliases=?1 WHERE id=?2",
                params![serde_json::to_string(&aliases)?, id],
            )?;
            id
        } else {
            let id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO concepts VALUES(?1,?2,?3,?4,?5)",
                params![
                    id,
                    w,
                    new.name,
                    serde_json::to_string(&new.aliases)?,
                    new.definition
                ],
            )?;
            id
        };
        if let Some(kind) = &new.kind {
            tx.execute("INSERT OR IGNORE INTO concept_kinds VALUES(?1,?2,?3)", params![id, w, kind])?;
        }
        concepts.insert(new.id.clone(), id);
    }
    for new in &result.relations {
        let id = if let Some((id, aliases)) = candidate_relation(tx, w, new)? {
            tx.execute(
                "UPDATE relations SET aliases=?1 WHERE id=?2",
                params![serde_json::to_string(&aliases)?, id],
            )?;
            id
        } else {
            let id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO relations VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    id,
                    w,
                    new.label,
                    serde_json::to_string(&new.aliases)?,
                    new.definition,
                    new.directed
                ],
            )?;
            id
        };
        if let Some(kind) = &new.kind {
            tx.execute("INSERT OR IGNORE INTO relation_kinds VALUES(?1,?2,?3)", params![id, w, kind])?;
        }
        relations.insert(new.id.clone(), id);
    }
    // Replace only this material's support. Other sources retain their claims and evidence.
    tx.execute(
        "DELETE FROM supports WHERE workspace=?1 AND material_id=?2",
        params![w, m.id],
    )?;
    tx.execute(
        "UPDATE sources SET current=0 WHERE workspace=?1 AND material_id=?2",
        params![w, m.id],
    )?;
    tx.execute("INSERT INTO sources VALUES(?1,?2,?3,?4,?5,1) ON CONFLICT(workspace,material_id,input_hash) DO UPDATE SET current=1,path=excluded.path,title=excluded.title",params![w,m.id,m.hash,m.immutable_path,m.title])?;
    for claim in &result.assertions {
        let mut subject = concepts[&claim.subject].clone();
        let mut object = concepts[&claim.object].clone();
        let relation = &relations[&claim.relation];
        let definition = result
            .relations
            .iter()
            .find(|r| r.id == claim.relation)
            .unwrap();
        if !definition.directed && subject > object {
            std::mem::swap(&mut subject, &mut object);
        }
        let old:Option<String>=tx.query_row("SELECT id FROM assertions WHERE workspace=?1 AND subject=?2 AND relation=?3 AND object=?4 AND condition=?5 AND negated=?6",params![w,subject,relation,object,claim.condition,claim.negated],|r|r.get(0)).optional()?;
        let id = old.unwrap_or_else(|| Uuid::new_v4().to_string());
        tx.execute(
            "INSERT OR IGNORE INTO assertions VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                w,
                subject,
                relation,
                object,
                claim.condition,
                claim.negated
            ],
        )?;
        for ev in &claim.evidence {
            tx.execute(
                "INSERT OR IGNORE INTO supports VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    id,
                    w,
                    m.id,
                    m.hash,
                    serde_json::to_string(&ev.locator)?,
                    ev.quote,
                    ev.image_path,
                    ev.region.as_ref().map(serde_json::to_string).transpose()?
                ],
            )?;
        }
    }
    crate::ontology::remove_deleted_links(tx, w)?;
    prune(tx)?;
    let analysis_id = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO analyses(id,workspace,material_id,input_hash,provider,model,rules_version,status) VALUES(?1,?2,?3,?4,?5,?6,?7,'succeeded')",params![analysis_id,w,m.id,m.hash,m.provider,m.model,m.rules])?;
    bump_generation(tx, w)?;
    Ok(analysis_id)
}
fn prune(c: &Connection) -> Result<()> {
    c.execute("DELETE FROM assertions WHERE NOT EXISTS(SELECT 1 FROM supports WHERE assertion_id=assertions.id)",[])?;
    c.execute("DELETE FROM concepts WHERE NOT EXISTS(SELECT 1 FROM ont_kept_concepts WHERE id=concepts.id AND workspace=concepts.workspace) AND NOT EXISTS(SELECT 1 FROM assertions WHERE subject=concepts.id OR object=concepts.id)",[])?;
    c.execute("DELETE FROM relations WHERE NOT EXISTS(SELECT 1 FROM assertions WHERE relation=relations.id)",[])?;
    c.execute("DELETE FROM concept_kinds WHERE NOT EXISTS(SELECT 1 FROM concepts WHERE id=concept_id)",[])?;
    c.execute("DELETE FROM relation_kinds WHERE NOT EXISTS(SELECT 1 FROM relations WHERE id=relation_id)",[])?;
    Ok(())
}
fn remove_source(root: &Path, id: &str) -> Result<Value> {
    let mut c = db(root)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let workspaces: Vec<String> = tx.prepare("SELECT DISTINCT workspace FROM sources WHERE material_id=?1")?
        .query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let removed = tx.execute("DELETE FROM supports WHERE material_id=?1", [id])?;
    tx.execute("DELETE FROM sources WHERE material_id=?1", [id])?;
    tx.execute("DELETE FROM ont_material_state WHERE material_id=?1", [id])?;
    prune(&tx)?;
    for w in &workspaces {
        bump_generation(&tx, w)?;
    }
    tx.commit()?;
    Ok(json!({"removed_evidence":removed,"material_id":id}))
}
fn get_graph(root: &Path, w: &str) -> Result<Value> {
    // Recover an interruption after storage tombstoned a document but before graph cleanup.
    // A missing/unreadable filesystem path is NOT a deletion and retains its evidence.
    if root.join("storage.sqlite").exists() {
        let storage = Connection::open_with_flags(
            root.join("storage.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let mut q =
            storage.prepare("SELECT id FROM documents WHERE workspace_id=?1 AND deleted=1")?;
        let deleted = q
            .query_map([w], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for id in deleted {
            remove_source(root, &id)?;
        }
    }
    let c = db(root)?;
    crate::ontology::graph(&c, root, w)
}
/// What the materials say, one concept per (name, definition): the map of a
/// subject that has not been organised, and the input of the subject layer.
pub(crate) fn lower_graph(c: &Connection, root: &Path, w: &str) -> Result<Value> {
    let mut concepts = Vec::new();
    let mut relations = Vec::new();
    let mut assertions = Vec::new();
    let mut sources = Vec::new();
    let mut stale = HashMap::new();
    let mut immutable_paths = HashMap::new();
    let mut q=c.prepare("SELECT material_id,input_hash,path,title,current FROM sources WHERE workspace=?1 ORDER BY title")?;
    for row in q.query_map([w], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, bool>(4)?,
        ))
    })? {
        let (id, hash, path, title, current) = row?;
        let outdated = match crate::storage::handle(root, &json!({"op":"source.get","id":id})) {
            Ok(src) => src["hash"].as_str() != Some(&hash),
            Err(_) => true,
        };
        stale.insert((id.clone(), hash.clone()), outdated);
        immutable_paths.insert((id.clone(), hash.clone()), path.clone());
        sources.push(json!({"material_id":id,"input_hash":hash,"path":path,"immutable_path":path,"title":title,"stale":outdated,"current":current}));
    }
    let mut q = c.prepare(
        "SELECT id,name,aliases,definition,(SELECT kind FROM concept_kinds WHERE concept_id=concepts.id) FROM concepts WHERE workspace=?1 ORDER BY name",
    )?;
    for row in q.query_map([w], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })? {
        let (id, name, aliases, definition, kind) = row?;
        concepts.push(json!({"id":id,"name":name,"aliases":serde_json::from_str::<Value>(&aliases)?,"definition":definition,"kind":kind.unwrap_or_else(||"concept".into())}));
    }
    let mut q=c.prepare("SELECT id,label,aliases,definition,directed,(SELECT kind FROM relation_kinds WHERE relation_id=relations.id) FROM relations WHERE workspace=?1 ORDER BY label")?;
    for row in q.query_map([w], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, bool>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })? {
        let (id, label, aliases, definition, directed, kind) = row?;
        relations.push(json!({"id":id,"label":label,"aliases":serde_json::from_str::<Value>(&aliases)?,"definition":definition,"directed":directed,"kind":kind.unwrap_or_else(||"related".into())}));
    }
    let mut q=c.prepare("SELECT id,subject,relation,object,condition,negated FROM assertions WHERE workspace=?1 ORDER BY id")?;
    let mut eq=c.prepare("SELECT material_id,input_hash,locator,quote,image_path,region FROM supports WHERE assertion_id=?1 ORDER BY material_id")?;
    for row in q.query_map([w], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, bool>(5)?,
        ))
    })? {
        let (id, subject, relation, object, condition, negated) = row?;
        let mut evidence = Vec::new();
        for ev in eq.query_map([&id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })? {
            let (material, hash, locator, quote, image_path, region) = ev?;
            let outdated = *stale
                .get(&(material.clone(), hash.clone()))
                .unwrap_or(&true);
            let immutable_path = immutable_paths.get(&(material.clone(), hash.clone()));
            evidence.push(json!({"material_id":material,"input_hash":hash,"locator":serde_json::from_str::<Value>(&locator)?,"quote":quote,"immutable_path":immutable_path,"image_path":image_path,"region":region.map(|x|serde_json::from_str::<Value>(&x)).transpose()?,"stale":outdated}));
        }
        assertions.push(json!({"id":id,"subject":subject,"relation":relation,"object":object,"condition":condition,"negated":negated,"evidence":evidence}));
    }
    Ok(json!({"concepts":concepts,"relations":relations,"assertions":assertions,"sources":sources}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path, text: &str, name: &str) -> (String, Value, Extraction) {
        let w = crate::storage::handle(root, &json!({"op":"workspace.create","name":name}))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let d = crate::storage::handle(
            root,
            &json!({"op":"draft.create","workspace_id":w,"body":text}),
        )
        .unwrap();
        let n = crate::storage::handle(
            root,
            &json!({"op":"note.save","draft_id":d["id"],"title":name}),
        )
        .unwrap();
        let source = crate::storage::handle(root, &json!({"op":"source.snapshot","id":n["id"]})).unwrap();
        let e = extract(root, Path::new(source["immutable_path"].as_str().unwrap())).unwrap();
        (w, n, e)
    }
    fn result() -> Value {
        json!({"concepts":[{"id":"a","name":"물","aliases":["water"],"definition":"액체 H2O"},{"id":"b","name":"수증기","aliases":["steam"],"definition":"기체 H2O"}],"relations":[{"id":"r","label":"변한다","aliases":["becomes"],"definition":"주체가 대상 상태로 전이한다","directed":true}],"assertions":[{"subject":"a","relation":"r","object":"b","condition":"100°C에서","negated":false,"evidence":[{"locator":{"kind":"paragraph","index":1},"quote":"물은 100°C에서 수증기로 변한다.","image_path":null,"region":null}]}]})
    }
    fn apply_req(w: &str, n: &Value, e: &Extraction) -> Value {
        json!({"op":"graph.apply","workspace_id":w,"material_id":n["id"],"input_hash":e.input_hash,"provider":"fixture","model":"deterministic-test","result":result()})
    }
    #[test]
    fn verifies_quotes_locations_and_structure() {
        let t = tempfile::tempdir().unwrap();
        let (w, n, e) = fixture(t.path(), "물은 100°C에서 수증기로 변한다.", "과학");
        let valid = apply_req(&w, &n, &e);
        for case in 0..5 {
            let mut req = valid.clone();
            match case {
                0 => {
                    req["result"]["assertions"][0]["evidence"][0]["quote"] =
                        json!("모델이 지어낸 인용문")
                }
                1 => req["result"]["assertions"][0]["evidence"][0]["locator"]["index"] = json!(99),
                2 => req["result"]["assertions"][0]["subject"] = json!("missing"),
                3 => req["result"]["assertions"] = json!([]),
                _ => req["result"]["unexpected"] = json!(true),
            };
            assert!(handle(t.path(), &req).is_err());
            assert!(get_graph(t.path(), &w).unwrap()["concepts"]
                .as_array()
                .unwrap()
                .is_empty());
        }
        assert_eq!(handle(t.path(), &valid).unwrap()["applied"], true);
    }
    #[test]
    fn reinput_is_idempotent_and_conditions_preserved() {
        let t = tempfile::tempdir().unwrap();
        let (w, n, e) = fixture(t.path(), "물은 100°C에서 수증기로 변한다.", "과학");
        let req = apply_req(&w, &n, &e);
        handle(t.path(), &req).unwrap();
        let g = get_graph(t.path(), &w).unwrap();
        assert_eq!(g["assertions"][0]["condition"], "100°C에서");
        assert_eq!(g["assertions"][0]["negated"], false);
        assert_eq!(handle(t.path(), &req).unwrap()["duplicate"], true);
        assert_eq!(g, get_graph(t.path(), &w).unwrap());
    }
    #[test]
    fn shared_evidence_survives_source_deletion_and_aliases_merge() {
        let t = tempfile::tempdir().unwrap();
        let (w, n, e) = fixture(t.path(), "물은 100°C에서 수증기로 변한다.", "과학");
        handle(t.path(), &apply_req(&w, &n, &e)).unwrap();
        let d=crate::storage::handle(t.path(),&json!({"op":"draft.create","workspace_id":w,"body":"물은 100°C에서 수증기로 변한다.\n\n다른 출처."})).unwrap();
        let n2 = crate::storage::handle(
            t.path(),
            &json!({"op":"note.save","draft_id":d["id"],"title":"두번째"}),
        )
        .unwrap();
        let source = crate::storage::handle(t.path(), &json!({"op":"source.snapshot","id":n2["id"]})).unwrap();
        let e2 = extract(t.path(), Path::new(source["immutable_path"].as_str().unwrap())).unwrap();
        let mut req = apply_req(&w, &n2, &e2);
        req["result"]["concepts"][0]["name"] = json!("water");
        req["result"]["concepts"][0]["aliases"] = json!(["물"]);
        handle(t.path(), &req).unwrap();
        let g = get_graph(t.path(), &w).unwrap();
        assert_eq!(g["concepts"].as_array().unwrap().len(), 2);
        assert_eq!(g["assertions"][0]["evidence"].as_array().unwrap().len(), 2);
        remove_source(t.path(), n["id"].as_str().unwrap()).unwrap();
        let g = get_graph(t.path(), &w).unwrap();
        assert_eq!(g["assertions"].as_array().unwrap().len(), 1);
        assert_eq!(g["assertions"][0]["evidence"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn edited_source_is_stale_and_old_apply_rejected() {
        let t = tempfile::tempdir().unwrap();
        let (w, n, e) = fixture(t.path(), "물은 100°C에서 수증기로 변한다.", "과학");
        let req = apply_req(&w, &n, &e);
        handle(t.path(), &req).unwrap();
        crate::storage::handle(t.path(),&json!({"op":"note.update","id":n["id"],"expected_version":n["version"],"body":"수정한 원본"})).unwrap();
        assert_eq!(
            get_graph(t.path(), &w).unwrap()["assertions"][0]["evidence"][0]["stale"],
            true
        );
        assert!(handle(t.path(), &req).is_err());
    }
    #[test]
    fn deletion_reconciles_after_interrupted_dispatch() {
        let t = tempfile::tempdir().unwrap();
        let (w, n, e) = fixture(t.path(), "물은 100°C에서 수증기로 변한다.", "과학");
        handle(t.path(), &apply_req(&w, &n, &e)).unwrap();
        crate::storage::handle(t.path(), &json!({"op":"note.delete","id":n["id"]})).unwrap();
        assert!(get_graph(t.path(), &w).unwrap()["assertions"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(handle(t.path(), &apply_req(&w, &n, &e)).is_err());
    }
    #[test]
    fn homonyms_and_direction_negation_remain_distinct() {
        let t = tempfile::tempdir().unwrap();
        let (w, n, e) = fixture(t.path(), "물은 100°C에서 수증기로 변한다.", "과학");
        let mut req = apply_req(&w, &n, &e);
        let mut claim = req["result"]["assertions"][0].clone();
        claim["negated"] = json!(true);
        claim["condition"] = json!("압력이 다른 경우");
        req["result"]["assertions"]
            .as_array_mut()
            .unwrap()
            .push(claim.clone());
        claim["subject"] = json!("b");
        claim["object"] = json!("a");
        req["result"]["assertions"]
            .as_array_mut()
            .unwrap()
            .push(claim);
        handle(t.path(), &req).unwrap();
        assert_eq!(
            get_graph(t.path(), &w).unwrap()["assertions"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let c = db(t.path()).unwrap();
        let different = Concept {
            id: "x".into(),
            name: "물".into(),
            aliases: vec![],
            definition: "질문의 대상 물건".into(),
            kind: None,
        };
        assert!(candidate_concept(&c, &w, &different).unwrap().is_none());
    }
    #[test]
    fn immutable_extraction_and_image_region_validation() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("source.md");
        fs::write(&p, "원본 한글\n\n두번째 문단").unwrap();
        let e = extract(t.path(), &p).unwrap();
        fs::write(&p, "변경").unwrap();
        assert_eq!(e.units[1].locator.index, 2);
        assert_eq!(
            fs::read_to_string(
                cache_path(t.path(), &e.input_hash)
                    .unwrap()
                    .join("original.md")
            )
            .unwrap(),
            "원본 한글\n\n두번째 문단"
        );
        let mut a: Analysis = serde_json::from_value(result()).unwrap();
        let ie = Extraction {
            input_hash: "x".into(),
            kind: "image".into(),
            warnings: vec![],
            units: vec![Unit {
                locator: Locator {
                    kind: "image".into(),
                    index: 1,
                },
                text: String::new(),
                images: vec![SourceImage {
                    path: "/cache/image.png".into(),
                    mime: "image/png".into(),
                }],
            }],
        };
        a.assertions[0].evidence = vec![Evidence {
            locator: Locator {
                kind: "image".into(),
                index: 1,
            },
            quote: String::new(),
            image_path: Some("/cache/image.png".into()),
            region: Some(vec![0., 0., 1., 1.]),
        }];
        let ok = check(a.clone(), &ie).unwrap();
        assert_eq!((ok.total, ok.dropped), (1, 0));
        a.assertions[0].evidence[0].region = Some(vec![0.9, 0., 1., 1.]);
        let bad = check(a, &ie).unwrap();
        assert_eq!((bad.total, bad.dropped), (1, 1));
        assert!(bad.analysis.concepts.is_empty());
    }
    #[test]
    fn a_wrong_claim_is_dropped_alone_and_ocr_spacing_is_tolerated() {
        let t = tempfile::tempdir().unwrap();
        let (w, n, e) = fixture(t.path(), "물은 100°C에서\n수증기로  변한다.", "과학");
        let mut req = apply_req(&w, &n, &e);
        req["result"]["assertions"][0]["evidence"][0]["quote"] = json!("물은 100°C에서 수증기로 변한다.");
        req["result"]["concepts"].as_array_mut().unwrap().push(json!({"id":"c","name":"얼음","aliases":[],"definition":"고체 H2O","type":"concept"}));
        let mut invented = req["result"]["assertions"][0].clone();
        invented["object"] = json!("c");
        invented["evidence"][0]["quote"] = json!("물은 얼음이 된다.");
        req["result"]["assertions"].as_array_mut().unwrap().push(invented);
        let applied = handle(t.path(), &req).unwrap();
        assert_eq!((applied["total"].as_u64(), applied["dropped"].as_u64()), (Some(2), Some(1)));
        let g = get_graph(t.path(), &w).unwrap();
        assert_eq!(g["assertions"].as_array().unwrap().len(), 1);
        assert_eq!(g["concepts"].as_array().unwrap().len(), 2);
        assert_eq!(g["layer"], "material");
    }
    #[test]
    fn rejects_invalid_and_unsupported_input() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("bad.pptx");
        fs::write(&p, "not zip").unwrap();
        assert!(extract(t.path(), &p)
            .unwrap_err()
            .to_string()
            .contains("PPTX"));
        let p = t.path().join("bad.md");
        fs::write(&p, [0xff]).unwrap();
        assert!(extract(t.path(), &p).is_err());
        assert!(zip_resolve("ppt/slides", "../../../outside").is_err());
    }
    #[test]
    fn pptx_keeps_presentation_order_notes_tables_images() {
        use std::io::Write;
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("ordered.pptx");
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in [
            (
                "ppt/presentation.xml",
                r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst><p:sldId r:id="second"/><p:sldId r:id="first"/></p:sldIdLst></p:presentation>"#,
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                r#"<Relationships><Relationship Id="first" Target="slides/slide1.xml" Type="x/slide"/><Relationship Id="second" Target="slides/slide2.xml" Type="x/slide"/></Relationships>"#,
            ),
            (
                "ppt/slides/slide2.xml",
                r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:t>두번째 파일 첫 슬라이드</a:t></a:p><a:tbl><a:p><a:t>표 셀</a:t></a:p></a:tbl></p:sld>"#,
            ),
            (
                "ppt/slides/slide1.xml",
                r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:t>마지막 슬라이드</a:t></a:p></p:sld>"#,
            ),
            (
                "ppt/slides/_rels/slide2.xml.rels",
                r#"<Relationships><Relationship Id="n" Target="../notesSlides/notesSlide2.xml" Type="x/notesSlide"/><Relationship Id="i" Target="../media/image1.png" Type="x/image"/></Relationships>"#,
            ),
            (
                "ppt/notesSlides/notesSlide2.xml",
                r#"<p:notes xmlns:p="p" xmlns:a="a"><a:p><a:t>발표자 설명</a:t></a:p></p:notes>"#,
            ),
        ] {
            z.start_file(name, options).unwrap();
            z.write_all(content.as_bytes()).unwrap();
        }
        z.start_file("ppt/media/image1.png", options).unwrap();
        z.write_all(b"test-image-payload").unwrap();
        z.finish().unwrap();
        let e = extract(t.path(), &p).unwrap();
        assert_eq!(e.units.len(), 2);
        assert_eq!(
            e.units[0].locator,
            Locator {
                kind: "slide".into(),
                index: 1
            }
        );
        assert!(e.units[0].text.contains("두번째 파일 첫 슬라이드"));
        assert!(e.units[0].text.contains("표 셀"));
        assert!(e.units[0].text.contains("발표자 설명"));
        assert!(e.units[1].text.contains("마지막 슬라이드"));
        assert_eq!(e.units[0].images.len(), 1);
        assert!(Path::new(&e.units[0].images[0].path).exists());
    }
    #[test]
    fn graph_reads_and_stale_checks_do_not_create_new_originals() {
        let t = tempfile::tempdir().unwrap();
        let (w, n, e) = fixture(t.path(), "물은 100°C에서 수증기로 변한다.", "과학");
        let req = apply_req(&w, &n, &e);
        handle(t.path(), &req).unwrap();
        let tree = crate::storage::handle(t.path(), &json!({"op":"tree","workspace_id":w})).unwrap();
        assert_eq!(tree["entries"][0]["analysis_state"], "analysed");
        assert_eq!(fs::read_dir(t.path().join("versions")).unwrap().count(), 1);
        crate::storage::handle(t.path(), &json!({"op":"note.update","id":n["id"],"expected_version":n["version"],"body":"필기로 달라진 내용"})).unwrap();
        for _ in 0..3 {
            let graph = get_graph(t.path(), &w).unwrap();
            assert_eq!(graph["sources"][0]["stale"], true);
            assert!(!graph["assertions"].as_array().unwrap().is_empty());
            assert!(handle(t.path(), &req).is_err());
        }
        let tree = crate::storage::handle(t.path(), &json!({"op":"tree","workspace_id":w})).unwrap();
        assert_eq!(tree["entries"][0]["analysis_state"], "stale");
        assert_eq!(fs::read_dir(t.path().join("versions")).unwrap().count(), 1);
    }

    #[test]
    fn pdf_extractor_upgrade_preserves_legacy_evidence_and_uses_new_cache() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("annotated.pdf");
        fs::write(&source, b"%PDF annotated fixture").unwrap();
        let hash = digest(&fs::read(&source).unwrap());
        let legacy = cache_path(t.path(), &hash).unwrap();
        fs::create_dir_all(&legacy).unwrap();
        let old_image = legacy.join("page-0001.png");
        fs::write(&old_image, b"old evidence pixels").unwrap();
        let old_json = json!({"input_hash":hash,"kind":"pdf","warnings":[],"units":[{
            "locator":{"kind":"page","index":1},"text":"old text without notes",
            "images":[{"path":old_image,"mime":"image/png"}]
        }]});
        let old_json_bytes = serde_json::to_vec(&old_json).unwrap();
        fs::write(legacy.join("extraction.json"), &old_json_bytes).unwrap();
        // A deterministic helper exercises the production cache selection and
        // publication path without depending on native PDFKit in this unit test.
        let helper = t.path().join("extract-fixture");
        fs::write(&helper, r##"#!/usr/bin/python3
import json, pathlib, sys
cache = pathlib.Path(sys.argv[2])
image = cache / 'page-0001.png'
image.write_bytes(b'new annotation pixels')
print(json.dumps({'input_hash':'', 'kind':'pdf', 'warnings':[], 'units':[{'locator':{'kind':'page','index':1},'text':'body and memo text','images':[{'path':str(image),'mime':'image/png'}]}]}))
"##).unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
        let extracted = extract_with_helper(t.path(), &source, Some(&helper)).unwrap();
        assert_eq!(extracted.units[0].text, "body and memo text");
        let new_image = Path::new(&extracted.units[0].images[0].path);
        assert_eq!(new_image.parent().unwrap(), legacy.join(PDF_EXTRACTION_VERSION));
        assert_eq!(fs::read(new_image).unwrap(), b"new annotation pixels");
        assert_eq!(fs::read(&old_image).unwrap(), b"old evidence pixels");
        assert_eq!(fs::read(legacy.join("extraction.json")).unwrap(), old_json_bytes);
        // Graph validation must select the new extraction namespace as well.
        let workspace = crate::storage::handle(t.path(), &json!({"op":"workspace.create","name":"PDF"})).unwrap();
        let note = crate::storage::handle(t.path(), &json!({"op":"material.import","workspace_id":workspace["id"],"path":source})).unwrap();
        crate::storage::handle(t.path(), &json!({"op":"source.snapshot","id":note["id"]})).unwrap();
        let mut request = apply_req(workspace["id"].as_str().unwrap(), &note, &extracted);
        request["result"]["assertions"][0]["evidence"][0]["locator"] = json!({"kind":"page","index":1});
        request["result"]["assertions"][0]["evidence"][0]["quote"] = json!("body and memo text");
        assert_eq!(handle(t.path(), &request).unwrap()["applied"], true);
        fs::remove_file(&helper).unwrap();
        // Subsequent reads use v2, without invoking a helper or falling back to v1.
        assert_eq!(extract_with_helper(t.path(), &source, Some(&helper)).unwrap().units[0].text, "body and memo text");
        // Recursive accounting includes both versions; eviction protects the
        // entire active hash, including old evidence under that same hash.
        let obsolete = t.path().join("extractions").join("unrelated");
        fs::create_dir_all(&obsolete).unwrap();
        fs::File::create(obsolete.join("large-cache")).unwrap().set_len(MAX_CACHE).unwrap();
        enforce_cache_limit(t.path(), &legacy).unwrap();
        assert!(!obsolete.exists());
        assert!(old_image.is_file() && new_image.is_file());
    }


    #[test]
    fn claims_naming_a_concept_or_relation_are_repaired_only_when_one_matches() {
        let e = Extraction {
            input_hash: "h".into(), kind: "md".into(), warnings: vec![],
            units: vec![Unit { locator: Locator { kind: "paragraph".into(), index: 1 }, text: "LLM과 에이전트는 대비된다. API 키는 .env에만 둔다.".into(), images: vec![] }],
        };
        let concept = |id: &str, name: &str, aliases: &[&str]| Concept { id: id.into(), name: name.into(), aliases: aliases.iter().map(|a| a.to_string()).collect(), definition: format!("{name}의 뜻"), kind: None };
        let relation = |id: &str, label: &str, kind: &str| Relation { id: id.into(), label: label.into(), definition: "뜻".into(), directed: true, aliases: vec![], kind: Some(kind.into()) };
        let claim = |s: &str, r: &str, o: &str, quote: &str| Assertion {
            subject: s.into(), relation: r.into(), object: o.into(), condition: String::new(), negated: false,
            evidence: vec![Evidence { locator: Locator { kind: "paragraph".into(), index: 1 }, quote: quote.into(), image_path: None, region: None }],
        };
        let a = Analysis {
            concepts: vec![concept("c1", "LLM", &[]), concept("c2", "에이전트", &["Agent"]), concept("c3", "API 키", &[]), concept("c4", ".env", &[]), concept("c5", "모델", &[]), concept("c6", "모델", &[])],
            relations: vec![relation("r1", "대비된다", "contrast"), relation("r2", "에만 둔다", "applies"), relation("r3", "쓰인다", "applies")],
            assertions: vec![
                claim("c1", "r1", "c2", "LLM과 에이전트는 대비된다."),
                // The relation's type instead of its id.
                claim("c1", "contrast", "agent", "LLM과 에이전트는 대비된다."),
                // Names for every part, the way a provider sometimes answers.
                claim("API 키", "에만  둔다", ".env", "API 키는 .env에만 둔다."),
                // Two relations share the type, two concepts share the name: dropped.
                claim("c3", "applies", "c4", "API 키는 .env에만 둔다."),
                claim("모델", "r1", "c2", "LLM과 에이전트는 대비된다."),
            ],
        };
        let checked = check(a, &e).unwrap();
        assert_eq!((checked.total, checked.dropped, checked.repaired), (5, 2, 2));
        let kept: Vec<(&str, &str, &str)> = checked.analysis.assertions.iter().map(|c| (c.subject.as_str(), c.relation.as_str(), c.object.as_str())).collect();
        assert_eq!(kept, [("c1", "r1", "c2"), ("c1", "r1", "c2"), ("c3", "r2", "c4")]);
    }
}
