//! Notes from study-assistant conversations against a fake Codex CLI: what is offered
//! without a model, one batched call in the back lane, and the suggestion box.
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

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
  elif m=='thread/start':r={'thread':{'id':'fixture-session'}}
  elif m=='turn/start':
   print(json.dumps({'id':i,'result':{'turn':{'id':'provider-turn'}}}),flush=True)
   print(json.dumps({'method':'item/agentMessage/delta','params':{'delta':'바로 답했어요'}}),flush=True)
   print(json.dumps({'method':'turn/completed','params':{'turn':{'status':'completed'}}}),flush=True)
   continue
  else:continue
  print(json.dumps({'id':i,'result':r}),flush=True)
 sys.exit(0)
prompt=sys.stdin.read()
model=a[a.index('--model')+1]
out=a[a.index('--output-last-message')+1]
skill=re.match(r'# .*\((.+)\)',prompt).group(1)
inp=json.loads(prompt.rsplit('## INPUT\n\n```json\n',1)[1].rsplit('\n```',1)[0])
text=json.dumps(inp,ensure_ascii=False)
ok=os.path.exists(os.path.join(root,'fake-ok'))
with open(os.path.join(root,'fake-calls.jsonl'),'a') as f:
 f.write(json.dumps({'skill':skill,'model':model,'turns':len(inp.get('turns',[])),'titles':inp.get('existing_titles',[])},ensure_ascii=False)+'\n')
def fail(msg):
 sys.stderr.write(msg+'\n');sys.exit(1)
if skill=='extract-material':
 concepts={};claims=[]
 def cid(name):
  if name not in concepts:concepts[name]='c%d'%(len(concepts)+1)
  return concepts[name]
 for u in inp['units']:
  for s,josa,o in re.findall(r'(\S+)([는은]) (\S+)의 한 종류이다\.',u['text']):
   claims.append({'subject':cid(s),'relation':'r1','object':cid(o),'condition':'','negated':False,'evidence':[{'locator':u['locator'],'quote':'%s%s %s의 한 종류이다.'%(s,josa,o),'image_path':None,'region':None}]})
 r={'concepts':[{'id':i,'name':n,'aliases':[],'definition':n+'의 뜻','type':'concept'} for n,i in concepts.items()],
    'relations':[{'id':'r1','label':'종류','definition':'한 종류이다','directed':True,'aliases':[],'type':'kind_of'}] if claims else [],'assertions':claims}
elif skill=='align-concepts':
 r={'decisions':[{'pair':p['pair'],'verdict':'different','name':'','definition':''} for p in inp['pairs']]}
elif skill=='connect-concepts':
 r={'units':[{'name':'기초','description':'','concepts':[c['id'] for c in inp['concepts']]}],'links':[]}
elif skill=='distill-chat':
 if 'SLOW' in text and not ok:time.sleep(30)
 if 'LIMIT' in text and not ok:fail('You have hit your usage limit')
 if '잡담' in text:r={'notes':[],'skipped_reason':'잡담이에요'}
 else:r={'notes':[{'title':'정리: '+t['question'][:16],'kind':'concept','body':'한 줄 요약\n\n'+t['answer'][:40],'concepts':inp['concepts'][:1]+['없는 개념'],'turns':[t['turn']]} for t in inp['turns']],'skipped_reason':''}
open(out,'w').write(json.dumps(r,ensure_ascii=False))
print(json.dumps({'type':'turn.completed','usage':{'input_tokens':100,'output_tokens':20}}))
"##;

fn fixture() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("runtime/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("codex"), FAKE_CODEX).unwrap();
    fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(dir.path().join("provider.json"), r#"{"provider":"codex"}"#).unwrap();
    let w = call(dir.path(), json!({"op":"workspace.create","name":"검증 과목"}))["id"].as_str().unwrap().to_owned();
    (dir, w)
}
fn request(root: &Path, request: Value) -> Value {
    let mut p = Command::new(env!("CARGO_BIN_EXE_stology-core"))
        .args(["--root", root.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(request.to_string().as_bytes()).unwrap();
    serde_json::from_slice(&p.wait_with_output().unwrap().stdout).unwrap()
}
fn call(root: &Path, r: Value) -> Value {
    let v = request(root, r.clone());
    assert_eq!(v["ok"], true, "{r} → {v}");
    v["result"].clone()
}
/// The subject knows 스택 and 자료구조 after one organised note.
fn organise(root: &Path, w: &str) {
    let d = call(root, json!({"op":"draft.create","workspace_id":w,"body":"스택은 자료구조의 한 종류이다."}));
    call(root, json!({"op":"note.save","draft_id":d["id"],"title":"1주차"}));
    call(root, json!({"op":"organize.start","workspace_id":w}));
    let deadline = Instant::now() + Duration::from_secs(60);
    while call(root, json!({"op":"organize.status","workspace_id":w}))["run"]["state"] == "running" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(100));
    }
    fs::remove_file(root.join("fake-calls.jsonl")).unwrap();
}
fn long(text: &str) -> String {
    format!("{text} {}", "자세한 설명이 이어진다. ".repeat(40))
}
/// A finished exchange, written the way the chat stream leaves it.
fn ask(root: &Path, w: &str, question: &str, answer: &str) -> String {
    let conversation = call(root, json!({"op":"chat.get","workspace_id":w,"provider":"codex"}))["id"].as_str().unwrap().to_owned();
    let id = uuid::Uuid::new_v4().to_string();
    let c = rusqlite::Connection::open(root.join("chat.sqlite3")).unwrap();
    c.busy_timeout(Duration::from_secs(5)).unwrap();
    c.execute(
        "INSERT INTO turns(id,conversation,run,user,answer,state,created) VALUES(?1,?2,'run',?3,?4,'completed','2026-11-17T00:00:00Z')",
        rusqlite::params![id, conversation, question, answer],
    )
    .unwrap();
    id
}
fn scan(root: &Path, w: &str) -> Value {
    call(root, json!({"op":"distill.scan","workspace_id":w,"provider":"codex"}))
}
fn status(root: &Path, w: &str) -> Value {
    call(root, json!({"op":"distill.status","workspace_id":w}))
}
/// Wait until no batch is waiting or running; each status call lets the next one start.
fn settle(root: &Path, w: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let s = status(root, w);
        if s["working"] == 0 {
            return s;
        }
        assert!(Instant::now() < deadline, "{s}");
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn calls(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("fake-calls.jsonl")).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}
fn mode(root: &Path, mode: &str) {
    call(root, json!({"op":"ai.settings.set","distill_mode":mode}));
}
fn lines(s: &Value, state: &str) -> Vec<Value> {
    s["suggestions"].as_array().unwrap().iter().filter(|x| x["state"] == state).cloned().collect()
}

#[test]
fn the_apps_records_decide_what_is_offered_and_no_model_is_called() {
    let (dir, w) = fixture();
    let root = dir.path();
    organise(root, &w);
    ask(root, &w, "안녕?", "안녕하세요!");
    ask(root, &w, "오늘 점심 뭐 먹지", &long("취향에 따라 다릅니다."));
    let s = scan(root, &w);
    assert_eq!(s["suggestions"], json!([]), "a short answer and an unrelated request offer nothing");
    assert_eq!(s["consent_needed"], false);

    // A subject concept in the question is a weak signal: one line, nothing more.
    ask(root, &w, "스택이 뭐야?", &long("스택은 나중에 넣은 것이 먼저 나오는 구조입니다."));
    let s = scan(root, &w);
    assert_eq!(lines(&s, "candidate").len(), 1);
    assert_eq!(s["suggestions"][0]["signal"], "concept");
    assert_eq!(s["consent_needed"], false, "a weak signal never asks");
    assert_eq!(s["unseen"], true);

    // Asking the same thing again is a strong signal: the same line, now counted twice.
    ask(root, &w, "스택이 뭐야??", &long("스택은 후입선출 구조입니다."));
    let s = scan(root, &w);
    assert_eq!(lines(&s, "candidate").len(), 1, "{s}");
    assert_eq!(s["suggestions"][0]["signal"], "repeated");
    assert_eq!(s["suggestions"][0]["repeats"], 2);
    assert_eq!(s["suggestions"][0]["questions"].as_array().unwrap().len(), 2);
    assert_eq!(s["consent_needed"], true, "the first strong signal asks once");
    assert!(calls(root).is_empty(), "nothing was sent to a model");

    // Discarded: the kept copy goes, and the same asks are not offered again.
    let id = s["suggestions"][0]["id"].as_str().unwrap().to_owned();
    call(root, json!({"op":"distill.dismiss","id":id}));
    assert_eq!(scan(root, &w)["suggestions"], json!([]));
    let c = rusqlite::Connection::open(root.join("chat.sqlite3")).unwrap();
    let kept: String = c.query_row("SELECT snapshot FROM suggestions WHERE id=?1", [&id], |r| r.get(0)).unwrap();
    assert_eq!(kept, "[]");
    // A new ask is new content.
    ask(root, &w, "스택이 뭐야", &long("스택은 한쪽 끝에서만 넣고 빼는 구조입니다."));
    assert_eq!(lines(&scan(root, &w), "candidate").len(), 1);

    // Turned off: nothing is offered, except what the user marked.
    mode(root, "off");
    ask(root, &w, "자료구조는 왜 배워?", &long("자료구조는 데이터를 다루는 방법입니다."));
    let marked = ask(root, &w, "큐는 언제 써?", "짧은 답");
    assert_eq!(lines(&scan(root, &w), "candidate").len(), 1);
    call(root, json!({"op":"distill.mark","turn_id":marked}));
    let s = scan(root, &w);
    assert_eq!(lines(&s, "candidate").len(), 2);
    assert!(s["suggestions"].as_array().unwrap().iter().any(|x| x["signal"] == "marked"));
    assert!(calls(root).is_empty());
}

#[test]
fn agreeing_once_drafts_in_the_back_lane_and_registering_makes_a_note() {
    let (dir, w) = fixture();
    let root = dir.path();
    organise(root, &w);
    ask(root, &w, "스택이 뭐야?", &long("스택은 나중에 넣은 것이 먼저 나오는 구조입니다."));
    ask(root, &w, "스택이 뭐야?", &long("스택은 후입선출 구조입니다."));
    assert_eq!(scan(root, &w)["consent_needed"], true);
    call(root, json!({"op":"distill.consent","workspace_id":w,"provider":"codex","choice":"once"}));
    let s = settle(root, &w);
    assert_eq!(s["mode"], "ask", "agreeing once does not turn automatic drafting on");
    assert_eq!(s["consent_needed"], false);
    assert_eq!(lines(&s, "candidate").len(), 0, "the drafts replace the line: {s}");
    let drafts = lines(&s, "draft");
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0]["automatic"], true);
    assert!(drafts.iter().any(|d| d["signal"] == "repeated" && d["repeats"] == 2), "a draft says why it was made: {drafts:?}");
    assert_eq!(drafts[0]["model"], "gpt-6.1-terra");
    assert_eq!(drafts[0]["concepts"], json!(["스택"]), "only the subject's own concept names are kept");
    let made = calls(root);
    assert_eq!(made.len(), 1, "both exchanges went in one call");
    assert_eq!((made[0]["skill"].as_str(), made[0]["model"].as_str(), made[0]["turns"].as_i64()), (Some("distill-chat"), Some("gpt-6.1-terra"), Some(2)));
    let usage = call(root, json!({"op":"ai.settings.get"}))["usage"].clone();
    assert_eq!(usage["background_today"], 1);

    // Edit, then register: a note in the subject, marked with where it came from.
    let id = drafts[0]["id"].as_str().unwrap();
    call(root, json!({"op":"distill.update","id":id,"title":"스택: 후입선출","body":"큐는 자료구조의 한 종류이다."}));
    let document = call(root, json!({"op":"distill.register","id":id}))["document"].clone();
    assert_eq!(document["path"], "학습도우미 정리/스택 후입선출.md");
    assert_eq!(document["origin_kind"], "distilled");
    assert_eq!(document["body"], "큐는 자료구조의 한 종류이다.");
    assert_eq!(document["analysis_state"], "unanalysed", "a registered note waits for the next 과목 정리 like any material");
    // The same title again gets its own file.
    let other = lines(&status(root, &w), "draft")[0]["id"].as_str().unwrap().to_owned();
    call(root, json!({"op":"distill.update","id":other,"title":"스택: 후입선출","body":"다른 내용"}));
    let second = call(root, json!({"op":"distill.register","id":other}))["document"].clone();
    assert_ne!(second["path"], document["path"]);
    assert_eq!(status(root, &w)["suggestions"], json!([]));
    assert_eq!(request(root, json!({"op":"distill.register","id":id}))["ok"], false, "a registered suggestion is not registered twice");

    // The next 과목 정리 reads it like any material, and the map says it came from the user's questions.
    call(root, json!({"op":"organize.start","workspace_id":w}));
    let deadline = Instant::now() + Duration::from_secs(60);
    while call(root, json!({"op":"organize.status","workspace_id":w}))["run"]["state"] == "running" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(100));
    }
    let graph = call(root, json!({"op":"graph.get","workspace_id":w}));
    let sources = graph["sources"].as_array().unwrap();
    assert!(sources.iter().any(|s| s["material_id"] == document["id"] && s["from_chat"] == true), "{sources:?}");
    assert!(sources.iter().any(|s| s["title"] == "1주차" && s["from_chat"] == false));
    assert!(graph["concepts"].as_array().unwrap().iter().any(|c| c["name"] == "큐"));

}

#[test]
fn a_run_of_questions_on_one_concept_is_strong_until_drafts_go_unused() {
    let (dir, w) = fixture();
    let root = dir.path();
    organise(root, &w);
    for question in ["자료구조는 왜 배워?", "자료구조 종류를 알려줘", "자료구조 공부 순서는 어떻게 잡아?"] {
        ask(root, &w, question, &long("답"));
    }
    let s = scan(root, &w);
    let topic: Vec<Value> = s["suggestions"].as_array().unwrap().iter().filter(|x| x["signal"] == "topic").cloned().collect();
    assert_eq!(topic.len(), 1, "{s}");
    assert_eq!(topic[0]["questions"].as_array().unwrap().len(), 3, "the whole run is kept together");
    assert_eq!(s["consent_needed"], true);

    // Ten automatic drafts, almost none registered: only repeats and marks stay automatic.
    mode(root, "auto");
    let c = rusqlite::Connection::open(root.join("chat.sqlite3")).unwrap();
    for n in 0..10 {
        c.execute(
            "INSERT INTO suggestions(id,workspace,provider,state,title,automatic,created,updated) VALUES(?1,?2,'codex',?3,'예전 초안',1,'2026-11-01T00:00:00Z','2026-11-01T00:00:00Z')",
            rusqlite::params![format!("old-{n}"), w, if n < 2 { "registered" } else { "dismissed" }],
        )
        .unwrap();
    }
    ask(root, &w, "자료구조 책을 추천해 줘", &long("답"));
    let marked = ask(root, &w, "전혀 다른 이야기인데 이건 남기고 싶어", &long("답"));
    call(root, json!({"op":"distill.mark","turn_id":marked}));
    let s = call(root, json!({"op":"distill.flush","workspace_id":w,"provider":"codex"}));
    assert!(s["notice"].as_str().unwrap().contains("반복해 물은 질문과 표시한 답만"), "{s}");
    let s = settle(root, &w);
    assert_eq!(lines(&s, "draft").len(), 1, "only the marked answer was drafted: {s}");
    assert_eq!(calls(root).last().unwrap()["turns"], 1);
    call(root, json!({"op":"distill.seen","workspace_id":w}));
    assert!(status(root, &w)["notice"].is_null(), "told once");
}

#[test]
fn automatic_targets_share_one_call_three_notes_and_outlive_a_new_conversation() {
    let (dir, w) = fixture();
    let root = dir.path();
    mode(root, "auto");
    for n in 1..=4 {
        let id = ask(root, &w, &format!("{n}번째로 궁금한 전혀 다른 주제 {}", "가나다라".repeat(n)), &long("답"));
        call(root, json!({"op":"distill.mark","turn_id":id}));
    }
    // `새 대화` erases the conversation; the marked exchanges are carried over first.
    let fresh = call(root, json!({"op":"chat.new","workspace_id":w,"provider":"codex"}));
    assert_eq!(fresh["turns"], json!([]));
    let s = settle(root, &w);
    let drafts = lines(&s, "draft");
    assert_eq!(drafts.len(), 3, "at most three notes from one call: {s}");
    assert!(drafts.iter().all(|d| d["automatic"] == true && d["questions"].as_array().unwrap().len() == 1));
    let made = calls(root);
    assert_eq!(made.len(), 1);
    assert_eq!(made[0]["turns"], 4);

    // The daily amount for the back lane holds further batches until tomorrow.
    call(root, json!({"op":"ai.settings.set","background":"low"}));
    let today = chrono::Local::now().to_rfc3339();
    let mut log = fs::OpenOptions::new().append(true).open(root.join("ai/usage.jsonl")).unwrap();
    for _ in 0..4 {
        writeln!(log, "{}", json!({"at":today,"lane":"background","skill":"distill-chat","provider":"codex","model":"m","run":"r"})).unwrap();
    }
    let id = ask(root, &w, "한도 뒤에 물은 질문", &long("답"));
    call(root, json!({"op":"distill.mark","turn_id":id}));
    let s = call(root, json!({"op":"distill.flush","workspace_id":w,"provider":"codex"}));
    assert_eq!(s["working"], 1);
    assert!(s["notice"].as_str().unwrap().contains("내일"), "{s}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(calls(root).len(), 1, "no call beyond the daily amount");
}

#[test]
fn a_limit_or_an_empty_judgement_loses_nothing() {
    let (dir, w) = fixture();
    let root = dir.path();
    mode(root, "ask");
    let turn = ask(root, &w, "LIMIT 질문", &long("답"));
    call(root, json!({"op":"distill.now","turn_id":turn}));
    let s = settle(root, &w);
    assert!(s["error"].as_str().unwrap().contains("한도"), "{s}");
    assert_eq!(lines(&s, "draft").len(), 0);
    let chat = call(root, json!({"op":"chat.get","workspace_id":w,"provider":"codex"}));
    assert_eq!(chat["turns"].as_array().unwrap().len(), 1, "the conversation stays");

    // The model may judge there is nothing to keep: the line leaves the box.
    let turn = ask(root, &w, "잡담 하나", &long("답"));
    call(root, json!({"op":"distill.mark","turn_id":turn}));
    let s = scan(root, &w);
    let line = lines(&s, "candidate")[0]["id"].as_str().unwrap().to_owned();
    call(root, json!({"op":"distill.draft","id":line}));
    let s = settle(root, &w);
    assert_eq!(s["suggestions"], json!([]), "{s}");
}

#[test]
fn keeping_an_exchange_as_it_is_calls_no_model() {
    let (dir, w) = fixture();
    let root = dir.path();
    let turn = ask(root, &w, "JWT는 어떻게 검증해?/서명", "서명을 확인합니다.");
    let document = call(root, json!({"op":"distill.keep","turn_id":turn}))["document"].clone();
    assert_eq!(document["path"], "학습도우미 정리/JWT는 어떻게 검증해? 서명.md");
    assert_eq!(document["origin_kind"], "verbatim");
    let body = document["body"].as_str().unwrap();
    assert!(body.contains("## 질문") && body.contains("JWT는 어떻게 검증해?/서명") && body.contains("서명을 확인합니다."));
    assert!(calls(root).is_empty());
    assert_eq!(status(root, &w)["suggestions"], json!([]));
}

#[test]
fn a_chat_turn_goes_ahead_of_note_making() {
    let (dir, w) = fixture();
    let root = dir.path();
    mode(root, "ask");
    let turn = ask(root, &w, "SLOW 질문", &long("답"));
    call(root, json!({"op":"distill.now","turn_id":turn}));
    // Wait until the note-making call holds the lane.
    let deadline = Instant::now() + Duration::from_secs(30);
    while calls(root).is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(100));
    }
    let started = Instant::now();
    let mut p = Command::new(env!("CARGO_BIN_EXE_stology-core"))
        .args(["--root", root.to_str().unwrap(), "--chat-stream"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = p.stdin.take().unwrap();
    writeln!(input, "{}", json!({"workspace_id":w,"provider":"codex","model":"gpt-6.1-terra","effort":"medium","text":"지금 물어볼게"})).unwrap();
    input.flush().unwrap();
    let mut last = Value::Null;
    for line in BufReader::new(p.stdout.take().unwrap()).lines() {
        last = serde_json::from_str(&line.unwrap()).unwrap();
        if ["completed", "interrupted", "failed"].contains(&last["event"].as_str().unwrap()) {
            break;
        }
    }
    drop(input);
    p.wait().unwrap();
    assert_eq!(last["event"], "completed", "{last}");
    assert_eq!(last["answer"], "바로 답했어요");
    assert!(started.elapsed() < Duration::from_secs(20), "the answer did not wait for the note");
    // The note is made afterwards: the batch went back to waiting, not lost.
    fs::write(root.join("fake-ok"), "").unwrap();
    let s = settle(root, &w);
    assert_eq!(lines(&s, "draft").len(), 1, "{s}");
    assert_eq!(calls(root).len(), 2);
}
