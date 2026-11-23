// 장면 런타임. requestAnimationFrame 타임라인 하나가 단계를 시각 순서대로 실행한다.
// 화면 밖·백그라운드에서는 멈추고, 동작 줄이기에서는 최종 상태를 바로 보여 준다.
export const easeInOut = t => (t < .5 ? 2 * t * t : -1 + (4 - 2 * t) * t);
export const easeOut = t => 1 - Math.pow(1 - t, 3);
export const linear = t => t;

export class Timeline {
  constructor() { this.steps = []; this.end = 0; }
  /** 즉시 실행 단계 */
  at(t, fn) { this.steps.push({ t, fn }); this.end = Math.max(this.end, t); return t; }
  /** t부터 dur 동안 fn(p) 호출. 마지막에 반드시 p=1로 호출된다 */
  during(t, dur, fn, ease = easeInOut) { this.steps.push({ t, dur, fn, ease }); this.end = Math.max(this.end, t + dur); return t + dur; }
  /** 아무 일도 없이 시간만 보낸다 */
  wait(t, dur) { this.end = Math.max(this.end, t + dur); return t + dur; }
}

const reduceQuery = typeof matchMedia === 'function' ? matchMedia('(prefers-reduced-motion: reduce)') : { matches: false, addEventListener() {} };

/**
 * createPlayer(host, build, options)
 * build({ root, tl, reduced, strings }) 는 root 안에 프레임 DOM을 만들고 tl에 단계를 예약한다.
 * 반환값 { hold, gap } 으로 마지막 유지 시간과 반복 사이 간격을 조정한다.
 */
export function createPlayer(host, build, options = {}) {
  const root = document.createElement('div');
  root.className = 'scene-root';
  host.replaceChildren(root);
  let tl, steps, instantIndex, actives, total, gap, start = 0, elapsedAtPause = 0, raf = 0, running = false, destroyed = false;
  let visible = true, pageVisible = !document.hidden;
  const reduced = () => reduceQuery.matches || options.reduced === true;

  function buildOnce() {
    tl = new Timeline();
    (root._cleanup || []).forEach(fn => fn());
    root._cleanup = [];
    root.replaceChildren();
    const info = build({ root, tl, reduced: reduced(), strings: options.strings }) || {};
    total = tl.end + (info.hold ?? 1600);
    gap = info.gap ?? 900;
    steps = tl.steps.slice().sort((a, b) => a.t - b.t || (a.dur ? 1 : -1));
    instantIndex = 0; actives = new Set();
    if (options.onBuild) options.onBuild(root);
  }
  function finishAll() {
    // 동작 줄이기: 모든 단계를 순서대로 끝 상태로 적용한다.
    for (const step of steps) step.dur ? step.fn(1) : step.fn();
  }
  function frame(now) {
    raf = 0;
    if (!running) return;
    const elapsed = now - start;
    while (instantIndex < steps.length && steps[instantIndex].t <= elapsed) {
      const step = steps[instantIndex++];
      if (step.dur) { actives.add(step); } else step.fn();
    }
    for (const step of actives) {
      const p = Math.min(1, (elapsed - step.t) / step.dur);
      step.fn(step.ease(p));
      if (p >= 1) actives.delete(step);
    }
    if (elapsed >= total + gap) { restart(now); return; }
    raf = requestAnimationFrame(frame);
  }
  function restart(now) {
    buildOnce();
    start = now ?? performance.now();
    elapsedAtPause = 0;
    raf = requestAnimationFrame(frame);
  }
  function play() {
    if (destroyed || running) return;
    if (reduced()) { buildOnce(); finishAll(); return; }
    running = true;
    start = performance.now() - elapsedAtPause;
    if (!raf) raf = requestAnimationFrame(frame);
  }
  function pause() {
    if (!running) return;
    running = false;
    elapsedAtPause = performance.now() - start;
    if (raf) cancelAnimationFrame(raf); raf = 0;
  }
  function sync() { (visible && pageVisible) ? play() : pause(); }
  const observer = new IntersectionObserver(entries => { visible = entries.some(e => e.isIntersecting); sync(); }, { threshold: .05 });
  observer.observe(host);
  const onVisibility = () => { pageVisible = !document.hidden; sync(); };
  document.addEventListener('visibilitychange', onVisibility);
  const onReduce = () => { pause(); elapsedAtPause = 0; buildOnce(); if (reduced()) finishAll(); else sync(); };
  reduceQuery.addEventListener('change', onReduce);
  buildOnce();
  if (reduced()) finishAll();
  return {
    play, pause,
    destroy() { destroyed = true; pause(); observer.disconnect(); document.removeEventListener('visibilitychange', onVisibility); reduceQuery.removeEventListener('change', onReduce); root.remove(); },
  };
}

/* ---------- 장면 도구 ---------- */

/** 글자 단위 입력. chars는 span 배열. 한국어 55ms, 영어 35ms 기본 */
export function typeChars(tl, t, chars, speed) {
  chars.forEach((ch, i) => tl.at(t + i * speed, () => { ch.classList.add('on'); }));
  return t + chars.length * speed;
}
/** 선택 영역이 왼쪽에서 오른쪽으로 늘어난다 */
export function sweepSelect(tl, t, chars, from, to, dur = 350) {
  return tl.during(t, dur, p => {
    const count = Math.round(p * (to - from));
    for (let i = from; i < to; i++) chars[i].classList.toggle('sel', i < from + count);
  }, easeOut);
}
export function clearSelect(chars) { chars.forEach(ch => ch.classList.remove('sel')); }
export function markRange(chars, from, to, cls, on = true) { for (let i = from; i < to; i++) chars[i].classList.toggle(cls, on); }

/** 키캡 토스트. 자막 대신 키를 누르는 순간 보여 준다 */
export function createKeyToast(frame, { reduced = false } = {}) {
  const el = document.createElement('div');
  el.className = 'key-toast';
  el.setAttribute('aria-hidden', 'true');
  frame.appendChild(el);
  let hideTimer = 0;
  const render = keys => { el.innerHTML = keys.map(k => `<kbd>${k}</kbd>`).join(''); };
  return {
    show(keys) { render(keys); el.classList.add('show'); el.classList.remove('pulse'); void el.offsetWidth; el.classList.add('pulse'); },
    pulse(keys) { render(keys); el.classList.add('show'); el.classList.remove('pulse'); void el.offsetWidth; el.classList.add('pulse'); },
    hide() { if (reduced) return; el.classList.remove('show'); },
    /** 타임라인에 표시·숨김을 함께 예약한다. 반환값은 숨김 시각 */
    key(tl, t, keys, hold = 900) { tl.at(t, () => this.show(keys)); tl.at(t + hold, () => this.hide()); return t + hold; },
  };
}

/** 포인터(macOS 화살표). 프레임 좌표계(설계 픽셀)로 움직인다 */
export function createPointer(frame, x = 40, y = 40) {
  const el = document.createElement('div');
  el.className = 'scene-pointer';
  el.innerHTML = '<svg viewBox="0 0 20 24" width="20" height="24" aria-hidden="true"><path d="M3 2l14 11-6 .6 3.6 7.4-2.6 1.2-3.5-7.5L3 19z" fill="#fff" stroke="#1d1e30" stroke-width="1.4" stroke-linejoin="round"/></svg><i class="ring"></i>';
  frame.appendChild(el);
  let cx = x, cy = y;
  const place = () => { el.style.transform = `translate(${cx}px,${cy}px)`; };
  place();
  return {
    el,
    moveTo(tl, t, nx, ny, dur = 420) {
      let sx, sy;
      return tl.during(t, dur, p => { if (p === 0 || sx === undefined) { sx = cx; sy = cy; } cx = sx + (nx - sx) * p; cy = sy + (ny - sy) * p; place(); });
    },
    jump(tl, t, nx, ny) { return tl.at(t, () => { cx = nx; cy = ny; place(); }); },
    click(tl, t) { tl.at(t, () => { el.classList.remove('click'); void el.offsetWidth; el.classList.add('click'); }); return t + 160; },
    down(tl, t) { tl.at(t, () => el.classList.add('down')); return t; },
    up(tl, t) { tl.at(t, () => el.classList.remove('down')); return t; },
    hide(tl, t) { tl.at(t, () => el.classList.add('hidden')); },
    show(tl, t) { tl.at(t, () => el.classList.remove('hidden')); },
    /** 드래그: 포인터가 움직이는 동안 onProgress(p, x, y) 호출 */
    drag(tl, t, from, to, dur, onProgress) {
      this.down(tl, t);
      tl.during(t, dur, p => { cx = from[0] + (to[0] - from[0]) * p; cy = from[1] + (to[1] - from[1]) * p; place(); onProgress?.(p, cx, cy); }, easeInOut);
      this.up(tl, t + dur);
      return t + dur;
    },
  };
}

/** 프레임을 컨테이너 폭에 맞춰 축소한다. 글이 다시 흐르지 않도록 transform만 쓴다 */
export function fitFrame(wrap, frame, width, height) {
  wrap.style.setProperty('--frame-w', width + 'px');
  wrap.style.setProperty('--frame-h', height + 'px');
  wrap.style.aspectRatio = `${width} / ${height}`;
  const apply = () => { const s = wrap.clientWidth / width; frame.style.transform = `scale(${s})`; wrap.dataset.scale = s.toFixed(4); };
  apply();
  const ro = new ResizeObserver(apply);
  ro.observe(wrap);
  return () => ro.disconnect();
}

/** 요소의 위치를 프레임 설계 좌표로 돌려준다(scale 보정) */
export function rectIn(frame, el) {
  const f = frame.getBoundingClientRect(), r = el.getBoundingClientRect();
  const s = f.width / frame.offsetWidth || 1;
  return { x: (r.left - f.left) / s, y: (r.top - f.top) / s, w: r.width / s, h: r.height / s };
}
