import { createVideoPlayer } from './video-player.js';
import './feature-explorer.css';
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const number = value => String(value).padStart(2, '0');
const keyGroup = keys => `<span class="key-group">${keys.map(key => `<kbd${String(key).includes('\n') ? ' class="multiline-key"' : ''}>${escape(key)}</kbd>`).join('')}</span>`;
const inputMarkup = (item, placement = 'panel') => {
  const primary = item.keys ? keyGroup(item.keys) : `<span class="tool-tag">${escape(item.tag || '클릭')}</span>`;
  const alternatives = (item.keyAlternatives || []).map(keys => {
    const multiline = keys.some(key => String(key).includes('\n'));
    if (multiline) return placement === 'list' ? '' : `<span class="source-example"><span class="source-label">Markdown 원문</span><code class="markdown-example">${escape(keys.join(' '))}</code></span>`;
    return `<span class="key-alternative"><span class="key-or">또는</span>${keyGroup(keys)}</span>`;
  }).join('');
  return primary + alternatives;
};
const normalize = value => value.toLowerCase().replace(/command|cmd/g,'⌘').replace(/option|alt/g,'⌥').replace(/shift/g,'⇧').replace(/[\s+]/g,'');
export function createExplorerMarkup(id, items) {
  const first = items[0]; const categories = [...new Set(items.map(item => item.category))];
  return `<div class="feature-explorer demo-layout" data-demo="${id}">
    <div class="explorer-sidebar"><div class="explorer-filter"><label class="search-field"><span aria-hidden="true">⌕</span><input type="search" aria-label="${id==='pdf'?'PDF 기능':'단축키와 입력 규칙'} 검색" placeholder="기능이나 키로 찾기" autocomplete="off"><span class="search-count">${items.length}</span></label><label class="category-field"><span>분류</span><select aria-label="${id==='pdf'?'PDF 기능':'단축키'} 분류"><option value="">전체 ${items.length}</option>${categories.map(c=>`<option value="${escape(c)}">${escape(c)}</option>`).join('')}</select></label></div>
    <div class="explorer-list" role="tablist" aria-label="${id==='pdf'?'PDF 주요 기능':'주요 단축키와 서식'}" aria-orientation="vertical"></div><div class="explorer-empty" hidden><p>찾는 기능이 없어요.</p><button class="clear-filter">검색 초기화</button></div><p class="demo-hint">${id==='pdf'?'도구를 골라 동작을 살펴보세요.':'⌘ Command · ⌥ Option · ⇧ Shift'}<br>항목을 고르면 동작을 확인할 수 있어요.</p></div>
    <div class="demo-panel" id="${id}-panel" role="tabpanel" tabindex="0"><div class="recording-player"><div class="video-topbar"><span class="live-dot"></span><span class="video-context">${escape(first.context)}</span><span class="video-source">Stology 실제 화면</span></div><div class="recording-stage"><video class="feature-recording" autoplay muted loop playsinline disablepictureinpicture disableremoteplayback controlslist="nodownload nofullscreen noremoteplayback noplaybackrate" tabindex="-1" aria-hidden="true" preload="metadata"></video><p class="recording-error" hidden>시연 화면을 불러오지 못했어요.</p></div><div class="recording-inputs" aria-label="사용하는 키와 입력"></div></div>
    <div class="demo-caption" aria-live="polite"><div class="caption-icon" aria-hidden="true">↳</div><div><h3></h3><p></p><p class="demo-detail"></p></div><span class="caption-count"></span></div></div></div>`;
}
export function mountExplorer(id, items) {
  const root = document.querySelector(`[data-demo="${id}"]`), list = root.querySelector('.explorer-list');
  const search = root.querySelector('input[type="search"]'), category = root.querySelector('select');
  const panel = root.querySelector('.demo-panel'), empty = root.querySelector('.explorer-empty');
  let current = items[0], filtered = items, player;
  function updatePanel(item) {
    current = item;
    panel.setAttribute('aria-labelledby', `${id}-tab-${item.id}`);
    root.querySelector('.video-context').textContent = item.context;
    root.querySelector('.recording-inputs').innerHTML = inputMarkup(item);
    root.querySelector('.demo-caption h3').textContent = item.title;
    root.querySelector('.demo-caption p').textContent = item.body;
    const detail = root.querySelector('.demo-detail'); detail.textContent = item.note || ''; detail.hidden = !item.note;
    root.querySelector('.caption-count').textContent = `${number(items.indexOf(item)+1)} / ${number(items.length)}`;
    if(player) player.setItem(item);
    list.querySelectorAll('[role="tab"]').forEach(tab=>{const selected=tab.dataset.item===item.id;tab.setAttribute('aria-selected',String(selected));tab.tabIndex=selected?0:-1;});
  }
  function paintList() {
    const term = normalize(search.value);
    filtered = items.filter(item => (!category.value || item.category === category.value) && normalize([item.label,item.title,item.body,item.category,...(item.keys||[]),...(item.keyAlternatives||[]).flat(),item.tag||''].join(' ')).includes(term));
    root.querySelector('.search-count').textContent = filtered.length;
    empty.hidden = !!filtered.length; list.hidden = !filtered.length; panel.hidden = !filtered.length;
    list.innerHTML = filtered.map(item => `<button class="demo-tab" role="tab" id="${id}-tab-${item.id}" aria-controls="${id}-panel" aria-selected="${item===current}" tabindex="${item===current?0:-1}" data-item="${item.id}"><span class="tab-number">${number(items.indexOf(item)+1)}</span><span class="tab-label">${escape(item.label)}</span><span class="tab-keys">${inputMarkup(item,'list')}</span></button>`).join('');
    if(filtered.length) updatePanel(filtered.includes(current)?current:filtered[0]);
  }
  list.addEventListener('click', event=>{const tab=event.target.closest('[role="tab"]');if(tab)updatePanel(items.find(item=>item.id===tab.dataset.item));});
  list.addEventListener('keydown',event=>{
    const tab=event.target.closest('[role="tab"]');if(!tab)return;const index=filtered.findIndex(item=>item.id===tab.dataset.item);let next;
    if(['ArrowDown','ArrowRight'].includes(event.key))next=(index+1)%filtered.length;
    if(['ArrowUp','ArrowLeft'].includes(event.key))next=(index-1+filtered.length)%filtered.length;
    if(event.key==='Home')next=0;if(event.key==='End')next=filtered.length-1;
    if(next!==undefined){event.preventDefault();updatePanel(filtered[next]);const focus=list.querySelector(`[data-item="${filtered[next].id}"]`);focus.focus({preventScroll:true});focus.scrollIntoView({block:'nearest',behavior:'instant'});}
  });
  search.addEventListener('input',paintList);category.addEventListener('change',paintList);
  root.querySelector('.clear-filter').addEventListener('click',()=>{search.value='';category.value='';paintList();search.focus();});
  paintList();player=createVideoPlayer(root.querySelector('.recording-player'),current);
  return {destroy:()=>player.destroy()};
}
