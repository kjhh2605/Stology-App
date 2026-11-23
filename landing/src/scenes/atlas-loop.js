// 06 과목 지도: 실제 앱의 지식지도 화면(지도 카드 + 개념과 원본 근거 패널)을 조작하는 장면을 약 23초로 반복한다.
// 돌려 보기 → 개념 열기 → 배운 곳(강의자료 쪽수) → 전체 보기 → 다시 돌려 개념 검색·Enter → 내 질문에서 배운 곳 → 연결로 이동.
// 지도는 atlas-engine.js가 캔버스에 그리고, 데이터는 로컬 앱 '검증용 학습 자료 (Claude)' 워크스페이스의 스냅숏이다.
import { rectIn, linear, easeInOut, createKeyToast, createPointer } from './runtime.js';
import { mountFrame, h, esc, icon } from './frames.js';
import { UI } from './ui-labels.js';
import { ATLAS } from './atlas-data.js';
import { createAtlas, unitColor } from './atlas-engine.js';

/** 장면이 차례로 여는 개념(데이터의 한국어 이름). 정적 검사가 영어 번역이 있는지 본다 */
export const ATLAS_TOUR = { lecture: 'Cursor based 페이징', question: '제1정규형', next: '제2정규형' };
/** 개념을 열기 전에 빈 곳을 끄는 거리(화면 px). 열린 개념 주변을 가까운 구체가 가리지 않도록 qa/atlas-composition.mjs로 골랐다 */
export const ATLAS_DRAGS = { lecture: [-260, 0], question: [400, 60] };

const SVG = {
  network: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><circle cx="12" cy="5" r="2.3"/><circle cx="5" cy="18" r="2.3"/><circle cx="19" cy="18" r="2.3"/><path d="M10.8 7.2 6.3 15.8M13.2 7.2l4.5 8.6M7.6 18h8.8" stroke-dasharray="1.5 2.4"/></svg>',
  quote: '<svg viewBox="0 0 12 12" fill="currentColor"><path d="M5.1 2.8C3.8 4.1 3.3 5.3 3.3 6.4c.8.1 1.2.6 1.2 1.4 0 .9-.5 1.5-1.3 1.5-1 0-1.6-.8-1.6-2.1 0-1.8 1.1-3.6 3-5zM9.8 2.8C8.5 4.1 8 5.3 8 6.4c.8.1 1.2.6 1.2 1.4 0 .9-.5 1.5-1.3 1.5-1 0-1.6-.8-1.6-2.1 0-1.8 1.1-3.6 3-5z"/></svg>',
  tag: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M3.5 12.5V4h8.5l9 9-8.5 8.5z"/><circle cx="8" cy="8.5" r="1.4"/></svg>',
  broom: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="m14 10 6.4-7a1.1 1.1 0 0 1 1.6 1.5l-6.4 7M10 9l7 6-2.2 2.5-7-6zM7.8 11.5C6.4 14 4 15.8 2 16.6c2.3 3.6 6.6 5.4 10.2 5.2l2.6-4.3M8.9 15.8l-3 3.2m5.7-1.8-2.3 3.6"/></svg>',
  importFile: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3.5v11M7.5 10.5l4.5 4.5 4.5-4.5M5 13.5V20h14v-6.5"/></svg>',
  updown: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M8 9.5l4-4 4 4M8 14.5l4 4 4-4"/></svg>',
};
const orb = color => `<i class="orb"${color ? ` style="background:${color}"` : ''}></i>`;

/* ---------- 사용자 데이터 번역(영어 페이지) ---------- */
function translator(A = {}) {
  const pick = (table, ko) => (table && table[ko]) || ko;
  return { name: ko => pick(A.names, ko), unit: ko => pick(A.units, ko), source: ko => pick(A.sources, ko), text: ko => pick(A.text, ko) };
}

/** core ontology.rs link_sentence와 같은 문장. 영어 페이지는 같은 뜻의 영어 틀을 쓴다 */
function particle(name, closed, open) {
  const c = name.trimEnd().slice(-1);
  if (c >= '가' && c <= '힣') return name + ((c.charCodeAt(0) - 0xac00) % 28 === 0 ? open : closed);
  return closed === '과' ? `${name}${open}(${closed})` : `${name}${closed}(${open})`;
}
export function linkSentence(kind, a, b, lang) {
  if (lang === 'en') {
    const en = { kind_of: `${a} is a kind of ${b}`, part_of: `${a} is part of ${b}`, prerequisite: `Learn ${a} first to understand ${b}`, cause: `${a} leads to ${b}`, contrast: `${a} and ${b} contrast`, example: `${a} is an example of ${b}`, applies: `${a} applies to ${b}` };
    return en[kind] || `${a} and ${b} are related`;
  }
  const topic = x => particle(x, '은', '는'), object = x => particle(x, '을', '를'), subject = x => particle(x, '이', '가'), with_ = x => particle(x, '과', '와');
  switch (kind) {
    case 'kind_of': return `${topic(a)} ${b}의 한 종류예요`;
    case 'part_of': return `${topic(a)} ${object(b)} 이루는 부분이에요`;
    case 'prerequisite': return `${object(a)} 먼저 알아야 ${object(b)} 이해해요`;
    case 'cause': return `${a} 때문에 ${subject(b)} 일어나요`;
    case 'contrast': return `${with_(a)} ${topic(b)} 서로 대비돼요`;
    case 'example': return `${topic(a)} ${b}의 예시예요`;
    case 'applies': return `${topic(a)} ${b}에 쓰여요`;
    default: return `${with_(a)} ${topic(b)} 관련이 있어요`;
  }
}

/* ---------- 패널(main.rs render_graph_side) ---------- */
const degreeOf = i => new Set(ATLAS.links.filter(l => l.s === i || l.o === i).map(l => (l.s === i ? l.o : l.s))).size;
let measureCtx = null;
/** NSLineBreakMode::ByTruncatingMiddle: 가운데를 줄여 앞(자료 이름)과 뒤(위치)를 함께 남긴다 */
function truncateMiddle(text, maxWidth, font) {
  measureCtx ||= document.createElement('canvas').getContext('2d');
  measureCtx.font = font;
  if (measureCtx.measureText(text).width <= maxWidth) return text;
  const chars = [...text];
  for (let cut = 1; cut < chars.length; cut++) {
    const keep = chars.length - cut, head = Math.ceil(keep / 2), tail = keep - head;
    const shown = chars.slice(0, head).join('') + '…' + chars.slice(chars.length - tail).join('');
    if (measureCtx.measureText(shown).width <= maxWidth) return shown;
  }
  return '…';
}

function panelMarkup(selected, { T, lang }) {
  const A = UI.atlas, out = [];
  const name = i => T.name(ATLAS.concepts[i].name);
  if (selected !== null) {
    const c = ATLAS.concepts[selected];
    out.push(`<h4 class="ap-title">${esc(name(selected))}</h4><p class="ap-meta">${esc([A.kinds[c.kind] || A.kinds.concept, T.unit(ATLAS.units[c.unit])].join(' · '))}</p><p class="ap-def">${esc(T.text(c.def))}</p><p class="ap-hint">${A.editHint}</p>`);
    const links = ATLAS.links.filter(l => l.s === selected || l.o === selected);
    if (links.length) {
      out.push(`<i class="ap-div"></i><div class="ap-sec">${icon('link')}<span>${A.links(links.length)}</span></div>`);
      for (const l of links.slice(0, 8)) {
        const other = l.s === selected ? l.o : l.s;
        const sentence = linkSentence(l.kind, name(l.s), name(l.o), lang) + (l.cond ? A.condition(T.text(l.cond)) : '');
        out.push(`<div class="gl" data-node="${other}">${l.inferred ? icon('link') : orb()}<span>${esc(sentence)}</span></div>`);
        if (l.inferred) out.push(`<p class="ap-reason">${A.inferred}${esc(T.text(l.reason))}</p>`);
      }
    }
    if (c.placeCount) {
      out.push(`<i class="ap-div"></i><div class="ap-sec" data-sec="places">${SVG.quote}<span>${A.places(c.placeCount)}</span></div>`);
      for (const [source, kind, index, quote] of c.places) {
        const src = ATLAS.sources[source];
        const title = truncateMiddle(`${src.fromChat ? A.fromChat : ''}${T.source(src.title)} · ${kind} ${index}`, 240, '400 12.5px SUIT, sans-serif');
        out.push(`<div class="gl place">${SVG.quote}<span>${esc(title)}</span></div><p class="ap-quote">${esc(T.text(quote))}</p>`);
      }
      if (c.placeCount > c.places.length) out.push(`<p class="ap-more">${A.more(c.placeCount - c.places.length)}</p>`);
    }
  }
  const counts = ATLAS.units.map((_, u) => ATLAS.concepts.filter(c => c.unit === u).length);
  out.push(`${selected !== null ? '<i class="ap-div"></i>' : ''}<div class="ap-sec">${icon('list')}<span>${A.units(ATLAS.units.length)}</span></div>`);
  ATLAS.units.forEach((u, i) => out.push(`<div class="gl">${orb(unitColor(i))}<span>${esc(A.unitRow(T.unit(u), counts[i]))}</span></div>`));
  const order = ATLAS.concepts.map((c, i) => i).sort((a, b) => (b === selected) - (a === selected) || degreeOf(b) - degreeOf(a) || (ATLAS.concepts[a].name < ATLAS.concepts[b].name ? -1 : 1));
  out.push(`<i class="ap-div"></i><div class="ap-sec">${SVG.tag}<span>${A.concepts(ATLAS.concepts.length)}</span></div>`);
  for (const i of order.slice(0, 16)) out.push(`<div class="${i === selected ? 'gl sel' : 'ap-row'}">${orb()}<span>${esc(A.conceptRow(name(i), degreeOf(i)))}</span></div>`);
  return out.join('');
}

/* ---------- 프레임 ---------- */
function buildAtlasFrame(root, { S, lang, reduced }) {
  const { frame, wrap } = mountFrame(root, 'atlas', 'grid-atlas');
  const card = h('section', 'doc-card atlas-card');
  card.innerHTML = `<div class="doc-head"><div><h2 class="doc-title">${UI.map.title}</h2><p class="doc-sub">${UI.map.sub}</p></div><div class="doc-actions"><button class="btn off">${UI.del}</button><button class="btn primary off">${UI.save}</button></div></div>
    <div class="atlas-view"><canvas aria-hidden="true"></canvas>
      <div class="atlas-filter"><div class="combo atlas-search"><span class="typed"></span><span class="ph">${UI.map.search}</span><i class="arrow">${icon('chevDown')}</i></div><span class="combo-x">×</span><div class="combo atlas-rel"><span>${UI.map.relations}</span>${SVG.updown}</div></div>
      <span class="hint">${UI.map.hint}</span></div>
    <div class="doc-foot"><span class="atlas-status">${UI.map.footer}</span><button class="btn ghost">${UI.map.back}</button></div>`;
  const panel = h('aside', 'side-panel atlas-panel');
  panel.innerHTML = `<div class="tabs"><button class="tab on">${icon('folder')}<span>${UI.tabs.materials}</span></button><button class="tab">${icon('spark')}<span>${UI.tabs.ai}</span></button></div>
    <div class="ap-head">${SVG.network}<b>${UI.atlas.panelTitle}</b><span class="ap-tools"><i class="ap-tool" title="${UI.atlas.cleanup}">${SVG.broom}</i><i class="ap-tool primary" title="${UI.atlas.importFiles}">${SVG.importFile}</i></span></div>
    <div class="ap-scroll"><div class="ap-list"></div></div>`;
  frame.append(card, panel);
  const view = card.querySelector('.atlas-view'), canvas = view.querySelector('canvas');
  const vw = view.clientWidth, vh = view.clientHeight;
  const scale = parseFloat(wrap.dataset.scale) || 1;
  const T = translator(S.scenes.atlas);
  const engine = createAtlas(canvas, {
    concepts: ATLAS.concepts.map(c => ({ id: c.id, name: T.name(c.name), unit: c.unit })),
    links: ATLAS.links,
  }, { width: vw, height: vh, pixelRatio: Math.min(2.5, Math.max(1, scale * (window.devicePixelRatio || 1))), reduced });
  const list = panel.querySelector('.ap-list');
  const combo = card.querySelector('.atlas-search');
  return {
    frame, card, panel, view, canvas, vw, vh, engine, list, combo, T,
    clear: card.querySelector('.combo-x'),
    status: card.querySelector('.atlas-status'),
    typed: combo.querySelector('.typed'), ph: combo.querySelector('.ph'),
    toast: createKeyToast(frame, { reduced }),
    pointer: createPointer(frame, 0, 0),
    showPanel(selected) {
      list.innerHTML = panelMarkup(selected, { T, lang });
      list.style.transform = '';
      if (reduced) return;
      list.classList.add('swap');
      requestAnimationFrame(() => requestAnimationFrame(() => list.classList.remove('swap')));
    },
  };
}

/* ---------- 타임라인 ---------- */
export function atlasLoop(ctx) {
  const { root, tl, S, lang, reduced } = ctx;
  const f = buildAtlasFrame(root, { S, lang, reduced });
  const E = f.engine, P = f.pointer, A = S.scenes.atlas;
  const idx = name => ATLAS.concepts.findIndex(c => c.name === name);
  const lecture = idx(ATLAS_TOUR.lecture), question = idx(ATLAS_TOUR.question), next = idx(ATLAS_TOUR.next);
  const R = el => rectIn(f.frame, el);
  const viewAt = (fx, fy) => { const v = R(f.view); return [v.x + v.w * fx, v.y + v.h * fy]; };
  const nodeAt = i => { const v = R(f.view), p = E.screenOf(i); return p ? [v.x + p.x, v.y + p.y] : viewAt(.5, .5); };
  const center = el => () => { const r = R(el); return [r.x + r.w / 2, r.y + r.h / 2]; };
  const rowOf = selector => () => { const el = f.list.querySelector(selector); const r = el ? R(el) : R(f.list); return [r.x + Math.min(r.w * .45, 150), r.y + r.h / 2]; };
  const hot = selector => { f.list.querySelectorAll('.gl.hot').forEach(el => el.classList.remove('hot')); f.list.querySelector(selector)?.classList.add('hot'); };
  /** 개념 구체와 이름표에서 가장 먼 빈 자리. 더블클릭이 개념을 고르지 않게 한다 */
  const emptySpot = () => {
    const v = R(f.view);
    let best = viewAt(.2, .5), score = -1;
    for (let gx = .12; gx <= .88; gx += .04) for (let gy = .3; gy <= .82; gy += .04) {
      const x = v.w * gx, y = v.h * gy;
      let d = Infinity;
      for (let i = 0; i < ATLAS.concepts.length; i++) { const p = E.screenOf(i); if (p) d = Math.min(d, Math.hypot(p.x - x, p.y - y) - p.r, Math.hypot(p.x - x, p.y - 30 - y) - 24); }
      if (d > score) { score = d; best = [v.x + x, v.y + y]; }
    }
    return best;
  };

  // 포인터는 목표를 매 프레임 다시 읽는다. 지도가 움직이는 동안에도 개념 위에 정확히 멈춘다.
  let pos = viewAt(.8, .78);
  const place = () => { P.el.style.transform = `translate(${pos[0]}px,${pos[1]}px)`; };
  place();
  P.el.classList.add('hidden');
  const glide = (t, dur, target, ease = easeInOut) => {
    let from = null;
    return tl.during(t, dur, p => { if (!from) from = [...pos]; const to = target(); pos = [from[0] + (to[0] - from[0]) * p, from[1] + (to[1] - from[1]) * p]; place(); }, ease);
  };
  /** 빈 곳을 끌어 돌린다. 끈 거리만큼 앱과 같은 비율로 회전한다(graph.rs mouse_dragged) */
  const dragMap = (t, dur, dx, dy) => {
    let origin = null, prev = null;
    P.down(tl, t);
    tl.during(t, dur, p => {
      origin ||= [...pos];
      const now = [origin[0] + dx * p, origin[1] + dy * p];
      if (prev) E.orbit(now[0] - prev[0], now[1] - prev[1]);
      else if (p >= 1) E.orbit(dx, dy);
      prev = now; pos = now; place();
    }, easeInOut);
    P.up(tl, t + dur);
    return t + dur;
  };
  const click = t => P.click(tl, t);
  const setStatus = text => { f.status.textContent = text; };
  const setQuery = text => { f.typed.textContent = text; f.ph.classList.toggle('off', !!text); E.search(text); };

  // 0. 지도가 열린다: 개념이 가운데서 펼쳐진다
  tl.at(0, () => { f.showPanel(null); E.enter(); });
  tl.at(650, () => P.el.classList.remove('hidden'));

  // 1. 빈 곳을 끌어 3D로 돌려 본다. 회전량은 다음에 여는 개념 주변을 가리는 구체가 없도록 골랐다
  let t = glide(700, 550, () => viewAt(.68, .58));
  t = dragMap(t + 120, 1600, ...ATLAS_DRAGS.lecture);

  // 2. 개념에 올려 이웃을 보고 눌러 연다. 오른쪽 패널에 설명·연결·배운 곳이 열린다
  t = glide(t + 350, 950, () => nodeAt(lecture));
  tl.at(t, () => E.hover(lecture));
  t = click(t + 300);
  tl.at(t, () => { E.select(lecture); f.showPanel(lecture); });

  // 3. 패널을 내려 배운 곳(강의자료 쪽수와 인용)을 확인한다
  t = glide(t + 1000, 700, () => { const r = R(f.list.parentElement); return [r.x + r.w * .5, r.y + r.h * .55]; });
  let shift = 0;
  t = tl.during(t + 150, 1000, p => {
    if (!shift) {
      const sec = f.list.querySelector('[data-sec="places"]'), box = f.list.parentElement;
      shift = sec ? Math.max(1, Math.min(sec.offsetTop - 18, f.list.scrollHeight - box.clientHeight)) : 1;
    }
    f.list.style.transform = `translateY(${-shift * p}px)`;
  }, easeInOut);
  t = glide(t + 200, 600, rowOf('.gl.place'));
  tl.at(t, () => hot('.gl.place'));

  // 4. 빈 곳을 더블클릭해 전체 보기로 돌아온다(카메라도 기본 각도로)
  let spot = null;
  t = glide(t + 1000, 800, () => (spot ||= emptySpot()));
  click(t + 80);
  tl.at(t + 80, () => { E.clear(); f.showPanel(null); });
  t = click(t + 230);

  // 5. 다시 돌린 뒤 개념 검색에 입력하고 Enter: 이름이 맞는 개념이 표시되고 첫 결과가 열린다
  t = glide(t + 700, 600, () => viewAt(.2, .36));
  t = dragMap(t + 120, 1600, ...ATLAS_DRAGS.question);
  t = glide(t + 350, 650, center(f.combo));
  t = click(t + 60);
  tl.at(t, () => f.combo.classList.add('focus'));
  const chars = [...A.query], speed = lang === 'en' ? 70 : 150;
  chars.forEach((_, i) => tl.at(t + 250 + i * speed, () => { setQuery(chars.slice(0, i + 1).join('')); setStatus(UI.atlas.searching); }));
  t += 250 + chars.length * speed + 800;
  f.toast.key(tl, t, ['Enter'], 900);
  tl.at(t + 60, () => { f.combo.classList.remove('focus'); E.select(question); f.showPanel(question); });

  // 6. 내 질문에서 배운 곳을 보고, 연결 문장을 눌러 이어진 개념으로 넘어간다
  t = glide(t + 1100, 750, rowOf('.gl.place'));
  tl.at(t, () => hot('.gl.place'));
  t = glide(t + 800, 600, rowOf(`.gl[data-node="${next}"]`));
  tl.at(t, () => hot(`.gl[data-node="${next}"]`));
  t = click(t + 250);
  tl.at(t, () => { E.select(next); f.showPanel(next); });

  // 지도·이름표의 움직임은 장면이 끝날 때까지 매 프레임 진행한다
  const END = t + 2600;
  let clock = 0;
  tl.during(0, END, p => { const now = p * END / 1000; E.tick(now - clock); clock = now; }, linear);
  if (reduced) tl.at(END, () => E.settle());
  return { hold: 0, gap: 900 };
}
