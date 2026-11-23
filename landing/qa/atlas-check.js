// 06 과목 지도 장면 확인(Aside REPL): 데스크톱 1440×900 한국어 장면 시간대별 캡처·프레임 속도·크기,
// 영어 페이지 두 장면, 동작 줄이기의 고정 화면, 콘솔 오류. 실행: aside repl --file landing/qa/atlas-check.js
// 개발 서버(npm run dev --prefix landing)가 http://127.0.0.1:5173 에 떠 있어야 한다. 캡처는 ./artifacts/에 남고
// 일회성 세션은 끝나면 폴더를 지우므로, 마지막 대기 동안 ~/.aside/u/0/sessions/<id>/artifacts/에서 복사한다.
const base = 'http://127.0.0.1:5173';
await fs.mkdir('./artifacts', { recursive: true });
await fs.writeFile('./artifacts/atlas-check-start.txt', 'start');
const report = { errors: [] };
const toStage = async (pg, offset = 96) => {
  await pg.evaluate(() => document.fonts.ready);
  await sleep(800);
  const top = await pg.evaluate(o => document.querySelector('.atlas-stage').getBoundingClientRect().top + window.scrollY - o, offset);
  await pg.evaluate(y => window.scrollTo({ top: y, behavior: 'instant' }), top);
  return Date.now();
};
const shootAt = async (pg, start, list) => {
  for (const [ms, name] of list) {
    const wait = ms - (Date.now() - start);
    if (wait > 0) await sleep(wait);
    await pg.screenshot({ path: `./artifacts/${name}.png` });
  }
};
const collect = async (pg, tag) => {
  const logs = await pg.console.logs();
  report.errors.push(...logs.filter(l => l.type === 'error' || l.type === 'warning').map(l => `${tag} ${l.type}: ${l.text}`));
};

// 1. 한국어 데스크톱
const ko = await openTab(base + '/');
await ko.setViewportSize({ width: 1440, height: 900 });
const koStart = await toStage(ko);
await sleep(2500);
report.size = await ko.evaluate(() => { const v = document.querySelector('.atlas-view'), w = document.querySelector('.atlas-stage .app-frame-wrap'), c = v.querySelector('canvas'); return { view: [v.clientWidth, v.clientHeight], wrap: [w.clientWidth, w.clientHeight], scale: w.dataset.scale, canvas: [c.width, c.height] }; });
report.fps = await ko.evaluate(() => new Promise(done => { let n = 0; const t0 = performance.now(); const f = now => { n++; if (now - t0 < 2000) requestAnimationFrame(f); else done(Math.round(n / ((now - t0) / 1000))); }; requestAnimationFrame(f); }));
await shootAt(ko, koStart, [[8600, 'atlas-ko-lecture'], [15900, 'atlas-ko-search'], [17700, 'atlas-ko-question'], [22500, 'atlas-ko-next']]);
await collect(ko, 'ko');

// 2. 영어 데스크톱
const en = await openTab(base + '/en/');
await en.setViewportSize({ width: 1440, height: 900 });
const enStart = await toStage(en);
await shootAt(en, enStart, [[7100, 'atlas-en-lecture'], [17700, 'atlas-en-question']]);
await collect(en, 'en');

// 3. 동작 줄이기: 마지막 상태가 고정되어 보인다
await en.emulateMedia({ reducedMotion: 'reduce' });
await en.goto(base + '/');
await toStage(en);
await sleep(1200);
await en.screenshot({ path: './artifacts/atlas-reduced.png' });
report.reduced = await en.evaluate(() => matchMedia('(prefers-reduced-motion: reduce)').matches);
await collect(en, 'reduced');

console.log(JSON.stringify(report));
await fs.writeFile('./artifacts/atlas-check-done.txt', 'done');
await sleep(15000);
