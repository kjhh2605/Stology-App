import './pdf-scenes.css';

// Deterministic, seekable explanations of the native PDF tools. The caller owns
// time, repeat, pause and reduced-motion behaviour; this module never schedules.
const clamp=(v,a=0,b=1)=>Math.min(b,Math.max(a,v));
const ease=v=>{v=clamp(v);return 1-Math.pow(1-v,3);};
const between=(p,a,b)=>ease((p-a)/(b-a));
const escape=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const iconPaths=[
 'M7 3h7l4 4v14H7z M14 3v5h4 M10 12h5 M10 16h5',
 'M6 3l13 10-6 1-3 7-4-18z',
 'M4 8V4h4 M16 4h4v4 M20 16v4h-4 M8 20H4v-4',
 'M5 15l9-10 5 5-9 10H5v-5z M3 22h13 M11 8l5 5',
 'M3 4h18v16H3z M6 7h12v10H6z',
 'M3 4h18v16H3z',
 'M3 4h18v13H9l-5 4v-4H3z M7 8h10 M7 12h7',
 'M5 20l2-6L17 4l4 4-10 10-6 2z M14 7l4 4',
 'M9 4L3 9l6 5 M3 9h11a6 6 0 010 12h-5',
 'M15 4l6 5-6 5 M21 9H10a6 6 0 000 12h5',
 'M4 6h16 M8 6V3h8v3 M6 6l1 15h10l1-15 M10 9v8 M14 9v8'
];
const toolLabels=['읽기','선택','영역 선택','강조','영역','박스','메모','펜','실행 취소','다시 실행','삭제'];
const baseText='반복과 연결은 오래 기억하는 데 도움을 줍니다.';
const fullMemo='내 경험과 연결해 보기';
function modeFrom(item){
 const action=String(item.scene?.action||item.id||'read').replace(/^pdf-/,'');
 const aliases={highlight:'highlight','text-highlight':'highlight','highlight-text':'highlight',pen:'pen',ink:'pen','freehand':'pen','draw-ink':'pen','text-note':'memo',note:'memo','create-note':'memo',area:'area','area-highlight':'area','area-select':'marquee','note-fit':'note-fit','save-copy':'recovery-copy','reopen':'recovery-discard','highlight-area':'area',box:'box','draw-box':'box','select':'select','select-move':'move','move-selection':'move',selection:'move','resize-selection':'resize','select-area':'marquee','multi-select':'marquee','marquee-select':'marquee','edit-note':'edit-memo','note-edit':'edit-memo','double-click-note':'edit-memo','note-edit':'edit-memo','undo-redo':'undo','escape':'escape','deselect':'escape','clear-selection':'escape','zoom-pan':'zoom','pinch':'zoom','pinch-zoom':'zoom','scroll-page':'scroll','autosave':'save','save-flush':'save','save-shortcut':'save','share-pdf':'share','pptx-convert':'pptx','pptx-to-pdf':'pptx','convert-pptx':'pptx','pptx-new':'pptx-new','pptx-regenerate':'pptx-new','resolve-conflict':'conflict','recover-copy':'recovery-copy','conflict-copy':'recovery-copy','discard-reload':'recovery-discard','conflict-discard':'recovery-discard','read-only':'protected','protected-pdf':'protected','readonly':'protected','analysis':'analyze','explicit-analysis':'analyze','delete-key':'delete','delete-backspace':'delete','delete-selected':'delete','read-text':'read','text-selection':'read'};
 return aliases[action]||action;
}
export function createPdfScene(host,item){
 const mode=modeFrom(item);
 const uid=`pm-${Math.random().toString(36).slice(2,9)}`;
 host.classList.add('pm-scene-host');
 host.innerHTML=`<svg class="pm-scene" viewBox="0 0 800 500" role="img" aria-label="${escape(item.label)} 기능을 설명하는 모션그래픽">
 <defs><linearGradient id="${uid}-bg" x2="1" y2="1"><stop stop-color="#f4f1ff"/><stop offset="1" stop-color="#e6e3f3"/></linearGradient><filter id="${uid}-shadow" x="-30%" y="-30%" width="160%" height="170%"><feDropShadow dx="0" dy="8" stdDeviation="10" flood-color="#292042" flood-opacity=".11"/></filter><clipPath id="${uid}-clip"><rect x="18" y="112" width="764" height="366" rx="8"/></clipPath></defs>
 <rect width="800" height="500" rx="16" fill="url(#${uid}-bg)"/>
 <rect x="14" y="14" width="772" height="472" rx="15" fill="#ffffff" fill-opacity=".76" stroke="#fff"/>
 <g class="pm-head"><rect x="32" y="30" width="25" height="29" rx="5" fill="#eee9ff"/><path d="M40 36h7l3 4v13H40z M46 36v5h4" fill="none" stroke="#7765b5" stroke-width="1.5"/><text x="69" y="43" font-size="15" font-weight="700" fill="#282433">심리학의 이해</text><text x="69" y="60" font-size="9" fill="#8c8496">기억과 학습.pdf</text><rect class="pm-save-button" x="696" y="29" width="66" height="29" rx="6" fill="#6450df"/><text x="729" y="48" text-anchor="middle" font-size="11" fill="#fff">저장</text></g>
 <g class="pm-toolbar">${iconPaths.map((d,i)=>`<g data-pm-tool="${i}" transform="translate(${32+i*35} 77)"><rect class="pm-tool-bg" width="30" height="25" rx="5" fill="#f0eef3"/><svg x="6" y="4" width="18" height="18" viewBox="0 0 24 24"><path d="${d}" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg></g>`).join('')}<circle class="pm-save-dot" cx="662" cy="90" r="3" fill="#9b91aa"/><text class="pm-status" x="674" y="94" font-size="9" fill="#8b8198">PDF에 저장됨</text></g>
 <g clip-path="url(#${uid}-clip)"><rect x="18" y="112" width="764" height="366" rx="8" fill="#eae8ee"/>
 <g class="pm-paper" filter="url(#${uid}-shadow)"><rect x="128" y="124" width="548" height="595" rx="1" fill="#fff"/>
 <text x="168" y="157" font-size="10" font-weight="700" fill="#8b77bd">심리학의 이해 · 수업 자료 03</text><text x="168" y="191" font-size="23" font-weight="700" fill="#302c37">기억은 어떻게 만들어질까</text><path d="M168 210H635" stroke="#e8e5ed"/>
 <text x="168" y="240" font-size="14" font-weight="700" fill="#38323f">오늘의 핵심</text>
 <rect class="pm-text-selection" x="168" y="280" width="0" height="21" rx="2" fill="#c5b9ff" opacity="0"/>
 <rect class="pm-highlight" x="168" y="280" width="0" height="21" rx="1" fill="#ffdc54" opacity="0"/>
 <text x="168" y="269" font-size="13" fill="#55505d">단기 기억은 잠시 정보를 붙잡아 둡니다.</text>
 <text class="pm-highlight-target" x="168" y="296" font-size="13" fill="#55505d">${baseText}</text>
 <text x="168" y="345" font-size="14" font-weight="700" fill="#38323f">생활 속에서 떠올려 보기</text>
 <text x="168" y="374" font-size="12" fill="#625b6a">01　친구의 이름을 여러 번 불러 보기</text>
 <text x="168" y="402" font-size="12" fill="#625b6a">02　배운 내용을 내 경험과 연결하기</text>
 <text x="168" y="430" font-size="12" fill="#625b6a">03　다음 날 책을 덮고 다시 떠올리기</text>
 <text x="168" y="490" font-size="14" font-weight="700" fill="#38323f">나의 한 줄 정리</text>
 <path d="M168 521H631 M168 551H631 M168 581H631" stroke="#eeebf2" stroke-width="1"/>
 <g class="pm-area-group" opacity="0"><rect class="pm-area" x="160" y="251" width="0" height="0" rx="2" fill="#ffd344" fill-opacity=".32"/></g>
 <g class="pm-box-group" opacity="0"><rect class="pm-box" x="160" y="251" width="0" height="0" rx="1" fill="none" stroke="#2157e0" stroke-width="1.8"/></g>
 <g class="pm-ink-group" opacity="0"><path class="pm-ink" d="M194 382C237 378 287 386 330 381S396 383 420 378" fill="none" stroke="#2157e0" stroke-width="2.6" stroke-linecap="round" stroke-dasharray="230" stroke-dashoffset="230"/></g>
 <g class="pm-note" opacity="0"><rect class="pm-note-editor" x="448" y="332" width="177" height="65" rx="2" fill="#fffbd6" stroke="#b8c7ec"/><text class="pm-note-text" x="455" y="352" font-size="11.5" fill="#2157e0"></text><rect class="pm-note-caret" x="455" y="338" width="1.1" height="16" fill="#2157e0"/></g>
 <g class="pm-handles" opacity="0"><rect class="pm-selection-box" x="160" y="251" width="330" height="57" fill="#2157e0" fill-opacity=".05" stroke="#2157e0" stroke-width="1"/><rect class="pm-resize-handle" x="486" y="304" width="7" height="7" fill="#fff" stroke="#2157e0" stroke-width="1.3"/></g>
 <rect class="pm-marquee" x="152" y="246" width="0" height="0" fill="#2157e0" fill-opacity=".06" stroke="#2157e0" stroke-width="1" stroke-dasharray="4 3" opacity="0"/>
 <rect class="pm-multi-second" x="189" y="373" width="236" height="16" fill="none" stroke="#2157e0" opacity="0"/>
 </g><rect x="773" y="120" width="4" height="344" rx="2" fill="#e0dce6"/><rect class="pm-scrollbar" x="773" y="121" width="4" height="175" rx="2" fill="#b1aaba"/></g>
 <g class="pm-file-view" opacity="0"><rect x="45" y="121" width="710" height="346" rx="10" fill="#f6f3fb"/><text x="72" y="154" font-size="12" fill="#80728e">자료 보관함 / 심리학의 이해</text><g class="pm-pptx-file"><rect x="72" y="179" width="657" height="55" rx="7" fill="#fff"/><rect x="88" y="192" width="23" height="28" rx="4" fill="#fae4da"/><text x="99" y="211" text-anchor="middle" font-size="10" fill="#bc6e47">P</text><text x="126" y="212" font-size="14" fill="#4c425a">기억과 학습.pptx</text></g><g class="pm-existing-file" opacity="0"><rect x="72" y="245" width="657" height="55" rx="7" fill="#fff"/><rect x="88" y="258" width="23" height="28" rx="4" fill="#f5dce8"/><text x="99" y="277" text-anchor="middle" font-size="8" fill="#ba6484">PDF</text><text x="126" y="278" font-size="14" fill="#65529a">기억과 학습.pdf</text></g><g class="pm-created-file" opacity="0"><rect x="72" y="245" width="657" height="55" rx="7" fill="#e8e0ff"/><rect x="88" y="258" width="23" height="28" rx="4" fill="#f5dce8"/><text x="99" y="277" text-anchor="middle" font-size="8" fill="#ba6484">PDF</text><text class="pm-created-name" x="126" y="278" font-size="14" fill="#65529a">기억과 학습.pdf</text></g><g class="pm-file-menu"><rect x="464" y="215" width="226" height="82" rx="8" fill="#fff" stroke="#ded5e8" filter="url(#${uid}-shadow)"/><rect class="pm-file-menu-active" x="470" y="221" width="214" height="31" rx="4" fill="#ede7ff"/><text x="487" y="242" font-size="12" fill="#574477">PDF로 필기</text><text x="487" y="278" font-size="12" fill="#574477">새 PDF로 변환</text></g></g>
 <g class="pm-modal" opacity="0"><rect x="18" y="112" width="764" height="366" rx="8" fill="#252032" fill-opacity=".18"/><g class="pm-modal-card"><rect x="229" y="172" width="342" height="218" rx="14" fill="#faf8fe" stroke="#e4ddec" filter="url(#${uid}-shadow)"/><rect x="255" y="196" width="31" height="35" rx="6" fill="#e8dfff"/><path d="M265 204h9l5 5v14h-14z" fill="none" stroke="#9a85bc" stroke-width="1.5"/><text class="pm-modal-title" x="300" y="219" font-size="16" font-weight="700" fill="#45354f">PDF 공유</text><text class="pm-modal-line1" x="255" y="260" font-size="12" fill="#776582">기억과 학습.pdf</text><text class="pm-modal-line2" x="255" y="282" font-size="11" fill="#8d7f97">최신 필기가 반영된 PDF</text><rect class="pm-modal-choice" x="255" y="310" width="290" height="34" rx="6" fill="#ede7f8"/><text class="pm-modal-button" x="400" y="332" text-anchor="middle" font-size="11" fill="#675580">AirDrop · 메모</text><text class="pm-modal-secondary" x="400" y="370" text-anchor="middle" font-size="10" fill="#92839c"></text></g></g>
 <g class="pm-cursor" transform="translate(680 425)"><circle class="pm-click" r="15" fill="#8b6eea" fill-opacity=".17" opacity="0"/><path d="M0 0l0 20 5-5 5 11 5-2-5-11 8-1z" fill="#292332" stroke="#fff" stroke-width="1.6" stroke-linejoin="round"/></g>
 </svg>`;
 const $=s=>host.querySelector(s), nodes={};
 for(const name of ['paper','highlight','text-selection','area-group','area','box-group','box','ink-group','ink','note','note-editor','note-text','note-caret','handles','selection-box','resize-handle','marquee','multi-second','cursor','click','status','save-dot','scrollbar','file-view','file-menu','file-menu-active','created-file','existing-file','created-name','modal','modal-card','modal-title','modal-line1','modal-line2','modal-choice','modal-button','modal-secondary'])nodes[name]=$(`.pm-${name}`);
 const svg=host.querySelector('.pm-scene');
 let currentProgress=0,disposed=false,compact=host.getBoundingClientRect().width<520;
 const toolNodes=[...host.querySelectorAll('[data-pm-tool]')];
 let lineWidth=Math.max(1,host.querySelector('.pm-highlight-target').getComputedTextLength())||268;
 if(lineWidth<20)lineWidth=268;
 document.fonts?.ready.then(()=>{if(host.isConnected){const measured=host.querySelector('.pm-highlight-target')?.getComputedTextLength();if(measured>20)lineWidth=measured+2;}});
 const alpha=(name,v)=>nodes[name].setAttribute('opacity',String(clamp(v)));
 const attr=(name,key,value)=>nodes[name].setAttribute(key,String(value));
 const text=(name,value)=>{if(nodes[name].textContent!==value)nodes[name].textContent=value;};
 let lastTool=-1;
 function setTool(n){if(n===lastTool)return;toolNodes.forEach((el,i)=>el.setAttribute('class',i===n?'pm-active':''));lastTool=n;}
 function cursor(x,y,pulse=0){attr('cursor','transform',`translate(${x.toFixed(2)} ${y.toFixed(2)})`);alpha('click',pulse);}
 function dragCursor(p,from,to,a=.25,b=.6){const t=between(p,a,b);cursor(from[0]+(to[0]-from[0])*t,from[1]+(to[1]-from[1])*t,p>a&&p<a+.035?1:0);}
 function modal(title,line1,line2,button,secondary=''){text('modal-title',title);text('modal-line1',line1);text('modal-line2',line2);text('modal-button',button);text('modal-secondary',secondary);}
 function setRect(name,x,y,w,h){attr(name,'x',x);attr(name,'y',y);attr(name,'width',w);attr(name,'height',h);}
 function existingMarks(){alpha('highlight',1);attr('highlight','width',lineWidth);alpha('ink-group',1);attr('ink','stroke-dashoffset',0);}
 function updateCamera(p){
  if(!compact){svg.setAttribute('viewBox','0 0 800 500');return;}
  // Mobile keeps the tool gesture in view, then moves the camera to the result.
  // Every crop retains a 1.6 ratio so the SVG cannot letterbox or distort.
  let target=[137,205,446,278.75],start=.38,end=.64;
  if(['select','move','memo','edit-memo','note-fit','delete'].includes(mode))target=[336,238,420,262.5];
  if(mode==='resize')target=[128,177,500,312.5];
  if(['marquee','delete-many','escape'].includes(mode))target=[135,209,450,281.25];
  if(['pen','undo','redo'].includes(mode))target=[143,219,430,268.75];
  if(mode==='highlight'){target=[138,200,440,275];start=.53;end=.71;}
  if(mode==='read'){target=[138,192,440,275];start=.3;end=.6;}
  if(['scroll','zoom','scroll-zoom'].includes(mode))target=[100,128,520,325];
  if(['share','conflict','recovery-copy','recovery-discard','protected','analyze','reanalyze'].includes(mode))target=[190,135,420,262.5];
  if(mode==='save')target=[409,18,379,236.875];
  if(['pptx','pptx-new'].includes(mode)){target=[60,160,470,293.75];start=.54;end=.73;}
  if(['analyze','reanalyze'].includes(mode)&&p>.72){const move=between(p,.72,.88),status=[409,18,379,236.875];target=target.map((v,i)=>v+(status[i]-v)*move);}
  if(mode==='recovery-discard'&&p>.86){const move=between(p,.86,.97),paper=[137,205,446,278.75];target=target.map((v,i)=>v+(paper[i]-v)*move);}
  const amount=between(p,start,end),full=[0,0,800,500];
  svg.setAttribute('viewBox',target.map((v,i)=>(full[i]+(v-full[i])*amount).toFixed(3)).join(' '));
 }
 function render(progress){
  const p=clamp(progress), draw=between(p,.32,.64), finish=p>.69;
  currentProgress=p;updateCamera(p);
  host.dataset.motionAction=mode;host.dataset.motionPhase=String(Math.round(p*1000));
  // Reset every animated attribute: seeking backwards produces the same frame.
  for(const n of ['highlight','text-selection','area-group','box-group','ink-group','note','handles','marquee','multi-second','modal','file-view','created-file','existing-file'])alpha(n,0);
  alpha('paper',1);alpha('note-editor',1);alpha('note-caret',1);alpha('file-menu',1);alpha('cursor',1);
  attr('paper','transform','translate(0 0)');attr('note','transform','translate(0 0)');attr('handles','transform','translate(0 0)');
  setRect('selection-box',160,251,330,57);setRect('resize-handle',486,304,7,7);attr('scrollbar','y',121);attr('scrollbar','height',175);
  text('status','PDF에 저장됨');attr('save-dot','fill','#9b91aa');setTool(0);cursor(681,425);
  text('note-text',fullMemo);attr('note-caret','x',455);attr('note-editor','width',177);attr('note-editor','height',65);attr('note-text','y',352);attr('note-caret','y',338);
  const toolFor={highlight:3,area:4,box:5,memo:6,pen:7,select:1,move:1,resize:1,marquee:2,'edit-memo':1,'note-fit':6,undo:8,redo:9,delete:10,'delete-many':2,escape:1};
  if(toolFor[mode]!==undefined)setTool(p>.17?toolFor[mode]:0);
  if(['highlight','area','box','memo','pen'].includes(mode)&&p>.35&&!finish){text('status','저장 중');attr('save-dot','fill','#b09a70');}
  switch(mode){
   case 'highlight':{
    const selection=between(p,.16,.38);alpha('text-selection',p<.52?1:0);attr('text-selection','width',lineWidth*selection);alpha('highlight',between(p,.52,.59));attr('highlight','width',lineWidth);
    if(p<.4)dragCursor(p,[168,292],[168+lineWidth,292],.16,.38);else if(p<.54)dragCursor(p,[168+lineWidth,292],[153,90],.4,.51);else cursor(153,90,p<.58?1:0);setTool(p>.52?3:0);break;}
   case 'area':case 'box':{
    const which=mode==='area'?'area':'box';alpha(`${which}-group`,p>.26?1:0);setRect(which,160,251,350*draw,60*draw);if(p<.26)dragCursor(p,[681,425],[47+toolFor[mode]*35,90],.04,.18);else dragCursor(p,[160,251],[510,311],.32,.64);break;}
   case 'pen':{
    alpha('ink-group',1);attr('ink','stroke-dashoffset',230*(1-draw));if(p<.28)dragCursor(p,[681,425],[292,90],.03,.18);else dragCursor(p,[194,382],[420,378],.32,.64);break;}
   case 'memo':case 'edit-memo':{
    const edit=mode==='edit-memo';existingMarks();alpha('note',edit?1:between(p,.27,.34));alpha('note-editor',p>.72?0:1);alpha('note-caret',p>.72?0:1);const t=clamp((p-.35)/.31);const str=edit?'기억은 반복과 연결로 남아요.':fullMemo;text('note-text',str.slice(0,Math.floor(str.length*t)));attr('note-caret','x',455+Math.min(str.length,Math.floor(str.length*t))*10.4);if(edit&&p<.35)text('note-text','기억은 반복으로 남아요.');if(p<.32)dragCursor(p,[681,425],[462,348],.05,.27);else if(p>.66)dragCursor(p,[462,348],[47,90],.66,.72);else cursor(462,348,p<.36?1:0);if(edit&&((p>.26&&p<.29)||(p>.31&&p<.34)))alpha('click',1);if(p>.72)setTool(0);break;}
   case 'select':case 'move':{
    alpha('note',1);alpha('note-editor',0);alpha('note-caret',0);const dy=mode==='move'?65*draw:0;attr('note','transform',`translate(0 ${dy})`);alpha('handles',p>.22?1:0);setRect('selection-box',449,334+dy,160,27);setRect('resize-handle',605,357+dy,7,7);if(mode==='move')dragCursor(p,[503,349],[503,414],.32,.64);else dragCursor(p,[681,425],[503,349],.12,.32);break;}
   case 'resize':{
    alpha('box-group',1);const dx=mode==='move'?90*draw:0,dy=mode==='move'?35*draw:0;const w=330+(mode==='resize'?95*draw:0),h=57+(mode==='resize'?66*draw:0);setRect('box',160+dx,251+dy,w,h);alpha('handles',p>.2?1:0);setRect('selection-box',160+dx,251+dy,w,h);setRect('resize-handle',156+dx+w,247+dy+h,7,7);
    if(mode==='move')dragCursor(p,[220,280],[310,315],.32,.64);else if(mode==='resize')dragCursor(p,[490,308],[585,374],.32,.64);else dragCursor(p,[681,425],[220,280],.12,.32);break;}
   case 'marquee':{
    existingMarks();alpha('marquee',p>.24&&p<.69?1:0);setRect('marquee',153,271,360*draw,125*draw);alpha('handles',p>.66?1:0);setRect('selection-box',165,278,340,25);setRect('resize-handle',501,299,7,7);alpha('multi-second',p>.66?1:0);dragCursor(p,[153,271],[513,396],.32,.64);break;}
   case 'undo':case 'redo':{
    const shown=mode==='undo'?1-between(p,.43,.5):between(p,.43,.5);alpha('ink-group',shown);attr('ink','stroke-dashoffset',0);dragCursor(p,[681,425],[47+toolFor[mode]*35,90],.14,.4);if(p>.4&&p<.46)alpha('click',1);break;}
   case 'delete-many':{
    const remain=1-between(p,.57,.63)+between(p,.78,.84);existingMarks();alpha('highlight',remain);alpha('ink-group',remain);alpha('marquee',p>.2&&p<.51?1:0);setRect('marquee',153,271,360*between(p,.2,.49),125*between(p,.2,.49));alpha('handles',p>.49&&p<.59?1:0);setRect('selection-box',165,278,340,25);setRect('resize-handle',501,299,7,7);alpha('multi-second',p>.49&&p<.59?1:0);if(p<.52)dragCursor(p,[153,271],[513,396],.2,.49);else dragCursor(p,[513,396],[397,90],.52,.64);if(p>.57)setTool(p>.77?8:10);if(p>.68)dragCursor(p,[397,90],[327,90],.68,.77);break;}
   case 'delete':{
    alpha('note',1-between(p,.46,.53));alpha('note-editor',0);alpha('note-caret',0);alpha('handles',p>.2&&p<.48?1:0);setRect('selection-box',449,334,160,27);setRect('resize-handle',605,357,7,7);dragCursor(p,[503,349],[397,90],.22,.44);if(p>.43&&p<.48)alpha('click',1);break;}
   case 'escape':{
    alpha('box-group',1);setRect('box',160,251,350,60);alpha('handles',p<.46?1:0);setTool(p<.46?1:0);cursor(510,313);break;}
   case 'read':{
    alpha('text-selection',1);attr('text-selection','width',lineWidth*between(p,.26,.62));dragCursor(p,[168,292],[168+lineWidth,292],.26,.62);break;}
   case 'note-fit':{
    alpha('note',1);const edited=p>.28,done=p>.69;
    attr('note-editor','width',210);attr('note-editor','height',82);
    if(!edited){nodes['note-text'].innerHTML='<tspan x="455" y="352">배운 내용을 내 경험과 연결하면</tspan><tspan x="455" y="371">더 오래 기억할 수 있어요.</tspan>';alpha('note-caret',0);}else{
      const short='내 경험과 연결하기';const typed=short.slice(0,Math.floor(short.length*clamp((p-.28)/.28)));text('note-text',typed);attr('note-caret','x',455+typed.length*10.4);attr('note-caret','y',338);
    }
    if(done){alpha('note-editor',0);alpha('note-caret',0);alpha('handles',1);const settle=between(p,.69,.77);const w=210-(210-119)*settle,h=82-(82-25)*settle;setRect('selection-box',448,332,w,h);setRect('resize-handle',444+w,328+h,7,7);setTool(1);}
    if(p<.57)cursor(461,348,p>.2&&p<.24?1:0);else dragCursor(p,[461,348],[710,403],.57,.69);break;}
   case 'scroll-zoom':{
    existingMarks();const distance=110*between(p,.16,.31),z=1+.3*between(p,.37,.53)-.3*between(p,.68,.8);attr('paper','transform',`translate(${400*(1-z)} ${295*(1-z)-distance}) scale(${z})`);attr('scrollbar','y',121+distance*.65);attr('scrollbar','height',175/z);cursor(603,325);break;}
   case 'scroll':{
    existingMarks();const distance=190*between(p,.25,.67);attr('paper','transform',`translate(0 ${-distance})`);attr('scrollbar','y',121+distance*.65);cursor(726,316);break;}
   case 'zoom':{
    existingMarks();const z=1+.36*between(p,.25,.64);attr('paper','transform',`translate(${400*(1-z)} ${295*(1-z)}) scale(${z})`);attr('scrollbar','height',175/z);cursor(400,295);break;}
   case 'save':{
    existingMarks();alpha('note',1);alpha('note-editor',0);alpha('note-caret',0);const saving=p>.3&&p<.6;text('status',saving?'저장 중':'PDF에 저장됨');attr('save-dot','fill',saving?'#c5a16f':'#7e9e8c');dragCursor(p,[681,425],[729,43],.08,.3);if(p>.3&&p<.36)alpha('click',1);break;}
   case 'reopen':{
    existingMarks();alpha('note',1);alpha('note-editor',0);alpha('note-caret',0);const visible=p<.3?1:p<.45?1-between(p,.3,.4):between(p,.45,.57);alpha('paper',visible);text('status',p>.3&&p<.57?'PDF 복구 확인 중':'PDF에 저장됨');cursor(69,43,p>.28&&p<.34?1:0);break;}
   case 'share':{
    existingMarks();alpha('modal',between(p,.36,.46));modal('PDF 공유','기억과 학습.pdf','최신 필기가 반영된 PDF','AirDrop · 메모');dragCursor(p,[681,425],[460,328],.16,.5);break;}
   case 'pptx':case 'pptx-new':{
    alpha('file-view',1);alpha('existing-file',mode==='pptx-new'?1:0);alpha('file-menu',p<.54?between(p,.2,.27):0);attr('file-menu-active','y',mode==='pptx-new'?260:221);text('created-name',mode==='pptx-new'?'기억과 학습 (2).pdf':'기억과 학습.pdf');alpha('created-file',between(p,.56,.67));attr('created-file','transform',`translate(0 ${(mode==='pptx-new'?66:0)+12*(1-between(p,.56,.67))})`);dragCursor(p,[695,210],[567,mode==='pptx-new'?273:238],.1,.4);text('status',p>.4&&p<.6?'PDF 변환 중':'PDF에 저장됨');break;}
   case 'conflict':case 'recovery-copy':case 'recovery-discard':{
    existingMarks();alpha('modal',between(p,.2,.3));const copy=mode!=='recovery-discard';modal('PDF 저장 문제 해결','작업 PDF가 외부에서 변경됐어요.','내 필기와 원본을 보존하고 해결하세요.',copy?'내 필기를 별도 PDF로 저장':'버리고 다시 열기',copy?'원본 PDF는 그대로 보관합니다.':'변경을 버릴지 다시 확인합니다.');text('status','저장 실패');attr('save-dot','fill','#ba796f');dragCursor(p,[681,425],[410,327],.3,.6);if(mode!=='conflict'&&p>.73){modal(copy?'별도 PDF 저장':'변경 폐기 확인',copy?'기억과 학습 — 내 필기.pdf':'아직 반영되지 않은 필기를 버릴까요?',copy?'위치: 문서':'현재 파일을 다시 엽니다.',copy?'저장':'버리고 다시 열기');}if(mode==='recovery-discard'&&p>.87){alpha('modal',1-between(p,.87,.93));alpha('highlight',0);alpha('ink-group',0);text('status','PDF에 저장됨');attr('save-dot','fill','#9b91aa');}break;}
   case 'protected':{
    alpha('modal',between(p,.17,.27));modal('필기 전 확인','이 PDF에는 양식 또는 서명이 있습니다.','필기하면 서명 상태가 달라질 수 있어요.','읽기만 하기','필기 시작');dragCursor(p,[681,425],[408,327],.25,.54);if(p>.67)alpha('modal',1-between(p,.67,.73));break;}
   case 'analyze':case 'reanalyze':{
    existingMarks();alpha('modal',between(p,.3,.39));modal(mode==='reanalyze'?'자료 다시 분석':'자료 분석','기억과 학습.pdf','선택한 AI 공급자에 자료를 보냅니다.','분석 시작','직접 요청한 자료만 분석합니다.');dragCursor(p,[681,425],[408,327],.2,.51);if(p>.67){alpha('modal',1-between(p,.67,.74));text('status','분석 중');}break;}
   default:{
    // Unknown actions remain a neutral reading view rather than invent a feature.
    host.dataset.motionUnsupported=mode;existingMarks();cursor(683,420);break;}
  }
  if(p>.91)alpha('cursor',1-between(p,.91,.97));
 }
 const resizeObserver=new ResizeObserver(entries=>{if(disposed)return;compact=entries[0].contentRect.width<520;updateCamera(currentProgress);});
 resizeObserver.observe(host);
 render(0);
 return {render,destroy(){disposed=true;resizeObserver.disconnect();host.replaceChildren();host.classList.remove('pm-scene-host');delete host.dataset.motionAction;delete host.dataset.motionPhase;delete host.dataset.motionUnsupported;}};
}
