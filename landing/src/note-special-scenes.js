/** Purpose-built UI recipes for notebook, code, tables and the study assistant.
 * These scenes demonstrate input/results; they do not depict a live AI response.
 */
const esc = value => String(value ?? '').replace(/[&<>"']/g, character => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[character]));
const caret = '<i class="nm-caret" aria-hidden="true"></i>';
const selected = text => `<span class="nm-selection">${text}</span>`;
const paragraph = text => `<p>${text}</p>`;
const context = '<h3>기억과 학습</h3><p class="nm-muted">반복과 연결을 작은 예시로 정리해요.</p>';
const codeText = 'memory = ["반복", "연결"]';
const code = (text, { language = 'Python', copied = false, menu = false } = {}) => `<div class="nm-code"><div class="nm-codebar"><span class="nm-code-language">${esc(language)} <span aria-hidden="true">⌄</span></span><span class="nm-code-copy${copied ? ' nm-confirmed' : ''}">${copied ? '✓' : '⧉'}</span></div>${menu ? '<div class="nm-language-menu"><span>일반 텍스트</span><span class="nm-selected-option">Python</span><span>JavaScript</span></div>' : ''}<pre class="nm-code-lines"><code>${text}</code></pre></div>`;
const editorCode = (text, options = {}, tail = '<p class="nm-muted">반복할 때마다 떠올려 보기</p>') => `${context}${code(text, options)}${tail}`;
const table = (active = '', editing = false, saved = false) => `<h3>기억을 돕는 방법</h3><table class="nm-table"><thead><tr><th>방법</th><th>생활 속 예시</th></tr></thead><tbody><tr><td class="${active === 'method' ? 'nm-cell-active' : ''}">반복${active === 'method' ? caret : ''}</td><td class="${active === 'example' ? 'nm-cell-active' : ''}">${saved || editing ? '책을 덮고 떠올리기' : '내용 다시 읽기'}${editing && active === 'example' ? caret : ''}</td></tr><tr><td class="${active === 'next' ? 'nm-cell-active' : ''}">연결${active === 'next' ? caret : ''}</td><td>내 경험과 이어 보기</td></tr></tbody></table>`;
const chat = ({ input = '', sent = false, selectedMessage = false, copied = false, draft = false, reply = false, selectedAnswer = false } = {}) => `<div class="nm-chat"><div class="nm-chat-header"><span>✧ 학습도우미</span><span class="nm-status-dot"></span></div><div class="nm-chat-history">${reply ? '<div class="nm-chat-bubble nm-chat-assistant">' + (selectedAnswer ? selected('반복과 연결을 함께 생각해보세요.') : '반복과 연결을 함께 생각해보세요.') + '<span class="nm-message-copy">' + (copied ? '✓' : '⧉') + '</span></div>' : ''}${sent || selectedMessage || copied ? `<div class="nm-chat-bubble nm-chat-user">${selectedMessage ? selected('기억은 어떻게 오래 남을까요?') : '기억은 어떻게 오래 남을까요?'}</div>` : ''}${draft ? '<div class="nm-chat-bubble nm-chat-assistant nm-chat-wait"><i></i><i></i><i></i></div>' : ''}</div><div class="nm-chat-input">${input || '<span class="nm-placeholder">궁금한 내용을 적어보세요</span>'}${input ? caret : ''}<div class="nm-chat-inputbar"><span>Claude <span aria-hidden="true">⌄</span></span><span class="nm-chat-send">${draft ? '■' : '↑'}</span></div></div></div>`;
const search = (value, { map = false, focus = false } = {}) => map
 ? `<div class="nm-map"><div class="nm-search${focus ? ' nm-input-focused' : ''}"><span>⌕</span>${value ? esc(value) : '<span class="nm-placeholder">개념 검색</span>'}${focus ? caret : ''}</div><svg viewBox="0 0 400 180" class="nm-map-svg" aria-hidden="true"><path d="M80 100L200 45L315 110M80 100L215 155L315 110M200 45L215 155" fill="none" stroke="currentColor"/><circle cx="80" cy="100" r="8"/><circle cx="200" cy="45" r="12"/><circle cx="315" cy="110" r="8"/><circle cx="215" cy="155" r="6"/></svg><div class="nm-map-names"><span>반복</span><span>기억</span><span>연결</span></div></div>`
 : `<h3>자료 보관함</h3><div class="nm-search${focus ? ' nm-input-focused' : ''}"><span>⌕</span>${value ? esc(value) : '<span class="nm-placeholder">파일 검색</span>'}${focus ? caret : ''}</div><div class="nm-file-row"><span>▤</span> 기억과 학습</div>${!value ? '<div class="nm-file-row"><span>▤</span> 감정과 마음</div><div class="nm-file-row"><span>▤</span> 수업 안내</div>' : ''}`;
const titleField = (title, focused = false) => `<div class="nm-title-field${focused ? ' nm-input-focused' : ''}">${esc(title)}${focused ? caret : ''}</div>`;
const saveDialog = () => `<div class="nm-dialog"><h3>노트 저장</h3><label>제목<div class="nm-input nm-input-focused">기억과 학습${caret}</div></label><label>폴더<div class="nm-input">심리학의 이해 <span>⌄</span></div></label><div class="nm-dialog-actions"><span>취소</span><span class="nm-dialog-primary">저장</span></div></div>`;
const baseNote = () => '<h3>기억과 학습</h3><p>반복은 기억을 오래 남기는 데 도움을 줍니다.</p><p>오늘 배운 내용을 내 경험과 연결해 보세요.</p>';

function resolve(item) {
 const candidates = [item.scene?.action, item.id, item.scene?.kind].filter(Boolean).map(value => String(value).toLowerCase());
 const alias = {
  'save-note':'save','note-save':'save','save-first':'save','save-current':'save-existing',
  'code-tab':'code-indent','code-newline':'code-enter','code-shift-enter':'code-exit','code-exit-shift-enter':'code-exit','code-empty-exit':'code-exit-empty','code-empty-lines':'code-exit-empty','code-exit-enter':'code-exit-empty',
  'code-all':'code-select-all','code-select':'code-select-all','code-cmd-a':'code-select-all','code-empty-backspace':'code-remove','code-backspace':'code-remove','code-language-change':'code-language','code-copy-button':'code-copy',
  'table-apply':'table-enter','table-commit':'table-enter','table-next':'table-tab','table-prev':'table-shift-tab','table-previous':'table-shift-tab',
  'chat-newline':'chat-enter','chat-submit':'chat-send','chat-command-enter':'chat-send','chat-cmd-enter':'chat-send','chat-select':'chat-select-copy','chat-copy':'chat-select-copy','chat-answer-copy':'chat-copy-answer',
  'title-confirm':'title-enter','title-apply':'title-enter','title-cancel':'title-escape','title-esc':'title-escape',
  'file-search-clear':'file-search-escape','map-search-clear':'map-search-escape','search-escape':'file-search-escape','search-esc':'file-search-escape','file-search-esc':'file-search-escape','map-search-esc':'map-search-escape','graph-search-escape':'map-search-escape','graph-search-focus':'map-search-focus','map-search':'map-search-focus','map-search-cmd-k':'map-search-focus',
  'app-quit':'quit','exit-app':'quit','chat-draft':'chat-next-draft',
 };
 for (const candidate of candidates) { const action = alias[candidate] || candidate; if (specialActions.includes(action)) return action; }
 return null;
}
export const specialActions = ['save','save-existing','quit','code-indent','code-enter','code-exit','code-exit-empty','code-select-all','code-selection-expand','code-remove','code-language','code-copy','table-enter','table-tab','table-shift-tab','chat-enter','chat-send','chat-select-copy','chat-copy-answer','chat-next-draft','title-enter','title-escape','file-search-escape','map-search-escape','map-search-focus'];

function makeRecipe(item) {
 const action = resolve(item);
 if (!action) return null;
 const config = { action, variant:'code', focus:{x:55,y:48} };
 switch (action) {
  case 'code-indent': return {...config,before:editorCode(caret + esc(codeText)),after:editorCode('<span class="nm-code-indent">  </span>' + caret + esc(codeText))};
  case 'code-enter': return {...config,before:editorCode(esc(codeText) + caret),after:editorCode(esc(codeText) + '\n' + caret)};
  case 'code-exit': return {...config,before:editorCode(esc(codeText) + caret),after:editorCode(esc(codeText),{},paragraph(caret) + '<p class="nm-muted">반복할 때마다 떠올려 보기</p>'),focus:{x:38,y:70}};
  case 'code-exit-empty': return {...config,before:editorCode(esc(codeText) + '\n\n' + caret),after:editorCode(esc(codeText),{},paragraph(caret)),focus:{x:38,y:70}};
  case 'code-select-all': return {...config,before:editorCode(esc(codeText) + '\nprint(memory)' + caret),after:editorCode(selected(esc(codeText) + '\nprint(memory)'))};
  case 'code-selection-expand': {const first='memory = experience',second='review = recall(memory)';return {...config,before:editorCode(esc(first)+'\n'+esc(second)),after:editorCode(selected(esc(first)+'\n'+esc(second))),frames:[{at:0,html:editorCode(esc(first)+'\n'+esc(second))},{at:.25,html:editorCode('memory = '+selected('experience')+'\n'+esc(second))},{at:.43,html:editorCode(selected(esc(first))+'\n'+esc(second))},{at:.61,html:editorCode(selected(esc(first)+'\n'+esc(second)))}]};}
  case 'code-remove': return {...config,before:editorCode(caret,{},''),after:context + paragraph(caret),focus:{x:29,y:50}};
  case 'code-language': return {...config,before:editorCode(esc(codeText),{language:'일반 텍스트'}),middle:editorCode(esc(codeText),{language:'일반 텍스트',menu:true}),after:editorCode('<span class="nm-syntax-name">memory</span> = [<span class="nm-syntax-string">"반복"</span>, <span class="nm-syntax-string">"연결"</span>]',{language:'Python'}),focus:{x:31,y:41}};
  case 'code-copy': return {...config,before:editorCode(esc(codeText)),after:editorCode(esc(codeText),{copied:true}) + '<div class="nm-clipboard"><span>⧉</span><code>memory = ["반복", "연결"]</code></div>',focus:{x:85,y:41}};
  case 'table-enter': return {...config,variant:'table',before:table('example',true),after:table('',false,true),focus:{x:65,y:48}};
  case 'table-tab': return {...config,variant:'table',before:table('example',true),after:table('next',false,true),focus:{x:34,y:63}};
  case 'table-shift-tab': return {...config,variant:'table',before:table('example',true),after:table('method',false,true),focus:{x:34,y:48}};
  case 'chat-enter': return {...config,variant:'chat',before:chat({input:'기억은 어떻게 오래 남을까요?'}),after:chat({input:'기억은 어떻게 오래 남을까요?<br><br>'}),focus:{x:45,y:72}};
  case 'chat-send': return {...config,variant:'chat',before:chat({input:'기억은 어떻게 오래 남을까요?'}),after:chat({sent:true}),focus:{x:84,y:80}};
  case 'chat-select-copy': return {...config,variant:'chat',before:chat({reply:true,sent:true}),after:chat({reply:true,sent:true,selectedAnswer:true,copied:true}) + '<div class="nm-clipboard"><span>⧉</span>반복과 연결을 함께 생각해보세요.</div>',focus:{x:56,y:40}};
  case 'chat-copy-answer': return {...config,variant:'chat',before:chat({reply:true}),after:chat({reply:true,copied:true}),focus:{x:56,y:40}};
  case 'chat-next-draft': return {...config,variant:'chat',before:chat({sent:true,draft:true}),after:chat({sent:true,draft:true,input:'생활 속 예시도 궁금해요.'}),focus:{x:45,y:73}};
  case 'title-enter': return {...config,variant:'title',before:titleField('기억과 학습',true) + paragraph('<span class="nm-placeholder">새로운 생각을 적어보세요</span>'),after:titleField('기억과 학습') + paragraph(caret),focus:{x:34,y:52}};
  case 'title-escape': return {...config,variant:'title',before:titleField('기억과 학습',true) + paragraph('수업 내용을 정리해요.'),after:titleField('새 노트') + paragraph('수업 내용을 정리해요.' + caret),focus:{x:34,y:52}};
  case 'file-search-escape': return {...config,variant:'search',before:search('기억',{focus:true}),after:search('',{focus:true}),focus:{x:40,y:32}};
  case 'map-search-escape': return {...config,variant:'search',before:search('기억',{map:true,focus:true}),after:search('',{map:true,focus:true}),focus:{x:40,y:20}};
  case 'map-search-focus': return {...config,variant:'search',before:search('',{map:true}),after:search('',{map:true,focus:true}),focus:{x:40,y:20}};
  case 'save': return {...config,variant:'dialog',before:baseNote() + '<div class="nm-save-status">변경 사항 있음</div>',middle:saveDialog(),after:baseNote() + '<div class="nm-save-status nm-confirmed">✓ 이 Mac에 저장됨</div>',focus:{x:76,y:76}};
  case 'save-existing': return {...config,variant:'dialog',before:baseNote() + '<div class="nm-save-status">저장 중</div>',after:baseNote() + '<div class="nm-save-status nm-confirmed">✓ 이 Mac에 저장됨</div>',focus:{x:81,y:18}};
  case 'quit': return {...config,variant:'quit',before:baseNote() + '<div class="nm-save-status">저장 중</div>',middle:baseNote() + '<div class="nm-save-status nm-confirmed">✓ 이 Mac에 저장됨</div>',after:'<div class="nm-quit-desktop"><div class="nm-desktop-orbit"></div><img src="/mark.svg" alt="" width="56" height="56"></div>',focus:{x:20,y:15}};
  default: return null;
 }
}

export function specialRecipe(item) {
 const recipe = makeRecipe(item);
 if (!recipe) return null;
 return { ...recipe, mode: recipe.variant, dialog: recipe.action==='save'?recipe.middle:null, middle:recipe.action==='code-language'?recipe.middle:null, focus: { x: 158 + recipe.focus.x * 4.6, y: 127 + recipe.focus.y * 3.1 } };
}
