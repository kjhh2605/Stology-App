// 06 과목 지도 장면의 구도 검사. 브라우저 없이 지도 엔진을 돌려, 장면이 개념을 여는 순간마다
// 가까운 구체가 열린 개념·이웃·이름표를 얼마나 가리는지 점수로 낸다(낮을수록 깔끔).
// 실행: node landing/qa/atlas-composition.mjs          장면의 회전 거리(ATLAS_DRAGS)로 점수
//       node landing/qa/atlas-composition.mjs --scan   회전 거리 후보를 훑어 낮은 순으로
// 지도 영역은 데스크톱 장면의 설계 크기(752×534)다. 이름표 폭은 글자 수로 어림하므로 이름표 겹침은 참고값이다.
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const scenes = resolve(dirname(fileURLToPath(import.meta.url)), '../src/scenes');
globalThis.OffscreenCanvas = class { getContext() { return { createImageData: (w, h) => ({ data: new Uint8ClampedArray(w * h * 4) }), putImageData() {} }; } };
const noop = () => {};
const ctx = new Proxy({ measureText: t => ({ width: [...t].reduce((s, c) => s + (c.charCodeAt(0) < 128 ? 7.6 : 13.6), 0) }) }, { get: (o, k) => (k in o ? o[k] : noop), set: () => true });
const { createAtlas } = await import(resolve(scenes, 'atlas-engine.js'));
const { ATLAS } = await import(resolve(scenes, 'atlas-data.js'));
const { ATLAS_TOUR, ATLAS_DRAGS } = await import(resolve(scenes, 'atlas-loop.js'));
const W = 752, H = 534;
const idx = name => ATLAS.concepts.findIndex(c => c.name === name);
const make = () => createAtlas({ getContext: () => ctx }, { concepts: ATLAS.concepts.map(c => ({ id: c.id, name: c.name, unit: c.unit })), links: ATLAS.links }, { width: W, height: H, pixelRatio: 1 });
const run = (E, seconds) => { for (let i = 0; i < seconds * 60; i++) E.tick(1 / 60); };
const drag = (E, [dx, dy]) => { for (let i = 0; i < 100; i++) { E.orbit(dx / 100, dy / 100); E.tick(1 / 60); } run(E, 1); };
const area = (a, b) => Math.max(0, Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)) * Math.max(0, Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y));

/** cover: 지름 32px 넘는 다른 구체가 열린 개념·이웃·이름표와 겹친 넓이, big: 반지름 26px 넘는 구체 수, out: 화면 밖 이웃 수 */
function score(E, sel) {
  const ring = new Set([sel, ...E.neighbours(sel)]), keep = [], labels = E.labels.filter(l => l.alpha > .5);
  for (const i of ring) { const p = E.screenOf(i); if (p) keep.push({ x: p.x - p.r, y: p.y - p.r, w: 2 * p.r, h: 2 * p.r }); }
  keep.push(...labels);
  let cover = 0, big = 0, out = 0, labelHit = 0;
  ATLAS.concepts.forEach((_, i) => {
    const p = E.screenOf(i);
    if (ring.has(i) || !p || p.r < 16) return;
    keep.forEach(k => { cover += area({ x: p.x - p.r, y: p.y - p.r, w: 2 * p.r, h: 2 * p.r }, k); });
    if (p.r > 26) big += 1;
  });
  for (const i of ring) { const p = E.screenOf(i); if (!p || p.x < 30 || p.x > W - 30 || p.y < 60 || p.y > H - 40) out += 1; }
  labels.forEach((a, i) => labels.slice(i + 1).forEach(b => { labelHit += area(a, b); }));
  return { cover: Math.round(cover), big, out, labelHit: Math.round(labelHit), score: Math.round(cover + labelHit * 2 + out * 4000 + big * 300) };
}
/** 장면과 같은 순서: 돌리기 → 강의 개념 → 전체 보기 → 돌리기 → 검색·Enter → 질문 개념 → 연결된 개념 */
function tour(drags) {
  const E = make(); run(E, 1);
  drag(E, drags.lecture); E.hover(idx(ATLAS_TOUR.lecture)); run(E, .3);
  E.select(idx(ATLAS_TOUR.lecture)); run(E, 1.5);
  const lecture = score(E, idx(ATLAS_TOUR.lecture));
  E.clear(); run(E, 1.2);
  drag(E, drags.question);
  E.select(idx(ATLAS_TOUR.question)); run(E, 1.5);
  const question = score(E, idx(ATLAS_TOUR.question));
  E.select(idx(ATLAS_TOUR.next)); run(E, 1.5);
  return { lecture, question, next: score(E, idx(ATLAS_TOUR.next)) };
}

if (process.argv.includes('--scan')) {
  const rows = [];
  for (let dx = -400; dx <= 400; dx += 40) for (const dy of [-60, -30, 0, 30, 60]) {
    const at = name => { const F = make(); run(F, 1); drag(F, [dx, dy]); F.select(idx(name)); run(F, 1.5); return score(F, idx(name)).score; };
    rows.push({ dx, dy, lecture: at(ATLAS_TOUR.lecture), question: at(ATLAS_TOUR.question), next: (() => { const F = make(); run(F, 1); drag(F, [dx, dy]); F.select(idx(ATLAS_TOUR.question)); run(F, 1.5); F.select(idx(ATLAS_TOUR.next)); run(F, 1.5); return score(F, idx(ATLAS_TOUR.next)).score; })() });
  }
  console.log('강의 개념 앞 회전 후보'); console.table([...rows].sort((a, b) => a.lecture - b.lecture).slice(0, 6).map(({ dx, dy, lecture }) => ({ dx, dy, lecture })));
  console.log('질문 개념 앞 회전 후보'); console.table([...rows].sort((a, b) => a.question + a.next - b.question - b.next).slice(0, 6).map(({ dx, dy, question, next }) => ({ dx, dy, question, next })));
} else {
  console.log('ATLAS_DRAGS', JSON.stringify(ATLAS_DRAGS));
  console.table(tour(ATLAS_DRAGS));
  console.log('회전 없이', JSON.stringify(tour({ lecture: [0, 0], question: [0, 0] })));
}
