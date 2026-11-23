// 노트 기능 장면 16개. 실제 앱의 중앙 문서를 재현한 프레임 위에서 입력 → 선택 → 키 → 결과를 보여 준다.
import { typeChars, sweepSelect, clearSelect, markRange, rectIn } from './runtime.js';
import { buildDocumentFrame, buildMapFrame, linkPopover } from './frames.js';
import { UI } from './ui-labels.js';

const speed = lang => (lang === 'ko' ? 55 : 35);
const base = (ctx, lines) => buildDocumentFrame(ctx.root, { S: ctx.S, reduced: ctx.reduced, lines });

/** 공통: 문장 입력 → 구절 선택 → 단축키 → 인라인 서식 */
function inlineFormat(ctx, keys, cls, wordKey = 'selWord') {
  const { tl, S, lang } = ctx, D = S.scenes.doc;
  const f = base(ctx, [{ text: D.h2, kind: 'h2' }, { text: D.line1 }, { text: D.target, typed: true }]);
  const line = f.doc.lines[2];
  line.caretOn();
  let t = typeChars(tl, 400, line.chars, speed(lang));
  const [a, b] = line.range(D[wordKey]);
  t = sweepSelect(tl, t + 600, line.chars, a, b);
  const press = t + 450;
  f.toast.key(tl, press, keys, 1000);
  tl.at(press + 60, () => markRange(line.chars, a, b, cls));
  tl.at(press + 1000, () => { clearSelect(line.chars); });
  return { hold: 1500 };
}

function heading(ctx, level) {
  const { tl, S, lang } = ctx, D = S.scenes.doc, sp = speed(lang);
  const f = base(ctx, [{ text: D.line1 }, { text: D.h2b, typed: true }]);
  const line = f.doc.lines[1];
  line.caretOn();
  let t = typeChars(tl, 400, line.chars, sp);
  const press = t + 500;
  f.toast.key(tl, press, ['⌘', '⌥', String(level)], 950);
  tl.at(press + 60, () => line.setKind(`h${level}`));
  // 두 번째 방법: Markdown 입력. `#` 다음 Space에서 바뀐다.
  const md = f.doc.addLine('#'.repeat(level) + ' ' + D.h2c, 'p', { typed: true });
  const hashes = level + 1;
  tl.at(press + 1500, () => { line.caretOff(); md.caretOn(); });
  t = typeChars(tl, press + 1600, md.chars.slice(0, hashes), 140);
  f.toast.key(tl, t - 140, ['Space'], 800);
  tl.at(t, () => { md.hideRange(0, hashes); md.setKind(`h${level}`); });
  t = typeChars(tl, t + 300, md.chars.slice(hashes), sp);
  tl.at(t + 200, () => md.caretOff());
  return { hold: 1700 };
}

function listScene(ctx, keys, kind) {
  const { tl, S, lang } = ctx, D = S.scenes.doc, sp = speed(lang);
  const f = base(ctx, [{ text: D.h2b, kind: 'h2' }, ...D.list.map(text => ({ text, typed: true }))]);
  const items = f.doc.lines.slice(1);
  let t = 400;
  items.forEach((l, i) => { tl.at(t, () => { items[i - 1]?.caretOff(); l.caretOn(); }); t = typeChars(tl, t, l.chars, Math.max(24, sp - 20)) + 120; });
  tl.at(t, () => items.at(-1).caretOff());
  // 세 줄 선택
  items.forEach((l, i) => { sweepSelect(tl, t + 300 + i * 160, l.chars, 0, l.chars.length, 220); });
  t += 300 + items.length * 160 + 150;
  f.toast.key(tl, t, keys, 950);
  tl.at(t + 60, () => items.forEach((l, i) => { l.setKind(kind, { n: i + 1 }); }));
  tl.at(t + 950, () => items.forEach(l => clearSelect(l.chars)));
  return { f, items, t: t + 1000 };
}

export const noteScenes = {
  bold: ctx => inlineFormat(ctx, ['⌘', 'B'], 'b'),
  italic: ctx => inlineFormat(ctx, ['⌘', 'I'], 'i'),
  underline: ctx => inlineFormat(ctx, ['⌘', 'U'], 'u'),
  strikethrough: ctx => inlineFormat(ctx, ['⌘', '⇧', 'S'], 's'),
  'inline-code': ctx => inlineFormat(ctx, ['⌘', 'E'], 'code', 'selSmall'),

  link: ctx => {
    const { tl, S, lang } = ctx, D = S.scenes.doc;
    const f = base(ctx, [{ text: D.h2, kind: 'h2' }, { text: D.line1 }, { text: D.target, typed: true }]);
    const line = f.doc.lines[2];
    line.caretOn();
    let t = typeChars(tl, 400, line.chars, speed(lang));
    const [a, b] = line.range(D.linkWord);
    t = sweepSelect(tl, t + 500, line.chars, a, b);
    const press = t + 400;
    f.toast.key(tl, press, ['⌘', 'K'], 900);
    let pop;
    tl.at(press + 80, () => {
      const r = rectIn(f.frame, line.chars[a]);
      pop = linkPopover(f.frame, Math.min(r.x, 380), r.y + 30); pop.show();
      pop.chars = pop.typeTarget(D.url);
    });
    tl.at(press + 700, () => pop.ph.classList.add('off'));
    [...D.url].forEach((_, i) => tl.at(press + 700 + i * 28, () => pop.chars[i].classList.add('on')));
    const enter = press + 700 + D.url.length * 28 + 350;
    f.toast.key(tl, enter, ['Enter'], 800);
    tl.at(enter + 60, () => { pop.hide(); markRange(line.chars, a, b, 'a'); clearSelect(line.chars); });
    return { hold: 1500 };
  },

  'block-1': ctx => heading(ctx, 1),
  'block-2': ctx => heading(ctx, 2),
  'block-3': ctx => heading(ctx, 3),

  checklist: ctx => {
    const { tl } = ctx;
    const { f, items, t } = listScene(ctx, ['⌘', '⌥', '4'], 'check');
    const r = rectIn(f.frame, items[0].el);
    f.pointer.show(tl, t + 200);
    f.pointer.jump(tl, t + 200, r.x + 80, r.y + 70);
    f.pointer.moveTo(tl, t + 250, r.x + 9, r.y + 13, 500);
    f.pointer.click(tl, t + 800);
    tl.at(t + 860, () => items[0].check(true));
    return { hold: 1700 };
  },

  bullet: ctx => {
    const { tl } = ctx;
    const { f, items, t } = listScene(ctx, ['⌘', '⌥', '5'], 'ul');
    tl.at(t + 300, () => items[1].caretOn());
    f.toast.key(tl, t + 700, ['Tab'], 850);
    tl.at(t + 760, () => items[1].setKind('ul', { indent: 1 }));
    tl.at(t + 1500, () => items[1].caretOff());
    return { hold: 1700 };
  },

  numbered: ctx => {
    const { tl, S, lang } = ctx;
    const { f, items, t } = listScene(ctx, ['⌘', '⌥', '6'], 'ol');
    tl.at(t + 300, () => items[2].caretOn());
    f.toast.key(tl, t + 700, ['Enter'], 850);
    const next = f.doc.addLine(S.scenes.doc.list4, 'ol', { typed: true, kindOpts: { n: 4 } });
    tl.at(t + 760, () => { items[2].caretOff(); next.caretOn(); });
    const end = typeChars(tl, t + 1000, next.chars, speed(lang));
    tl.at(end + 200, () => next.caretOff());
    return { hold: 1600 };
  },

  'block-code': ctx => {
    const { tl, S } = ctx, D = S.scenes.doc;
    const f = base(ctx, [{ text: D.h2, kind: 'h2' }, { text: D.line1 }, { text: D.code, typed: true }]);
    const line = f.doc.lines[2];
    line.caretOn();
    let t = typeChars(tl, 400, line.chars, 40);
    t = sweepSelect(tl, t + 500, line.chars, 0, line.chars.length, 300);
    const press = t + 400;
    f.toast.key(tl, press, ['⌘', '⌥', '8'], 950);
    tl.at(press + 60, () => { line.caretOff(); line.setKind('code', { lang: UI.codeLang }); clearSelect(line.chars); });
    return { hold: 1700 };
  },

  'markdown-quote': ctx => {
    const { tl, S, lang } = ctx, D = S.scenes.doc, sp = speed(lang);
    const f = base(ctx, [{ text: D.h2, kind: 'h2' }, { text: D.line1 }, { text: '> ' + D.quote, typed: true }]);
    const line = f.doc.lines[2];
    line.caretOn();
    let t = typeChars(tl, 500, line.chars.slice(0, 2), 180);
    f.toast.key(tl, t - 180, ['Space'], 800);
    tl.at(t, () => { line.hideRange(0, 2); line.setKind('quote'); });
    t = typeChars(tl, t + 300, line.chars.slice(2), sp);
    f.toast.key(tl, t + 500, ['Enter'], 850);
    const next = f.doc.addLine(D.afterQuote, 'p', { typed: true });
    tl.at(t + 560, () => { line.caretOff(); next.caretOn(); });
    const end = typeChars(tl, t + 900, next.chars, sp);
    tl.at(end + 200, () => next.caretOff());
    return { hold: 1500 };
  },

  selection: ctx => {
    const { tl, S } = ctx, D = S.scenes.doc;
    const f = base(ctx, [{ text: D.h2, kind: 'h2' }, { text: D.line1 }, { text: D.target }]);
    const [h2, l1, l2] = f.doc.lines;
    const [a, b] = l2.range(D.selSmall);
    // 캐럿을 단어 안에 둔다
    tl.at(300, () => { l2.chars[a].before(l2.caret); });
    const k1 = 900;
    f.toast.key(tl, k1, ['⌘', 'D'], 1200);
    tl.at(k1 + 60, () => markRange(l2.chars, a, b, 'sel'));
    const k2 = k1 + 1500;
    tl.at(k2, () => f.toast.pulse(['⌘', 'D']));
    tl.at(k2 + 60, () => markRange(l2.chars, 0, l2.chars.length, 'sel'));
    const k3 = k2 + 1500;
    tl.at(k3, () => f.toast.pulse(['⌘', 'D']));
    tl.at(k3 + 60, () => { [h2, l1].forEach(l => markRange(l.chars, 0, l.chars.length, 'sel')); });
    tl.at(k3 + 1200, () => f.toast.hide());
    tl.wait(k3, 1300);
    return { hold: 1500 };
  },

  'map-search-focus': ctx => {
    const { tl, S, lang } = ctx, M = S.scenes.map;
    const f = buildMapFrame(ctx.root, { S, reduced: ctx.reduced });
    const press = 700;
    f.toast.key(tl, press, ['⌘', 'K'], 900);
    tl.at(press + 60, () => f.focus(true));
    const { chars, ph } = f.typeTarget(M.query);
    tl.at(press + 600, () => ph.classList.add('off'));
    let t = typeChars(tl, press + 650, chars, speed(lang) + 20);
    tl.at(t + 150, () => {
      f.sugg.classList.add('on');
      [...f.sugg.children].forEach((d, i) => { d.innerHTML = esc(M.suggestions[i]).replace(esc(M.query), `<em>${esc(M.query)}</em>`); d.classList.toggle('hot', i === 0); });
    });
    tl.at(t + 900, () => { f.glow.style.left = '150px'; f.glow.style.top = '120px'; f.glow.classList.add('on'); });
    tl.wait(t, 1400);
    return { hold: 1600 };
  },
};

const esc = v => String(v ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
