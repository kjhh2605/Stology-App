// 02 한 화면: 앱 창 전체에서 PDF 열기 → 형광펜 → AI 질문 ⌘↩ → 노트로 정리 네 박자를 약 20초로 반복한다.
import { typeChars, rectIn } from './runtime.js';
import { buildWindowFrame, setHighlight, flash } from './frames.js';
import { UI } from './ui-labels.js';

const speed = lang => (lang === 'ko' ? 50 : 32);

export function windowLoop(ctx) {
  const { root, tl, S, lang, reduced } = ctx;
  const f = buildWindowFrame(root, { S, reduced, connected: true });
  const R = el => rectIn(f.frame, el);
  const onStep = ctx.onStep || (() => {});

  // 1. 자료 트리의 PDF 행 클릭 → 중앙에 PDF
  tl.at(0, () => onStep(0));
  const row = R(f.panel.rows.pdf);
  let t = f.pointer.moveTo(tl, 500, row.x + 70, row.y + 15, 700);
  t = f.pointer.click(tl, t + 100);
  let pdf;
  tl.at(t, () => { f.panel.select('pdf'); pdf = f.showPdf({ tool: 'read' }); });

  // 2. 형광펜 → 문장 드래그
  tl.at(t + 600, () => onStep(1));
  let tb, p2;
  tl.at(t + 10, () => { tb = R(pdf.toolbar.button('highlight')); });
  t = tl.during(t + 600, 500, p => { if (!tb) return; const sx = row.x + 70, sy = row.y + 15; f.pointer.el.style.transform = `translate(${sx + (tb.x + 14 - sx) * p}px,${sy + (tb.y + 14 - sy) * p}px)`; });
  t = f.pointer.click(tl, t + 80);
  tl.at(t, () => { pdf.toolbar.set('highlight'); p2 = R(pdf.els.p2); });
  t = tl.during(t + 300, 500, p => { if (!p2) return; const sx = tb.x + 14, sy = tb.y + 14; f.pointer.el.style.transform = `translate(${sx + (p2.x - 2 - sx) * p}px,${sy + (p2.y + p2.h / 2 - sy) * p}px)`; });
  t = tl.during(t + 150, 1100, p => { if (!p2) return; setHighlight(pdf.els.p2, p); f.pointer.el.style.transform = `translate(${p2.x - 2 + (p2.w + 4) * p}px,${p2.y + p2.h / 2}px)`; });
  tl.at(t + 100, () => pdf.setStatus(UI.pdfSaving, true));
  tl.at(t + 800, () => pdf.setStatus(UI.pdfSaved));

  // 3. AI 탭 → 질문 입력 → ⌘↩ → 답변
  tl.at(t + 900, () => onStep(2));
  const aiTab = R(f.panel.tabs.ai);
  t = tl.during(t + 900, 550, p => { if (!p2) return; const sx = p2.x + p2.w + 2, sy = p2.y + p2.h / 2; f.pointer.el.style.transform = `translate(${sx + (aiTab.x + aiTab.w / 2 - sx) * p}px,${sy + (aiTab.y + aiTab.h / 2 - sy) * p}px)`; });
  t = f.pointer.click(tl, t + 80);
  tl.at(t, () => { f.panel.switchTo('ai'); f.pointer.el.classList.add('hidden'); });
  const q = S.scenes.chat.question;
  let qChars;
  tl.at(t + 400, () => { qChars = f.panel.ai.typeTarget(q); f.panel.ai.ph.classList.add('off'); f.panel.ai.send.classList.add('on'); });
  const n = [...q].length, sp = speed(lang);
  for (let i = 0; i < n; i++) tl.at(t + 500 + i * sp, () => qChars?.[i]?.classList.add('on'));
  t = t + 500 + n * sp;
  f.toast.key(tl, t + 400, ['⌘', 'Enter'], 900);
  let answer;
  tl.at(t + 460, () => { f.panel.ai.clearField(); f.panel.ai.userMsg(q); });
  tl.at(t + 1100, () => { answer = f.panel.ai.aiMsg(S.scenes.chat.answer); });
  let cursor = t + 1200;
  S.scenes.chat.answer.forEach((line, li) => {
    const len = [...line].length;
    for (let i = 0; i < len; i++) tl.at(cursor + i * 22, () => answer?.chars[li][i]?.classList.add('on'));
    cursor += len * 22 + 250;
  });
  tl.at(cursor + 200, () => answer.acts.classList.add('on'));

  // 4. 노트로 정리 → 자료 탭 정리 제안에 초안
  tl.at(cursor + 600, () => onStep(3));
  let btn;
  tl.at(cursor + 600, () => { btn = R(answer.button); f.pointer.el.classList.remove('hidden'); f.pointer.el.style.transform = `translate(${btn.x - 60}px,${btn.y + 60}px)`; });
  t = tl.during(cursor + 700, 500, p => { if (!btn) return; f.pointer.el.style.transform = `translate(${btn.x - 60 + (btn.w / 2 + 60) * p}px,${btn.y + 60 + (btn.h / 2 - 60) * p}px)`; });
  t = f.pointer.click(tl, t + 80);
  let fl;
  tl.at(t, () => { fl = flash(f.panel.el, UI.chat.making); requestAnimationFrame(() => fl.show()); f.panel.showDot(true); });
  tl.at(t + 1700, () => { fl.hide(); });
  const mTab = R(f.panel.tabs.materials);
  t = tl.during(t + 1900, 500, p => { if (!btn) return; const sx = btn.x + btn.w / 2, sy = btn.y + btn.h / 2; f.pointer.el.style.transform = `translate(${sx + (mTab.x + mTab.w / 2 - sx) * p}px,${sy + (mTab.y + mTab.h / 2 - sy) * p}px)`; });
  t = f.pointer.click(tl, t + 80);
  tl.at(t, () => { f.panel.switchTo('materials'); f.panel.showDot(false); f.panel.addSuggestion(S.scenes.chat.draftTitle, S.scenes.chat.draftMeta); });
  tl.wait(t, 800);
  return { hold: 2600, gap: 1200 };
}
