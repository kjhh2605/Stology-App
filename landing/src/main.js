import './style.css';
import './sections.css';
import './app-ui.css';
import ko from './i18n/ko.js';
import en from './i18n/en.js';
import { initHeroScene } from './hero-scene.js';
import { noteItems, pdfItems } from './catalog.js';
import { createExplorerMarkup, mountExplorer } from './feature-explorer.js';
import { createPlayer } from './scenes/runtime.js';
import { windowLoop } from './scenes/window-loop.js';
import { noteScenes } from './scenes/note-scenes.js';
import { pdfScenes } from './scenes/pdf-scenes.js';
import { initAnalytics, bindTracking, track } from './analytics.js';
import { DOWNLOAD_URL, DOWNLOAD_VERSION, RELEASES_URL, DISCORD_URL } from './config.js';

const lang = document.documentElement.lang === 'en' ? 'en' : 'ko';
const S = lang === 'en' ? en : ko;
initAnalytics();

const esc = v => String(v ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const downloadIcon = '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 4v11M7 10l5 5 5-5M5 20h14" stroke="currentColor" stroke-width="1.5"/></svg>';
const discordIcon = '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M19.6 5.6A17 17 0 0 0 15.4 4l-.5 1a15.7 15.7 0 0 0-5.8 0l-.5-1a17 17 0 0 0-4.2 1.6C1.7 9.6 1 13.5 1.3 17.3A17.3 17.3 0 0 0 6.5 20l1.1-1.7c-.6-.2-1.2-.5-1.7-.8l.4-.3a12.3 12.3 0 0 0 11.4 0l.4.3c-.5.3-1.1.6-1.7.8l1.1 1.7a17.2 17.2 0 0 0 5.2-2.7c.4-4.4-.7-8.3-3.1-11.7zM8.7 15c-1 0-1.9-1-1.9-2.1s.8-2.2 1.9-2.2 1.9 1 1.9 2.2S9.8 15 8.7 15zm6.6 0c-1 0-1.9-1-1.9-2.1s.8-2.2 1.9-2.2 1.9 1 1.9 2.2-.8 2.1-1.9 2.1z"/></svg>';
const icon = (name) => {
  const paths = {
    note: 'M6 3h9l4 4v14H6z M14 3v5h5 M9 12h7 M9 16h5',
    account: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8z M4 21a8 8 0 0 1 16 0 M17 7l2 2 3-3',
    files: 'M7 3h7l4 4v4 M7 3v14h5 M14 3v4h4 M13 13h8v8h-8z M16 17h2',
    shield: 'M12 3l8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3z M8 12l3 3 5-6',
    map: 'M6 6l12 3-6 10L6 6 M6 6l6 13 M6 6h.01 M18 9h.01',
    spark: 'M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z M5 17l.7 2 2 .7-2 .7-.7 2-.7-2-2-.7 2-.7z',
  };
  return `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="${paths[name] || paths.note}" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
};
const downloadLink = (place, cls = 'button button-primary') => `<a class="${cls}" href="${DOWNLOAD_URL}" download data-track="download_click:${place}">${esc(S.nav.download)} ${downloadIcon}</a>`;
const discordLink = (label, place, cls = 'discord-link') => `<a class="${cls}" href="${DISCORD_URL}" target="_blank" rel="noopener" aria-label="${esc(S.discordUrlLabel)}" data-track="outbound:discord-${place}">${discordIcon}<span>${esc(label)}</span></a>`;
const heading = (sec, extra = '') => `<div class="section-heading reveal"><div><p class="section-index">${sec.index} <span>/</span> ${esc(sec.label)}</p><h2>${sec.title}</h2></div><p>${esc(sec.body || '')}${extra}</p></div>`;

document.querySelector('#app').innerHTML = `
<a class="skip-link" href="#main">${esc(S.nav.skip)}</a>
<header class="site-header"><div class="header-inner">
  <a class="brand" href="${S.path}" aria-label="${esc(S.nav.brandAria)}"><img src="/mark.png" alt="" width="30" height="30"><span>stology<span class="brand-dot">.</span></span></a>
  <nav aria-label="${esc(S.nav.menuAria)}"><a href="#one-window">${esc(S.nav.overview)}</a><a href="#notes">${esc(S.nav.features)}</a><a href="#start">${esc(S.nav.start)}</a><a href="#faq">${esc(S.nav.faq)}</a></nav>
  <div class="header-right"><div class="lang-toggle" role="group" aria-label="${esc(S.nav.langAria)}">${lang === 'ko' ? `<span lang="ko" aria-current="true">KO</span><a href="/en/" hreflang="en" lang="en" aria-label="${esc(S.other.aria)}" data-lang-link>EN</a>` : `<a href="/" hreflang="ko" lang="ko" aria-label="${esc(S.other.aria)}" data-lang-link>KO</a><span lang="en" aria-current="true">EN</span>`}</div>${downloadLink('header', 'environment-link')}</div>
</div></header>
<main id="main">
<div class="hero-stage"><canvas id="hero-scene" role="img" aria-label="${esc(S.hero.canvasAria)}"></canvas><section class="hero container" aria-labelledby="hero-title">
  <div class="hero-copy"><div class="eyebrow"><span class="status-light"></span> ${esc(S.hero.eyebrow)} <span class="eyebrow-line"></span></div><h1 id="hero-title">${esc(S.hero.title)}<br><span>${esc(S.hero.accent)}</span></h1><p class="hero-description">${esc(S.hero.body)}</p><div class="hero-actions">${downloadLink('hero')}<a class="text-link" href="#one-window">${esc(S.hero.secondary)} <span>↘</span></a></div><div class="hero-platform"><svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="4" y="3" width="16" height="12" rx="2" stroke="currentColor"/><path d="M12 15v5m-5 0h10" stroke="currentColor"/></svg> ${S.hero.req.map(esc).join(' <i></i> ')}</div></div>
  <div class="hero-art"><div class="art-label label-top"><span class="tiny-cross">+</span> ${esc(S.hero.labelTop)}</div><div class="art-label label-bottom"><span class="tiny-cross">+</span> ${esc(S.hero.labelBottom)}</div><div class="art-coordinate">${esc(S.hero.coordinate)}</div><button class="motion-toggle" aria-label="${esc(S.hero.pause)}" aria-pressed="false">Ⅱ</button></div>
  <div class="hero-bottom"><div class="trust-list">${S.hero.trust.map(t => `<span><i></i>${esc(t)}</span>`).join('')}</div><a href="#one-window" aria-label="${esc(S.hero.scrollAria)}">↓</a></div>
</section></div>

<section id="one-window" class="one-window section container">${heading(S.window)}
  <div class="reveal"><div class="frame-label"><span class="status-light"></span> ${esc(S.window.frameLabel)} <span>${esc(S.window.frameNote)}</span></div><div class="window-stage" role="img" aria-label="${esc(S.window.aria)}"></div><div class="step-chips" aria-hidden="true">${S.window.steps.map((s, i) => `<span data-step="${i}"><b>0${i + 1}</b>${esc(s)}</span>`).join('')}</div></div>
</section>

<section id="today" class="problem section container">${heading(S.problem)}
  <div class="problem-stage"><div class="problem-cards">${S.problem.cards.map(c => `<article class="pcard"><div class="bar"><i></i><i></i><i></i><span>${esc(c.tag)}</span></div><div class="lines"><i></i><i></i><i></i></div><h3>${esc(c.h)}</h3><p>${esc(c.p)}</p></article>`).join('')}</div></div>
  <p class="resolve"><span>${esc(S.problem.resolve)}</span></p>
</section>

<section id="promises" class="section container">${heading(S.promises)}
  <div class="promise-grid reveal">${S.promises.items.map((p, i) => `<article class="promise"><span class="p-num">0${i + 1}</span><div class="p-icon">${icon(p.icon)}</div><h3>${esc(p.h)}</h3><p>${esc(p.p)}</p></article>`).join('')}</div>
  <div class="promise-foot reveal"><span>${esc(S.promises.note)}</span><a href="#faq-transfer" data-open-faq="transfer">${esc(S.promises.link)}</a></div>
</section>

<section id="notes" class="section demo-section container">${heading(S.notes)}${createExplorerMarkup('notes', noteItems, S)}</section>

<section id="pdf" class="pdf-section"><div class="container section">${heading(S.pdf)}${createExplorerMarkup('pdf', pdfItems, S)}<div class="section-footnote">${icon('note')}<p>${esc(S.pdf.footnote)}</p></div></div></section>

<section id="map" class="map-section section container">${heading(S.map)}
  <div class="atlas-stage reveal" role="img" aria-label="${esc(S.map.aria)}"></div>
  <p class="map-note">${esc(S.map.note)}</p>
</section>

<section id="start" class="steps-section section container">${heading(S.start)}
  <div class="step-grid reveal">${S.start.steps.map((s, i) => `<article class="step"><div class="s-num">0${i + 1}</div><h3>${esc(s.h)}</h3><p>${esc(s.p)}</p></article>`).join('')}</div>
  <div class="step-foot reveal"><p class="note">${esc(S.start.note)}</p><div class="actions">${downloadLink('steps')}${discordLink(S.start.discord, 'steps')}</div></div>
</section>

<section id="faq" class="faq-section container section"><div><p class="section-index">${S.faq.index} <span>/</span> ${esc(S.faq.label)}</p><h2>${S.faq.title}</h2></div><div class="faq-list">${S.faq.items.map(item => `<details${item.id ? ` id="faq-${item.id}"` : ''}><summary>${esc(item.q)}<span>+</span></summary><p>${esc(item.a)}</p>${item.discord ? discordLink('Discord', 'faq') : ''}</details>`).join('')}</div></section>

<section class="closing container"><div class="closing-orbit" aria-hidden="true"></div><div class="closing-mark" aria-hidden="true"><img src="/mark.png" alt="" width="64" height="64"><img class="closing-star" src="/media/hero/star.webp" alt="" width="46" height="46" loading="lazy"></div><p class="section-index">${esc(S.closing.index)}</p><h2>${esc(S.closing.title)}</h2><p>${esc(S.closing.body)}</p><div class="closing-links">${downloadLink('closing')}${discordLink(S.closing.discord, 'closing')}</div></section>
</main>
<footer class="container"><a class="brand" href="${S.path}"><img src="/mark.png" alt="" width="26" height="26"><span>stology.</span></a><span>${esc(S.footer.tagline)}</span><div><div class="foot-links"><a href="${RELEASES_URL}" target="_blank" rel="noopener" data-track="outbound:release-notes">${esc(S.footer.release)}</a><a href="${DISCORD_URL}" target="_blank" rel="noopener" data-track="outbound:discord-footer">${discordIcon}${esc(S.footer.discord)}</a><button data-open-info>${esc(S.footer.data)}</button></div><small>© ${new Date().getFullYear()} ${esc(S.footer.copyright)}</small></div></footer>
<dialog id="data-dialog" aria-labelledby="dialog-title"><button class="dialog-close" aria-label="${esc(S.dataDialog.close)}">×</button><img src="/mark.png" alt="" width="40" height="40"><p class="section-index">STOLOGY</p><h2 id="dialog-title">${esc(S.dataDialog.title)}</h2><p>${esc(S.dataDialog.p1)}</p><p>${esc(S.dataDialog.p2)}</p></dialog>`;

/* ---------- 히어로 ---------- */
const heroScene = initHeroScene(document.querySelector('#hero-scene'), { anchor: document.querySelector('.hero-art'), surface: document.querySelector('.hero-stage') });
const motionToggle = document.querySelector('.motion-toggle');
const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)');
motionToggle.addEventListener('click', () => { const paused = motionToggle.getAttribute('aria-pressed') !== 'true'; heroScene.setPaused?.(paused); motionToggle.setAttribute('aria-pressed', String(paused)); motionToggle.setAttribute('aria-label', paused ? S.hero.play : S.hero.pause); motionToggle.textContent = paused ? '▷' : 'Ⅱ'; });
function syncMotionPreference() { motionToggle.disabled = reduceMotion.matches; if (reduceMotion.matches) { motionToggle.setAttribute('aria-label', S.hero.reduced); motionToggle.textContent = '▷'; } else { const paused = motionToggle.getAttribute('aria-pressed') === 'true'; motionToggle.setAttribute('aria-label', paused ? S.hero.play : S.hero.pause); motionToggle.textContent = paused ? '▷' : 'Ⅱ'; } }
reduceMotion.addEventListener('change', syncMotionPreference); syncMotionPreference();

/* ---------- 장면: 글꼴이 준비된 뒤에 치수를 재야 한다 ---------- */
const chips = [...document.querySelectorAll('.step-chips span')];
const mountScenes = () => {
  createPlayer(document.querySelector('.window-stage'), ctx => windowLoop({ ...ctx, S, lang, onStep: i => chips.forEach((c, n) => c.classList.toggle('on', n === i)) }), {});
  mountExplorer('notes', noteItems, S, { lang, scenes: noteScenes });
  mountExplorer('pdf', pdfItems, S, { lang, scenes: pdfScenes });
  // 과목 지도 장면(지도 데이터·엔진)은 페이지 아래쪽이라 따로 받아 첫 화면 번들을 늘리지 않는다
  import('./scenes/atlas-loop.js').then(({ atlasLoop }) => createPlayer(document.querySelector('.atlas-stage'), ctx => atlasLoop({ ...ctx, S, lang }), {}));
};
(document.fonts?.ready || Promise.resolve()).then(mountScenes);

/* ---------- 드러나기 · 문제 섹션 ---------- */
const revealObserver = new IntersectionObserver(entries => { entries.forEach(entry => { if (entry.isIntersecting) { entry.target.classList.add('visible'); revealObserver.unobserve(entry.target); } }); }, { threshold: .08 });
document.querySelectorAll('.reveal').forEach(el => revealObserver.observe(el));
const problemStage = document.querySelector('.problem-stage');
const problemObserver = new IntersectionObserver(entries => { entries.forEach(entry => { if (!entry.isIntersecting) return; problemObserver.disconnect(); problemStage.classList.add('visible'); setTimeout(() => problemStage.classList.add('merged'), reduceMotion.matches ? 0 : 1500); }); }, { threshold: .35 });
problemObserver.observe(problemStage);

/* ---------- 대화상자 · FAQ 연결 ---------- */
const dialog = document.querySelector('#data-dialog');
document.querySelectorAll('[data-open-info]').forEach(b => b.addEventListener('click', () => { dialog.showModal(); track('data_notice_open'); }));
dialog.querySelector('.dialog-close').addEventListener('click', () => dialog.close());
dialog.addEventListener('click', event => { if (event.target === dialog) dialog.close(); });
document.querySelectorAll('[data-open-faq]').forEach(a => a.addEventListener('click', () => { const d = document.querySelector(`#faq-${a.dataset.openFaq}`); if (d) d.open = true; }));
document.querySelectorAll('.faq-list details').forEach(d => d.addEventListener('toggle', () => { if (d.open) track('faq_open', { id: d.id || d.querySelector('summary').textContent.slice(0, 40) }); }));

/* ---------- 메뉴 현재 위치 · 언어 전환 ---------- */
const navLinks = [...document.querySelectorAll('nav a')];
const anchors = ['one-window', 'notes', 'start', 'faq'];
let navScheduled = false;
function updateNav() { navScheduled = false; let activeId = ''; for (const id of anchors) { const el = document.getElementById(id); if (el && el.getBoundingClientRect().top <= innerHeight * .35) activeId = id; } navLinks.forEach(link => { const active = link.hash === `#${activeId}`; link.classList.toggle('active', active); if (active) link.setAttribute('aria-current', 'location'); else link.removeAttribute('aria-current'); }); }
window.addEventListener('scroll', () => { if (!navScheduled) { navScheduled = true; requestAnimationFrame(updateNav); } }, { passive: true }); updateNav();
document.querySelector('[data-lang-link]')?.addEventListener('click', event => { const a = event.currentTarget; a.href = a.getAttribute('href').split('#')[0] + location.hash; try { localStorage.setItem('stology-lang', a.lang); } catch { /* 저장 못 해도 이동은 한다 */ } track('lang_switch', { to: a.lang }); });
bindTracking();
