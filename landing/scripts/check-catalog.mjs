import assert from 'node:assert/strict';
import {readFileSync,existsSync,readdirSync,statSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {resolve,dirname,basename} from 'node:path';
import {fileURLToPath} from 'node:url';
import {shortcuts,pdfTools} from '../src/catalog.js';
const repo=fileURLToPath(new URL('../../',import.meta.url));
// The landing page intentionally presents a curated selection, not every row of the app guide.
const expectedShortcuts=['bold','italic','underline','strikethrough','inline-code','link','block-1','block-2','block-3','checklist','bullet','numbered','block-code','markdown-quote','selection','map-search-focus'];
const expectedPdf=['highlight','pdf-area-highlight','pdf-box','text-note','pen','pdf-move','pdf-resize','pdf-area-select','pdf-note-edit','pdf-delete-many','pdf-share','pdf-pptx','pdf-analyze'];
assert.deepEqual(shortcuts.map(item=>item.id),expectedShortcuts,'Curated note selection/order changed');
assert.deepEqual(pdfTools.map(item=>item.id),expectedPdf,'Curated PDF selection/order changed');
const allItems=[...shortcuts,...pdfTools];
assert.equal(new Set(allItems.map(item=>item.id)).size,allItems.length,'Duplicate entry ID');
const basicCommands=new Set(['⌘Z','⌘⇧Z','⌘C','⌘V','⌘X','⌘A','⌘S']);
for(const list of [shortcuts,pdfTools]){
 for(const item of list){
  assert.match(item.id,/^[a-z0-9-]+$/);assert.equal(item.scene.action,item.id);
  for(const key of ['label','title','body','category','context'])assert.ok(item[key],`${item.id}: ${key}`);
  assert.ok(!basicCommands.has((item.keys??[]).join('').toUpperCase()),`Basic command returned: ${item.id}`);
  for(const alternative of item.keyAlternatives??[]){
   assert.ok(Array.isArray(alternative)&&alternative.length&&alternative.every(key=>typeof key==='string'&&key.trim()),`Invalid alternative input: ${item.id}`);
   assert.ok(!basicCommands.has(alternative.join('').toUpperCase()),`Basic alternative command returned: ${item.id}`);
  }
  assert.ok(!/^(arrow-|enter-|tab-|quote-exit)/.test(item.id),`Supplementary input returned: ${item.id}`);
  assert.ok(existsSync(repo+item.source.path),`Missing source: ${item.source.path}`);
  const sourceLines=readFileSync(repo+item.source.path,'utf8').split('\n');
  assert.ok(Number.isInteger(item.source.line)&&item.source.line>0&&sourceLines[item.source.line-1]?.trim(),`Invalid source line: ${item.id}`);
 }
}
const coverage=JSON.parse(readFileSync(repo+'landing/qa/coverage.json','utf8'));
assert.equal(coverage.scope,'curated-core-features');
assert.equal(coverage.noteEntryCount,shortcuts.length);
assert.equal(coverage.pdfEntryCount,pdfTools.length);
for(const [key,list] of [['shortcuts',shortcuts],['pdfTools',pdfTools]]){
 assert.deepEqual(coverage[key],list.map(({id,label,source},index)=>({number:index+1,id,label,source})),`Coverage record is stale: ${key}`);
}
// Markdown alternatives belong to the same feature, not duplicate list entries.
for(const level of [1,2,3]){
 const heading=shortcuts.find(item=>item.id===`block-${level}`);
 assert.ok(heading.keyAlternatives?.some(keys=>keys.length===2&&keys[0]==='#'.repeat(level)&&keys[1]==='Space'),`Heading ${level} must show its # + Space input`);
}
const code=shortcuts.find(item=>item.id==='block-code');
assert.ok(code.keyAlternatives?.some(keys=>keys[0]==='```'&&keys.at(-1)==='Enter'), 'Code block must show the opening fence + Enter input');
assert.ok(code.keyAlternatives?.some(keys=>keys.some(key=>/^```\n[^]+\n```$/.test(key))), 'Code block must show both fences around code');
assert.deepEqual(shortcuts.find(item=>item.id==='markdown-quote').keys,['>','Space'],'Quote must show > + Space');

// Inspect only modules reachable from the live entrypoint; archived scene files are not runtime code.
const runtimeFiles=new Map();
function visitModule(path){
 if(runtimeFiles.has(path))return;
 const source=readFileSync(path,'utf8');runtimeFiles.set(path,source);
 const imports=source.matchAll(/(?:import\s+(?:[^;]*?\s+from\s+)?|export\s+[^;]*?\s+from\s+)["'](\.[^"']+)["']/g);
 for(const match of imports){
  const dependency=resolve(dirname(path),match[1]);
  if(dependency.endsWith('.js'))visitModule(dependency);
 }
}
visitModule(repo+'landing/src/main.js');
assert.ok([...runtimeFiles.keys()].some(path=>basename(path)==='video-player.js'),'Live page must load the recording player');
for(const [path,source] of runtimeFiles){
 assert.ok(!['motion-player.js','note-scenes.js','note-special-scenes.js','pdf-scenes.js'].includes(basename(path)),`Legacy feature motion is still loaded: ${basename(path)}`);
 assert.ok(!/<track\b/i.test(source),`Caption track markup remains: ${basename(path)}`);
 assert.ok(!/class=["'][^"']*\bmotion-(?:controls|scrubber|play|restart)\b/.test(source),`Playback controls remain: ${basename(path)}`);
 assert.ok(!/\.controls\s*=\s*true\b/.test(source),`Native video controls enabled: ${basename(path)}`);
}
const explorer=runtimeFiles.get(repo+'landing/src/feature-explorer.js');
const videoTag=explorer.match(/<video\b[^>]*>/i)?.[0];
assert.ok(videoTag,'Feature explorer must contain a real video element');
for(const attribute of ['autoplay','muted','loop','playsinline','disablepictureinpicture','disableremoteplayback']){
 assert.match(videoTag,new RegExp(`\\s${attribute}(?:\\s|=|>)`,'i'),`Video is missing ${attribute}`);
}
assert.ok(!/\scontrols(?:\s|=|>)/i.test(videoTag),'Video controls must be absent');
assert.ok(!/<input\b[^>]*type=["']range["']/i.test(explorer),'Video seeking controls must be absent');
assert.match(explorer,/keyAlternatives/,'Alternative inputs must be rendered');

const media=readdirSync(repo+'landing/public/media',{recursive:true});
assert.ok(!media.some(name=>/\.(vtt|srt)$/i.test(name)),'Caption assets remain');
const missing=[];
for(const item of allItems){
 for(const extension of ['mp4','jpg']){
  const path=repo+`landing/public/media/demos/${item.id}.${extension}`;
  if(!existsSync(path)||statSync(path).size===0)missing.push(`${item.id}.${extension}`);
 }
}
assert.deepEqual(missing,[],`Missing recording assets: ${missing.join(', ')}`);
const expectedVideos=new Set(allItems.map(item=>`demos/${item.id}.mp4`));
assert.deepEqual(media.filter(name=>name.endsWith('.mp4')&&!expectedVideos.has(name)),[],'Unreferenced MP4 recordings remain');
for(const item of allItems){
 const path=repo+`landing/public/media/demos/${item.id}.mp4`;
 const result=spawnSync('ffprobe',['-v','error','-show_streams','-show_format','-of','json',path],{encoding:'utf8'});
 assert.ok(!result.error,`ffprobe is required to verify recordings: ${result.error?.message}`);
 assert.equal(result.status,0,`Invalid recording ${item.id}: ${result.stderr}`);
 const probe=JSON.parse(result.stdout);
 assert.equal(probe.streams.length,1,`${item.id}: recording must have exactly one video stream and no audio/subtitles`);
 const video=probe.streams[0];
 assert.equal(video.codec_type,'video',`${item.id}: not a video stream`);
 assert.equal(video.codec_name,'h264',`${item.id}: MP4 must use H.264`);
 assert.equal(video.pix_fmt,'yuv420p',`${item.id}: use broadly supported yuv420p`);
 assert.ok(video.width>0&&video.height>0,`${item.id}: empty video dimensions`);
 assert.ok(Number(probe.format.duration)>0,`${item.id}: empty recording duration`);
}
console.log(`PASS: ${shortcuts.length} note entries and ${pdfTools.length} PDF entries, Markdown alternatives, ${allItems.length} H.264 recordings with posters, passive looping player, and no audio/caption tracks.`);
