//! 과목 정리 end to end against a fake Codex CLI: chunked reading, evidence checks
//! and re-reading, aligning, connecting, publishing at once, and stopping.
mod common;
use common::{call, calls, request};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

/// Reads "A는 B의 한 종류이다." sentences. Markers in the text steer failures.
const FAKE_CODEX: &str = r##"#!/usr/bin/env python3
import sys,json,os,re,time
root=os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
a=sys.argv
if 'login' in a:
 print('Logged in using ChatGPT');sys.exit(0)
if 'app-server' in a:
 for line in sys.stdin:
  v=json.loads(line);m=v.get('method');i=v.get('id')
  if m=='initialize':r={}
  elif m=='model/list':
   e=[{'reasoningEffort':x} for x in ['low','medium','high']]
   r={'data':[{'model':n,'displayName':n,'isDefault':n=='gpt-6.1-sol','defaultReasoningEffort':'medium','supportedReasoningEfforts':e} for n in ['gpt-6.1-sol','gpt-6.1-terra','gpt-6-luna']]}
  elif m=='config/read':r={'config':{'mcp_servers':{},'default_permissions':'stology_chat','permissions':{'stology_chat':{'network':{'enabled':False}}},'features':{'skip_host_skill_discovery':True,'hooks':False,'plugins':False}}}
  else:continue
  print(json.dumps({'id':i,'result':r}),flush=True)
 sys.exit(0)
prompt=sys.stdin.read()
model=a[a.index('--model')+1]
out=a[a.index('--output-last-message')+1]
if prompt.startswith('과목 전체 지식지도'):
 inp=json.loads(prompt.split('\n',1)[1])
 selected=[{'id':c['id'],'reason':'자료에서 예시로만 등장해요.'} for c in inp['concepts'] if c['name'] in ['물','얼음']]
 open(out,'w').write(json.dumps({'candidates':selected}))
 print(json.dumps({'type':'turn.completed'}));sys.exit(0)
skill=re.match(r'# .*\((.+)\)',prompt).group(1)
inp=json.loads(prompt.rsplit('## INPUT\n\n```json\n',1)[1].rsplit('\n```',1)[0])
text=json.dumps(inp,ensure_ascii=False)
ok=os.path.exists(os.path.join(root,'fake-ok'))
with open(os.path.join(root,'fake-calls.jsonl'),'a') as f:
 f.write(json.dumps({'skill':skill,'model':model,'guided':'이 과목의 정리 지침' in prompt,'effort':[x for x in a if x.startswith('model_reasoning_effort')]})+'\n')
def fail(msg):
 sys.stderr.write(msg+'\n');sys.exit(1)
if 'SLOW' in text and not ok:time.sleep(3)
if skill=='extract-material':
 if 'LIMIT' in text and not ok:fail('You have hit your usage limit')
 if 'BROKEN' in text:fail('boom')
 concepts={};claims=[]
 def cid(name):
  if name not in concepts:concepts[name]='c%d'%(len(concepts)+1)
  return concepts[name]
 for u in inp['units']:
  for bad,s,josa,o in re.findall(r'(ALWAYSBAD )?(\S+)([는은]) (\S+)의 한 종류이다\.',u['text']):
   quote='%s%s %s의 한 종류이다.'%(s,josa,o)
   if bad or ('BADQUOTE' in u['text'] and model!='gpt-6.1-sol'):quote='지어낸 인용'
   claims.append({'subject':cid(s),'relation':'r1','object':cid(o),'condition':'','negated':False,'evidence':[{'locator':u['locator'],'quote':quote,'image_path':None,'region':None}]})
 r={'concepts':[{'id':i,'name':n,'aliases':[],'definition':'%s: %s에서 본 뜻'%(n,inp['material']),'type':'concept'} for n,i in concepts.items()],
    'relations':[{'id':'r1','label':'종류','definition':'한 종류이다','directed':True,'aliases':[],'type':'kind_of'}] if claims else [],'assertions':claims}
elif skill=='align-concepts':
 r={'decisions':[{'pair':p['pair'],'verdict':'same','name':p['a']['name'],'definition':p['a']['definition']+' / '+p['b']['definition']} if p['a']['name'].lower()==p['b']['name'].lower() else {'pair':p['pair'],'verdict':'different','name':'','definition':''} for p in inp['pairs']]}
elif skill=='connect-concepts':
 if 'CONNECTFAIL' in text and not ok:fail('boom')
 ids=[c['id'] for c in inp['concepts']]
 half=max(1,len(ids)//2)
 units=[{'name':'기초','description':'먼저 배우는 개념','concepts':ids[:half]},{'name':'응용','description':'이어서 배우는 개념','concepts':ids[half:]}]
 links=[{'subject':ids[i],'object':ids[i+1],'type':'related','reason':'정의에서 함께 다룬다','condition':'','negated':False} for i in range(len(ids)-1)]
 links.append({'subject':'c999','object':ids[0],'type':'related','reason':'없는 개념','condition':'','negated':False})
 if 'BADLINKS' in text:links=[dict(l,subject='c999') for l in links]
 r={'units':[u for u in units if u['concepts']],'links':links if inp['make_links'] else []}
else:
 r={'reviews':[{'link':l['link'],'verdict':'unsupported' if n==0 else 'supported'} for n,l in enumerate(inp['links'])]}
open(out,'w').write(json.dumps(r,ensure_ascii=False))
print(json.dumps({'type':'turn.completed','usage':{'input_tokens':100,'output_tokens':20}}))
"##;

fn fixture() -> (tempfile::TempDir, String) {
    common::fixture(FAKE_CODEX)
}
fn note(root: &Path, w: &str, title: &str, body: &str) -> Value {
    let d = call(root, json!({"op":"draft.create","workspace_id":w,"body":body}));
    call(root, json!({"op":"note.save","draft_id":d["id"],"title":title}))
}
fn finish(root: &Path, w: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let run = call(root, json!({"op":"organize.status","workspace_id":w}))["run"].clone();
        if run["state"] != "running" {
            return run;
        }
        assert!(Instant::now() < deadline, "{run}");
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn organize(root: &Path, w: &str, extra: Value) -> Value {
    let mut r = json!({"op":"organize.start","workspace_id":w});
    for (k, v) in extra.as_object().unwrap() {
        r[k] = v.clone();
    }
    call(root, r);
    finish(root, w)
}
fn count(root: &Path, skill: &str) -> usize {
    calls(root).iter().filter(|c| c["skill"] == skill).count()
}
fn graph(root: &Path, w: &str) -> Value {
    call(root, json!({"op":"graph.get","workspace_id":w}))
}
fn two_weeks(root: &Path, w: &str) {
    // Two chunks for the first note: about 20 characters per paragraph, 40 per chunk.
    fs::create_dir_all(root.join("ai/skills")).unwrap();
    fs::write(root.join("ai/skills/routing.json"), r#"{"chunk":{"max_chars":40}}"#).unwrap();
    note(root, w, "1주차", "JWT는 토큰의 한 종류이다.\n\n세션은 상태의 한 종류이다.\n\n쿠키는 저장소의 한 종류이다.");
    note(root, w, "2주차", "jwt는 인증수단의 한 종류이다.");
}

#[test]
fn subject_is_read_in_chunks_aligned_connected_and_published_once() {
    let (dir, w) = fixture();
    let root = dir.path();
    two_weeks(root, &w);
    let plan = call(root, json!({"op":"organize.plan","workspace_id":w}));
    assert_eq!(plan["targets"], 2);
    assert_eq!(plan["calls"]["extract"], 3, "{plan}");
    assert_eq!(plan["stages"]["extract"]["model"], "gpt-6.1-terra");
    assert_eq!(plan["stages"]["connect"]["model"], "gpt-6.1-sol");
    assert!(plan["stages"]["review"].is_null());

    let run = organize(root, &w, json!({}));
    assert_eq!(run["state"], "succeeded", "{run}");
    assert_eq!(run["result"]["merged"], 1);
    assert_eq!(run["result"]["revision"], 1);
    // Materials are read in the order of their names, not in the order they were last edited.
    assert_eq!(run["materials"][0]["title"], "1주차", "{}", run["materials"]);
    assert_eq!(run["materials"][0]["chunks"].as_array().unwrap().len(), 2, "{}", run["materials"]);
    assert_eq!((count(root, "extract-material"), count(root, "align-concepts"), count(root, "connect-concepts"), count(root, "review-links")), (3, 1, 1, 0));
    assert!(calls(root).iter().filter(|c| c["skill"] == "extract-material").all(|c| c["model"] == "gpt-6.1-terra"));
    assert!(calls(root).iter().filter(|c| c["skill"] != "extract-material").all(|c| c["model"] == "gpt-6.1-sol"));
    let how = run["how"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect::<Vec<_>>().join("\n");
    assert!(how.contains("자료 읽기: AI 호출 3번") && how.contains("gpt-6.1-terra") && how.contains("토큰 입력 300 / 출력 60"), "{how}");

    let g = graph(root, &w);
    assert_eq!(g["layer"], "subject");
    assert_eq!(g["concepts"].as_array().unwrap().len(), 7);
    let jwt = g["concepts"].as_array().unwrap().iter().find(|c| c["merged"].as_array().is_some_and(|m| !m.is_empty())).unwrap();
    assert_eq!(jwt["materials"].as_array().unwrap().len(), 2);
    assert_eq!(g["units"].as_array().unwrap().len(), 2);
    let links = g["assertions"].as_array().unwrap();
    assert_eq!(links.iter().filter(|l| l["basis"] == "written").count(), 4);
    // Six proposed links exist; the one pointing at an unknown concept was dropped.
    assert_eq!(links.iter().filter(|l| l["basis"] == "inferred").count(), 6);
    assert!(links.iter().filter(|l| l["basis"] == "written").all(|l| l["relation"] == "kind_of" && !l["evidence"].as_array().unwrap().is_empty()));
    assert_eq!(call(root, json!({"op":"tree","workspace_id":w}))["entries"][0]["analysis_state"], "analysed");

    // Nothing changed: every step is reused and no revision is added.
    let again = organize(root, &w, json!({}));
    assert_eq!(again["result"]["changed"], false, "{again}");
    assert_eq!(calls(root).len(), 5);
    assert_eq!(graph(root, &w)["revision"]["n"], 1);

    // The user's corrections outlive the next run.
    let inferred = links.iter().find(|l| l["basis"] == "inferred").unwrap();
    call(root, json!({"op":"ontology.link.hide","workspace_id":w,"subject":inferred["subject"],"object":inferred["object"],"kind":inferred["relation"]}));
    call(root, json!({"op":"ontology.unmerge","workspace_id":w,"member_id":jwt["merged"][0]["id"]}));
    assert_eq!(graph(root, &w)["concepts"].as_array().unwrap().len(), 8);
    call(root, json!({"op":"ontology.guidance.set","workspace_id":w,"text":"영어 약어는 풀어 쓴다"}));
    let third = organize(root, &w, json!({"mode":"thorough"}));
    assert_eq!(third["state"], "succeeded", "{third}");
    assert_eq!(count(root, "align-concepts"), 1, "separated concepts are not offered again");
    assert_eq!(count(root, "review-links"), 1);
    assert_eq!(third["review"]["removed"], 1);
    assert!(calls(root).last().unwrap()["skill"] == "review-links" && calls(root).iter().rev().nth(1).unwrap()["guided"] == true);
    let g = graph(root, &w);
    assert_eq!(g["concepts"].as_array().unwrap().len(), 8);
    // Which of the two merged ids survived is not fixed (the smaller generated id), so the redone links
    // may equal the old ones. Either way the stored revision agrees with what the run reported.
    let changed = third["result"]["changed"] == true;
    assert_eq!(g["revision"]["n"], if changed { 2 } else { 1 }, "{}", third["result"]);
    assert_eq!(g["hidden"].as_array().unwrap().len() + g["assertions"].as_array().unwrap().iter().filter(|l| l["basis"] == "inferred").count(), 7 - 1);
}

#[test]
fn weak_evidence_is_read_again_a_tier_up_and_marked_when_it_stays_weak() {
    let (dir, w) = fixture();
    let root = dir.path();
    note(root, &w, "고쳐지는 자료", "BADQUOTE 물은 액체의 한 종류이다.");
    note(root, &w, "계속 틀리는 자료", "얼음은 고체의 한 종류이다. ALWAYSBAD 김은 기체의 한 종류이다.");
    let run = organize(root, &w, json!({}));
    assert_eq!(run["state"], "succeeded", "{run}");
    let extracts: Vec<Value> = calls(root).into_iter().filter(|c| c["skill"] == "extract-material").collect();
    assert_eq!(extracts.iter().map(|c| c["model"].as_str().unwrap()).collect::<Vec<_>>(), ["gpt-6.1-terra", "gpt-6.1-sol", "gpt-6.1-terra", "gpt-6.1-sol"]);
    let materials = run["materials"].as_array().unwrap();
    let fixed = materials.iter().find(|m| m["title"] == "고쳐지는 자료").unwrap();
    let weak = materials.iter().find(|m| m["title"] == "계속 틀리는 자료").unwrap();
    assert_eq!((fixed["quality"].as_str(), weak["quality"].as_str()), (Some("ok"), Some("low")));
    assert!(run["how"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().contains("다시 읽음: 고쳐지는 자료") && l.as_str().unwrap().contains("정밀 등급으로 다시 읽음")), "{}", run["how"]);
    assert_eq!(run["result"]["low_quality"], json!(["계속 틀리는 자료"]));
    let g = graph(root, &w);
    // The invented claim is gone; the supported one from the same material stays and is marked.
    assert_eq!(g["assertions"].as_array().unwrap().iter().filter(|l| l["basis"] == "written").count(), 2);
    let ice = g["concepts"].as_array().unwrap().iter().find(|c| c["name"] == "얼음").unwrap();
    assert_eq!(ice["low_quality"], true);
    assert_eq!(g["concepts"].as_array().unwrap().iter().find(|c| c["name"] == "물").unwrap()["low_quality"], false);
    assert!(g["concepts"].as_array().unwrap().iter().all(|c| c["name"] != "김"));

    // `꼼꼼하게 다시 정리` reads that material alone with the highest tier.
    let before = calls(root).len();
    let reread = organize(root, &w, json!({"materials":[weak["id"]],"reread":true}));
    assert_eq!(reread["state"], "succeeded", "{reread}");
    assert_eq!(calls(root)[before]["model"], "gpt-6.1-sol");
}

#[test]
fn a_failed_step_keeps_the_previous_map_and_resume_reuses_what_was_read() {
    let (dir, w) = fixture();
    let root = dir.path();
    two_weeks(root, &w);
    assert_eq!(organize(root, &w, json!({}))["state"], "succeeded");
    let before = graph(root, &w);

    // Connecting fails: the newly read material must not appear.
    note(root, &w, "3주차", "CONNECTFAIL는 실패의 한 종류이다. 락는 잠금의 한 종류이다.");
    let failed = organize(root, &w, json!({}));
    assert_eq!(failed["state"], "failed", "{failed}");
    assert!(failed["error"].as_str().unwrap().contains("개념 잇기 단계가 실패"));
    assert_eq!(graph(root, &w), before);
    assert_eq!(call(root, json!({"op":"ontology.status","workspace_id":w}))["needs"], 1);
    let read = count(root, "extract-material");
    fs::write(root.join("fake-ok"), "").unwrap();
    call(root, json!({"op":"organize.resume","workspace_id":w}));
    let resumed = finish(root, &w);
    assert_eq!(resumed["state"], "succeeded", "{resumed}");
    assert_eq!(count(root, "extract-material"), read, "finished reads are reused");
    // The read made before the failure belongs to this run: it is reported as a call, not as a reuse.
    let how = resumed["how"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect::<Vec<_>>().join("\n");
    assert!(how.contains("자료 읽기: AI 호출 1번 ·") && !how.contains("재사용"), "{how}");
    assert_eq!(graph(root, &w)["revision"]["n"], 2);
    fs::remove_file(root.join("fake-ok")).unwrap();

    // A subscription limit leaves the run stopped, without touching the map.
    let before = graph(root, &w);
    note(root, &w, "4주차", "LIMIT 큐는 자료구조의 한 종류이다.");
    let limited = organize(root, &w, json!({}));
    assert_eq!(limited["state"], "limit", "{limited}");
    assert!(limited["error"].as_str().unwrap().contains("구독 사용 한도"));
    assert_eq!(graph(root, &w), before);

    // One unreadable material is skipped and reported; the others are organised.
    fs::write(root.join("fake-ok"), "").unwrap();
    note(root, &w, "5주차", "BROKEN 스택은 자료구조의 한 종류이다.");
    let partial = organize(root, &w, json!({}));
    assert_eq!(partial["state"], "succeeded", "{partial}");
    assert_eq!(partial["result"]["failed"][0]["title"], "5주차");
    assert!(graph(root, &w)["concepts"].as_array().unwrap().iter().any(|c| c["name"] == "큐"));
    assert_eq!(call(root, json!({"op":"ontology.status","workspace_id":w}))["needs"], 1);
}

#[test]
fn bad_links_leave_units_without_inferred_links() {
    let (dir, w) = fixture();
    let root = dir.path();
    note(root, &w, "자료", "BADLINKS는 표식의 한 종류이다. 물은 액체의 한 종류이다.");
    let run = organize(root, &w, json!({}));
    assert_eq!(run["state"], "succeeded", "{run}");
    let g = graph(root, &w);
    assert_eq!(g["units"].as_array().unwrap().len(), 2);
    assert!(g["assertions"].as_array().unwrap().iter().all(|l| l["basis"] == "written"));
    assert!(run["how"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().contains("점선 연결을 넣지 않았어요")), "{}", run["how"]);
}

#[test]
fn one_material_analysis_reads_only_that_material() {
    let (dir, w) = fixture();
    let root = dir.path();
    let n = note(root, &w, "자료", "물은 액체의 한 종류이다.");
    note(root, &w, "다른 자료", "불은 현상의 한 종류이다.");
    let job = call(root, json!({"op":"analysis.begin","workspace_id":w,"material_id":n["id"],"expected_hash":n["hash"]}));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = call(root, json!({"op":"analysis.status","id":job["id"]}));
        if state["state"] == "succeeded" {
            assert_eq!(state["result"]["single"], true);
            break;
        }
        assert_ne!(state["state"], "failed", "{state}");
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(calls(root).len(), 1);
    let g = graph(root, &w);
    assert_eq!(g["layer"], "material");
    assert_eq!(g["assertions"].as_array().unwrap().len(), 1);
    assert!(call(root, json!({"op":"analysis.active"}))["active"].is_null());
}

#[test]
fn stopping_and_waiting_never_publish_a_partial_map_and_edits_do_not_block_publishing() {
    let (dir, w) = fixture();
    let root = dir.path();
    let n = note(root, &w, "느린 자료", "SLOW 물은 액체의 한 종류이다.");

    // A chat turn asked to go first: no step starts until it is done.
    fs::create_dir_all(root.join("ai")).unwrap();
    fs::write(root.join("ai/yield-request"), "chat").unwrap();
    call(root, json!({"op":"organize.start","workspace_id":w}));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(calls(root).is_empty());
    fs::remove_file(root.join("ai/yield-request")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while calls(root).is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }

    // 멈춤 during the step, then 이어서 정리.
    let paused = call(root, json!({"op":"organize.pause","workspace_id":w}))["run"].clone();
    assert_eq!(paused["state"], "paused");
    assert!(call(root, json!({"op":"analysis.active"}))["active"].is_null());
    assert_eq!(graph(root, &w)["concepts"], json!([]));
    call(root, json!({"op":"organize.resume","workspace_id":w}));

    // The material is edited while the run reads it: the copy fixed at the
    // request is published, and the material waits to be read again.
    while calls(root).len() < 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    call(root, json!({"op":"note.update","id":n["id"],"expected_version":n["version"],"body":"SLOW 불은 현상의 한 종류이다."}));
    let run = finish(root, &w);
    assert_eq!(run["state"], "succeeded", "{run}");
    let g = graph(root, &w);
    let names: Vec<&str> = g["concepts"].as_array().unwrap().iter().filter_map(|c| c["name"].as_str()).collect();
    assert!(names.contains(&"물") && !names.contains(&"불"), "{names:?}");
    assert_eq!(call(root, json!({"op":"ontology.status","workspace_id":w}))["materials"][0]["state"], "changed");
}

#[test]
fn a_material_deleted_during_a_run_is_not_published() {
    let (dir, w) = fixture();
    let root = dir.path();
    let n = note(root, &w, "느린 자료", "SLOW 물은 액체의 한 종류이다.");
    call(root, json!({"op":"organize.start","workspace_id":w}));
    let deadline = Instant::now() + Duration::from_secs(20);
    while calls(root).is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    call(root, json!({"op":"note.delete","id":n["id"]}));
    let run = finish(root, &w);
    assert_eq!(run["state"], "failed", "{run}");
    assert!(run["error"].as_str().unwrap().contains("삭제"), "{run}");
    assert_eq!(graph(root, &w)["concepts"], json!([]));
}

#[test]
fn deleting_a_written_link_removes_its_support_and_survives_reorganizing() {
    let (dir, workspace) = fixture();
    let root = dir.path();
    note(root, &workspace, "분류", "물은 액체의 한 종류이다. 얼음은 고체의 한 종류이다.");
    assert_eq!(organize(root, &workspace, json!({}))["state"], "succeeded");
    let before = graph(root, &workspace);
    let link = before["assertions"].as_array().unwrap().iter().find(|l| l["basis"] == "written").unwrap();
    call(root, json!({"op":"ontology.link.delete","workspace_id":workspace,"link":link}));
    let db = rusqlite::Connection::open(root.join("knowledge.sqlite")).unwrap();
    for query in ["SELECT count(*) FROM assertions WHERE id=?1", "SELECT count(*) FROM supports WHERE assertion_id=?1"] {
        assert_eq!(db.query_row(query, [link["id"].as_str().unwrap()], |r| r.get::<_,i64>(0)).unwrap(), 0);
    }
    assert_eq!(organize(root, &workspace, json!({"mode":"thorough"}))["state"], "succeeded");
    let after = graph(root, &workspace);
    assert_eq!(before["concepts"].as_array().unwrap().len(), after["concepts"].as_array().unwrap().len());
    assert!(after["assertions"].as_array().unwrap().iter().all(|l| l["sentence"] != link["sentence"]));
    assert_eq!(after["assertions"].as_array().unwrap().iter().filter(|l| l["basis"] == "written").count(), 1);
}

#[test]
fn cleanup_reviews_and_deletes_joint_neighbors_only_after_approval() {
    let (dir, w) = fixture();
    let root = dir.path();
    note(root, &w, "분류", "물은 물질의 한 종류이다. 얼음은 물질의 한 종류이다. 얼음은 고체의 한 종류이다.");
    assert_eq!(organize(root, &w, json!({"make_links":false}))["state"], "succeeded");
    let review = call(root,json!({"op":"ontology.cleanup.review","workspace_id":w,"provider":"codex"}));
    let ids: Vec<_> = review["candidates"].as_array().unwrap().iter().map(|c| &c["id"]).collect();
    assert_eq!(ids.len(),2);
    let mut req=json!({"op":"ontology.cleanup.preview","workspace_id":w,"version":review["version"],"ids":ids});
    let preview=call(root,req.clone());
    let affected=preview["affected"].as_array().unwrap();
    assert!(affected.iter().any(|c| c["name"]=="물질"));
    req["op"]=json!("ontology.cleanup.delete");
    req["extra"]=json!([affected.iter().find(|c|c["name"]=="물질").unwrap()["id"]]);
    call(root,req.clone());
    assert_eq!(request(root,req)["ok"],false);
    let after=graph(root,&w);
    let remaining=after["concepts"].as_array().unwrap();
    assert_eq!(remaining.len(),1);
    assert_eq!(remaining[0]["name"],"고체");
    assert_eq!(after["assertions"],json!([]));
    assert_eq!(organize(root, &w, json!({"mode":"thorough"}))["state"], "succeeded");
    assert_eq!(graph(root,&w)["concepts"][0]["name"],"고체");
    assert_eq!(graph(root,&w)["concepts"].as_array().unwrap().len(),1);
}
