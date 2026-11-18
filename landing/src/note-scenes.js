import './note-scenes.css';
import { specialRecipe } from './note-special-scenes.js';

const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const line = (text, extra='') => `<p class="nm-line ${extra}">${escape(text)}</p>`;
const sentence='반복과 연결은 오래 기억하는 데 도움을 줍니다.';
const picked=text=>`<span class="nm-pick">${escape(text)}</span>`;
const documentBody=content=>`<div class="nm-doc-kicker">심리학의 이해 · 수업 노트</div><h2 class="nm-doc-title">기억은 어떻게 만들어질까</h2>${content}<div class="nm-context-lines"><i></i><i></i></div>`;
const formatTag={bold:'strong',italic:'em',underline:'u',strike:'s',strikethrough:'s',code:'code','inline-code':'code'};
const selectedLine=text=>`<p class="nm-line">${picked(text)}</p>`;
const list=(rows,type='bullet',indent=-1)=>`<div class="nm-list nm-list-${type}">${rows.map((t,i)=>`<div class="nm-list-row ${indent===i?'nm-indent':''}"><span class="nm-marker">${type==='number'?`${12+i}.`:type==='check'?'<i class="nm-checkbox"></i>':'•'}</span><span>${escape(t)}${t===''?'<i class="nm-caret"></i>':''}</span></div>`).join('')}</div>`;

function rich(text){
 const raw=String(text??'');
 const inline=s=>{const t=String(s);for(const [pattern,tag] of [[/^\*\*(.+)\*\*$/,'strong'],[/^__(.+)__$/,'strong'],[/^\*(.+)\*$/,'em'],[/^_(.+)_$/,'em'],[/^~~(.+)~~$/,'s'],[/^<u>(.+)<\/u>$/,'u'],[/^`(.+)`$/,'code']]){const m=t.match(pattern);if(m)return `<${tag} class="nm-formatted">${escape(m[1])}</${tag}>`;}const link=t.match(/^\[([^\]]+)\]\([^)]+\)$/);return link?`<span class="nm-link">${escape(link[1])} ↗</span>`:escape(t);};
 return raw.split('\n').map(row=>{
  const h=row.match(/^(#{1,6})\s+(.*)/);if(h)return `<h${h[1].length} class="nm-block-heading nm-h${h[1].length}">${inline(h[2])}</h${h[1].length}>`;
  const q=row.match(/^> ?(.*)/);if(q)return `<blockquote class="nm-quote">${inline(q[1])||'<i class="nm-caret"></i>'}</blockquote>`;
  const li=row.match(/^(\s*)([-*+]|\d+\.)\s+(?:\[([ xX])\] ?)?(.*)/);if(li){const marker=li[3]!==undefined?`<i class="nm-checkbox ${li[3].toLowerCase()==='x'?'nm-checked':''}">${li[3].toLowerCase()==='x'?'✓':''}</i>`:/\d/.test(li[2])?li[2]:'•';return `<div class="nm-list-row" style="margin-left:${li[1].length*14}px"><span class="nm-marker">${marker}</span><span>${inline(li[4])||'<i class="nm-caret"></i>'}</span></div>`;}
  return `<p class="nm-line">${inline(row)||'<i class="nm-caret"></i>'}</p>`;
 }).join('');
}

function basicRecipe(item){
 const scene=item.scene||{}, action=scene.action||item.id, id=item.id, kind=scene.kind||'', before=scene.before??sentence, after=scene.after??sentence;
 const selected=text=>`<p class="nm-line">${picked(text)}</p>`;
 if(id==='block-code'||id==='markdown-code'){const codeText=id==='markdown-code'?'memory = experience':String(before);return {mode:'code',before:documentBody(id==='markdown-code'?line(before,'nm-syntax'):selected(before)),after:documentBody(`<div class="nm-code"><div class="nm-codebar">${id==='markdown-code'?'Python':'일반 텍스트'}<span>⧉</span></div><pre class="nm-code-lines">${escape(codeText)}<i class="nm-caret"></i></pre></div>`),focus:{x:397,y:245}};}

 if(kind==='arrow')return {mode:'arrows',before:documentBody(`<p class="nm-line nm-large-line">${escape(before)}<i class="nm-caret"></i></p>`),after:documentBody(`<p class="nm-line nm-large-line nm-final-text">${escape(after)}</p>`),focus:{x:410,y:243}};
 if(kind==='new-note')return {mode:'new',before:documentBody(line(sentence)),after:'<div class="nm-doc-kicker">심리학의 이해 · 새 초안</div><h2 class="nm-doc-title">새 노트</h2><p class="nm-line nm-type-target">오늘 배운 것을 내 말로 정리해요.<i class="nm-caret"></i></p>',focus:{x:363,y:245}};
 if(kind==='quit')return {mode:'quit',before:documentBody(line(sentence)),after:documentBody(line(sentence)),focus:{x:655,y:84},render(stage,p){stage.querySelector('.nm-save-status').textContent=p<.4?'저장 중':'이 Mac에 저장됨';}};
 if(id==='selection'){
  const first='단기 기억은 정보를 잠시 붙잡습니다.',second='반복과 연결은 기억을 오래 남깁니다.';
  const firstWord='단기 <span class="nm-selection-all">기억</span>은 정보를 잠시 붙잡습니다.';
  return {mode:'expand',before:documentBody(line(first)+line(second)),after:documentBody(line(first)+line(second)),frames:[
   {at:0,html:documentBody(line(first)+line(second))},
   {at:.25,html:documentBody(`<p class="nm-line">${firstWord}</p>`+line(second))},
   {at:.43,html:documentBody(`<p class="nm-line"><span class="nm-selection-all">${escape(first)}</span></p>`+line(second))},
   {at:.61,html:documentBody(`<div class="nm-selection-all">${line(first)+line(second)}</div>`)}
  ],focus:{x:309,y:241}};
 }
 if(kind==='selection')return {mode:'expand',before:documentBody(`<div class="nm-select-paragraph">${rich(before)}</div>`),after:documentBody(`<div class="nm-selection-all">${rich(after)}</div>`),focus:{x:365,y:240}};
 if(kind==='clipboard'){
  const copied=id==='copy',cut=id==='cut';
  return {mode:'clipboard',before:documentBody(copied||cut?selected(before):rich(before)),after:documentBody(copied?rich(before):rich(after)),float:copied||cut?`<div class="nm-clipboard-sheet">${escape(cut?'단기 기억':before)}</div>`:'',focus:{x:397,y:242}};
 }
 if(kind==='history')return {mode:id,before:documentBody(rich(before)),after:documentBody(rich(after)),focus:{x:397,y:242}};
 if(kind==='format')return {mode:'format',before:documentBody(selected(before)),after:documentBody(rich(after)),focus:{x:417,y:242}};
 if(kind==='block'){
  return {mode:id==='block-0'?'paragraph':id==='checklist'?'check':id.startsWith('block-')?'heading':'list',before:documentBody(id==='block-0'?`<h2 class="nm-block-heading nm-h1">${escape(before)}</h2>`:selected(before)),after:documentBody(rich(after)),focus:{x:385,y:242}};
 }
 if(id==='empty-list-exit')return {mode:'list',before:documentBody(rich(before)),middle:documentBody(rich('1. 기억의 종류\n2. ')),after:documentBody(rich(after)),focus:{x:255,y:244}};
 if(kind==='list'){
  return {mode:id.includes('indent')?'indent':id.includes('outdent')?'outdent':id.includes('toggle')?'check':id.includes('quote')?'quote':'list',before:documentBody(rich(before)),after:documentBody(rich(after)),focus:{x:255,y:244}};
 }
 if(kind==='markdown')return {mode:'markdown',before:documentBody(`<p class="nm-line nm-syntax">${escape(before)}<i class="nm-caret"></i></p>`),after:documentBody(rich(after)),focus:{x:397,y:243}};
 if(kind==='link')return {mode:'link',before:documentBody(selected(before)),after:documentBody(rich(after)),dialog:'<div class="nm-dialog"><h3>링크 추가</h3><div class="nm-input">https://example.com</div><span class="nm-dialog-button">적용</span></div>',focus:{x:423,y:246}};
 return {mode:'text',before:documentBody(rich(before)),after:documentBody(rich(after)),focus:{x:410,y:242}};
}

export function createNoteScene(host,item){
 const recipe=specialRecipe(item)||basicRecipe(item);
 const stage=document.createElement('div');stage.className='nm-stage';stage.setAttribute('aria-hidden','true');
 stage.innerHTML=`<div class="nm-board nm-mode-${escape(recipe.mode||'text')}"><div class="nm-window"><div class="nm-window-bar"><span class="nm-window-dots"><i></i><i></i><i></i></span><span>Stology</span><span class="nm-window-status">심리학의 이해</span></div><div class="nm-window-body"><aside class="nm-side"><span class="nm-side-title">워크스페이스</span><div class="nm-subject nm-active">▤ &nbsp;심리학의 이해</div><div class="nm-subject">▤ &nbsp;환경과 사회</div><div class="nm-subject">▤ &nbsp;예술과 일상</div><div class="nm-side-bottom"><img src="/mark.svg" alt=""/> 내 Mac에 보관</div></aside><article class="nm-paper"><div class="nm-paper-toolbar"><span>기억과 학습</span><span class="nm-save-button">저장</span></div><div class="nm-contents"><div class="nm-state nm-before">${recipe.before}</div><div class="nm-state nm-after">${recipe.after}</div>${recipe.middle?`<div class="nm-state nm-middle">${recipe.middle}</div>`:""}${recipe.frames?recipe.frames.map((frame,i)=>`<div class="nm-state nm-step-state" data-step="${i}">${frame.html}</div>`).join(""):""}</div><div class="nm-save-status">이 Mac에 저장됨</div></article><aside class="nm-files"><span class="nm-side-title">자료 보관함</span><div class="nm-file-search">⌕ &nbsp;파일 검색</div><div class="nm-folder">⌄ &nbsp; 과목 루트</div><div class="nm-file">▧ 수업 안내</div><div class="nm-file nm-file-active">▧ 기억과 학습</div><div class="nm-drafts"><span>⌁ &nbsp; 작성 중</span><div class="nm-draft-item">▧ &nbsp;새 노트</div></div></aside></div></div><div class="nm-intermediate">${recipe.dialog||''}</div><div class="nm-float">${recipe.float||''}</div><div class="nm-pointer"><svg viewBox="0 0 24 30"><path d="M2 1v24l6-6 5 10 5-3-5-9h9Z" fill="#5141bd" stroke="#fff" stroke-width="1.6"/></svg><i></i></div></div>`;
 host.append(stage);const board=stage.querySelector('.nm-board'),before=stage.querySelector('.nm-before'),after=stage.querySelector('.nm-after'),cursor=stage.querySelector('.nm-pointer'),mid=stage.querySelector('.nm-intermediate'),floating=stage.querySelector('.nm-float');
 const resize=()=>{stage.classList.toggle("nm-compact",host.clientWidth<520);board.style.transform=`scale(${host.clientWidth/800})`;};const observer=new ResizeObserver(resize);observer.observe(host);resize();
 const clamp=x=>Math.min(1,Math.max(0,x)),ease=x=>1-Math.pow(1-clamp(x),3);let lastState='';
 function render(progress){
  const p=clamp(progress),transition=ease((p-.43)/.12),select=ease((p-.17)/.16),focus=recipe.focus||{x:410,y:245};
  stage.dataset.progress=p.toFixed(3);board.style.setProperty('--nm-phase',p);board.style.setProperty('--nm-select',select*(1-transition));
  before.style.opacity=String(1-transition);after.style.opacity=String(transition);after.style.transform=`translateY(${(1-transition)*8}px)`;
  const travel=ease((p-.08)/.2);cursor.style.transform=`translate(${640+(focus.x-640)*travel}px,${370+(focus.y-370)*travel}px) scale(${p>.37&&p<.43?.88:1})`;cursor.style.opacity=String(p<.08?0:p>.7?1-ease((p-.7)/.08):1);
  const state=p<.17?'before':p<.43?'action':'after';if(state!==lastState){board.dataset.state=state;lastState=state;}
  mid.style.opacity=recipe.dialog?String(ease((p-.22)/.06)*(1-ease((p-.56)/.07))):'0';mid.style.transform=`translateY(${(1-ease((p-.22)/.06))*10}px)`;
  floating.style.opacity=String(ease((p-.4)/.08)*(1-ease((p-.68)/.1)));floating.style.transform=`translate(${ease((p-.4)/.25)*45}px,${-ease((p-.4)/.25)*35}px)`;
  if(recipe.middle){const middle=stage.querySelector('.nm-middle');middle.style.opacity=String(ease((p-.25)/.06)*(1-ease((p-.56)/.07)));before.style.opacity=String(1-ease((p-.25)/.06));after.style.opacity=String(ease((p-.56)/.07));}
  if(recipe.mode==='dialog'){before.style.opacity=String(1-ease((p-.65)/.07));after.style.opacity=String(ease((p-.65)/.07));}
  if(recipe.mode==='link'){const v=ease((p-.63)/.08);before.style.opacity=String(1-v);after.style.opacity=String(v);}
  if(recipe.mode==='quit'){stage.querySelector('.nm-window').style.opacity=String(1-ease((p-.58)/.14));stage.querySelector('.nm-window').style.transform=`scale(${1-ease((p-.58)/.14)*.05})`;}
  if(recipe.mode==='new')stage.querySelector('.nm-draft-item').style.opacity=String(transition);
  if(recipe.frames){before.style.opacity='0';after.style.opacity='0';let chosen=0;recipe.frames.forEach((frame,i)=>{if(p>=frame.at)chosen=i;});stage.querySelectorAll('.nm-step-state').forEach((layer,i)=>layer.style.opacity=i===chosen?'1':'0');}
  if(recipe.render)recipe.render(stage,p);
 }
 render(0);
 return {render,destroy(){observer.disconnect();stage.remove();}};
}
