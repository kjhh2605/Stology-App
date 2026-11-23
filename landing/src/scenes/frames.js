// 실제 앱 화면을 재현한 프레임. 값은 app-tokens.css(theme.rs)와 기준 이미지(app-overview.jpg, pdf-overview.jpg)를 따른다.
import { UI } from './ui-labels.js';
import { createKeyToast, createPointer, fitFrame } from './runtime.js';

export const FRAME = { document: [720, 450], pdf: [720, 450], map: [720, 450], window: [1120, 700] };

export const esc = v => String(v ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
export const h = (tag, cls, html) => { const e = document.createElement(tag); if (cls) e.className = cls; if (html != null) e.innerHTML = html; return e; };

const I = {
  folder: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
  file: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/></svg>',
  pdf: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M6 3h8l4 4v14H6z"/><path d="M9 13h6M9 17h4"/></svg>',
  chev: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>',
  chevDown: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M6 9l6 6 6-6"/></svg>',
  plus: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="12" cy="12" r="9"/><path d="M12 8v8M8 12h8"/></svg>',
  book: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M5 4h11a3 3 0 0 1 3 3v13H8a3 3 0 0 0-3 3z"/><path d="M5 4v16a3 3 0 0 1 3-3h11"/></svg>',
  person: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/></svg>',
  archive: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M4 7h16v3H4zM5 10v10h14V10M10 14h4"/></svg>',
  search: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><circle cx="11" cy="11" r="6"/><path d="M20 20l-4-4"/></svg>',
  pencil: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M4 20l4-1L19 8l-3-3L5 16z"/></svg>',
  spark: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 2l1.8 5.6L19 9l-5.2 1.6L12 16l-1.8-5.4L5 9l5.2-1.4zM5 15l.9 2.6L8.5 18l-2.6.8L5 21l-.9-2.2L1.5 18l2.6-.4z"/></svg>',
  list: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01"/></svg>',
  newchat: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M4 5h16v11H9l-5 4z"/><path d="M12 8v5M9.5 10.5h5"/></svg>',
  up: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V6M6 12l6-6 6 6"/></svg>',
  note: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M6 3h9l4 4v14H6z"/><path d="M9 12h7M9 16h5"/></svg>',
  link: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round"><path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1.5 1.5"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1.5-1.5"/></svg>',
  finder: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="3"/><path d="M12 4v16M8 9v2M16 9v2M7 15c1.5 1.3 3 2 5 2s3.5-.7 5-2"/></svg>',
  // PDF 도구(pdf_annotations.rs 순서)
  read: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M6 3h8l4 4v14H6z"/><path d="M9 11h6M9 15h6"/></svg>',
  select: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M6 3l12 10-5 .5 3 6.5-2.4 1.1-3-6.6L6 18z"/></svg>',
  'area-select': '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-dasharray="3 2.5"><rect x="4" y="5" width="16" height="14" rx="1.5"/></svg>',
  highlight: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M14 4l6 6-8 8H8l-2-2z"/><path d="M4 20h16"/></svg>',
  area: '<svg viewBox="0 0 24 24" fill="currentColor" opacity=".9"><rect x="4" y="6" width="16" height="12" rx="2"/></svg>',
  box: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="4" y="6" width="16" height="12" rx="2"/></svg>',
  bubble: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M4 5h16v11H10l-5 4z"/><path d="M8 9h8M8 12h5"/></svg>',
  pen: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M12 3l3 3-9 12-4 1 1-4z"/><path d="M12 3l5-1 1 5"/></svg>',
  undo: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M9 7L4 12l5 5"/><path d="M4 12h10a5 5 0 0 1 0 10h-2"/></svg>',
  redo: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M15 7l5 5-5 5"/><path d="M20 12H10a5 5 0 0 0 0 10h2"/></svg>',
  trash: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/></svg>',
};
export const icon = name => I[name] || '';

/* ---------- 프레임 ---------- */
export function mountFrame(root, kind, extraClass = '') {
  const [w, hgt] = FRAME[kind];
  const wrap = h('div', 'app-frame-wrap');
  const frame = h('div', `app-frame app-frame-${kind} ${extraClass}`);
  frame.style.width = w + 'px'; frame.style.height = hgt + 'px';
  wrap.appendChild(frame); root.appendChild(wrap);
  const stop = fitFrame(wrap, frame, w, hgt);
  (root._cleanup ||= []).push(stop);
  return { wrap, frame, w, h: hgt };
}

/* ---------- 본문 줄 ---------- */
export function makeLine(text, kind = 'p', { typed = false } = {}) {
  const el = h('div', `ln ${kind}`), pre = h('span', 'pre'), tx = h('span', 'tx');
  el.append(pre, tx);
  const chars = [...text].map(c => { const s = h('span', 'ch' + (typed ? '' : ' on')); s.textContent = c; tx.appendChild(s); return s; });
  const caret = h('i', 'caret');
  const line = {
    el, pre, tx, chars, caret, kind, text,
    setKind(k, opts = {}) {
      el.className = `ln ${k}`; line.kind = k;
      pre.innerHTML = k === 'ul' ? '<i class="dot"></i>' : k === 'ol' ? `<span class="num">${opts.n || 1}.</span>` : k === 'check' ? `<i class="cb${opts.checked ? ' checked' : ''}"></i>` : k === 'code' ? `<span class="lang">${esc(opts.lang || '')}</span>` : '';
      el.style.marginLeft = opts.indent ? `${opts.indent * 22}px` : '';
    },
    check(on = true) { pre.querySelector('.cb')?.classList.toggle('checked', on); },
    caretOn() { tx.appendChild(caret); }, caretOff() { caret.remove(); },
    showAll() { chars.forEach(c => c.classList.add('on')); },
    hideRange(from, to) { for (let i = from; i < to; i++) chars[i].classList.remove('on'); },
    range(sub) { const i = text.indexOf(sub); return i < 0 ? [0, chars.length] : [i, i + [...sub].length]; },
  };
  line.setKind(kind);
  return line;
}

/* ---------- 왼쪽 사이드바 ---------- */
export function buildLeftBar(S, { connected = false } = {}) {
  const el = h('aside', 'left-bar');
  el.innerHTML = `<div class="lb-head">${UI.workspaceHeader}</div><div class="lb-add">${icon('plus')}<span>${UI.addWorkspace}</span></div>` +
    S.scenes.subjects.map((s, i) => `<div class="lb-row${i === 0 ? ' selected' : ''}">${icon('book')}<span>${esc(s)}</span></div>`).join('') +
    `<div class="lb-foot"><div class="lb-connect">${icon('person')}<span>${UI.connect}</span><i class="dot${connected ? ' on' : ''}"></i></div><div class="lb-hint">${connected ? UI.connectedHint : UI.connectHint}</div><div class="lb-local">${UI.localNote}</div></div>`;
  return el;
}

/* ---------- 오른쪽 사이드패널(자료 / AI) ---------- */
export function buildSidePanel(S, { mode = 'materials', selected = 'note2', dot = false, draft = true } = {}) {
  const el = h('aside', 'side-panel');
  const F = S.scenes.files;
  const row = (key, name, { lvl = 1, ico = 'file', badge = '' } = {}) => `<div class="tree-row lvl${lvl}${selected === key ? ' selected' : ''}" data-file="${key}">${icon(ico)}<span class="name">${esc(name)}</span>${badge ? `<span class="badge ${badge.toLowerCase()}">${badge}</span>` : ''}</div>`;
  el.innerHTML = `
    <div class="tabs"><button class="tab tab-materials${mode === 'materials' ? ' on' : ''}" data-tab="materials">${icon('folder')}<span>${UI.tabs.materials}</span>${dot ? '<i class="ndot"></i>' : ''}</button><button class="tab tab-ai${mode === 'ai' ? ' on' : ''}" data-tab="ai">${icon('spark')}<span>${UI.tabs.ai}</span></button></div>
    <div class="panel-body body-materials${mode === 'materials' ? ' on' : ''}">
      <div class="sp-section">${icon('archive')}<span>${UI.archive}</span></div>
      <div class="sp-search">${icon('search')}<span>${UI.search}</span><span class="spark">${icon('spark')}</span></div>
      <div class="tree">
        <div class="tree-row lvl0" data-file="root"><span class="chev">${icon('chevDown')}</span>${icon('folder')}<span class="name">${UI.root}</span></div>
        ${row('note0', F.notes[0])}${row('note1', F.notes[1])}${row('note2', F.notes[2])}${row('pdf', S.scenes.pdf.file, { ico: 'pdf', badge: 'PDF' })}${row('pptx', F.pptx, { ico: 'file', badge: 'PPTX' })}
      </div>
      <div class="sp-sugg" hidden><div class="sp-section" style="margin:0 4px 2px">${icon('spark')}<span>${UI.chat.suggestions}</span></div></div>
      <div class="sp-divider"></div>
      <div class="sp-section">${icon('pencil')}<span>${UI.drafts}</span></div>
      ${draft ? `<div class="sp-draft">${icon('note')}<span>${esc(S.scenes.draft)}</span></div>` : ''}
    </div>
    <div class="panel-body body-ai${mode === 'ai' ? ' on' : ''}">
      <div class="ai-top"><span class="ico">${icon('list')}</span><div class="sp-search">${icon('search')}<span>${UI.chat.searchChats}</span></div><button class="newchat">${icon('newchat')}<span>${UI.chat.newChat}</span></button></div>
      <div class="ai-msgs"></div>
      <div class="ai-input"><div class="field"><span class="ph">${UI.chat.placeholder}</span></div><div class="row"><span class="chip">${UI.chat.provider}</span><button class="send" aria-label="${UI.chat.send}">${icon('up')}</button></div></div>
      <div class="ai-hint">${UI.chat.hint}</div>
    </div>`;
  const api = {
    el,
    rows: Object.fromEntries([...el.querySelectorAll('.tree-row')].map(r => [r.dataset.file, r])),
    tabs: { materials: el.querySelector('.tab-materials'), ai: el.querySelector('.tab-ai') },
    switchTo(next) {
      ['materials', 'ai'].forEach(m => { el.querySelector(`.tab-${m}`).classList.toggle('on', m === next); el.querySelector(`.body-${m}`).classList.toggle('on', m === next); });
    },
    select(key) { el.querySelectorAll('.tree-row').forEach(r => r.classList.toggle('selected', r.dataset.file === key)); },
    showDot(on = true) { let d = el.querySelector('.tab-materials .ndot'); if (on && !d) { d = h('i', 'ndot'); el.querySelector('.tab-materials').appendChild(d); } if (!on) d?.remove(); },
    addSuggestion(title, meta) {
      const box = el.querySelector('.sp-sugg'); box.hidden = false;
      const sg = h('div', 'sg', `${icon('spark')}<span>${esc(title)}</span><small>${esc(meta)}</small>`); box.appendChild(sg);
      requestAnimationFrame(() => sg.classList.add('on')); return sg;
    },
    ai: {
      msgs: el.querySelector('.ai-msgs'), field: el.querySelector('.ai-input .field'), ph: el.querySelector('.ai-input .ph'), send: el.querySelector('.ai-input .send'),
      typeTarget(text) { const chars = [...text].map(c => { const s = h('span', 'ch'); s.textContent = c; return s; }); const f = el.querySelector('.ai-input .field'); chars.forEach(c => f.appendChild(c)); return chars; },
      clearField() { el.querySelectorAll('.ai-input .field .ch').forEach(c => c.remove()); el.querySelector('.ai-input .ph').classList.remove('off'); el.querySelector('.ai-input .send').classList.remove('on'); },
      userMsg(text) { const m = h('div', 'msg user'); m.textContent = text; el.querySelector('.ai-msgs').appendChild(m); requestAnimationFrame(() => m.classList.add('on')); return m; },
      aiMsg(lines) {
        const m = h('div', 'msg ai', `<div class="who"><i></i>${UI.chat.provider}</div>`);
        const chars = lines.map(line => { const l = h('span', 'line'); const cs = [...line].map(c => { const s = h('span', 'ch'); s.textContent = c; l.appendChild(s); return s; }); m.appendChild(l); return cs; });
        const acts = h('div', 'acts', `<button>${icon('note')}<span>${UI.chat.toNote}</span></button>`); m.appendChild(acts);
        el.querySelector('.ai-msgs').appendChild(m); requestAnimationFrame(() => m.classList.add('on'));
        return { el: m, chars, acts, button: acts.querySelector('button') };
      },
    },
  };
  return api;
}

/* ---------- 문서 카드 ---------- */
export function buildDocCard(S, { title, lines = [] } = {}) {
  const el = h('section', 'doc-card');
  el.innerHTML = `<div class="doc-head"><div><h2 class="doc-title">${esc(title ?? S.scenes.doc.title)} <span class="doc-meta">${UI.docMeta}</span></h2><p class="doc-sub">${UI.docSub}</p></div><div class="doc-actions"><button class="btn">${UI.del}</button><button class="btn primary">${UI.save}</button></div></div><div class="doc-body"></div><div class="doc-foot"><span>${UI.local}</span><button class="btn ghost">${UI.openMap}</button></div>`;
  const body = el.querySelector('.doc-body');
  const made = lines.map(spec => { const l = makeLine(spec.text, spec.kind || 'p', { typed: !!spec.typed }); if (spec.opts) l.setKind(spec.kind || 'p', spec.opts); body.appendChild(l.el); return l; });
  return { el, body, lines: made, addLine(text, kind = 'p', opts = {}) { const l = makeLine(text, kind, opts); if (opts.kindOpts) l.setKind(kind, opts.kindOpts); body.appendChild(l.el); return l; } };
}

/* ---------- PDF 카드 ---------- */
export function buildPdfCard(S, { title, tool = 'read', content = 'pdf' } = {}) {
  const P = S.scenes.pdf;
  const el = h('section', 'doc-card pdf-card');
  const tools = UI.tools.map((t, i) => `${i === 2 || i === 8 ? '<span class="tool sep"></span>' : ''}<button class="tool${t.id === tool ? ' on' : ''}" data-tool="${t.id}" title="${t.label}">${icon(t.id === 'highlight' ? 'highlight' : t.id === 'note' ? 'bubble' : t.id)}</button>`).join('');
  const body = content === 'pptx'
    ? `<div class="course">${esc(P.course)}</div><div class="pdf-h1">${esc(P.pptxTitle)}</div><div class="pdf-sub">${esc(P.pptxSub)}</div><hr><div class="pdf-h2">${esc(P.h1)}</div><div class="pdf-p" data-p="1">${esc(P.p1)}</div><div class="pdf-p" data-p="2"><span class="hl">${esc(P.p2)}</span></div>`
    : `<div class="course">${esc(P.course)}</div><div class="pdf-h1">${esc(P.title)}</div><div class="pdf-sub">${esc(P.sub)}</div><hr><div class="pdf-h2" data-h="1">${esc(P.h1)}</div><div class="pdf-p" data-p="1">${esc(P.p1)}</div><div class="pdf-p" data-p="2"><span class="hl">${esc(P.p2)}</span></div><div class="pdf-h2" data-h="2">${esc(P.h2)}</div>${P.items.map((it, i) => `<div class="pdf-li" data-li="${i}"><span class="n">0${i + 1}</span><span class="t">${esc(it)}</span></div>`).join('')}<div class="pdf-tail">${esc(P.tail)}</div>`;
  el.innerHTML = `<div class="doc-head"><div><h2 class="doc-title">${esc(title ?? P.file)} <span class="doc-meta">${UI.docMeta}</span></h2><p class="doc-sub">${UI.pdfSub}</p></div><div class="doc-actions"><button class="btn">${UI.del}</button><button class="btn primary">${UI.save}</button></div></div>
    <div class="pdf-toolbar">${tools}</div><div class="pdf-status"><i></i><span>${UI.pdfSaved}</span></div>
    <div class="pdf-scroll"><div class="pdf-page">${body}<div class="anno-layer"><svg></svg></div></div><span class="bar"></span></div>
    <div class="doc-foot"><span class="foot-status">${UI.pdfSaved}</span><button class="btn ghost">${UI.openMap}</button></div>`;
  const page = el.querySelector('.pdf-page'), layer = el.querySelector('.anno-layer'), status = el.querySelector('.pdf-status');
  return {
    el, page, layer, svg: layer.querySelector('svg'),
    els: { p1: el.querySelector('[data-p="1"]'), p2: el.querySelector('[data-p="2"] .hl'), p2wrap: el.querySelector('[data-p="2"]'), h1: el.querySelector('[data-h="1"]'), h2: el.querySelector('[data-h="2"]'), li: [...el.querySelectorAll('.pdf-li')] },
    toolbar: { set(id) { el.querySelectorAll('.tool').forEach(b => b.classList.toggle('on', b.dataset.tool === id)); }, button(id) { return el.querySelector(`.tool[data-tool="${id}"]`); } },
    setStatus(text, busy = false) { status.querySelector('span').textContent = text; status.classList.toggle('busy', busy); el.querySelector('.foot-status').textContent = text; },
  };
}

/* ---------- 조립: 문서 프레임 / PDF 프레임 / 지도 프레임 / 창 전체 ---------- */
export function buildDocumentFrame(root, { S, reduced, title, lines = [], selected = 'note2' }) {
  const { frame } = mountFrame(root, 'document', 'grid-doc');
  const doc = buildDocCard(S, { title, lines });
  const panel = buildSidePanel(S, { selected });
  frame.append(doc.el, panel.el);
  const toast = createKeyToast(frame, { reduced });
  const pointer = createPointer(frame, 420, 300); pointer.el.classList.add('hidden');
  return { frame, doc, panel, toast, pointer };
}

export function buildPdfFrame(root, { S, reduced, tool = 'read', selected = 'pdf', content = 'pdf', title }) {
  const { frame } = mountFrame(root, 'pdf', 'grid-pdf');
  const pdf = buildPdfCard(S, { tool, content, title });
  const panel = buildSidePanel(S, { selected });
  frame.append(pdf.el, panel.el);
  const toast = createKeyToast(frame, { reduced });
  const pointer = createPointer(frame, 300, 240);
  return { frame, pdf, panel, toast, pointer };
}

export function buildMapFrame(root, { S, reduced }) {
  const { frame } = mountFrame(root, 'map', 'grid-map');
  const M = S.scenes.map;
  const card = h('section', 'doc-card map-card');
  card.innerHTML = `<div class="doc-head"><div><h2 class="doc-title">${UI.map.title}</h2><p class="doc-sub">${UI.map.sub}</p></div><div class="doc-actions"><button class="btn">${UI.del}</button><button class="btn primary off">${UI.save}</button></div></div>
    <div class="map-filter"><div class="combo"><span class="ph">${UI.map.search}</span>${icon('chevDown')}<div class="sugg">${M.suggestions.map(s => `<div>${esc(s)}</div>`).join('')}</div></div><span class="combo-x">×</span><div class="combo" style="min-width:150px"><span>${UI.map.relations}</span>${icon('chevDown')}</div></div>
    <div class="map-view"><img src="/media/map-graph-crop.jpg" alt="" width="515" height="505"><span class="glow"></span><span class="hint">${UI.map.hint}</span></div>
    <div class="doc-foot"><span>${UI.map.footer}</span><button class="btn ghost">${UI.map.back}</button></div>`;
  frame.append(card);
  const combo = card.querySelector('.combo');
  const toast = createKeyToast(frame, { reduced });
  const pointer = createPointer(frame, 400, 300); pointer.el.classList.add('hidden');
  return {
    frame, card, combo, sugg: card.querySelector('.sugg'), glow: card.querySelector('.glow'), toast, pointer,
    focus(on = true) { combo.classList.toggle('focus', on); },
    typeTarget(text) { const ph = combo.querySelector('.ph'); const wrap = h('span', 'typed'); ph.before(wrap); const chars = [...text].map(c => { const s = h('span', 'ch'); s.textContent = c; wrap.appendChild(s); return s; }); return { chars, ph }; },
  };
}

export function buildWindowFrame(root, { S, reduced, connected = true }) {
  const { frame } = mountFrame(root, 'window', 'app-window');
  const left = buildLeftBar(S, { connected });
  const center = h('div', 'win-center');
  center.style.cssText = 'min-width:0;display:flex;flex-direction:column';
  const doc = buildDocCard(S, { lines: [{ text: S.scenes.doc.h2, kind: 'h2' }, { text: S.scenes.doc.line1 }, { text: S.scenes.doc.target }] });
  doc.el.style.flex = '1';
  center.append(doc.el);
  const panel = buildSidePanel(S, { selected: 'note2' });
  frame.append(left, center, panel.el);
  const toast = createKeyToast(frame, { reduced });
  const pointer = createPointer(frame, 560, 360);
  let pdf = null;
  return {
    frame, left, center, panel, toast, pointer, doc,
    showPdf(opts = {}) { pdf = buildPdfCard(S, opts); pdf.el.style.flex = '1'; center.replaceChildren(pdf.el); return pdf; },
    showDoc(lines) { const d = buildDocCard(S, { lines }); d.el.style.flex = '1'; center.replaceChildren(d.el); return d; },
    get pdf() { return pdf; },
  };
}

/* ---------- 겹쳐 뜨는 요소 ---------- */
export function contextMenu(frame, x, y, items) {
  const el = h('div', 'ctx-menu');
  el.style.left = x + 'px'; el.style.top = y + 'px';
  el.innerHTML = items.map(it => `<div class="mi${it.disabled ? ' off' : ''}" data-mi="${it.id || ''}">${icon(it.icon)}<span>${esc(it.label)}</span></div>`).join('');
  frame.appendChild(el);
  return { el, items: Object.fromEntries([...el.querySelectorAll('.mi')].map(m => [m.dataset.mi, m])), show() { el.classList.add('on'); }, hide() { el.classList.remove('on'); }, hot(id) { el.querySelectorAll('.mi').forEach(m => m.classList.toggle('hot', m.dataset.mi === id)); } };
}
export function analysisDialog(frame, name, provider = UI.chat.provider) {
  const el = h('div', 'app-dialog', `<img src="/mark.png" alt=""><h4>${UI.analysis.title}</h4><p>${esc(UI.analysis.body(name, provider))}</p><div class="btns"><button>${UI.analysis.cancel}</button><button class="start">${UI.analysis.start}</button></div>`);
  frame.appendChild(el);
  return { el, start: el.querySelector('.start'), show() { el.classList.add('on'); }, hide() { el.classList.remove('on'); } };
}
export function menubar(frame) {
  const bar = h('div', 'menubar', `<span class="apple"></span><span class="app">Stology Local</span><span data-m="file">파일</span><span>편집</span><span>보기</span><span>윈도우</span><span>도움말</span>`);
  frame.appendChild(bar);
  const file = bar.querySelector('[data-m="file"]');
  const drop = h('div', 'menu-drop', `<div class="mi" data-mi="new"><span>새 노트</span><small>⌘N</small></div><div class="mi" data-mi="save"><span>저장</span><small>⌘S</small></div><hr><div class="mi" data-mi="share"><span>${UI.ctx.share}</span></div><div class="mi" data-mi="fix"><span>PDF 저장 문제 해결…</span></div>`);
  frame.appendChild(drop);
  const place = () => { drop.style.left = (file.offsetLeft) + 'px'; };
  place();
  return { bar, file, drop, open() { place(); file.classList.add('open'); drop.classList.add('on'); }, close() { file.classList.remove('open'); drop.classList.remove('on'); }, hot(id) { drop.querySelectorAll('.mi').forEach(m => m.classList.toggle('hot', m.dataset.mi === id)); }, item(id) { return drop.querySelector(`[data-mi="${id}"]`); } };
}
export function sharePopover(frame, x, y, fileName) {
  const el = h('div', 'share-pop', `<div class="file">${icon('pdf')}<span>${esc(fileName)}</span></div><div class="hd">공유</div><div class="row"><i></i><span>AirDrop</span></div><div class="row"><i class="b"></i><span>메시지</span></div><div class="row"><i class="c"></i><span>Mail</span></div><div class="row"><i class="d"></i><span>메모</span></div>`);
  el.style.left = x + 'px'; el.style.top = y + 'px';
  frame.appendChild(el);
  return { el, show() { el.classList.add('on'); }, hide() { el.classList.remove('on'); } };
}
export function linkPopover(frame, x, y) {
  const el = h('div', 'link-pop', `${icon('link')}<span class="ph">${UI.link.placeholder}</span>`);
  el.style.left = x + 'px'; el.style.top = y + 'px';
  frame.appendChild(el);
  return { el, ph: el.querySelector('.ph'), show() { el.classList.add('on'); }, hide() { el.classList.remove('on'); }, typeTarget(text) { const chars = [...text].map(c => { const s = h('span', 'ch'); s.textContent = c; el.appendChild(s); return s; }); return chars; } };
}
export function flash(host, text) {
  const el = h('div', 'flash'); el.textContent = text; host.appendChild(el);
  return { el, show() { el.classList.add('on'); }, hide() { el.classList.remove('on'); } };
}

/* ---------- PDF 필기 요소 ---------- */
export function setHighlight(el, p) { el.style.setProperty('--hl', `${Math.round(p * 100)}%`); }
export function addAnno(layer, cls, x, y, w, hgt) {
  const el = h('div', `anno ${cls}`, '<i class="handle tl"></i><i class="handle tr"></i><i class="handle bl"></i><i class="handle br"></i>');
  Object.assign(el.style, { left: x + 'px', top: y + 'px', width: w + 'px', height: hgt + 'px' });
  layer.appendChild(el);
  return el;
}
export function setRect(el, x, y, w, hgt) { Object.assign(el.style, { left: x + 'px', top: y + 'px', width: Math.max(0, w) + 'px', height: Math.max(0, hgt) + 'px' }); }
export function addNote(layer, x, y, text, { typed = true, width = 170 } = {}) {
  const el = h('div', 'anno note', '<i class="handle tl"></i><i class="handle tr"></i><i class="handle bl"></i><i class="handle br"></i>');
  Object.assign(el.style, { left: x + 'px', top: y + 'px', width: width + 'px' });
  const tx = h('span', 'tx');
  const chars = [...text].map(c => { const s = h('span', 'ch' + (typed ? '' : ' on')); s.textContent = c; tx.appendChild(s); return s; });
  const caret = h('i', 'caret');
  el.appendChild(tx); layer.appendChild(el);
  return { el, tx, chars, caret, caretOn() { tx.appendChild(caret); }, caretOff() { caret.remove(); }, append(text) { const more = [...text].map(c => { const s = h('span', 'ch'); s.textContent = c; tx.insertBefore(s, caret.parentNode === tx ? caret : null); return s; }); return more; } };
}
export function addPath(svg, d, { width = 2 } = {}) {
  const p = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  p.setAttribute('d', d); p.style.strokeWidth = width; svg.appendChild(p);
  const len = p.getTotalLength ? p.getTotalLength() : 300;
  p.style.strokeDasharray = `${len}`; p.style.strokeDashoffset = `${len}`;
  return { el: p, len, draw(pct) { p.style.strokeDashoffset = `${len * (1 - pct)}`; } };
}
export function addMarquee(layer) { const el = h('div', 'marquee'); layer.appendChild(el); return el; }
