import { createNoteScene } from './note-scenes.js';
import { createPdfScene } from './pdf-scenes.js';
import './note-scenes.css';
import './pdf-scenes.css';

export function createMotionPlayer(element, initialItem, kind) {
  const stage = element.querySelector('.motion-stage');
  const play = element.querySelector('.motion-play');
  const restart = element.querySelector('.motion-restart');
  const scrubber = element.querySelector('.motion-scrubber');
  const clock = element.querySelector('.motion-clock');
  const keyboard = element.querySelector('.motion-keyboard');
  const preference = matchMedia('(prefers-reduced-motion: reduce)');
  const duration = 7200;
  let scene, item, frame = 0, last = 0, elapsed = 0, visible = false;
  let paused = preference.matches, cycle = 0, destroyed = false;
  const wantsToPlay = () => !paused && visible && !document.hidden && !destroyed;
  const draw = () => {
    const progress = elapsed / duration;
    stage.dataset.progress = progress.toFixed(4);
    stage.dataset.cycle = String(cycle);
    scene?.render(progress);
    scrubber.value = String(progress * 100);
    clock.textContent = `0:0${Math.min(7, Math.floor(elapsed / 1000))} / 0:07`;
    const pulses = ['selection','code-selection-expand'].includes(item?.id) ? [.25,.43,.61] : item?.id === 'empty-list-exit' ? [.25,.56] : [.25];
    keyboard.classList.toggle('is-pressed', pulses.some(start=>progress>=start&&progress<start+.105));
  };
  const updateControl = () => {
    play.textContent = paused ? '▷' : 'Ⅱ';
    play.setAttribute('aria-label', paused ? '모션 재생' : '모션 일시정지');
    play.setAttribute('aria-pressed', String(paused));
  };
  function tick(now) {
    frame = 0;
    if (!wantsToPlay()) { last = 0; return; }
    if (last) elapsed += Math.min(now - last, 80);
    last = now;
    if (elapsed >= duration) { elapsed %= duration; cycle++; }
    draw();
    frame = requestAnimationFrame(tick);
  }
  const schedule = () => { cancelAnimationFrame(frame); frame = 0; last = 0; if (wantsToPlay()) frame = requestAnimationFrame(tick); };
  const setItem = next => {
    scene?.destroy(); item = next;
    stage.dataset.scene = item.id;
    stage.setAttribute('aria-label', `${item.label} 동작을 설명하는 모션그래픽`);
    scene = (kind === 'pdf' ? createPdfScene : createNoteScene)(stage, item);
    keyboard.replaceChildren();
    for (const key of item.keys || [item.tag || '클릭']) { const cap = document.createElement('kbd'); cap.textContent = key; keyboard.append(cap); }
    elapsed = preference.matches ? duration * .8 : 0;
    cycle = 0; paused = preference.matches;
    updateControl(); draw(); schedule();
  };
  const toggle = () => { paused = !paused; if (!paused && elapsed >= duration) elapsed = 0; updateControl(); schedule(); };
  const replay = () => { elapsed = 0; cycle = 0; paused = false; updateControl(); draw(); schedule(); };
  const seek = () => { elapsed = Number(scrubber.value) / 100 * (duration - 1); paused = true; updateControl(); draw(); schedule(); };
  const onPreference = () => { paused = preference.matches; if (paused) elapsed = duration * .8; updateControl(); draw(); schedule(); };
  play.addEventListener('click', toggle); restart.addEventListener('click', replay); scrubber.addEventListener('input', seek);
  preference.addEventListener('change', onPreference); document.addEventListener('visibilitychange', schedule);
  const observer = new IntersectionObserver(entries => { visible = entries[0].isIntersecting; schedule(); }, { threshold: .15 });
  observer.observe(element);
  setItem(initialItem);
  return { setItem, destroy() { destroyed = true; cancelAnimationFrame(frame); observer.disconnect(); scene?.destroy(); preference.removeEventListener('change', onPreference); document.removeEventListener('visibilitychange', schedule); play.removeEventListener('click', toggle); restart.removeEventListener('click', replay); scrubber.removeEventListener('input', seek); } };
}
