await openTab('http://127.0.0.1:5174/');
await page.setViewportSize({width:1440,height:1000});
console.log((await snapshot(page,{selector:'#shortcuts'})).tree);
const results=[];const check=(ok,label)=>{results.push({label,pass:!!ok});if(!ok)throw Error(label);};
for(const [group,count] of [['shortcuts',16],['pdf',13]]){
 const actual=await page.evaluate(group=>[...document.querySelectorAll(`[data-demo="${group}"] .tab-number`)].map(el=>el.textContent),group);
 check(JSON.stringify(actual)===JSON.stringify(Array.from({length:count},(_,i)=>String(i+1).padStart(2,'0'))),`${group} continuous numbers 01–${count}`);
}
check(await page.evaluate(()=>!document.querySelector('[data-item^="arrow-"],[data-item="quote-exit"],[data-item="undo"],[data-item="redo"],[data-item="copy"],[data-item="paste"],[data-item="cut"],[data-item="select-all"],[data-item="quit"],[data-item="pdf-undo"],[data-item="pdf-redo"]')),'Basic and incidental commands absent');
await page.locator('#shortcuts-tab-map-search-focus').click();console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
check(await page.evaluate(()=>document.querySelector('[data-demo="shortcuts"] .caption-count').textContent==='16 / 16'),'Last note item shows 16 / 16');
await page.locator('#pdf-tab-pdf-analyze').click();console.log((await snapshot(page,{selector:'#pdf-panel'})).diff);
check(await page.evaluate(()=>document.querySelector('[data-demo="pdf"] .caption-count').textContent==='13 / 13'),'Last PDF item shows 13 / 13');
await page.locator('[data-demo="shortcuts"] input[type="search"]').fill('화살표');console.log((await snapshot(page,{selector:'#shortcuts'})).diff);
check(await page.locator('[data-demo="shortcuts"] [role="tab"]').count()===0,'Removed arrow category not searchable');
await page.locator('[data-demo="shortcuts"] .clear-filter').click();console.log((await snapshot(page,{selector:'#shortcuts-panel'})).diff);
check(await page.locator('[data-demo="shortcuts"] [role="tab"]').count()===16,'Search reset restores selected catalog only');
console.log('RESULTS:'+JSON.stringify(results));
console.log('CONSOLE:'+JSON.stringify(await page.console.logs()));
