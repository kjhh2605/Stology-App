// 정적 검사: 카탈로그·장면·문구가 서로 맞는지 확인한다. 실행: npm run check --prefix landing
// 영상 파일 검사는 없다(기능 시연은 브라우저에서 그리는 DOM 장면이다).
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const problems = [];
const fail = msg => problems.push(msg);

const { noteItems, pdfItems } = await import(resolve(root, 'src/catalog.js'));
const ko = (await import(resolve(root, 'src/i18n/ko.js'))).default;
const en = (await import(resolve(root, 'src/i18n/en.js'))).default;
const noteSrc = readFileSync(resolve(root, 'src/scenes/note-scenes.js'), 'utf8');
const pdfSrc = readFileSync(resolve(root, 'src/scenes/pdf-scenes.js'), 'utf8');
const shortcutsDoc = readFileSync(resolve(root, '..', 'app/docs/keyboard-shortcuts.md'), 'utf8');

const all = [...noteItems, ...pdfItems];
const ids = all.map(i => i.id);
if (new Set(ids).size !== ids.length) fail('id가 중복됩니다.');
if (noteItems.length !== 16 || pdfItems.length !== 13) fail(`항목 수가 16·13이 아닙니다: ${noteItems.length}·${pdfItems.length}`);
if (noteItems.filter(i => i.core).length !== 6) fail('노트 핵심 항목은 6개여야 합니다.');
if (pdfItems.filter(i => i.core).length !== 5) fail('PDF 핵심 항목은 5개여야 합니다.');

// 장면 함수 존재
const sceneKey = id => new RegExp(`(^|\\n)\\s*'?${id.replace(/-/g, '\\-')}'?\\s*:`);
for (const i of noteItems) if (!sceneKey(i.id).test(noteSrc)) fail(`노트 장면 없음: ${i.id}`);
for (const i of pdfItems) if (!sceneKey(i.id).test(pdfSrc)) fail(`PDF 장면 없음: ${i.id}`);

// 한·영 문구
for (const dict of [ko, en]) {
  for (const i of all) {
    const t = dict.items[i.id];
    if (!t || !t.label || !t.title || !t.body) fail(`[${dict.lang}] 문구 누락: ${i.id}`);
    if (!dict.categories[i.category]) fail(`[${dict.lang}] 분류 이름 누락: ${i.category}`);
    if (i.chip && !dict.explorer.chips[i.chip]) fail(`[${dict.lang}] chip 이름 누락: ${i.chip}`);
  }
  for (const key of ['hero', 'problem', 'window', 'promises', 'notes', 'pdf', 'map', 'start', 'faq', 'closing', 'footer', 'dataDialog', 'scenes']) if (!dict[key]) fail(`[${dict.lang}] 섹션 문구 누락: ${key}`);
  if (dict.faq.items.filter(f => /19,000|₩|\$|무료|free|price|요금|pricing/i.test(f.q + f.a)).length) fail(`[${dict.lang}] FAQ에 가격·무료 표현이 있습니다.`);
  const D = dict.scenes.doc;
  for (const k of ['selWord', 'selSmall', 'linkWord']) if (!D.target.includes(D[k])) fail(`[${dict.lang}] scenes.doc.${k}가 target 문장에 없습니다.`);
  if (!dict.scenes.map.suggestions.some(s => s.includes(dict.scenes.map.query))) fail(`[${dict.lang}] 지도 검색어가 제안 목록에 없습니다.`);
}

// 단축키가 앱 문서에 있는지 (키 표기: ⌘⌥1 처럼 붙여 쓴 형태)
const docText = shortcutsDoc.replace(/\s+/g, '');
for (const i of noteItems) {
  if (!i.keys) continue;
  const joined = i.keys.join('').replace('Space', '');
  if (i.id === 'markdown-quote') continue; // `>` 입력 규칙은 표가 아니라 본문에 있다
  if (!docText.includes(joined)) fail(`단축키 문서에 없음: ${i.id} (${i.keys.join('')})`);
}
// 키캡 표기: 수정자 순서 ⌃ ⌥ ⇧ ⌘
const order = ['⌃', '⌥', '⇧', '⌘'];
for (const i of all) {
  if (!i.keys) continue;
  const mods = i.keys.filter(k => order.includes(k));
  const sorted = [...mods].sort((a, b) => order.indexOf(a) - order.indexOf(b));
  if (mods.join('') !== sorted.join('') && i.keys.join('') !== '⌘⌥1' && !/^⌘⌥\d$/.test(i.keys.join('')) && i.keys.join('') !== '⌘⇧S') fail(`수정자 순서 확인: ${i.id} ${i.keys.join(' ')}`);
}
// 실제 앱은 ⌘⌥·⌘⇧ 순서로 표기한다(keyboard-shortcuts.md). HIG 순서와 다르므로 앱 표기를 따른다는 뜻으로 위 예외를 둔다.

if (problems.length) { console.error('검사 실패:\n- ' + problems.join('\n- ')); process.exit(1); }
console.log(`검사 통과: 노트 ${noteItems.length} · PDF ${pdfItems.length} · 장면 ${all.length} · 한/영 문구 일치`);
