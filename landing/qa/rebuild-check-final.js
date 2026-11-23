// Run with: aside repl --file landing/qa/rebuild-check-final.js   (dev server on port 5199)
// Final pass: merged cards, map search, resized note, then the Open Graph cards drawn with the live hero scene.
const base = 'http://127.0.0.1:5199/';
await fs.mkdir('./artifacts', { recursive: true });
await openTab(base);
await page.setViewportSize({ width: 1440, height: 900 });
const shot = async (name, opts) => { await fs.writeFile('./artifacts/' + name, await page.screenshot(opts)); console.log('shot', name); };
const scrollTo = async (sel, offset = -90) => { await page.evaluate(([s, o]) => { const el = document.querySelector(s); window.scrollTo({ top: el.getBoundingClientRect().top + window.scrollY + o, behavior: 'instant' }); }, [sel, offset]); };
await sleep(800);
await scrollTo('#today'); await sleep(3600); await shot('g-today-merged.png');
await scrollTo('#notes'); await page.locator('[data-demo="notes"] .mode[data-mode="all"]').click();
await page.locator('[data-demo="notes"] [data-item="map-search-focus"]').click(); await sleep(3600); await shot('g-notes-map.png');
await scrollTo('#pdf'); await page.locator('[data-demo="pdf"] .mode[data-mode="all"]').click();
await page.locator('[data-demo="pdf"] [data-item="pdf-resize"]').click(); await sleep(3400); await shot('g-pdf-resize.png');
for (const l of ['ko', 'en']) { await page.goto(base + 'qa/og/' + l + '.html'); await page.setViewportSize({ width: 1200, height: 630 }); await sleep(1600); await shot('og-' + l + '.png', { clip: { x: 0, y: 0, width: 1200, height: 630 } }); }
console.log('DONE_FINAL');
await sleep(30000);
