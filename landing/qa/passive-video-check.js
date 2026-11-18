// Run with: aside repl --file landing/qa/passive-video-check.js
// This task opens its own tab. Playback checks observe real time; none seek the video.
await openTab('http://127.0.0.1:5174/');
await page.setViewportSize({width:1440,height:1000});
console.log((await snapshot(page,{selector:'#shortcuts'})).tree);
const results=[];
const check=(pass,label,details)=>results.push({label,pass:!!pass,...(details===undefined?{}:{details})});
const expected={shortcuts:['bold','italic','underline','strikethrough','inline-code','link','block-1','block-2','block-3','checklist','bullet','numbered','block-code','markdown-quote','selection','map-search-focus'],pdf:['highlight','pdf-area-highlight','pdf-box','text-note','pen','pdf-move','pdf-resize','pdf-area-select','pdf-note-edit','pdf-delete-many','pdf-share','pdf-pptx','pdf-analyze']};
for(const [group,ids] of Object.entries(expected)){
 const listed=await page.evaluate(group=>Array.from(document.querySelectorAll(`[data-demo="${group}"] [role="tab"]`),el=>el.dataset.item),group);
 check(JSON.stringify(listed)===JSON.stringify(ids),`${group}: selected catalog and order`);
 for(const id of ids){
  await page.locator(`#${group}-tab-${id}`).click();
  console.log((await snapshot(page,{selector:`#${group}-panel`})).diff);
  await page.locator(`[data-demo="${group}"] .recording-stage`).scrollIntoViewIfNeeded();
  console.log((await snapshot(page,{selector:`#${group}-panel`})).diff);
  try{await page.waitForFunction(({group,id})=>{const v=document.querySelector(`[data-demo="${group}"] video`);return v.dataset.item===id&&v.currentSrc.endsWith(`/${id}.mp4`)&&v.readyState>=2&&v.videoWidth>0},{group,id},{timeout:2500});}catch{}
  const state=await page.evaluate(group=>{const v=document.querySelector(`[data-demo="${group}"] video`);return {id:v.dataset.item,src:v.currentSrc,readyState:v.readyState,width:v.videoWidth,height:v.videoHeight,duration:v.duration,muted:v.muted,autoplay:v.autoplay,loop:v.loop,inline:v.playsInline,controls:v.controls,pip:v.disablePictureInPicture,remote:v.disableRemotePlayback,tabIndex:v.tabIndex,pointerEvents:getComputedStyle(v).pointerEvents,error:v.error?.code||null,trackCount:v.querySelectorAll('track').length};},group);
  check(state.id===id&&state.src.endsWith(`/${id}.mp4`)&&state.readyState>=2&&state.width>0&&!state.error,`${group}/${id}: selected recording decodes`,state);
  check(state.autoplay&&state.muted&&state.loop&&state.inline&&!state.controls&&state.pip&&state.remote&&state.tabIndex===-1&&state.pointerEvents==='none'&&state.trackCount===0,`${group}/${id}: passive video attributes`);
 }
}
check(await page.evaluate(()=>document.querySelectorAll('.motion-controls,.motion-scrubber,.motion-play,.motion-restart,.recording-player button,.recording-player input[type="range"],video[controls],track').length===0),'No playback controls, scrubber, or caption track in page');
await page.locator('#shortcuts-tab-block-1').click();
console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
await page.locator('[data-demo="shortcuts"] .recording-stage').scrollIntoViewIfNeeded();
console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
await page.waitForFunction(()=>document.querySelector('[data-demo="shortcuts"] video').currentTime>.2);
const progress=await page.evaluate(()=>new Promise(resolve=>{const v=document.querySelector('[data-demo="shortcuts"] video');const initial=v.currentTime;const started=performance.now();const read=()=>{if(v.currentTime>initial+.3){v.removeEventListener('timeupdate',read);resolve({initial,after:v.currentTime,elapsed:performance.now()-started,paused:v.paused});}};v.addEventListener('timeupdate',read);setTimeout(()=>{v.removeEventListener('timeupdate',read);resolve({initial,after:v.currentTime,elapsed:performance.now()-started,paused:v.paused});},3000);}));
check(!progress.paused&&progress.after>progress.initial,'Autoplay advances naturally',progress);
await page.locator('[data-demo="shortcuts"] .recording-stage').click();
console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
check(await page.evaluate(()=>!document.querySelector('[data-demo="shortcuts"] video').paused),'Clicking recording does not pause playback');
await page.evaluate(()=>{window.__passiveQaContextPrevented=null;document.addEventListener('contextmenu',event=>window.__passiveQaContextPrevented=event.defaultPrevented,{once:true});});
const contextBox=await page.locator('[data-demo="shortcuts"] .recording-stage').boundingBox();
await page.mouse.click(contextBox.x+contextBox.width/2,contextBox.y+contextBox.height/2,{button:'right'});
console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
check(await page.evaluate(()=>window.__passiveQaContextPrevented===true),'Native context menu is prevented on real right-click');
const loop=await page.evaluate(()=>new Promise(resolve=>{const v=document.querySelector('[data-demo="shortcuts"] video');let previous=v.currentTime;const started=performance.now();const samples=[previous];const read=()=>{const now=v.currentTime;if(samples.length<50)samples.push(now);if(now+.5<previous){v.removeEventListener('timeupdate',read);resolve({looped:true,duration:v.duration,elapsed:performance.now()-started,beforeWrap:previous,afterWrap:now,samples});}previous=now;};v.addEventListener('timeupdate',read);setTimeout(()=>{v.removeEventListener('timeupdate',read);resolve({looped:false,duration:v.duration,elapsed:performance.now()-started,samples});},15000);}));
check(loop.looped&&loop.beforeWrap>loop.duration-.7&&loop.afterWrap<.7,'Real recording loops naturally without seeking',loop);
for(const level of [1,2,3]){
 await page.locator(`#shortcuts-tab-block-${level}`).click();console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
 const keyText=await page.evaluate(level=>({list:document.querySelector(`#shortcuts-tab-block-${level}`).innerText,strip:document.querySelector('[data-demo="shortcuts"] .recording-inputs').innerText}),level);
 check(keyText.list.includes('#'.repeat(level))&&keyText.list.includes('Space')&&keyText.strip.includes('#'.repeat(level))&&keyText.strip.includes('Space'),`Heading ${level}: Markdown and Space visible in list and input strip`,keyText);
}
await page.locator('#shortcuts-tab-block-code').click();console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
const codeKeys=await page.evaluate(()=>({list:document.querySelector('#shortcuts-tab-block-code').innerText,strip:document.querySelector('[data-demo="shortcuts"] .recording-inputs').innerText,whiteSpace:getComputedStyle(document.querySelector('.recording-inputs .markdown-example')).whiteSpace}));
check(codeKeys.list.includes('```')&&codeKeys.strip.includes('```\n코드\n```')&&codeKeys.whiteSpace==='pre-wrap'&&codeKeys.strip.includes('Markdown 원문')&&!codeKeys.list.includes('```\n코드\n```'),'Code block: opening input in list and Markdown source under recording',codeKeys);
await page.locator('[data-demo="shortcuts"] .recording-stage').scrollIntoViewIfNeeded();console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'Desktop 1440px: no horizontal page overflow');
await page.waitForFunction(()=>{const v=document.querySelector('[data-demo="shortcuts"] video');return v.currentTime>v.duration*.65;});
console.log('CAPTURE_PASSIVE_DESKTOP:'+(await page.screenshot()).toString('base64'));
await page.setViewportSize({width:390,height:844});console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
await page.locator('[data-demo="shortcuts"] .recording-inputs').scrollIntoViewIfNeeded();console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
const mobile=await page.evaluate(()=>({width:innerWidth,scrollWidth:document.documentElement.scrollWidth,stripWidth:document.querySelector('[data-demo="shortcuts"] .recording-inputs').getBoundingClientRect().width,keys:Array.from(document.querySelectorAll('[data-demo="shortcuts"] .recording-inputs kbd'),el=>({text:el.innerText,left:el.getBoundingClientRect().left,right:el.getBoundingClientRect().right,top:el.getBoundingClientRect().top,bottom:el.getBoundingClientRect().bottom}))}));
check(mobile.scrollWidth<=mobile.width&&mobile.keys.every(key=>key.left>=0&&key.right<=mobile.width),'Mobile 390px: code input keys fit without horizontal overflow',mobile);
console.log('CAPTURE_PASSIVE_MOBILE_CODE:'+(await page.screenshot()).toString('base64'));
await page.locator('#shortcuts-tab-block-3').click();console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
await page.locator('[data-demo="shortcuts"] .recording-inputs').scrollIntoViewIfNeeded();console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
check(await page.evaluate(()=>document.querySelector('[data-demo="shortcuts"] .recording-inputs').innerText.includes('###')&&document.documentElement.scrollWidth<=innerWidth),'Mobile heading: ### + Space visible without overflow');
console.log('CAPTURE_PASSIVE_MOBILE_HEADING:'+(await page.screenshot()).toString('base64'));
const logs=await page.console.logs();
check(!logs.some(log=>log.type==='error'||log.level==='error'),'Browser console has no errors',logs);
console.log('PASSIVE_VIDEO_RESULTS:'+JSON.stringify({date:new Date().toISOString(),viewport:{desktop:1440,mobile:390},results,logs}));
console.log('PASSIVE_VIDEO_SUMMARY:'+JSON.stringify({passed:results.filter(r=>r.pass).length,total:results.length,failed:results.filter(r=>!r.pass).map(r=>r.label)}));
