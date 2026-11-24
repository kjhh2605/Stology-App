// 마무리 섹션 소개 영상 확인(Aside REPL): 위쪽에서는 영상을 받지 않는지, 보이면 소리 없이 재생하고 벗어나면 멈추는지,
// 소리 켜기가 처음부터 들려주는지, 일시정지, 데스크톱 한국어·모바일 영어·동작 줄이기 화면, 콘솔 오류.
// 실행: aside repl --file landing/qa/closing-video-check.js (개발 서버 http://127.0.0.1:5173 필요)
// 일회성 세션은 끝나면 폴더를 지우므로, 마지막 대기 동안 ~/.aside/u/0/sessions/<id>/artifacts/에서 복사한다.
const base = 'http://127.0.0.1:5173';
await fs.mkdir('./artifacts', { recursive: true });
await fs.writeFile('./artifacts/closing-check-start.txt', 'start');
const report = { errors: [] };
const state = pg => pg.evaluate(() => { const v = document.querySelector('.film-video'), s = document.querySelector('.film-sound'), p = document.querySelector('.film-play'); return { src: v.getAttribute('src') || '', paused: v.paused, muted: v.muted, t: Math.round(v.currentTime * 10) / 10, ready: v.readyState, play: p.getAttribute('aria-label'), sound: s.textContent, pressed: s.getAttribute('aria-pressed') }; });
const toClosing = async pg => {
  await pg.evaluate(() => document.fonts.ready);
  const y = await pg.evaluate(() => document.querySelector('.closing').getBoundingClientRect().top + window.scrollY - 40);
  await pg.evaluate(top => window.scrollTo({ top, behavior: 'instant' }), y);
};
const collect = async (pg, tag) => {
  const logs = await pg.console.logs();
  report.errors.push(...logs.filter(l => l.type === 'error' || l.type === 'warning').map(l => `${tag} ${l.type}: ${l.text}`));
};

// 1. 한국어 데스크톱
const ko = await openTab(base + '/');
await ko.setViewportSize({ width: 1440, height: 900 });
await sleep(1000);
report.atTop = await state(ko);
await toClosing(ko);
await sleep(3000);
report.inView = await state(ko);
await ko.screenshot({ path: './artifacts/closing-ko-desktop.png' });
await ko.evaluate(() => window.scrollTo({ top: 0, behavior: 'instant' }));
await sleep(600);
report.scrolledAway = await state(ko);
await toClosing(ko);
await sleep(1500);
report.back = await state(ko);
await ko.locator('.film-sound').click();
await sleep(400);
report.soundOn = await state(ko);
await ko.locator('.film-play').click();
await sleep(300);
report.pausedByUser = await state(ko);
await ko.screenshot({ path: './artifacts/closing-ko-paused.png' });
await collect(ko, 'ko');

// 2. 영어 모바일
const en = await openTab(base + '/en/');
await en.setViewportSize({ width: 390, height: 844 });
await toClosing(en);
await sleep(2500);
report.enMobile = await state(en);
await en.screenshot({ path: './artifacts/closing-en-mobile.png' });
await collect(en, 'en');

// 3. 동작 줄이기: 포스터만 보이고 누르기 전에는 받지 않는다
await en.emulateMedia({ reducedMotion: 'reduce' });
await en.setViewportSize({ width: 1440, height: 900 });
await en.goto(base + '/');
await toClosing(en);
await sleep(1500);
report.reduced = await state(en);
await en.screenshot({ path: './artifacts/closing-reduced.png' });
await en.locator('.film-play').click();
await sleep(1500);
report.reducedAfterPlay = await state(en);
await collect(en, 'reduced');

console.log(JSON.stringify(report, null, 1));
await fs.writeFile('./artifacts/closing-check-done.txt', 'done');
await sleep(15000);
