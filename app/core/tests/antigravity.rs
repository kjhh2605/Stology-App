use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};
fn fixture() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("runtime/bin")).unwrap();
    let bin = dir.path().join("runtime/bin/agy");
    fs::write(&bin,r##"#!/usr/bin/env python3
import sys,json,os,pathlib
assert 'acp' in sys.argv and '--include-default-customizations=false' in sys.argv
assert '--dangerously-skip-permissions' not in sys.argv
assert not any(k in os.environ for k in ['GEMINI_API_KEY','GOOGLE_API_KEY','AGY_LLM_GATEWAY_API_KEY'])
home=pathlib.Path(os.environ['HOME'])
assert home.name=='antigravity-home'
agent=pathlib.Path('.agents/agents/stology.md').read_text()
assert 'excludeDefaultComponents: true' in agent and 'inheritMcp: false' in agent
settings=json.loads((home/'.gemini/antigravity-cli/settings.json').read_text())
assert settings['useG1Credits']==False and 'read_file(*)' in settings['permissions']['deny']
def send(v): print(json.dumps(dict(jsonrpc='2.0',**v)),flush=True)
def opts():return [{'id':'model','category':'model','currentValue':'gemini-4-carbon','options':[{'value':'gemini-4-carbon','name':'Gemini 4 Carbon'},{'value':'auto','name':'Automatic'}]}, {'id':'thinking','category':'thought_level','options':[{'value':'L','name':'Low'},{'value':'X','name':'Extra'}]}]
for line in sys.stdin:
 v=json.loads(line);m=v.get('method');i=v.get('id');p=v.get('params',{})
 if m=='initialize':r={'protocolVersion':1,'agentCapabilities':{'loadSession':True,'promptCapabilities':{'image':True}}}
 elif m=='authenticate':assert p['methodId']=='oauth-personal';r={}
 elif m in ['session/new','session/load']:
  assert p['mcpServers']==[]
  if p.get('sessionId')=='missing':send({'id':i,'error':{'message':'session not found'}});continue
  r={'sessionId':'agy-test-session','configOptions':opts()}
 elif m=='session/set_config_option':
  assert (p['configId']=='model' and p['value']=='gemini-4-carbon') or (p['configId']=='thinking' and p['value'] in ['L','X'])
  r={'configOptions':opts()}
 elif m=='session/prompt':
  blocks=p['prompt'];text=blocks[-1]['text'].split('\n',1)[1];pending=i
  assert p['sessionId']=='agy-test-session'
  if text=='image':assert blocks[0]=={'type':'image','mimeType':'image/png','data':'iVBORw0KGgo='}
  if text.startswith('# 자료 읽기 (extract-material)'):
   chunk=json.dumps({'concepts':[{'id':'c1','name':'물','aliases':[],'definition':'액체의 한 종류','type':'concept'},{'id':'c2','name':'액체','aliases':[],'definition':'물질의 상태','type':'concept'}],'relations':[{'id':'r1','label':'종류','definition':'한 종류이다','directed':True,'aliases':[],'type':'kind_of'}],'assertions':[{'subject':'c1','relation':'r1','object':'c2','condition':'','negated':False,'evidence':[{'locator':{'kind':'paragraph','index':1},'quote':'물은 액체의 한 종류이다.','image_path':None,'region':None}]}]})
  else:chunk='hello from Antigravity'
  send({'method':'session/update','params':{'sessionId':'other-session','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'wrong conversation'}}}})
  send({'method':'session/update','params':{'sessionId':p['sessionId'],'update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':chunk}}}})
  send({'id':'permission','method':'session/request_permission','params':{'sessionId':p['sessionId'],'options':[{'optionId':'allow','kind':'allow_always'}]}})
  wait_cancel=text=='cancel'
  continue
 elif i=='permission':
  assert v['result']['outcome']['outcome']=='cancelled'
  if not wait_cancel:send({'id':pending,'result':{'stopReason':'end_turn'}})
  continue
 elif m=='session/cancel':send({'id':pending,'result':{'stopReason':'cancelled'}});continue
 else:continue
 send({'id':i,'result':r})
"##).unwrap();
    fs::set_permissions(bin, fs::Permissions::from_mode(0o755)).unwrap();
    (dir, uuid::Uuid::new_v4().to_string())
}
fn run(dir: &std::path::Path, ws: &str, text: &str) -> Vec<Value> {
    let mut p = Command::new(env!("CARGO_BIN_EXE_stology-core"))
        .args(["--root", dir.to_str().unwrap(), "--chat-stream"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = p.stdin.take().unwrap();
    writeln!(input,"{}",json!({"workspace_id":ws,"provider":"antigravity","text":text,"model":"gemini-4-carbon","effort":"xhigh","images":if text=="image"{json!([{"mime":"image/png","data":"iVBORw0KGgo="}])}else{json!([])}})).unwrap();
    input.flush().unwrap();
    let mut events = Vec::new();
    for line in BufReader::new(p.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let event = v["event"].as_str().unwrap().to_owned();
        events.push(v);
        if event == "text_delta" && text == "cancel" {
            writeln!(input, "{{\"op\":\"stop\"}}").unwrap();
            input.flush().unwrap();
        }
        if ["completed", "failed", "interrupted"].contains(&event.as_str()) {
            break;
        }
    }
    drop(input);
    assert!(p.wait().unwrap().success());
    events
}
#[test]
fn catalog_preferences_images_resume_and_cancel() {
    let (d, ws) = fixture();
    let root = d.path();
    let catalog =
        stology_core::chat::handle(root, &json!({"op":"chat.models","provider":"antigravity"}))
            .unwrap();
    assert_eq!(catalog["models"].as_array().unwrap().len(), 1);
    assert_eq!(catalog["models"][0]["efforts"], json!(["low", "xhigh"]));
    stology_core::chat::handle(root,&json!({"op":"chat.preferences","provider":"antigravity","model":"gemini-4-carbon","effort":"xhigh"})).unwrap();
    assert_eq!(
        stology_core::chat::handle(root, &json!({"op":"chat.preferences"})).unwrap()["models"]
            ["antigravity"],
        "gemini-4-carbon"
    );
    for (text, state) in [
        ("image", "completed"),
        ("again", "completed"),
        ("cancel", "interrupted"),
    ] {
        let e = run(root, &ws, text);
        assert_eq!(e[0]["event"], "accepted", "{e:?}");
        assert_eq!(e.last().unwrap()["event"], state, "{e:?}");
        assert_eq!(e.last().unwrap()["answer"], "hello from Antigravity");
    }
    let data = stology_core::chat::handle(
        root,
        &json!({"op":"chat.get","workspace_id":ws,"provider":"antigravity"}),
    )
    .unwrap();
    assert_eq!(data["turns"].as_array().unwrap().len(), 3);
    assert_eq!(data["turns"][0]["images"], 1);
    let other = stology_core::chat::handle(
        root,
        &json!({"op":"chat.get","workspace_id":ws,"provider":"codex"}),
    )
    .unwrap();
    assert!(other["turns"].as_array().unwrap().is_empty());
    let db = rusqlite::Connection::open(root.join("chat.sqlite3")).unwrap();
    db.execute(
        "UPDATE conversations SET session='missing' WHERE provider='antigravity'",
        [],
    )
    .unwrap();
    assert_eq!(run(root, &ws, "missing").last().unwrap()["event"], "failed");
    let data = stology_core::chat::handle(
        root,
        &json!({"op":"chat.get","workspace_id":ws,"provider":"antigravity"}),
    )
    .unwrap();
    assert_eq!(data["needs_new"], true);
}
#[test]
fn one_material_analysis_uses_acp_and_publishes_the_checked_result() {
    let (d, _) = fixture();
    let root = d.path();
    let call = |request: Value| {
        let mut p = Command::new(env!("CARGO_BIN_EXE_stology-core"))
            .args(["--root", root.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        p.stdin
            .take()
            .unwrap()
            .write_all(request.to_string().as_bytes())
            .unwrap();
        let output = p.wait_with_output().unwrap();
        let v = serde_json::from_slice::<Value>(&output.stdout).unwrap();
        assert_eq!(v["ok"], true, "{v}");
        v["result"].clone()
    };
    let w = call(json!({"op":"workspace.create","name":"과목"}))["id"].clone();
    let draft = call(json!({"op":"draft.create","workspace_id":w,"body":"물은 액체의 한 종류이다."}));
    let note = call(json!({"op":"note.save","draft_id":draft["id"],"title":"자료"}));
    let start = call(json!({"op":"analysis.begin","provider":"antigravity","workspace_id":w,"material_id":note["id"],"expected_hash":note["hash"]}));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let v = call(json!({"op":"analysis.status","id":start["id"]}));
        if v["state"] == "succeeded" {
            break;
        }
        assert_ne!(v["state"], "failed", "{v}");
        assert!(std::time::Instant::now() < deadline, "{v}");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let run = call(json!({"op":"organize.status","workspace_id":w}))["run"].clone();
    assert_eq!(run["materials"][0]["chunks"][0]["attempts"][0]["model"], "gemini-4-carbon", "{run}");
    let graph = call(json!({"op":"graph.get","workspace_id":w}));
    assert_eq!(graph["assertions"][0]["evidence"][0]["quote"], "물은 액체의 한 종류이다.");
}
