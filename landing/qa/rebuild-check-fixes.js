// Run with: aside repl --file landing/qa/rebuild-check-fixes.js   (dev server on port 5199)
// Re-checks after the first pass: PDF scenes that target the list items, the merging cards, the mobile eyebrow.
const base = 'http://127.0.0.1:5199/';
await fs.mkdir('./artifacts', { recursive: true });
await openTab(base);
await page.setViewportSize({ width: 1440, height: 900 });
const shot = async (name) => { await fs.writeFile('./artifacts/' + name, await page.screenshot()); console.log('shot', name); };
const scrollTo = async (sel, offset = -90) => { await page.evaluate(([s, o]) => { const el = document.querySelector(s); window.scrollTo({ top: el.getBoundingClientRect().top + window.scrollY + o, behavior: 'instant' }); }, [sel, offset]); };
await sleep(800);
await scrollTo('#today'); await sleep(3400); await shot('f-today-merged.png');
await scrollTo('#pdf'); await sleep(300);
await page.locator('[data-demo="pdf"] [data-item="text-note"]').click(); await sleep(4300); await shot('f-pdf-note.png');
await page.locator('[data-demo="pdf"] [data-item="pen"]').click(); await sleep(4200); await shot('f-pdf-pen.png');
await page.locator('[data-demo="pdf"] [data-item="pdf-area-select"]').click(); await sleep(3600); await shot('f-pdf-area-select.png');
await page.locator('[data-demo="pdf"] .mode[data-mode="all"]').click();
await page.locator('[data-demo="pdf"] [data-item="pdf-delete-many"]').click(); await sleep(3300); await shot('f-pdf-delete.png');
await page.locator('[data-demo="pdf"] [data-item="pdf-resize"]').click(); await sleep(3200); await shot('f-pdf-resize.png');
await page.locator('[data-demo="pdf"] [data-item="pdf-pptx"]').click(); await sleep(4200); await shot('f-pdf-pptx.png');
await scrollTo('#notes'); await page.locator('[data-demo="notes"] .mode[data-mode="all"]').click();
await page.locator('[data-demo="notes"] [data-item="map-search-focus"]').click(); await sleep(3600); await shot('f-notes-map.png');
await page.locator('[data-demo="notes"] [data-item="block-1"]').click(); await sleep(7200); await shot('f-notes-h1.png');
await page.setViewportSize({ width: 390, height: 844 }); await page.goto(base); await sleep(1200); await shot('f-m-hero.png');
console.log('DONE_FIXES');
await sleep(40000);
