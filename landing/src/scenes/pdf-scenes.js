// PDF 필기 장면 13개. 실제 PDF 도구 막대·페이지·자료 패널을 재현한 프레임 위에서 포인터가 도구를 고르고 움직인다.
import { typeChars, rectIn } from './runtime.js';
import { buildPdfFrame, setHighlight, addAnno, setRect, addNote, addPath, addMarquee, contextMenu, analysisDialog, menubar, sharePopover } from './frames.js';
import { UI } from './ui-labels.js';

const speed = lang => (lang === 'ko' ? 55 : 35);
const pageRect = (f, el) => rectIn(f.pdf.page, el);
const frameRect = (f, el) => rectIn(f.frame, el);
const center = r => [r.x + r.w / 2, r.y + r.h / 2];
/** 페이지 좌표 → 프레임 좌표 (포인터용) */
const toFrame = (f, x, y) => { const p = rectIn(f.frame, f.pdf.page); return [p.x + x, p.y + y]; };
/** 메모 자리: 페이지 오른쪽 여백(페이지 폭 기준) */
const noteX = f => f.pdf.page.clientWidth - 172;
const NOTE_W = 152;

function pickTool(f, tl, t, id) {
  const r = frameRect(f, f.pdf.toolbar.button(id));
  f.pointer.moveTo(tl, t, r.x + 14, r.y + 14, 450);
  f.pointer.click(tl, t + 480);
  tl.at(t + 520, () => f.pdf.toolbar.set(id));
  return t + 700;
}
function saveStatus(f, tl, t) {
  tl.at(t, () => f.pdf.setStatus(UI.pdfSaving, true));
  tl.at(t + 700, () => f.pdf.setStatus(UI.pdfSaved));
  return t + 700;
}
const open = (ctx, opts = {}) => buildPdfFrame(ctx.root, { S: ctx.S, reduced: ctx.reduced, ...opts });

/** 기존 필기(강조 + 메모 + 박스)가 있는 상태 */
function withExisting(ctx, which = ['hl', 'note', 'box']) {
  const f = open(ctx, { tool: 'select' });
  const P = ctx.S.scenes.pdf, out = {};
  if (which.includes('hl')) { setHighlight(f.pdf.els.p2, 1); out.hl = f.pdf.els.p2; }
  if (which.includes('note')) { const r = pageRect(f, f.pdf.els.li[0]); out.note = addNote(f.pdf.layer, noteX(f), r.y - 4, P.note, { typed: false, width: NOTE_W }); }
  if (which.includes('box')) { const a = pageRect(f, f.pdf.els.h2), b = pageRect(f, f.pdf.els.li[2]); out.box = addAnno(f.pdf.layer, 'box', a.x - 8, a.y - 4, 300, b.y + b.h - a.y + 8); }
  if (which.includes('pen')) { const r = pageRect(f, f.pdf.els.li[0]); out.pen = addPath(f.pdf.svg, `M${r.x + 26} ${r.y + r.h - 2} C ${r.x + 90} ${r.y + r.h + 3}, ${r.x + 160} ${r.y + r.h - 4}, ${r.x + 236} ${r.y + r.h - 1}`); out.pen.draw(1); }
  return { f, ...out };
}

export const pdfScenes = {
  highlight: ctx => {
    const { tl } = ctx; const f = open(ctx);
    let t = pickTool(f, tl, 300, 'highlight');
    const r = pageRect(f, f.pdf.els.p2);
    const [sx, sy] = toFrame(f, r.x - 2, r.y + r.h / 2), [ex] = toFrame(f, r.x + r.w + 2, 0);
    t = f.pointer.moveTo(tl, t, sx, sy, 500);
    t = f.pointer.drag(tl, t + 150, [sx, sy], [ex, sy], 1100, p => setHighlight(f.pdf.els.p2, p));
    saveStatus(f, tl, t + 100);
    return { hold: 1700 };
  },

  'pdf-area-highlight': ctx => {
    const { tl } = ctx; const f = open(ctx);
    let t = pickTool(f, tl, 300, 'area');
    const a = pageRect(f, f.pdf.els.li[0]), b = pageRect(f, f.pdf.els.li[2]);
    const x0 = a.x - 6, y0 = a.y - 4, x1 = a.x + 250, y1 = b.y + b.h + 4;
    const [sx, sy] = toFrame(f, x0, y0), [ex, ey] = toFrame(f, x1, y1);
    const box = addAnno(f.pdf.layer, 'area', x0, y0, 0, 0);
    t = f.pointer.moveTo(tl, t, sx, sy, 500);
    t = f.pointer.drag(tl, t + 150, [sx, sy], [ex, ey], 1000, p => setRect(box, x0, y0, (x1 - x0) * p, (y1 - y0) * p));
    saveStatus(f, tl, t + 100);
    return { hold: 1700 };
  },

  'pdf-box': ctx => {
    const { tl } = ctx; const f = open(ctx);
    let t = pickTool(f, tl, 300, 'box');
    const a = pageRect(f, f.pdf.els.h1), b = pageRect(f, f.pdf.els.p2wrap);
    const x0 = a.x - 10, y0 = a.y - 6, x1 = a.x + 310, y1 = b.y + b.h + 6;
    const [sx, sy] = toFrame(f, x0, y0), [ex, ey] = toFrame(f, x1, y1);
    const box = addAnno(f.pdf.layer, 'box', x0, y0, 0, 0);
    t = f.pointer.moveTo(tl, t, sx, sy, 500);
    t = f.pointer.drag(tl, t + 150, [sx, sy], [ex, ey], 1000, p => setRect(box, x0, y0, (x1 - x0) * p, (y1 - y0) * p));
    saveStatus(f, tl, t + 100);
    return { hold: 1700 };
  },

  'text-note': ctx => {
    const { tl, S, lang } = ctx; const f = open(ctx);
    let t = pickTool(f, tl, 300, 'note');
    const r = pageRect(f, f.pdf.els.li[0]);
    const nx = noteX(f), ny = r.y - 2;
    const [px, py] = toFrame(f, nx, ny + 8);
    t = f.pointer.moveTo(tl, t, px, py, 550);
    t = f.pointer.click(tl, t + 100);
    let note;
    tl.at(t, () => { note = addNote(f.pdf.layer, nx, ny, S.scenes.pdf.note, { width: NOTE_W }); note.caretOn(); });
    const chars = []; // 글자 span은 생성 시점에 만들어지므로 간접 참조
    tl.at(t + 10, () => chars.push(...note.chars));
    const n = [...S.scenes.pdf.note].length, sp = speed(lang);
    for (let i = 0; i < n; i++) tl.at(t + 350 + i * sp, () => chars[i]?.classList.add('on'));
    const end = t + 350 + n * sp;
    tl.at(end + 300, () => note.caretOff());
    saveStatus(f, tl, end + 400);
    return { hold: 1600 };
  },

  pen: ctx => {
    const { tl } = ctx; const f = open(ctx);
    let t = pickTool(f, tl, 300, 'pen');
    const r = pageRect(f, f.pdf.els.li[0]);
    const y = r.y + r.h - 1;
    const under = addPath(f.pdf.svg, `M${r.x + 26} ${y} C ${r.x + 90} ${y + 4}, ${r.x + 160} ${y - 3}, ${r.x + 236} ${y}`);
    const [sx, sy] = toFrame(f, r.x + 26, y), [ex] = toFrame(f, r.x + 236, y);
    t = f.pointer.moveTo(tl, t, sx, sy, 500);
    t = f.pointer.drag(tl, t + 150, [sx, sy], [ex, sy], 900, p => under.draw(p));
    // 화살표: 첫 항목에서 여백의 메모 자리로
    const ax = r.x + 244, ay = y - 6, bx = noteX(f) - 6, by = r.y - 14;
    const arrow = addPath(f.pdf.svg, `M${ax} ${ay} C ${ax + 20} ${ay - 2}, ${bx - 24} ${by + 6}, ${bx} ${by} M${bx - 9} ${by + 1} L${bx} ${by} L${bx - 3} ${by + 9}`);
    const [asx, asy] = toFrame(f, ax, ay), [bex, bey] = toFrame(f, bx, by);
    t = f.pointer.moveTo(tl, t + 200, asx, asy, 300);
    t = f.pointer.drag(tl, t + 100, [asx, asy], [bex, bey], 700, p => arrow.draw(p));
    saveStatus(f, tl, t + 100);
    return { hold: 1700 };
  },

  'pdf-move': ctx => {
    const { tl } = ctx; const { f, note } = withExisting(ctx, ['hl', 'note']);
    const r = pageRect(f, note.el);
    const [sx, sy] = toFrame(f, r.x + 40, r.y + 12);
    let t = f.pointer.moveTo(tl, 400, sx, sy, 550);
    t = f.pointer.click(tl, t + 100);
    tl.at(t, () => note.el.classList.add('selected'));
    const dx = -40, dy = 92;
    t = f.pointer.drag(tl, t + 400, [sx, sy], [sx + dx, sy + dy], 900, p => { note.el.style.left = (r.x + dx * p) + 'px'; note.el.style.top = (r.y + dy * p) + 'px'; });
    saveStatus(f, tl, t + 100);
    tl.at(t + 900, () => note.el.classList.remove('selected'));
    return { hold: 1500 };
  },

  'pdf-resize': ctx => {
    const { tl } = ctx; const { f, note } = withExisting(ctx, ['hl', 'note']);
    note.el.style.width = '118px'; // 좁게 시작해 페이지 안에서 넓힌다
    const r = pageRect(f, note.el);
    const [cx, cy] = toFrame(f, r.x + 30, r.y + 10);
    let t = f.pointer.moveTo(tl, 400, cx, cy, 500);
    t = f.pointer.click(tl, t + 100);
    tl.at(t, () => note.el.classList.add('selected'));
    const [hx, hy] = toFrame(f, r.x + r.w + 4, r.y + r.h + 4); // 오른쪽 아래 손잡이
    t = f.pointer.moveTo(tl, t + 400, hx, hy, 400);
    const grow = 50;
    t = f.pointer.drag(tl, t + 150, [hx, hy], [hx + grow, hy], 900, p => { note.el.style.width = (r.w + grow * p) + 'px'; });
    saveStatus(f, tl, t + 100);
    tl.at(t + 900, () => note.el.classList.remove('selected'));
    return { hold: 1500 };
  },

  'pdf-area-select': ctx => {
    const { tl } = ctx; const { f, note, box, hl } = withExisting(ctx, ['hl', 'note', 'box']);
    let t = pickTool(f, tl, 300, 'area-select');
    const a = pageRect(f, f.pdf.els.p2wrap), b = pageRect(f, f.pdf.els.li[2]);
    const x0 = a.x - 16, y0 = a.y - 10, x1 = f.pdf.page.clientWidth - 24, y1 = b.y + b.h + 12;
    const m = addMarquee(f.pdf.layer);
    const [sx, sy] = toFrame(f, x0, y0), [ex, ey] = toFrame(f, x1, y1);
    t = f.pointer.moveTo(tl, t, sx, sy, 500);
    t = f.pointer.drag(tl, t + 150, [sx, sy], [ex, ey], 1000, p => setRect(m, x0, y0, (x1 - x0) * p, (y1 - y0) * p));
    tl.at(t + 50, () => { m.remove(); note.el.classList.add('selected'); box.classList.add('selected'); hl.classList.add('text-selected'); });
    return { hold: 1900 };
  },

  'pdf-note-edit': ctx => {
    const { tl, S, lang } = ctx; const { f, note } = withExisting(ctx, ['hl', 'note']);
    const r = pageRect(f, note.el);
    const [cx, cy] = toFrame(f, r.x + r.w - 20, r.y + r.h - 8);
    let t = f.pointer.moveTo(tl, 400, cx, cy, 550);
    t = f.pointer.click(tl, t + 80); t = f.pointer.click(tl, t + 40);
    let more;
    tl.at(t + 50, () => { note.caretOn(); more = note.append(S.scenes.pdf.noteMore); });
    const n = [...S.scenes.pdf.noteMore].length, sp = speed(lang);
    for (let i = 0; i < n; i++) tl.at(t + 450 + i * sp, () => more?.[i]?.classList.add('on'));
    const end = t + 450 + n * sp;
    tl.at(end + 300, () => note.caretOff());
    saveStatus(f, tl, end + 400);
    return { hold: 1500 };
  },

  'pdf-delete-many': ctx => {
    const { tl } = ctx; const { f, note, hl, pen } = withExisting(ctx, ['hl', 'note', 'pen']);
    let t = pickTool(f, tl, 300, 'area-select');
    const a = pageRect(f, f.pdf.els.p2wrap), b = pageRect(f, f.pdf.els.li[1]);
    const x0 = a.x - 16, y0 = a.y - 10, x1 = f.pdf.page.clientWidth - 24, y1 = b.y + b.h + 6;
    const m = addMarquee(f.pdf.layer);
    const [sx, sy] = toFrame(f, x0, y0), [ex, ey] = toFrame(f, x1, y1);
    t = f.pointer.moveTo(tl, t, sx, sy, 500);
    t = f.pointer.drag(tl, t + 150, [sx, sy], [ex, ey], 900, p => setRect(m, x0, y0, (x1 - x0) * p, (y1 - y0) * p));
    tl.at(t + 50, () => { m.remove(); note.el.classList.add('selected'); hl.classList.add('text-selected'); pen.el.style.filter = 'drop-shadow(0 0 2px rgba(79,70,229,.8))'; });
    f.toast.key(tl, t + 900, ['⌫'], 900);
    tl.at(t + 960, () => { note.el.classList.add('gone'); hl.classList.remove('text-selected'); setHighlight(hl, 0); pen.el.style.opacity = '0'; pen.el.style.transition = 'opacity .25s'; });
    saveStatus(f, tl, t + 1300);
    return { hold: 1600 };
  },

  'pdf-share': ctx => {
    const { tl, S } = ctx; const { f } = withExisting(ctx, ['hl', 'note']);
    const mb = menubar(f.frame);
    const fr = frameRect(f, mb.file);
    let t = f.pointer.moveTo(tl, 400, fr.x + 14, fr.y + 12, 600);
    t = f.pointer.click(tl, t + 100);
    tl.at(t, () => mb.open());
    let item;
    tl.at(t + 10, () => { item = frameRect(f, mb.item('share')); });
    t = tl.during(t + 300, 450, p => { if (!item) return; const sx = fr.x + 14, sy = fr.y + 12; f.pointer.el.style.transform = `translate(${sx + (item.x + 40 - sx) * p}px,${sy + (item.y + 12 - sy) * p}px)`; if (p > .85) mb.hot('share'); });
    t = f.pointer.click(tl, t + 150);
    tl.at(t, () => { mb.close(); f.pdf.setStatus(UI.share.saving, true); });
    tl.at(t + 900, () => { f.pdf.setStatus(UI.share.done); const pop = sharePopover(f.frame, 250, 90, S.scenes.pdf.shareName); requestAnimationFrame(() => pop.show()); });
    tl.wait(t, 1200);
    return { hold: 1900 };
  },

  'pdf-pptx': ctx => {
    const { tl, S } = ctx; const f = open(ctx, { selected: 'note2', content: 'pdf' });
    // 처음엔 노트가 열려 있는 척 하지 않고 PPTX 행을 고르는 흐름만 보여 준다.
    const row = f.panel.rows.pptx; const rr = frameRect(f, row);
    let t = f.pointer.moveTo(tl, 400, rr.x + 60, rr.y + 15, 600);
    t = f.pointer.click(tl, t + 100);
    let menu;
    tl.at(t, () => { f.panel.select('pptx'); menu = contextMenu(f.frame, rr.x + 70, rr.y + 20, [{ id: 'annotate', label: UI.ctx.pptxAnnotate, icon: 'pencil' }, { id: 'convert', label: UI.ctx.pptxConvert, icon: 'file' }, { id: 'rename', label: UI.ctx.rename, icon: 'pencil' }, { id: 'finder', label: UI.ctx.finder, icon: 'finder' }]); requestAnimationFrame(() => menu.show()); });
    t = f.pointer.moveTo(tl, t + 350, rr.x + 110, rr.y + 38, 350);
    tl.at(t, () => menu.hot('annotate'));
    t = f.pointer.click(tl, t + 200);
    tl.at(t, () => { menu.hide(); f.pdf.setStatus(UI.pptx.preparing, true); });
    tl.at(t + 1100, () => {
      const card = f.pdf.el; const next = buildPdfCardPptx(S); card.replaceWith(next.el); f.pdf = next; f.pdf.setStatus(UI.pdfSaved);
    });
    tl.wait(t, 1300);
    return { hold: 1800 };
  },

  'pdf-analyze': ctx => {
    const { tl, S } = ctx; const { f } = withExisting(ctx, ['hl', 'note']);
    const row = f.panel.rows.pdf; const rr = frameRect(f, row);
    let t = f.pointer.moveTo(tl, 400, rr.x + 60, rr.y + 15, 600);
    t = f.pointer.click(tl, t + 100);
    let menu;
    tl.at(t, () => { menu = contextMenu(f.frame, rr.x + 70, rr.y + 20, [{ id: 'analyze', label: UI.ctx.analyze, icon: 'spark' }, { id: 'reorganize', label: UI.ctx.reorganize, icon: 'spark', disabled: true }, { id: 'rename', label: UI.ctx.rename, icon: 'pencil' }, { id: 'finder', label: UI.ctx.finder, icon: 'finder' }]); requestAnimationFrame(() => menu.show()); });
    t = f.pointer.moveTo(tl, t + 350, rr.x + 110, rr.y + 38, 300);
    tl.at(t, () => menu.hot('analyze'));
    t = f.pointer.click(tl, t + 200);
    let dlg;
    tl.at(t, () => { menu.hide(); dlg = analysisDialog(f.frame, S.scenes.pdf.file); requestAnimationFrame(() => dlg.show()); });
    tl.at(t + 100, () => { const b = frameRect(f, dlg.start); dlg._btn = b; });
    t = tl.during(t + 1300, 500, p => { const b = dlg?._btn; if (!b) return; const sx = rr.x + 110, sy = rr.y + 38; f.pointer.el.style.transform = `translate(${sx + (b.x + b.w / 2 - sx) * p}px,${sy + (b.y + b.h / 2 - sy) * p}px)`; });
    t = f.pointer.click(tl, t + 200);
    tl.at(t, () => { dlg.hide(); f.pdf.setStatus(UI.analysis.starting, true); });
    tl.at(t + 1000, () => f.pdf.setStatus(UI.analysis.running, true));
    tl.wait(t, 1200);
    return { hold: 2000 };
  },
};

import { buildPdfCard } from './frames.js';
function buildPdfCardPptx(S) { return buildPdfCard(S, { title: S.scenes.files.pptx, content: 'pptx', tool: 'read' }); }
