await openTab('http://127.0.0.1:5174/');
await page.setViewportSize({width:1440,height:1000});
console.log((await snapshot(page,{interactive:true})).tree);
const results=[];
const assert=(ok,label)=>{results.push({label,pass:!!ok});if(!ok)throw new Error(label);};
for(const [group,ids] of [['shortcuts',['new-note','bold','selection','checklist']],['pdf',['highlight','pen','text-note']]]){
 for(let i=0;i<ids.length;i++){
  await page.locator(`#${group}-tab-${i}`).click();
  console.log((await snapshot(page,{interactive:true})).diff);
  await page.waitForFunction(({group,id})=>{const v=document.querySelector(`[data-demo="${group}"] video`);return v.readyState>=2&&v.currentSrc.endsWith(`/${id}.mp4`)},{group,id:ids[i]});
  const media=await page.evaluate(group=>{const v=document.querySelector(`[data-demo="${group}"] video`);return {duration:v.duration,width:v.videoWidth,height:v.videoHeight,error:v.error?.message||null,selected:document.querySelectorAll(`[data-demo="${group}"] [aria-selected="true"]`).length};},group);
  assert(media.width===1280&&media.height===800&&media.duration>1&&!media.error&&media.selected===1,`${group}/${ids[i]} video decode and tab state`);
 }
}
await page.locator('#shortcuts-tab-1').click();console.log((await snapshot(page,{interactive:true})).diff);
await page.locator('#shortcuts-tab-1').press('ArrowDown');console.log((await snapshot(page,{interactive:true})).diff);
assert(await page.locator('#shortcuts-tab-2').getAttribute('aria-selected')==='true','ArrowDown changes tab');
await page.locator('#shortcuts-tab-2').press('Home');console.log((await snapshot(page,{interactive:true})).diff);
assert(await page.locator('#shortcuts-tab-0').getAttribute('aria-selected')==='true','Home selects first tab');
await page.locator('#shortcuts-tab-0').press('End');console.log((await snapshot(page,{interactive:true})).diff);
assert(await page.locator('#shortcuts-tab-3').getAttribute('aria-selected')==='true','End selects final tab');
await page.locator('#shortcuts-tab-1').click();console.log((await snapshot(page,{interactive:true})).diff);
console.log('CAPTURE_DESKTOP_SHORTCUTS:'+(await page.screenshot()).toString('base64'));
await page.locator('#pdf-tab-1').click();console.log((await snapshot(page,{interactive:true})).diff);
console.log('CAPTURE_DESKTOP_PDF:'+(await page.screenshot()).toString('base64'));
await page.locator('.environment-link').click();console.log((await snapshot(page,{interactive:true})).diff);
assert(await page.locator('dialog').getAttribute('open')!==null,'support dialog opens');
await page.locator('.dialog-close').press('Escape');console.log((await snapshot(page,{interactive:true})).diff);
assert(await page.locator('dialog').getAttribute('open')===null,'Escape closes dialog');
await page.locator('.faq-list summary').first().click();console.log((await snapshot(page,{interactive:true})).diff);
assert(await page.locator('.faq-list details').first().getAttribute('open')!==null,'FAQ expands');
await page.setViewportSize({width:390,height:844});
await page.locator('header .brand').click();console.log((await snapshot(page,{interactive:true})).diff);
await page.evaluate(()=>window.scrollTo({top:0,behavior:'instant'}));
assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'390px no horizontal overflow');
console.log('CAPTURE_MOBILE_HERO:'+(await page.screenshot()).toString('base64'));
await page.locator('#shortcuts-tab-3').click();console.log((await snapshot(page,{interactive:true})).diff);
assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'390px shortcut no overflow');
console.log('CAPTURE_MOBILE_SHORTCUTS:'+(await page.screenshot()).toString('base64'));
await page.locator('#pdf-tab-2').click();console.log((await snapshot(page,{interactive:true})).diff);
console.log('CAPTURE_MOBILE_PDF:'+(await page.screenshot()).toString('base64'));
await page.setViewportSize({width:320,height:740});
assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'320px no horizontal overflow');
console.log('QA_RESULTS:'+JSON.stringify(results));
