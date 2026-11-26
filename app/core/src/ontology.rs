//! Subject layer: merged concepts, units and inferred links derived from what the
//! materials say. Everything here can be rebuilt from the lower layer, the user's
//! corrections and a run; the lower layer is never rewritten by this module.
#[path = "ontology_cleanup.rs"]
mod cleanup;
use crate::knowledge::{self, LINK_KINDS};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
};

pub(crate) fn link_label(kind: &str) -> &'static str {
    match kind {
        "kind_of" => "종류",
        "part_of" => "구성",
        "prerequisite" => "선행",
        "cause" => "원인",
        "contrast" => "대조",
        "example" => "예시",
        "applies" => "적용",
        _ => "관련",
    }
}
/// A name with the particle its last syllable takes. A name that does not end in Hangul keeps both forms.
fn with_particle(name: &str, closed: &str, open: &str) -> String {
    match name.trim_end().chars().last() {
        Some(c @ '가'..='힣') => format!("{name}{}", if (c as u32 - 0xAC00) % 28 == 0 { open } else { closed }),
        _ if closed == "과" => format!("{name}{open}({closed})"),
        _ => format!("{name}{closed}({open})"),
    }
}
/// A link read as a sentence, subject first.
pub(crate) fn link_sentence(kind: &str, a: &str, b: &str, negated: bool) -> String {
    let (a_topic, a_object, a_with) = (with_particle(a, "은", "는"), with_particle(a, "을", "를"), with_particle(a, "과", "와"));
    let (b_topic, b_object, b_subject) = (with_particle(b, "은", "는"), with_particle(b, "을", "를"), with_particle(b, "이", "가"));
    match (kind, negated) {
        ("kind_of", false) => format!("{a_topic} {b}의 한 종류예요"),
        ("kind_of", true) => format!("{a_topic} {b}의 한 종류가 아니에요"),
        ("part_of", false) => format!("{a_topic} {b_object} 이루는 부분이에요"),
        ("part_of", true) => format!("{a_topic} {b_object} 이루는 부분이 아니에요"),
        ("prerequisite", false) => format!("{a_object} 먼저 알아야 {b_object} 이해해요"),
        ("prerequisite", true) => format!("{a_object} 몰라도 {b_object} 이해할 수 있어요"),
        ("cause", false) => format!("{a} 때문에 {b_subject} 일어나요"),
        ("cause", true) => format!("{a} 때문에 {b_subject} 일어나지는 않아요"),
        ("contrast", false) => format!("{a_with} {b_topic} 서로 대비돼요"),
        ("contrast", true) => format!("{a_with} {b_topic} 대비되지 않아요"),
        ("example", false) => format!("{a_topic} {b}의 예시예요"),
        ("example", true) => format!("{a_topic} {b}의 예시가 아니에요"),
        ("applies", false) => format!("{a_topic} {b}에 쓰여요"),
        ("applies", true) => format!("{a_topic} {b}에 쓰이지 않아요"),
        (_, false) => format!("{a_with} {b_topic} 관련이 있어요"),
        (_, true) => format!("{a_with} {b_topic} 관련이 없어요"),
    }
}
fn link_reading(kind: &str) -> &'static str {
    match kind {
        "kind_of" => "앞 개념은 뒤 개념의 한 종류예요",
        "part_of" => "앞 개념은 뒤 개념을 이루는 부분이에요",
        "prerequisite" => "앞 개념을 먼저 알아야 뒤 개념을 이해해요",
        "cause" => "앞 개념 때문에 뒤 개념이 일어나요",
        "contrast" => "두 개념은 서로 대비돼요",
        "example" => "앞 개념은 뒤 개념의 예시예요",
        "applies" => "앞 개념은 뒤 개념에 쓰여요",
        _ => "두 개념은 관련이 있어요",
    }
}
pub(crate) fn concept_kind_label(kind: &str) -> &'static str {
    match kind {
        "principle" => "원리·법칙",
        "formula" => "공식",
        "procedure" => "절차·방법",
        "example" => "사례",
        "person_event" => "인물·사건",
        _ => "개념",
    }
}
fn directed(kind: &str) -> bool {
    !matches!(kind, "contrast" | "related")
}
/// Letters and digits only, lower-cased: how names are compared across materials.
fn name_key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}
/// Every way a concept may be written: its name, aliases, and both halves of
/// "약어(풀어 쓴 이름)".
fn name_keys(name: &str, aliases: &Value) -> HashSet<String> {
    let mut keys = HashSet::new();
    for text in std::iter::once(name).chain(aliases.as_array().into_iter().flatten().filter_map(Value::as_str)) {
        let mut parts = vec![text.to_owned()];
        if let (Some(open), Some(close)) = (text.find(['(', '（']), text.rfind([')', '）'])) {
            if open < close {
                parts.push(text[..open].to_owned());
                parts.push(text[open..close].chars().skip(1).collect());
            }
        }
        for part in parts {
            let key = name_key(&part);
            if key.chars().count() >= 2 {
                keys.insert(key);
            }
        }
    }
    keys
}

pub(crate) fn latest_revision(c: &Connection, w: &str) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT MAX(revision) FROM ont_revisions WHERE workspace=?1", [w], |r| r.get(0))?)
}
fn members(c: &Connection, w: &str) -> Result<HashMap<String, String>> {
    let mut q = c.prepare("SELECT member_id,concept_id FROM ont_members WHERE workspace=?1")?;
    let rows = q.query_map([w], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
/// The map of a subject: the lower layer as it is until the subject is first
/// organised, then the subject layer.
pub(crate) fn graph(c: &Connection, root: &Path, w: &str) -> Result<Value> {
    if latest_revision(c, w)?.is_none() {
        let mut lower = knowledge::lower_graph(c, root, w)?;
        lower["layer"] = json!("material");
        lower["units"] = json!([]);
        lower["hidden"] = json!([]);
        mark_origins(root, w, &mut lower["sources"])?;
        return Ok(lower);
    }
    view(c, root, w)
}
/// Notes registered from study-assistant conversations are shown as the user's own questions.
fn mark_origins(root: &Path, w: &str, sources: &mut Value) -> Result<()> {
    let storage = crate::storage::connect(root)?;
    let mut q = storage.prepare("SELECT id FROM documents WHERE workspace_id=?1 AND origin_kind IS NOT NULL AND deleted=0")?;
    let from_chat: HashSet<String> = q.query_map([w], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for source in sources.as_array_mut().into_iter().flatten() {
        source["from_chat"] = json!(source["material_id"].as_str().is_some_and(|id| from_chat.contains(id)));
    }
    Ok(())
}
/// Subject layer in the shape the map draws: concepts are groups of lower
/// concepts, relations are the eight link kinds, assertions are links.
pub(crate) fn view(c: &Connection, root: &Path, w: &str) -> Result<Value> {
    let lower = knowledge::lower_graph(c, root, w)?;
    let members = members(c, w)?;
    let upper = |id: &str| members.get(id).cloned().unwrap_or_else(|| id.to_owned());
    let mut overrides = HashMap::new();
    let mut q = c.prepare("SELECT id,name,definition,unit_id FROM ont_concepts WHERE workspace=?1")?;
    for row in q.query_map([w], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?)))? {
        let (id, name, definition, unit) = row?;
        overrides.insert(id, (name, definition, unit));
    }
    // The user's own wording of a concept; a rerun never overwrites it.
    let mut notes: HashMap<String, String> = HashMap::new();
    let mut q = c.prepare("SELECT concept_id,definition FROM ont_notes WHERE workspace=?1")?;
    for row in q.query_map([w], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, definition) = row?;
        notes.insert(upper(&id), definition);
    }
    let mut quality = HashMap::new();
    let mut q = c.prepare("SELECT material_id,quality FROM ont_material_state WHERE workspace=?1")?;
    for row in q.query_map([w], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, q) = row?;
        quality.insert(id, q);
    }
    let mut hidden = HashSet::new();
    let mut q = c.prepare("SELECT subject,object,kind FROM ont_hidden WHERE workspace=?1")?;
    for row in q.query_map([w], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
        hidden.insert(row?);
    }
    // Group lower concepts under their subject concept, representative first.
    let mut order = Vec::new();
    let mut groups: HashMap<String, Vec<&Value>> = HashMap::new();
    for concept in lower["concepts"].as_array().into_iter().flatten() {
        let id = upper(concept["id"].as_str().unwrap_or(""));
        let group = groups.entry(id.clone()).or_insert_with(|| {
            order.push(id.clone());
            Vec::new()
        });
        if concept["id"] == id.as_str() {
            group.insert(0, concept);
        } else {
            group.push(concept);
        }
    }
    let relation_kind: HashMap<&str, (&str, &str)> = lower["relations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| (r["id"].as_str().unwrap_or(""), (r["kind"].as_str().unwrap_or("related"), r["label"].as_str().unwrap_or(""))))
        .collect();
    // Written links: the materials' claims, between subject concepts, by kind.
    let mut written: BTreeMap<(String, String, String, String, bool), Value> = BTreeMap::new();
    let mut learned: HashMap<String, Vec<Value>> = HashMap::new();
    for a in lower["assertions"].as_array().into_iter().flatten() {
        let (s, o) = (upper(a["subject"].as_str().unwrap_or("")), upper(a["object"].as_str().unwrap_or("")));
        for id in [&s, &o] {
            learned.entry(id.clone()).or_default().extend(a["evidence"].as_array().cloned().unwrap_or_default());
        }
        if s == o {
            continue;
        }
        let (kind, label) = relation_kind.get(a["relation"].as_str().unwrap_or("")).copied().unwrap_or(("related", ""));
        let (s, o) = if !directed(kind) && s > o { (o, s) } else { (s, o) };
        let key = (s.clone(), o.clone(), kind.to_owned(), a["condition"].as_str().unwrap_or("").to_owned(), a["negated"] == true);
        let entry = written.entry(key).or_insert_with(|| json!({"id":a["id"],"subject":s,"relation":kind,"object":o,"condition":a["condition"],"negated":a["negated"],"evidence":[],"basis":"written","reason":"","labels":[]}));
        entry["evidence"].as_array_mut().unwrap().extend(a["evidence"].as_array().cloned().unwrap_or_default());
        if !label.is_empty() && !entry["labels"].as_array().unwrap().iter().any(|l| l == label) {
            entry["labels"].as_array_mut().unwrap().push(json!(label));
        }
    }
    let mut links = Vec::new();
    let mut hidden_links = Vec::new();
    let mut taken = HashSet::new();
    for link in written.into_values() {
        let key = (link["subject"].as_str().unwrap().to_owned(), link["object"].as_str().unwrap().to_owned(), link["relation"].as_str().unwrap().to_owned());
        taken.insert(key.clone());
        if hidden.contains(&key) { hidden_links.push(link) } else { links.push(link) }
    }
    let mut q = c.prepare("SELECT id,subject,object,kind,reason,condition,negated FROM ont_links WHERE workspace=?1 ORDER BY id")?;
    for row in q.query_map([w], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, bool>(6)?))
    })? {
        let (id, s, o, kind, reason, condition, negated) = row?;
        let (s, o) = (upper(&s), upper(&o));
        // A link the materials now state themselves is drawn once, as written.
        if s == o || !groups.contains_key(&s) || !groups.contains_key(&o) || !taken.insert((s.clone(), o.clone(), kind.clone())) {
            continue;
        }
        let link = json!({"id":id,"subject":s,"relation":kind,"object":o,"condition":condition,"negated":negated,"evidence":[],"basis":"inferred","reason":reason,"labels":[]});
        if hidden.contains(&(s, o, kind)) { hidden_links.push(link) } else { links.push(link) }
    }
    let mut units = Vec::new();
    let mut q = c.prepare("SELECT id,name,description,position FROM ont_units WHERE workspace=?1 ORDER BY position")?;
    for row in q.query_map([w], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)))? {
        let (id, name, description, position) = row?;
        units.push(json!({"id":id,"name":name,"description":description,"position":position,"concepts":0}));
    }
    let mut concepts = Vec::new();
    for id in &order {
        let group = &groups[id];
        let first = group[0];
        let over = overrides.get(id);
        let name = over.map(|o| o.0.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| first["name"].as_str().unwrap_or("").to_owned());
        let original = over.map(|o| o.1.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| first["definition"].as_str().unwrap_or("").to_owned());
        let edited = notes.get(id).filter(|d| !d.is_empty() && **d != original);
        let definition = edited.cloned().unwrap_or_else(|| original.clone());
        let mut aliases: Vec<String> = Vec::new();
        for member in group {
            for text in std::iter::once(member["name"].as_str().unwrap_or("")).chain(member["aliases"].as_array().into_iter().flatten().filter_map(Value::as_str)) {
                if text != name && !text.is_empty() && !aliases.iter().any(|a| a == text) {
                    aliases.push(text.to_owned());
                }
            }
        }
        let unit = over.and_then(|o| o.2.clone()).filter(|u| units.iter().any(|x| x["id"] == u.as_str()));
        if let Some(entry) = unit.as_ref().and_then(|u| units.iter_mut().find(|x| x["id"] == u.as_str())) {
            entry["concepts"] = json!(entry["concepts"].as_u64().unwrap_or(0) + 1);
        }
        // One place is listed once however many statements cite it.
        let mut evidence = learned.remove(id).unwrap_or_default();
        let mut places = HashSet::new();
        evidence.retain(|e| places.insert((e["material_id"].to_string(), e["locator"].to_string(), e["quote"].to_string())));
        let materials: Vec<&str> = {
            let mut seen = Vec::new();
            for e in &evidence {
                if let Some(m) = e["material_id"].as_str() {
                    if !seen.contains(&m) {
                        seen.push(m);
                    }
                }
            }
            seen
        };
        let low = !materials.is_empty() && materials.iter().all(|m| quality.get(*m).is_some_and(|q| q == "low"));
        let merged: Vec<Value> = group.iter().filter(|m| m["id"] != id.as_str()).map(|m| json!({"id":m["id"],"name":m["name"],"definition":m["definition"]})).collect();
        concepts.push(json!({"id":id,"name":name,"aliases":aliases,"definition":definition,"original_definition":original,"edited":edited.is_some(),"kind":first["kind"],"kind_label":concept_kind_label(first["kind"].as_str().unwrap_or("")),"unit":unit,"merged":merged,"low_quality":low,"materials":materials,"learned":evidence}));
    }
    concepts.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    // Each link carries its sentence so the card and the export read it the same way.
    let names: std::collections::HashMap<String, String> =
        concepts.iter().map(|c| (c["id"].as_str().unwrap_or("").to_owned(), c["name"].as_str().unwrap_or("").to_owned())).collect();
    for link in links.iter_mut().chain(hidden_links.iter_mut()) {
        let name = |key: &str| names.get(link[key].as_str().unwrap_or("")).cloned().unwrap_or_default();
        let sentence = link_sentence(link["relation"].as_str().unwrap_or(""), &name("subject"), &name("object"), link["negated"] == true);
        link["sentence"] = json!(sentence);
    }
    let relations: Vec<Value> = LINK_KINDS
        .iter()
        .filter(|k| links.iter().any(|l| l["relation"] == **k))
        .map(|k| json!({"id":k,"label":link_label(k),"definition":link_reading(k),"directed":directed(k),"aliases":[]}))
        .collect();
    let mut sources = lower["sources"].clone();
    for source in sources.as_array_mut().into_iter().flatten() {
        let q = source["material_id"].as_str().and_then(|m| quality.get(m)).cloned();
        source["quality"] = json!(q);
    }
    let revision = c.query_row("SELECT revision,created_at,summary FROM ont_revisions WHERE workspace=?1 ORDER BY revision DESC LIMIT 1", [w], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    }).optional()?;
    mark_origins(root, w, &mut sources)?;
    Ok(json!({
        "layer":"subject","concepts":concepts,"relations":relations,"assertions":links,"hidden":hidden_links,"units":units,"sources":sources,
        "revision":revision.map(|(n, at, summary)| json!({"n":n,"created_at":at,"summary":serde_json::from_str::<Value>(&summary).unwrap_or(Value::Null)})),
    }))
}

// ── Aligning concepts ────────────────────────────────────────────────────
/// Pairs of lower concepts from different groups whose names overlap, each with
/// one evidence sentence. The user's "keep these apart" corrections are skipped.
pub(crate) fn align_candidates(c: &Connection, root: &Path, w: &str) -> Result<Vec<Value>> {
    let lower = knowledge::lower_graph(c, root, w)?;
    let members = members(c, w)?;
    let upper = |id: &str| members.get(id).cloned().unwrap_or_else(|| id.to_owned());
    let mut apart = HashSet::new();
    let mut q = c.prepare("SELECT a,b FROM ont_corrections WHERE workspace=?1")?;
    for row in q.query_map([w], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        apart.insert(row?);
    }
    let mut quote: HashMap<&str, &str> = HashMap::new();
    for a in lower["assertions"].as_array().into_iter().flatten() {
        let text = a["evidence"].as_array().into_iter().flatten().filter_map(|e| e["quote"].as_str()).find(|q| !q.is_empty());
        if let Some(text) = text {
            for id in [a["subject"].as_str(), a["object"].as_str()].into_iter().flatten() {
                quote.entry(id).or_insert(text);
            }
        }
    }
    let concepts: Vec<&Value> = lower["concepts"].as_array().into_iter().flatten().collect();
    let mut index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, concept) in concepts.iter().enumerate() {
        for key in name_keys(concept["name"].as_str().unwrap_or(""), &concept["aliases"]) {
            index.entry(key).or_default().push(i);
        }
    }
    let describe = |v: &Value| {
        let id = v["id"].as_str().unwrap_or("");
        json!({"id":id,"name":v["name"],"aliases":v["aliases"],"definition":v["definition"],"evidence":quote.get(id).map(|q| q.chars().take(300).collect::<String>()).unwrap_or_default()})
    };
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    for list in index.values() {
        for (n, &i) in list.iter().enumerate() {
            for &j in &list[n + 1..] {
                let (a, b) = (concepts[i]["id"].as_str().unwrap_or(""), concepts[j]["id"].as_str().unwrap_or(""));
                let (a, b, i, j) = if a <= b { (a, b, i, j) } else { (b, a, j, i) };
                if upper(a) == upper(b) || apart.contains(&(a.to_owned(), b.to_owned())) || !seen.insert((a, b)) {
                    continue;
                }
                pairs.push(json!({"pair":format!("p{}", pairs.len() + 1),"a":describe(concepts[i]),"b":describe(concepts[j])}));
            }
        }
    }
    Ok(pairs)
}
/// Merge the pairs judged the same. Returns how many concepts joined another one.
pub(crate) fn apply_alignment(c: &Connection, w: &str, same: &[(String, String, String, String)], revision: i64) -> Result<usize> {
    let members = members(c, w)?;
    let upper = |id: &str| members.get(id).cloned().unwrap_or_else(|| id.to_owned());
    let name_of = |id: &str| -> Result<String> { Ok(c.query_row("SELECT name FROM concepts WHERE id=?1", [id], |r| r.get(0)).optional()?.unwrap_or_default()) };
    let mut merged = 0;
    // Current groups move as a whole: the surviving representative is the smaller id.
    let mut moved: HashMap<String, String> = HashMap::new();
    let resolve = |moved: &HashMap<String, String>, id: &str| {
        let mut id = upper(id);
        while let Some(next) = moved.get(&id) {
            id = next.clone();
        }
        id
    };
    for (a, b, name, definition) in same {
        let (ra, rb) = (resolve(&moved, a), resolve(&moved, b));
        if ra == rb {
            continue;
        }
        let (keep, lose) = if ra <= rb { (ra, rb) } else { (rb, ra) };
        moved.insert(lose.clone(), keep.clone());
        let lost_name = name_of(&lose)?;
        c.execute("UPDATE ont_members SET concept_id=?1 WHERE workspace=?2 AND concept_id=?3", params![keep, w, lose])?;
        c.execute("INSERT OR REPLACE INTO ont_members VALUES(?1,?2,?3,?4,?5)", params![w, lose, keep, lost_name, revision])?;
        let unit: Option<String> = c.query_row("SELECT unit_id FROM ont_concepts WHERE workspace=?1 AND id IN (?2,?3) AND unit_id IS NOT NULL ORDER BY id=?2 DESC LIMIT 1", params![w, keep, lose], |r| r.get(0)).optional()?;
        c.execute("DELETE FROM ont_concepts WHERE workspace=?1 AND id=?2", params![w, lose])?;
        if !name.trim().is_empty() || !definition.trim().is_empty() {
            c.execute("INSERT INTO ont_concepts VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace,id) DO UPDATE SET name=excluded.name,definition=excluded.definition,unit_id=COALESCE(excluded.unit_id,unit_id)", params![w, keep, name.trim(), definition.trim(), unit])?;
        }
        for column in ["subject", "object"] {
            c.execute(&format!("UPDATE ont_links SET {column}=?1 WHERE workspace=?2 AND {column}=?3"), params![keep, w, lose])?;
            c.execute(&format!("UPDATE OR IGNORE ont_hidden SET {column}=?1 WHERE workspace=?2 AND {column}=?3"), params![keep, w, lose])?;
        }
        merged += 1;
    }
    Ok(merged)
}

// ── Connecting concepts ──────────────────────────────────────────────────
pub(crate) struct Structure {
    /// (name, description, subject concept ids) in teaching order.
    pub(crate) units: Vec<(String, String, Vec<String>)>,
    /// (subject, object, kind, reason, condition, negated)
    pub(crate) links: Vec<(String, String, String, String, String, bool)>,
    pub(crate) proposed: usize,
    pub(crate) dropped: usize,
}
fn reaches(edges: &HashMap<String, Vec<String>>, from: &str, to: &str) -> bool {
    let mut stack = vec![from.to_owned()];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == to {
            return true;
        }
        if seen.insert(node.clone()) {
            stack.extend(edges.get(&node).into_iter().flatten().cloned());
        }
    }
    false
}
/// Turn the provider's units and links into a consistent structure. Links that
/// point nowhere, loop, or exceed a cap are dropped and counted.
pub(crate) fn settle(view: &Value, outputs: &[Value], related_cap: usize, link_cap: usize) -> Structure {
    let known: HashSet<&str> = view["concepts"].as_array().into_iter().flatten().filter_map(|c| c["id"].as_str()).collect();
    let mut units: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut placed = HashSet::new();
    for unit in outputs.iter().flat_map(|o| o["units"].as_array().into_iter().flatten()) {
        let name = unit["name"].as_str().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let ids: Vec<String> = unit["concepts"].as_array().into_iter().flatten().filter_map(Value::as_str)
            .filter(|id| known.contains(id) && placed.insert(id.to_string())).map(str::to_owned).collect();
        match units.iter_mut().find(|u| knowledge::normalized(&u.0) == knowledge::normalized(name)) {
            Some(existing) => existing.2.extend(ids),
            None => units.push((name.to_owned(), unit["description"].as_str().unwrap_or("").trim().to_owned(), ids)),
        }
    }
    units.retain(|u| !u.2.is_empty());
    let rest: Vec<String> = view["concepts"].as_array().into_iter().flatten().filter_map(|c| c["id"].as_str()).filter(|id| !placed.contains(*id)).map(str::to_owned).collect();
    if !rest.is_empty() && !units.is_empty() {
        match units.iter_mut().find(|u| u.0 == "기타") {
            Some(existing) => existing.2.extend(rest),
            None => units.push(("기타".into(), "단원에 묶이지 않은 개념".into(), rest)),
        }
    }
    let mut taken: HashSet<(String, String, String)> = HashSet::new();
    let mut order: HashMap<&str, HashMap<String, Vec<String>>> = HashMap::new();
    for link in view["assertions"].as_array().into_iter().flatten().chain(view["hidden"].as_array().into_iter().flatten()).filter(|l| l["basis"] == "written") {
        let (s, o, kind) = (link["subject"].as_str().unwrap_or(""), link["object"].as_str().unwrap_or(""), link["relation"].as_str().unwrap_or(""));
        taken.insert((s.min(o).to_owned(), s.max(o).to_owned(), kind.to_owned()));
        if matches!(kind, "kind_of" | "prerequisite") {
            order.entry(if kind == "kind_of" { "kind_of" } else { "prerequisite" }).or_default().entry(s.to_owned()).or_default().push(o.to_owned());
        }
    }
    let mut links = Vec::new();
    let (mut proposed, mut dropped) = (0, 0);
    let mut related: HashMap<String, usize> = HashMap::new();
    let mut inferred: HashMap<String, usize> = HashMap::new();
    for link in outputs.iter().flat_map(|o| o["links"].as_array().into_iter().flatten()) {
        proposed += 1;
        let (s, o, kind) = (link["subject"].as_str().unwrap_or(""), link["object"].as_str().unwrap_or(""), link["type"].as_str().unwrap_or(""));
        if !known.contains(s) || !known.contains(o) || s == o || !LINK_KINDS.contains(&kind) {
            dropped += 1;
            continue;
        }
        // Repeating what is already there is not an error; it is simply not added.
        if !taken.insert((s.min(o).to_owned(), s.max(o).to_owned(), kind.to_owned())) {
            proposed -= 1;
            continue;
        }
        let ordered = matches!(kind, "kind_of" | "prerequisite");
        let over_related = kind == "related" && [s, o].iter().any(|id| related.get(*id).copied().unwrap_or(0) >= related_cap);
        let over_total = [s, o].iter().any(|id| inferred.get(*id).copied().unwrap_or(0) >= link_cap);
        let loops = ordered && reaches(order.get(kind).unwrap_or(&HashMap::new()), o, s);
        if over_related || over_total || loops {
            dropped += 1;
            continue;
        }
        if ordered {
            order.entry(if kind == "kind_of" { "kind_of" } else { "prerequisite" }).or_default().entry(s.to_owned()).or_default().push(o.to_owned());
        }
        for id in [s, o] {
            *inferred.entry(id.to_owned()).or_default() += 1;
            if kind == "related" {
                *related.entry(id.to_owned()).or_default() += 1;
            }
        }
        links.push((s.to_owned(), o.to_owned(), kind.to_owned(), link["reason"].as_str().unwrap_or("").trim().to_owned(), link["condition"].as_str().unwrap_or("").trim().to_owned(), link["negated"] == true));
    }
    Structure { units, links, proposed, dropped }
}
fn unit_id(name: &str) -> String {
    crate::skills::digest(knowledge::normalized(name).as_bytes())[..16].to_owned()
}
/// Replace the units and inferred links of a subject. `keep_links` false leaves
/// this revision without inferred links.
pub(crate) fn write_structure(c: &Connection, w: &str, view: &Value, s: &Structure, keep_links: bool) -> Result<()> {
    c.execute("DELETE FROM ont_units WHERE workspace=?1", [w])?;
    c.execute("UPDATE ont_concepts SET unit_id=NULL WHERE workspace=?1", [w])?;
    for (position, (name, description, ids)) in s.units.iter().enumerate() {
        let unit = unit_id(name);
        c.execute("INSERT OR REPLACE INTO ont_units VALUES(?1,?2,?3,?4,?5)", params![w, unit, name, description, position as i64])?;
        for id in ids {
            let concept = view["concepts"].as_array().into_iter().flatten().find(|x| x["id"] == id.as_str()).context("단원에 없는 개념")?;
            c.execute(
                "INSERT INTO ont_concepts VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace,id) DO UPDATE SET unit_id=excluded.unit_id",
                params![w, id, concept["name"].as_str().unwrap_or(""), concept["definition"].as_str().unwrap_or(""), unit],
            )?;
        }
    }
    c.execute("DELETE FROM ont_links WHERE workspace=?1", [w])?;
    if keep_links {
        for (s, o, kind, reason, condition, negated) in &s.links {
            let id = crate::skills::digest(format!("{s}|{o}|{kind}").as_bytes())[..24].to_owned();
            c.execute("INSERT OR REPLACE INTO ont_links VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![w, id, s, o, kind, reason, condition, negated])?;
        }
    }
    remove_deleted_links(c,w)
}

// ── Publishing a run ─────────────────────────────────────────────────────
/// Rows a run owns for one subject. The user's corrections are not among them.
const RUN_TABLES: [&str; 14] = [
    "supports", "assertions", "concepts", "relations", "sources", "analyses", "concept_kinds", "relation_kinds",
    "ont_concepts", "ont_members", "ont_units", "ont_links", "ont_material_state", "ont_revisions",
];
/// Replace the subject's map with the scratch copy in one transaction, while the
/// listed sources still exist and nothing else changed the map since the
/// scratch copy was taken. Edits made to a source while it was read do not
/// block publishing: the map holds what the fixed copy said, and the source
/// shows as changed until it is read again.
pub(crate) fn publish(root: &Path, scratch: &Path, w: &str, sources: &[String], generation: i64) -> Result<()> {
    let mut c = knowledge::db(root)?;
    c.execute("ATTACH DATABASE ?1 AS run", [scratch.to_string_lossy()])?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    anyhow::ensure!(knowledge::generation(&tx, w)? == generation, "정리하는 동안 이 과목의 자료나 지도가 바뀌어 이번 결과를 반영하지 않았습니다. 이전 지도는 그대로이고, 읽은 자료는 보관해 두었어요. 다시 정리하면 바뀐 부분만 이어서 합니다.");
    tx.execute_batch("PRAGMA defer_foreign_keys=ON;")?;
    for table in RUN_TABLES {
        tx.execute(&format!("DELETE FROM main.{table} WHERE workspace=?1"), [w])?;
    }
    for table in RUN_TABLES.iter().rev() {
        tx.execute(&format!("INSERT INTO main.{table} SELECT * FROM run.{table} WHERE workspace=?1"), [w])?;
    }
    remove_deleted_links(&tx,w)?;
    knowledge::bump_generation(&tx, w)?;
    crate::storage::with_live_sources(root, w, sources, || {
        tx.commit()?;
        Ok(())
    })?;
    c.execute("DETACH DATABASE run", [])?;
    Ok(())
}

// ── Reading and correcting ───────────────────────────────────────────────
pub(crate) fn guidance(c: &Connection, w: &str) -> Result<String> {
    Ok(c.query_row("SELECT text FROM ont_guidance WHERE workspace=?1", [w], |r| r.get(0)).optional()?.unwrap_or_default())
}
/// Which materials a run would read: new ones and ones changed since they were read.
pub(crate) fn material_states(c: &Connection, root: &Path, w: &str) -> Result<Vec<Value>> {
    let tree = crate::storage::handle(root, &json!({"op":"tree","workspace_id":w}))?;
    let mut read: HashMap<String, String> = HashMap::new();
    let mut q = c.prepare("SELECT material_id,input_hash FROM sources WHERE workspace=?1 AND current=1")?;
    for row in q.query_map([w], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, hash) = row?;
        read.insert(id, hash);
    }
    let mut quality: HashMap<String, String> = HashMap::new();
    let mut q = c.prepare("SELECT material_id,quality FROM ont_material_state WHERE workspace=?1")?;
    for row in q.query_map([w], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, value) = row?;
        quality.insert(id, value);
    }
    let mut out = Vec::new();
    for entry in tree["entries"].as_array().into_iter().flatten() {
        if entry["missing"] == true || !["md", "pdf", "png", "jpeg", "heic", "pptx"].contains(&entry["kind"].as_str().unwrap_or("")) {
            continue;
        }
        let id = entry["id"].as_str().unwrap_or("");
        let state = match read.get(id) {
            None => "new",
            Some(hash) if entry["hash"] == hash.as_str() => "organised",
            Some(_) => "changed",
        };
        out.push(json!({"id":id,"title":entry["title"],"kind":entry["kind"],"hash":entry["hash"],"path":entry["path"],"absolute_path":entry["absolute_path"],"state":state,"quality":quality.get(id)}));
    }
    // The tree lists the latest edit first; a run reads in the order of the names instead.
    let name = |m: &Value| Path::new(m["path"].as_str().unwrap_or("")).with_extension("").to_string_lossy().into_owned();
    out.sort_by(|a, b| reading_order(&name(a), &name(b)).then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
    Ok(out)
}
/// Names in reading order: digit runs compare as numbers, so "2주차" comes before "10주차".
fn reading_order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let run = |s: &[char], from: usize| s[from..].iter().take_while(|c| c.is_ascii_digit()).count();
            let (la, lb) = (run(&a, i), run(&b, j));
            let digits = |s: &[char], from: usize, len: usize| s[from..from + len].iter().skip_while(|c| **c == '0').collect::<String>();
            let (da, db) = (digits(&a, i, la), digits(&b, j, lb));
            let order = da.len().cmp(&db.len()).then_with(|| da.cmp(&db));
            if order != Ordering::Equal {
                return order;
            }
            i += la;
            j += lb;
        } else {
            let order = a[i].to_lowercase().cmp(b[j].to_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}
fn deletion_key(s: &str, o: &str, kind: &str, condition: &str, negated: bool) -> String {
    let (s, o) = if !directed(kind) && s > o { (o, s) } else { (s, o) };
    crate::skills::digest(json!([s,o,kind,condition,negated]).to_string().as_bytes())
}

// Resolve both material assertions and inferred links through the current subject groups.
pub(crate) fn remove_deleted_links(c: &Connection, w: &str) -> Result<()> {
    cleanup::purge(c,w)?;
    let deleted: HashSet<String> = c.prepare("SELECT fingerprint FROM ont_deleted_links WHERE workspace=?1")?
        .query_map([w], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    if deleted.is_empty() { return Ok(()); }
    let groups = members(c, w)?;
    let mut q = c.prepare("SELECT a.id,a.subject,a.object,COALESCE(k.kind,'related'),a.condition,a.negated,0
        FROM assertions a LEFT JOIN relation_kinds k ON k.relation_id=a.relation WHERE a.workspace=?1
        UNION ALL SELECT id,subject,object,kind,condition,negated,1 FROM ont_links WHERE workspace=?1")?;
    let rows = q.query_map([w], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,
        r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?,r.get::<_,bool>(6)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id,s,o,kind,condition,negated,inferred) in rows {
        let (s,o) = (groups.get(&s).unwrap_or(&s),groups.get(&o).unwrap_or(&o));
        if !deleted.contains(&deletion_key(s,o,&kind,&condition,negated)) { continue; }
        if inferred {
            c.execute("DELETE FROM ont_links WHERE workspace=?1 AND id=?2", params![w,id])?;
        } else {
            c.execute("DELETE FROM supports WHERE workspace=?1 AND assertion_id=?2", params![w,id])?;
            c.execute("DELETE FROM assertions WHERE workspace=?1 AND id=?2", params![w,id])?;
        }
    }
    Ok(())
}

pub fn handle(root: &Path, req: &Value) -> Result<Value> {
    let w = req["workspace_id"].as_str().context("workspace_id가 필요합니다")?;
    let c = knowledge::db(root)?;
    match req["op"].as_str().unwrap_or("") {
        "ontology.cleanup.review" => cleanup::review(root,w,req),
        "ontology.cleanup.preview" => cleanup::change(root,&c,w,req,false),
        "ontology.cleanup.delete" => cleanup::change(root,&c,w,req,true),
        "ontology.status" => {
            let materials = material_states(&c, root, w)?;
            Ok(json!({
                "organised":latest_revision(&c, w)?.is_some(),"revision":latest_revision(&c, w)?,
                "needs":materials.iter().filter(|m| m["state"] != "organised").count(),
                "materials":materials,"guidance":guidance(&c, w)?,
            }))
        }
        "ontology.link.delete" => {
            let tx = c.unchecked_transaction()?;
            let current = view(&tx, root, w)?;
            let selected = &req["link"];
            let link = current["assertions"].as_array().into_iter().flatten().find(|l|
                ["id","subject","object","relation","condition","negated","basis"].iter().all(|k| l[k] == selected[k]))
                .context("연결이 변경되었습니다. 다시 선택해 주세요.")?;
            let subject = link["subject"].as_str().unwrap();
            let object = link["object"].as_str().unwrap();
            for id in [subject,object] {
                tx.execute("INSERT OR IGNORE INTO ont_kept_concepts VALUES(?1,?2)",params![w,id])?;
            }
            let key = deletion_key(subject, object, link["relation"].as_str().unwrap(),
                link["condition"].as_str().unwrap_or(""), link["negated"] == true);
            tx.execute("INSERT OR IGNORE INTO ont_deleted_links VALUES(?1,?2)", params![w,key])?;
            remove_deleted_links(&tx,w)?;
            knowledge::bump_generation(&tx,w)?;
            tx.commit()?;
            Ok(json!({"deleted":true}))
        }
        "ontology.link.hide" => {
            let (s, o, kind) = (req["subject"].as_str().context("subject 없음")?, req["object"].as_str().context("object 없음")?, req["kind"].as_str().context("kind 없음")?);
            if req["hidden"] == false {
                c.execute("DELETE FROM ont_hidden WHERE workspace=?1 AND subject=?2 AND object=?3 AND kind=?4", params![w, s, o, kind])?;
            } else {
                c.execute("INSERT OR IGNORE INTO ont_hidden VALUES(?1,?2,?3,?4)", params![w, s, o, kind])?;
            }
            Ok(json!({"hidden":req["hidden"] != false}))
        }
        "ontology.unmerge" => {
            let member = req["member_id"].as_str().context("member_id 없음")?;
            let tx = c.unchecked_transaction()?;
            let concept: String = tx.query_row("SELECT concept_id FROM ont_members WHERE workspace=?1 AND member_id=?2", params![w, member], |r| r.get(0)).optional()?.context("합쳐진 개념이 아닙니다")?;
            let mut others: Vec<String> = tx.prepare("SELECT member_id FROM ont_members WHERE workspace=?1 AND concept_id=?2 AND member_id<>?3")?
                .query_map(params![w, concept, member], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            others.push(concept.clone());
            // Remember the decision so the next run does not merge them again.
            for other in others {
                let (a, b) = if member <= other.as_str() { (member.to_owned(), other) } else { (other, member.to_owned()) };
                tx.execute("INSERT OR IGNORE INTO ont_corrections VALUES(?1,?2,?3)", params![w, a, b])?;
            }
            tx.execute("DELETE FROM ont_members WHERE workspace=?1 AND member_id=?2", params![w, member])?;
            let unit: Option<String> = tx.query_row("SELECT unit_id FROM ont_concepts WHERE workspace=?1 AND id=?2", params![w, concept], |r| r.get(0)).optional()?.flatten();
            tx.execute("INSERT OR IGNORE INTO ont_concepts VALUES(?1,?2,'','',?3)", params![w, member, unit])?;
            knowledge::bump_generation(&tx, w)?;
            tx.commit()?;
            Ok(json!({"separated":member}))
        }
        "ontology.concept.describe" => {
            let id = req["concept_id"].as_str().context("concept_id 없음")?;
            let text = req["definition"].as_str().unwrap_or("").trim();
            anyhow::ensure!(text.chars().count() <= 200, "설명은 200자 이내로 적어 주세요");
            // An empty sentence, or the original one, takes the user's wording away again.
            if text.is_empty() {
                c.execute("DELETE FROM ont_notes WHERE workspace=?1 AND concept_id=?2", params![w, id])?;
            } else {
                c.execute("INSERT INTO ont_notes VALUES(?1,?2,?3) ON CONFLICT(workspace,concept_id) DO UPDATE SET definition=excluded.definition", params![w, id, text])?;
            }
            let concept = graph(&c, root, w)?["concepts"].as_array().into_iter().flatten().find(|x| x["id"] == id).cloned().unwrap_or(Value::Null);
            Ok(json!({"concept":concept}))
        }
        "ontology.guidance.set" => {
            let text = req["text"].as_str().context("지침 내용 없음")?;
            anyhow::ensure!(text.len() <= 4000, "정리 지침은 4,000바이트 이내로 적어 주세요");
            c.execute("INSERT OR REPLACE INTO ont_guidance VALUES(?1,?2)", params![w, text.trim()])?;
            Ok(json!({"guidance":text.trim()}))
        }
        op => bail!("지원하지 않는 과목 정리 명령: {op}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{handle, link_sentence, reading_order};
    use serde_json::json;

    #[test]
    fn a_link_reads_with_the_particle_its_names_take() {
        assert_eq!(link_sentence("related", "의존성 주입", "스프링", false), "의존성 주입과 스프링은 관련이 있어요");
        assert_eq!(link_sentence("part_of", "세터", "컨테이너", false), "세터는 컨테이너를 이루는 부분이에요");
        assert_eq!(link_sentence("cause", "락", "교착", false), "락 때문에 교착이 일어나요");
        // A name that does not end in Hangul cannot be read for certain.
        assert_eq!(link_sentence("contrast", "SQL", "NoSQL", true), "SQL와(과) NoSQL은(는) 대비되지 않아요");
    }

    #[test]
    fn materials_are_read_in_the_order_of_their_numbers() {
        let mut names = vec!["Chapter 10", "chapter 2", "Chapter 1", "Chapter 02 보충", "10주차", "2주차"];
        names.sort_by(|a, b| reading_order(a, b));
        assert_eq!(names, ["2주차", "10주차", "Chapter 1", "chapter 2", "Chapter 02 보충", "Chapter 10"]);
    }

    #[test]
    fn a_concept_sentence_written_by_the_user_is_kept_and_can_be_taken_back() {
        let t = tempfile::tempdir().unwrap();
        let c = crate::knowledge::db(t.path()).unwrap();
        c.execute("INSERT INTO concepts VALUES('c1','w','서브쿼리','[]','조건문 안에 들어가는 쿼리 형식이다.')", []).unwrap();
        c.execute("INSERT INTO ont_revisions VALUES('w',1,'now','run','')", []).unwrap();
        let read = |t: &std::path::Path| {
            let g = super::graph(&crate::knowledge::db(t).unwrap(), t, "w").unwrap();
            g["concepts"][0].clone()
        };
        assert_eq!(read(t.path())["edited"], false);
        let set = |text: &str| handle(t.path(), &json!({"op":"ontology.concept.describe","workspace_id":"w","concept_id":"c1","definition":text}));
        assert_eq!(set("  WHERE 조건에서 쓰는 쿼리다.  ").unwrap()["concept"]["definition"], "WHERE 조건에서 쓰는 쿼리다.");
        let concept = read(t.path());
        assert_eq!(concept["edited"], true);
        assert_eq!(concept["original_definition"], "조건문 안에 들어가는 쿼리 형식이다.");
        assert!(set(&"가".repeat(201)).is_err());
        assert_eq!(set("").unwrap()["concept"]["definition"], "조건문 안에 들어가는 쿼리 형식이다.");
        assert_eq!(read(t.path())["edited"], false);
    }
}
