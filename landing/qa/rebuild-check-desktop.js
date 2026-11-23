// Run with: aside repl --file landing/qa/rebuild-check-desktop.js   (dev server: npm run dev --prefix landing -- --port 5199 --strictPort)
// Desktop 1440×900, Korean page. Screenshots go to the session's ./artifacts; the final sleep keeps the session open so they can be copied out.
const base = 'http://127.0.0.1:5199/';
const errors = [];
await fs.mkdir('./artifacts', { recursive: true });
console.log('PWD', typeof pwd === 'function' ? await pwd() : pwd);
await openTab(base);
await page.setViewportSize({ width: 1440, height: 900 });
const shot = async (name) => { await fs.writeFile('./artifacts/' + name, await page.screenshot()); console.log('shot', name); };
const scrollTo = async (sel, offset = -90) => { await page.evaluate(([s, o]) => { const el = document.querySelector(s); window.scrollTo({ top: el.getBoundingClientRect().top + window.scrollY + o, behavior: 'instant' }); }, [sel, offset]); };
await sleep(1800);
await shot('d-hero.png');
await scrollTo('#one-window', -40); await sleep(3500); await shot('d-window-a.png'); await sleep(7000); await shot('d-window-b.png');
await scrollTo('#today'); await sleep(2800); await shot('d-today.png');
await scrollTo('#promises'); await sleep(900); await shot('d-promises.png');
await scrollTo('#notes'); await sleep(3900); await shot('d-notes-bold.png');
await page.locator('[data-demo="notes"] [data-item="checklist"]').click(); await sleep(6200); await shot('d-notes-checklist.png');
await page.locator('[data-demo="notes"] .mode[data-mode="all"]').click(); await page.locator('[data-demo="notes"] [data-item="markdown-quote"]').click(); await sleep(4200); await shot('d-notes-quote.png');
await scrollTo('#pdf'); await sleep(3200); await shot('d-pdf-highlight.png');
await page.locator('[data-demo="pdf"] [data-item="pdf-analyze"]').click(); await sleep(3300); await shot('d-pdf-analyze.png');
await page.locator('[data-demo="pdf"] .mode[data-mode="all"]').click(); await page.locator('[data-demo="pdf"] [data-item="pdf-share"]').click(); await sleep(3600); await shot('d-pdf-share.png');
await scrollTo('#map'); await sleep(900); await shot('d-map.png');
await scrollTo('#start'); await sleep(900); await shot('d-start.png');
await scrollTo('#faq'); await page.locator('#faq-transfer summary').click(); await sleep(600); await shot('d-faq.png');
await scrollTo('.closing', -120); await sleep(700); await shot('d-closing.png');
// frame scale sanity + fps on the window loop
await scrollTo('#one-window', -40); await sleep(500);
const metrics = await page.evaluate(() => new Promise(resolve => {
  const wraps = [...document.querySelectorAll('.app-frame-wrap')].map(w => ({ w: w.clientWidth, scale: w.dataset.scale }));
  let frames = 0; const t0 = performance.now();
  const tick = () => { frames++; if (performance.now() - t0 < 2000) requestAnimationFrame(tick); else resolve({ fps: Math.round(frames / 2), wraps, lang: document.documentElement.lang, title: document.title, keyToasts: document.querySelectorAll('.key-toast').length }); };
  requestAnimationFrame(tick);
}));
console.log('METRICS', JSON.stringify(metrics));
try { const logs = await page.console.logs(); const bad = (Array.isArray(logs) ? logs : []).filter(l => /error|warn/i.test(l.type || l.level || '')).map(l => (l.type || l.level) + ': ' + (l.text || l.message || JSON.stringify(l))); errors.push(...bad); } catch (e) { errors.push('console.logs unavailable: ' + e.message); }
console.log('ERRORS', JSON.stringify(errors.slice(0, 20)));
console.log('DONE_DESKTOP');
await sleep(45000);
