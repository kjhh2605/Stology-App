// 기능 탐색기: 왼쪽 목록에서 항목을 고르면 오른쪽에 실제 UI를 재현한 장면이 재생된다.
import './feature-explorer.css';
import { createPlayer } from './scenes/runtime.js';
import { UI } from './scenes/ui-labels.js';
import { track } from './analytics.js';

const esc = v => String(v ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const pad = n => String(n).padStart(2, '0');
const keyGroup = keys => `<span class="key-group">${keys.map(k => `<kbd>${esc(k)}</kbd>`).join('')}</span>`;
const toolLabel = id => UI.tools.find(t => t.id === id)?.label || id;
const normalize = v => String(v).toLowerCase().replace(/command|cmd/g, '⌘').replace(/option|alt/g, '⌥').replace(/shift/g, '⇧').replace(/[\s+]/g, '');

function inputMarkup(item, S, placement = 'panel') {
  const X = S.explorer;
  const parts = [];
  if (item.keys) parts.push(keyGroup(item.keys));
  if (item.tool) parts.push(`<span class="tool-tag">${esc(toolLabel(item.tool))}</span>`);
  if (item.chip) parts.push(`<span class="tool-tag">${esc(X.chips[item.chip])}</span>`);
  if (placement === 'panel' && item.alt) {
    item.alt.forEach(keys => {
      const shown = keys.map(k => k.replace('{t}', X.textToken));
      parts.push(`<span class="key-alternative"><span class="key-or">${X.or}</span>${keyGroup(shown)}</span>`);
    });
  }
  return parts.join('');
}

export function createExplorerMarkup(id, items, S) {
  const X = S.explorer;
  const categories = [...new Set(items.map(i => i.category))];
  const coreCount = items.filter(i => i.core).length;
  return `<div class="feature-explorer demo-layout" data-demo="${id}">
    <div class="explorer-sidebar">
      <div class="explorer-mode" role="group" aria-label="${esc(X.core)} / ${esc(X.everything)}"><button class="mode on" data-mode="core" aria-pressed="true">${esc(X.core)} <b>${coreCount}</b></button><button class="mode" data-mode="all" aria-pressed="false">${esc(X.everything)} <b>${items.length}</b></button></div>
      <div class="explorer-filter"><label class="search-field"><span aria-hidden="true">⌕</span><input type="search" aria-label="${esc(X.searchAria[id])}" placeholder="${esc(X.searchPlaceholder)}" autocomplete="off"><span class="search-count">${coreCount}</span></label><label class="category-field"><span>${esc(X.categoryLabel)}</span><select aria-label="${esc(X.categoryLabel)}"><option value="">${esc(X.all)}</option>${categories.map(c => `<option value="${c}">${esc(S.categories[c])}</option>`).join('')}</select></label></div>
      <div class="explorer-list" role="tablist" aria-label="${esc(X.listAria[id])}" aria-orientation="vertical"></div>
      <div class="explorer-empty" hidden><p>${esc(X.empty)}</p><button class="clear-filter">${esc(X.clear)}</button></div>
      <p class="demo-hint">${esc(X.hint[id])}</p>
    </div>
    <div class="demo-panel" id="${id}-panel" role="tabpanel" tabindex="0">
      <div class="scene-shell"><div class="scene-topbar"><span class="live-dot"></span><span class="scene-context"></span><span class="scene-source">${esc(X.stageLabel)}</span></div><div class="scene-stage" role="img" aria-label=""></div><div class="scene-inputs" aria-label="${esc(X.inputsLabel)}"></div></div>
      <div class="demo-caption" aria-live="polite"><div class="caption-icon" aria-hidden="true">↳</div><div><h3></h3><p></p></div><span class="caption-count"></span></div>
    </div></div>`;
}

export function mountExplorer(id, items, S, { lang, scenes }) {
  const root = document.querySelector(`[data-demo="${id}"]`);
  const list = root.querySelector('.explorer-list'), search = root.querySelector('input[type="search"]'), category = root.querySelector('select');
  const panel = root.querySelector('.demo-panel'), empty = root.querySelector('.explorer-empty'), stage = root.querySelector('.scene-stage');
  const modes = [...root.querySelectorAll('.mode')];
  let mode = 'core', current = items[0], filtered = items, player = null;

  function updatePanel(item) {
    current = item;
    const text = S.items[item.id];
    panel.setAttribute('aria-labelledby', `${id}-tab-${item.id}`);
    root.querySelector('.scene-context').textContent = `${S.categories[item.category]} · ${text.label}`;
    root.querySelector('.scene-inputs').innerHTML = inputMarkup(item, S);
    root.querySelector('.demo-caption h3').textContent = text.title;
    root.querySelector('.demo-caption p').textContent = text.body;
    root.querySelector('.caption-count').textContent = `${pad(items.indexOf(item) + 1)} / ${pad(items.length)}`;
    stage.setAttribute('aria-label', `${text.title}. ${text.body}`);
    list.querySelectorAll('[role="tab"]').forEach(tab => { const on = tab.dataset.item === item.id; tab.setAttribute('aria-selected', String(on)); tab.tabIndex = on ? 0 : -1; });
    player?.destroy();
    const scene = scenes[item.id];
    player = scene ? createPlayer(stage, ctx => scene({ ...ctx, S, lang }), {}) : null;
  }
  function paintList() {
    const term = normalize(search.value);
    const pool = term ? items : items.filter(i => mode === 'all' || i.core);
    filtered = pool.filter(item => {
      if (category.value && item.category !== category.value) return false;
      const text = S.items[item.id];
      const hay = normalize([text.label, text.title, text.body, S.categories[item.category], ...(item.keys || []), ...(item.alt || []).flat(), item.tool ? toolLabel(item.tool) : '', item.chip ? S.explorer.chips[item.chip] : ''].join(' '));
      return hay.includes(term);
    });
    root.querySelector('.search-count').textContent = filtered.length;
    empty.hidden = !!filtered.length; list.hidden = !filtered.length; panel.hidden = !filtered.length;
    list.innerHTML = filtered.map(item => `<button class="demo-tab" role="tab" id="${id}-tab-${item.id}" aria-controls="${id}-panel" aria-selected="${item === current}" tabindex="${item === current ? 0 : -1}" data-item="${item.id}"><span class="tab-number">${pad(items.indexOf(item) + 1)}</span><span class="tab-label">${esc(S.items[item.id].label)}</span><span class="tab-keys">${inputMarkup(item, S, 'list')}</span></button>`).join('');
    if (filtered.length) updatePanel(filtered.includes(current) ? current : filtered[0]);
    else { player?.destroy(); player = null; }
  }
  modes.forEach(btn => btn.addEventListener('click', () => {
    mode = btn.dataset.mode;
    modes.forEach(b => { const on = b === btn; b.classList.toggle('on', on); b.setAttribute('aria-pressed', String(on)); });
    track('scene_view_all', { section: id, mode });
    paintList();
  }));
  list.addEventListener('click', event => { const tab = event.target.closest('[role="tab"]'); if (!tab) return; const item = items.find(i => i.id === tab.dataset.item); updatePanel(item); track('scene_select', { id: item.id, section: id }); });
  list.addEventListener('keydown', event => {
    const tab = event.target.closest('[role="tab"]'); if (!tab) return;
    const index = filtered.findIndex(i => i.id === tab.dataset.item); let next;
    if (['ArrowDown', 'ArrowRight'].includes(event.key)) next = (index + 1) % filtered.length;
    if (['ArrowUp', 'ArrowLeft'].includes(event.key)) next = (index - 1 + filtered.length) % filtered.length;
    if (event.key === 'Home') next = 0; if (event.key === 'End') next = filtered.length - 1;
    if (next !== undefined) { event.preventDefault(); updatePanel(filtered[next]); const focus = list.querySelector(`[data-item="${filtered[next].id}"]`); focus.focus({ preventScroll: true }); focus.scrollIntoView({ block: 'nearest', behavior: 'instant' }); }
  });
  search.addEventListener('input', paintList); category.addEventListener('change', paintList);
  root.querySelector('.clear-filter').addEventListener('click', () => { search.value = ''; category.value = ''; paintList(); search.focus(); });
  paintList();
  return { destroy: () => player?.destroy() };
}
