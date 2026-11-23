// Run with: aside repl --file landing/qa/rebuild-check-mobile-en.js   (dev server on port 5199)
// Mobile 390×844 Korean page, English desktop page, and reduced-motion state of the explorer.
const base = 'http://127.0.0.1:5199/';
const errors = [];
await fs.mkdir('./artifacts', { recursive: true });
console.log('PWD', typeof pwd === 'function' ? await pwd() : pwd);
await openTab(base);
const shot = async (name) => { await fs.writeFile('./artifacts/' + name, await page.screenshot()); console.log('shot', name); };
const scrollTo = async (sel, offset = -80) => { await page.evaluate(([s, o]) => { const el = document.querySelector(s); window.scrollTo({ top: el.getBoundingClientRect().top + window.scrollY + o, behavior: 'instant' }); }, [sel, offset]); };
await page.setViewportSize({ width: 390, height: 844 });
await sleep(1500); await shot('m-hero.png');
await scrollTo('#one-window', -60); await sleep(4000); await shot('m-window.png');
await scrollTo('#today', -60); await sleep(2600); await shot('m-today.png');
await scrollTo('#notes', -60); await sleep(1200); await shot('m-notes-list.png');
await scrollTo('[data-demo="notes"] .scene-shell', -70); await sleep(3800); await shot('m-notes-scene.png');
await scrollTo('#start', -60); await sleep(800); await shot('m-start.png');
// English page
await page.setViewportSize({ width: 1440, height: 900 });
await page.goto(base + 'en/'); await sleep(1800); await shot('e-hero.png');
await scrollTo('#one-window', -40); await sleep(9000); await shot('e-window.png');
await scrollTo('#notes'); await sleep(3900); await shot('e-notes-bold.png');
await scrollTo('#pdf'); await page.locator('[data-demo="pdf"] [data-item="text-note"]').click(); await sleep(4200); await shot('e-pdf-note.png');
await scrollTo('#faq'); await sleep(500); await shot('e-faq.png');
const head = await page.evaluate(() => ({ lang: document.documentElement.lang, title: document.title, ogLocale: document.querySelector('meta[property="og:locale"]')?.content, toggle: document.querySelector('.lang-toggle')?.textContent.trim(), ogImage: document.querySelector('meta[property="og:image"]')?.content }));
console.log('HEAD_EN', JSON.stringify(head));
// Reduced motion: final states should be shown with the key toast sticky
await page.emulateMedia({ reducedMotion: 'reduce' });
await page.goto(base); await sleep(1200);
await scrollTo('#notes'); await sleep(1200); await shot('r-notes-bold.png');
await page.locator('[data-demo="notes"] [data-item="selection"]').click(); await sleep(800); await shot('r-notes-selection.png');
await scrollTo('#today', -60); await sleep(800); await shot('r-today.png');
const reduced = await page.evaluate(() => ({ toastShown: [...document.querySelectorAll('.key-toast')].some(t => t.classList.contains('show')), merged: document.querySelector('.problem-stage')?.classList.contains('merged'), motionToggleDisabled: document.querySelector('.motion-toggle')?.disabled }));
console.log('REDUCED', JSON.stringify(reduced));
try { const logs = await page.console.logs(); const bad = (Array.isArray(logs) ? logs : []).filter(l => /error|warn/i.test(l.type || l.level || '')).map(l => (l.type || l.level) + ': ' + (l.text || l.message || JSON.stringify(l))); errors.push(...bad); } catch (e) { errors.push('console.logs unavailable: ' + e.message); }
console.log('ERRORS', JSON.stringify(errors.slice(0, 20)));
console.log('DONE_MOBILE_EN');
await sleep(40000);
