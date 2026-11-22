// Run with: aside repl --file landing/qa/hero-scene-check.js
// Expects `npm run dev --prefix landing -- --port 5199 --strictPort`. Opens its own tab.
// Screenshots land in the session's ./artifacts; the final sleep keeps the session open to copy them out.
await openTab('http://127.0.0.1:5199/');
await page.setViewportSize({ width: 1440, height: 900 });
await fs.mkdir('./artifacts', { recursive: true });
const results = [];
const check = (pass, label, details) => results.push({ label, pass: !!pass, ...(details === undefined ? {} : { details }) });
const shot = async (name) => fs.writeFile('./artifacts/' + name, await page.screenshot());
const animates = () => page.evaluate(async () => { const c = document.querySelector('#hero-scene'); const first = c.toDataURL(); await new Promise((r) => setTimeout(r, 400)); return first !== c.toDataURL(); });
// Reads the canvas under an element (or a box in page coordinates): the share of clearly lit pixels, how they split between
// the top and bottom half, the area in CSS px² that is as pale as the star's body, and the mean brightness.
const coverage = (target) => page.evaluate((target) => {
  const c = document.querySelector('#hero-scene'), base = c.getBoundingClientRect(), scale = c.width / base.width;
  const r = typeof target === 'string' ? document.querySelector(target).getBoundingClientRect() : target;
  const x = Math.max(0, Math.round((r.left - base.left) * scale)), y = Math.max(0, Math.round((r.top - base.top) * scale));
  const w = Math.min(c.width - x, Math.round(r.width * scale)), h = Math.min(c.height - y, Math.round(r.height * scale));
  const d = c.getContext('2d').getImageData(x, y, w, h).data;
  let lit = 0, pale = 0, top = 0, sum = 0;
  for (let i = 0; i < d.length; i += 4) {
    const level = d[i] + d[i + 1] + d[i + 2];
    sum += level;
    if (level > 150) { lit++; if (i < d.length / 2) top++; }
    if (d[i] > 205 && d[i + 1] > 195 && d[i + 2] > 225) pale++;
  }
  const all = d.length / 4;
  return { lit: lit / all, halves: [top / lit, 1 - top / lit], pale: pale / scale / scale, mean: sum / all / 3 };
}, target);
const drawn = (c) => c.lit > .1 && c.halves[0] > .3 && c.halves[1] > .3;
// At the start the star rides the S close up: its body is far larger than every bright star core put together.
const starShown = (c) => c.pale > 1500 && c.pale < 9000;
const frameRate = () => page.evaluate(() => new Promise((resolve) => { let frames = 0; const start = performance.now(); (function count(now) { frames++; if (now - start < 2000) requestAnimationFrame(count); else resolve(frames / ((now - start) / 1000)); })(start); }));

await sleep(2500);
const assets = await page.evaluate(async () => Promise.all(['/media/hero/star-sheet.webp', '/mark.png', '/media/hero/star.webp'].map(async (url) => [url, (await fetch(url)).status])));
check(assets.every(([, status]) => status === 200), 'Hero assets load', assets);
check(await page.evaluate(async () => (await fetch('/media/hero/s-ribbon.webp')).headers.get('content-type')?.startsWith('image/') !== true), 'Removed ribbon image is no longer served');
const desktop = await coverage('.hero-art');
check(drawn(desktop), 'Galaxy is drawn across the S', desktop);
check(starShown(desktop), 'The star is drawn', desktop.pale);
// The sky reaches behind the copy, and the copy stays in front of it and clickable.
const layout = await page.evaluate(() => {
  const inside = (a, b) => a.left >= b.left - 1 && a.right <= b.right + 1 && a.top >= b.top - 1 && a.bottom <= b.bottom + 1;
  const canvas = document.querySelector('#hero-scene').getBoundingClientRect(), copy = document.querySelector('.hero-copy').getBoundingClientRect(), button = document.querySelector('.hero .button').getBoundingClientRect();
  return { covers: inside(copy, canvas), fullWidth: Math.abs(canvas.width - document.documentElement.clientWidth) < 2, clickable: document.elementFromPoint(button.left + button.width / 2, button.top + button.height / 2)?.closest('.button') !== null };
});
check(layout.covers && layout.fullWidth, 'Canvas spans the hero behind the copy', layout);
check(layout.clickable, 'Copy stays clickable above the canvas');
const behindCopy = await coverage('.hero-copy');
check(behindCopy.mean > 12 && behindCopy.mean < 45 && behindCopy.lit > .0005 && behindCopy.lit < .1, 'Sky behind the copy is lit but stays dim', behindCopy);
check(await animates(), 'Scene animates');
const rate = await frameRate();
check(rate > 50, 'Frame rate holds', rate);
check(await page.evaluate(() => [...document.querySelectorAll('img[src="/mark.png"]')].length === 4 && !document.querySelector('img[src$=".svg"]')), 'Brand marks use the icon ribbon');
await shot('hero-galaxy-desktop.png');

await page.locator('.motion-toggle').click();
check(!(await animates()), 'Manual pause freezes canvas');
check(await page.locator('.motion-toggle').getAttribute('aria-label') === '메인 애니메이션 재생', 'Pause toggle label');
await page.locator('.motion-toggle').click();
check(await animates(), 'Resume animates canvas');

// Resting the pointer on the top bar of the S lights that part of the stream; pressing must not throw.
const spotOnS = await page.evaluate(() => { const r = document.querySelector('.hero-art').getBoundingClientRect(), unit = Math.min(r.height * .7, r.width * .735); return { x: r.left + r.width / 2 + unit * .05, y: r.top + r.height / 2 - unit * .33 }; });
const around = { left: spotOnS.x - 14, top: spotOnS.y - 14, width: 28, height: 28 };
const before = await coverage(around);
await page.mouse.move(spotOnS.x - 60, spotOnS.y - 8); await page.mouse.move(spotOnS.x - 20, spotOnS.y - 3); await page.mouse.move(spotOnS.x, spotOnS.y);
await sleep(700);
const hovered = await coverage(around);
check(hovered.mean > before.mean * 1.2, 'Pointer lights the stream under it', [before.mean, hovered.mean]);
await shot('hero-galaxy-hover.png');
await page.mouse.down(); await page.mouse.up();
for (const [x, y] of [[1010, 430], [300, 400], [900, 300]]) { await page.mouse.move(x, y); await page.mouse.down(); await page.mouse.up(); }
check(await animates(), 'Still animating after pointer input');
await page.mouse.move(700, 880);

await page.setViewportSize({ width: 390, height: 844 });
await sleep(3000);
const mobile = await coverage('.hero-art');
check(drawn(mobile), 'Mobile draws the galaxy', mobile);
check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'No horizontal overflow on mobile');
await page.evaluate(() => document.querySelector('.hero-art').scrollIntoView({ behavior: 'instant', block: 'center' }));
await sleep(500); await shot('hero-galaxy-mobile.png');

await page.setViewportSize({ width: 1440, height: 900 });
await page.emulateMedia({ reducedMotion: 'reduce' });
await page.reload(); await sleep(2000);
// The reload restores the mobile scroll position.
await page.evaluate(() => scrollTo({ top: 0, behavior: 'instant' })); await sleep(400);
check(!(await animates()), 'Reduced motion freezes canvas');
check(await page.locator('.motion-toggle').isDisabled(), 'Reduced motion disables the toggle');
const still = await coverage('.hero-art');
check(drawn(still) && starShown(still), 'Reduced motion still shows galaxy and star', still);
await shot('hero-galaxy-reduced.png');
await page.emulateMedia({ reducedMotion: 'no-preference' });

const errors = (await page.console.logs()).filter((entry) => entry.level === 'error' || entry.level === 'warning');
check(errors.length === 0, 'No console errors or warnings', errors);
console.log(JSON.stringify({ passed: results.filter((r) => r.pass).length, total: results.length, results }, null, 1));
await fs.writeFile('./artifacts/HERO_SCENE_CHECK_DONE', 'x');
await sleep(25000);
