#!/usr/bin/env python3
"""Real executable integration, separate from UI/live-provider acceptance."""
import argparse, json, pathlib, subprocess, tempfile, datetime
p=argparse.ArgumentParser();p.add_argument('--binary',required=True);p.add_argument('--output');a=p.parse_args()
binary=pathlib.Path(a.binary).resolve();fixtures=pathlib.Path(__file__).resolve().parents[1]/'qa/fixtures'
results=[]
with tempfile.TemporaryDirectory(prefix='stology-integration-') as temp:
 def call(op, fail=False, **kw):
  proc=subprocess.run([str(binary),'--root',temp,'--request'],input=json.dumps(dict(op=op,**kw)),text=True,capture_output=True)
  v=json.loads(proc.stdout)
  if fail: assert not v['ok'],v;return v
  assert v['ok'],v
  return v['result']
 def passed(name): results.append({'case':name,'result':'pass'})
 w=call('workspace.create',name='통합 검증 생물학')['id'];w2=call('workspace.create',name='다른 과목')['id']
 assert call('draft.create',workspace_id=w,body=' \n')['id'] is None
 body=fixtures.joinpath('biology.md').read_text()
 d=call('draft.create',workspace_id=w,body=body)
 assert call('draft.get',id=d['id'])['body']==body
 assert not call('draft.list',workspace_id=w2)['drafts'];passed('draft persistence / empty suppression / workspace separation')
 call('folder.create',workspace_id=w,parent='',name='세포')
 n=call('note.save',draft_id=d['id'],title='세포 노트',folder='세포')
 assert pathlib.Path(n['absolute_path']).read_text()==body
 assert not call('draft.list',workspace_id=w)['drafts']
 d2=call('draft.create',workspace_id=w,body='충돌 노트')
 conflict=call('note.save',draft_id=d2['id'],title='세포 노트',folder='세포')
 assert conflict['conflict'];assert call('draft.get',id=d2['id'])['body']=='충돌 노트';passed('first save / real UTF-8 md / collision preserves draft')
 assert not list(pathlib.Path(temp).joinpath('versions').glob('*'));passed('create/save/read do not retain unrequested versions')
 src=call('source.snapshot',id=n['id']);ex=call('extract',path=src['immutable_path'])
 unit=next(u for u in ex['units'] if '광합성(photosynthesis)' in u['text'])
 result={'concepts':[{'id':'a','name':'엽록체','aliases':['chloroplast'],'definition':'광합성이 일어나는 세포 소기관'},{'id':'b','name':'광합성','aliases':['photosynthesis'],'definition':'빛에너지를 화학 에너지로 바꾸는 과정'}], 'relations':[{'id':'r','label':'수행한다','aliases':[],'definition':'소기관에서 과정이 일어난다','directed':True}], 'assertions':[{'subject':'a','relation':'r','object':'b','condition':'','negated':False,'evidence':[{'locator':unit['locator'],'quote':'엽록체는 광합성이 일어나는 세포 소기관이다.','image_path':None,'region':None}]}]}
 apply=dict(workspace_id=w,material_id=n['id'],input_hash=ex['input_hash'],provider='fixture-test',model='deterministic-fixture',result=result)
 assert call('graph.apply',**apply)['applied'];assert call('graph.apply',**apply)['duplicate']
 g=call('graph.get',workspace_id=w);assert len(g['concepts'])==2 and len(g['assertions'])==1;passed('evidence graph commit / restart / dedup (fixture, not AI)')
 bad=json.loads(json.dumps(apply));bad['result']['assertions'][0]['evidence'][0]['quote']='없는 문장'
 call('graph.apply',fail=True,**bad);assert call('graph.get',workspace_id=w)==g;passed('fabricated quote rejection retains graph')
 updated=call('note.update',id=n['id'],body=body+'\n추가 문장',expected_version=n['version'])
 call('note.update',id=n['id'],body='late overwrite',expected_version=n['version'],fail=True)
 assert pathlib.Path(src['immutable_path']).read_text()==body
 assert call('graph.get',workspace_id=w)['sources'][0]['stale'];passed('optimistic save / immutable snapshot / stale evidence')
 for f in ['biology.pdf','diagram-scan.pdf','diagram.png','diagram.jpg','diagram.heic']:
  doc=call('material.import',workspace_id=w,path=str(fixtures/f),folder='')
  before=set(pathlib.Path(temp).joinpath('versions').glob('*'))
  current=call('source.get',id=doc['id'])
  call('note.read',id=doc['id'])
  assert set(pathlib.Path(temp).joinpath('versions').glob('*'))==before
  ext=call('extract',path=call('source.snapshot',id=doc['id'])['immutable_path'])
  assert ext['units'];assert ext['units'][0]['images'];passed(f+' real native extraction')
 deleted=call('note.delete',id=n['id']);assert pathlib.Path(deleted['trash_path']).exists()
 call('note.update',id=n['id'],body='resurrect',expected_version=updated['version'],fail=True)
 assert not call('graph.get',workspace_id=w)['assertions'];passed('trash / no late-save resurrection / evidence cleanup')
report={'at':datetime.datetime.now().astimezone().isoformat(),'binary':str(binary),'scope':'Executable integration with deterministic graph fixture; not GUI, IME, or live provider acceptance','results':results}
text=json.dumps(report,ensure_ascii=False,indent=2)
if a.output:pathlib.Path(a.output).write_text(text+'\n')
print(text)
