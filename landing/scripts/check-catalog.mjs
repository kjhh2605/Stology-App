// 정적 검사: 카탈로그·장면·문구가 서로 맞는지 확인한다. 실행: npm run check --prefix landing
// 영상 파일 검사는 없다(기능 시연은 브라우저에서 그리는 DOM·캔버스 장면이다).
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

// 06 과목 지도 장면: 영어 번역표가 지도 데이터를 모두 덮는지, 검색어의 첫 결과가 장면이 여는 개념인지
const { ATLAS } = await import(resolve(root, 'src/scenes/atlas-data.js'));
const { ATLAS_TOUR } = await import(resolve(root, 'src/scenes/atlas-loop.js'));
const atlasIndex = name => ATLAS.concepts.findIndex(c => c.name === name);
for (const [key, name] of Object.entries(ATLAS_TOUR)) if (atlasIndex(name) < 0) fail(`지도 장면 개념 없음: ${key} ${name}`);
const tourTexts = new Set();
for (const name of Object.values(ATLAS_TOUR)) {
  const i = atlasIndex(name), c = ATLAS.concepts[i];
  if (!c) continue;
  tourTexts.add(c.def);
  c.places.forEach(p => tourTexts.add(p[3]));
  ATLAS.links.filter(l => l.s === i || l.o === i).forEach(l => { if (l.cond) tourTexts.add(l.cond); if (l.reason) tourTexts.add(l.reason); });
}
const E = en.scenes.atlas || {};
for (const c of ATLAS.concepts) if (!E.names?.[c.name]) fail(`[en] 지도 개념 이름 번역 누락: ${c.name}`);
for (const u of ATLAS.units) if (!E.units?.[u]) fail(`[en] 지도 단원 번역 누락: ${u}`);
for (const s of ATLAS.sources) if (!E.sources?.[s.title]) fail(`[en] 지도 자료 이름 번역 누락: ${s.title}`);
for (const t of tourTexts) if (!E.text?.[t]) fail(`[en] 지도 장면 문구 번역 누락: ${t}`);
// graph_controls.rs matches: 공백을 빼고 소문자로 비교, 이름이 검색어와 같은 개념이 먼저, 나머지는 이름순
const squash = v => String(v).replace(/\s/g, '').toLowerCase();
for (const dict of [ko, en]) {
  const query = dict.scenes.atlas?.query;
  if (!query) { fail(`[${dict.lang}] scenes.atlas.query 누락`); continue; }
  const nameOf = c => (dict.scenes.atlas.names?.[c.name] || c.name);
  const hits = ATLAS.concepts.filter(c => squash(nameOf(c)).includes(squash(query)));
  hits.sort((a, b) => (squash(nameOf(a)) === squash(query) ? 0 : 1) - (squash(nameOf(b)) === squash(query) ? 0 : 1) || (squash(nameOf(a)) < squash(nameOf(b)) ? -1 : 1));
  if (hits[0]?.name !== ATLAS_TOUR.question) fail(`[${dict.lang}] 지도 검색어 '${query}'의 첫 결과가 ${ATLAS_TOUR.question}가 아닙니다: ${hits[0]?.name}`);
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
console.log(`검사 통과: 노트 ${noteItems.length} · PDF ${pdfItems.length} · 장면 ${all.length} · 지도 개념 ${ATLAS.concepts.length} · 한/영 문구 일치`);
