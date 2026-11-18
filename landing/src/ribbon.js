/**
 * Stology's original S-field: parallel light fibres following the brand ribbon.
 * Canvas 2D, no network assets or WebGL context; one loop for the visible hero.
 */
export function initRibbon(canvas) {
  const ctx = canvas.getContext('2d', { alpha: true });
  if (!ctx) return () => {};

  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
  const pointer = { x: 0, y: 0, tx: 0, ty: 0 };
  let width = 600, height = 620, dpr = 1;
  let frame = 0, active = true, disposed = false, manualPaused = false, clock = 0, last = 0;
  const TAU = Math.PI * 2;
  const curves = [
    [[.55, -.67], [.08, -.69], [-.59, -.70], [-.57, -.27]],
    [[-.57, -.27], [-.56, .03], [.54, -.02], [.55, .30]],
    [[.55, .30], [.58, .68], [-.10, .70], [-.53, .68]],
  ];
  // A fixed seed keeps the first frame and reduced-motion rendering consistent.
  let seed = 2077;
  const random = () => { seed = (seed * 16807) % 2147483647; return (seed - 1) / 2147483646; };
  const dust = Array.from({ length: 68 }, () => ({
    x: random(), y: random(), r: .25 + random() * .8, a: .1 + random() * .36,
    phase: random() * TAU,
  }));

  function center(t) {
    const segment = Math.min(2, Math.floor(t * 3));
    const u = Math.min(1, t * 3 - segment), v = 1 - u;
    const p = curves[segment];
    return {
      x: v*v*v*p[0][0] + 3*v*v*u*p[1][0] + 3*v*u*u*p[2][0] + u*u*u*p[3][0],
      y: v*v*v*p[0][1] + 3*v*v*u*p[1][1] + 3*v*u*u*p[2][1] + u*u*u*p[3][1],
      dx: 3*v*v*(p[1][0]-p[0][0]) + 6*v*u*(p[2][0]-p[1][0]) + 3*u*u*(p[3][0]-p[2][0]),
      dy: 3*v*v*(p[1][1]-p[0][1]) + 6*v*u*(p[2][1]-p[1][1]) + 3*u*u*(p[3][1]-p[2][1]),
    };
  }

  function point(t, fibre, time) {
    const p = center(t), length = Math.hypot(p.dx, p.dy);
    const twist = Math.sin(t * TAU * 1.35 - .48);
    const edge = fibre * (.15 + .055 * Math.sin(t * Math.PI));
    const wave = Math.sin(t * 13 + time * .36 + fibre * 2.2) * .008;
    const spread = edge * (.64 + .36 * Math.abs(twist));
    const z = edge * Math.cos(t * TAU * 1.35 - .48);
    const scale = Math.min(width / 2.05, height / 2.05);
    return {
      x: width * .5 + (p.x - p.dy / length * (spread + wave) + z * .62 + pointer.x * z * .6) * scale,
      y: height * .49 + (p.y + p.dx / length * (spread + wave) + z * .12 + pointer.y * z * .4) * scale,
      z,
    };
  }

  function trace(fibre, time, start = 0, end = 1, samples = 130) {
    ctx.beginPath();
    for (let i = 0; i <= samples; i++) {
      const p = point(start + (end - start) * i / samples, fibre, time);
      if (i === 0) ctx.moveTo(p.x, p.y); else ctx.lineTo(p.x, p.y);
    }
  }

  function orbitalGrid(time) {
    const scale = Math.min(width, height);
    ctx.save();
    ctx.translate(width * .5, height * .51);
    ctx.rotate(-.36 + pointer.x * .012);
    ctx.lineWidth = .6;
    for (let ring = 0; ring < 3; ring++) {
      ctx.strokeStyle = `rgba(138,139,199,${.105 - ring * .021})`;
      ctx.setLineDash(ring === 1 ? [2, 6] : []);
      ctx.beginPath();
      ctx.ellipse(0, 0, scale * (.39 + ring * .045), scale * (.25 + ring * .027), 0, 0, TAU);
      ctx.stroke();
    }
    ctx.setLineDash([]);
    for (let i = 0; i < 48; i++) {
      const a = i / 48 * TAU;
      const x = Math.cos(a) * scale * .48, y = Math.sin(a) * scale * .307;
      ctx.strokeStyle = i % 4 === 0 ? 'rgba(150,153,201,.28)' : 'rgba(150,153,201,.12)';
      ctx.beginPath(); ctx.moveTo(x, y);
      ctx.lineTo(x + Math.cos(a) * (i % 4 === 0 ? 6 : 3), y + Math.sin(a) * (i % 4 === 0 ? 6 : 3));
      ctx.stroke();
    }
    const orbit = time * .07 + 2.8;
    const x = Math.cos(orbit) * scale * .48, y = Math.sin(orbit) * scale * .307;
    ctx.fillStyle = '#b4d7b9'; ctx.shadowColor = '#b4d7b9'; ctx.shadowBlur = 9;
    ctx.beginPath(); ctx.arc(x, y, 1.7, 0, TAU); ctx.fill();
    ctx.restore();
  }

  function draw(time) {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    const scale = Math.min(width, height);
    const halo = ctx.createRadialGradient(width*.5, height*.5, 0, width*.5, height*.5, scale*.46);
    halo.addColorStop(0, 'rgba(73,52,171,.14)');
    halo.addColorStop(.7, 'rgba(69,48,156,.045)');
    halo.addColorStop(1, 'rgba(50,40,120,0)');
    ctx.fillStyle = halo; ctx.fillRect(0, 0, width, height);
    orbitalGrid(time);

    for (const star of dust) {
      const opacity = star.a * (.8 + Math.sin(star.phase + time * .7) * .2);
      ctx.fillStyle = `rgba(179,181,222,${opacity})`;
      ctx.beginPath(); ctx.arc(star.x * width, star.y * height, star.r, 0, TAU); ctx.fill();
    }

    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    // Broad low-opacity light under the tightly spaced fibres gives volume.
    const glow = ctx.createLinearGradient(width*.2, height*.2, width*.7, height*.8);
    glow.addColorStop(0, 'rgba(116,89,245,.035)');
    glow.addColorStop(.5, 'rgba(82,61,228,.065)');
    glow.addColorStop(1, 'rgba(111,112,255,.04)');
    ctx.strokeStyle = glow; ctx.lineWidth = scale * .12; ctx.lineCap = 'round';
    ctx.shadowColor = 'rgba(90,69,240,.25)'; ctx.shadowBlur = scale * .045;
    trace(0, time); ctx.stroke();
    ctx.shadowBlur = 0;

    const count = width < 450 ? 66 : 98;
    for (let i = 0; i < count; i++) {
      const fibre = i / (count - 1) * 2 - 1;
      const edge = Math.abs(fibre);
      const alpha = .22 + edge * edge * .32 + (i % 7 === 0 ? .2 : 0);
      const gradient = ctx.createLinearGradient(width*.25, height*.15, width*.72, height*.83);
      gradient.addColorStop(0, `rgba(184,171,255,${alpha})`);
      gradient.addColorStop(.32, `rgba(116,91,240,${alpha * .86})`);
      gradient.addColorStop(.51, `rgba(105,103,250,${alpha * .58})`);
      gradient.addColorStop(.72, `rgba(143,117,255,${alpha})`);
      gradient.addColorStop(1, `rgba(186,181,255,${alpha * .8})`);
      ctx.strokeStyle = gradient;
      ctx.lineWidth = i % 9 === 0 ? .95 : .54;
      trace(fibre, time); ctx.stroke();
    }

    // Short travelling pulses expose the curve without distracting from text.
    for (let i = 0; i < 9; i++) {
      const t = (time * (.025 + i * .001) + i * .113) % 1;
      const fibre = Math.sin(i * 2.4) * .91;
      const head = point(t, fibre, time);
      const tail = point(Math.max(0, t - .047), fibre, time);
      const pulse = ctx.createLinearGradient(tail.x, tail.y, head.x, head.y);
      pulse.addColorStop(0, 'rgba(159,139,255,0)');
      pulse.addColorStop(1, i === 3 ? 'rgba(190,230,194,.9)' : 'rgba(213,198,255,.9)');
      ctx.strokeStyle = pulse; ctx.lineWidth = 1.12;
      trace(fibre, time, Math.max(0, t - .047), t, 12); ctx.stroke();
      ctx.fillStyle = i === 3 ? '#d4eed2' : '#ded1ff';
      ctx.shadowColor = '#a58eff'; ctx.shadowBlur = 8;
      ctx.beginPath(); ctx.arc(head.x, head.y, 1.25, 0, TAU); ctx.fill();
      ctx.shadowBlur = 0;
    }
    ctx.restore();

    // Small datum marks borrow the language of technical drawings.
    ctx.strokeStyle = 'rgba(164,166,206,.24)'; ctx.lineWidth = .6;
    for (const [x, y] of [[.14,.19],[.86,.81]]) {
      ctx.beginPath(); ctx.moveTo(width*x-5,height*y);ctx.lineTo(width*x+5,height*y);
      ctx.moveTo(width*x,height*y-5);ctx.lineTo(width*x,height*y+5);ctx.stroke();
    }
  }

  function shouldRun() { return active && !document.hidden && !reduced.matches && !manualPaused && !disposed; }
  function tick(timestamp) {
    frame = 0;
    if (!shouldRun()) return;
    clock += last ? Math.min((timestamp - last) / 1000, .05) : 0;
    last = timestamp;
    pointer.x += (pointer.tx - pointer.x) * .035;
    pointer.y += (pointer.ty - pointer.y) * .035;
    draw(clock);
    frame = requestAnimationFrame(tick);
  }
  function sync() {
    if (frame) cancelAnimationFrame(frame);
    frame = 0; last = 0;
    if (shouldRun()) frame = requestAnimationFrame(tick);
    else if (active && !document.hidden && !disposed) draw(clock);
  }
  function resize() {
    const rect = canvas.getBoundingClientRect();
    width = Math.max(1, rect.width); height = Math.max(1, rect.height);
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(width * dpr); canvas.height = Math.round(height * dpr);
    draw(clock);
  }
  function move(event) {
    const rect = canvas.getBoundingClientRect();
    pointer.tx = (event.clientX - rect.left) / width * 2 - 1;
    pointer.ty = (event.clientY - rect.top) / height * 2 - 1;
  }
  function leave() { pointer.tx = 0; pointer.ty = 0; }
  const resizeObserver = new ResizeObserver(resize);
  resizeObserver.observe(canvas);
  const intersection = new IntersectionObserver(([entry]) => { active = entry.isIntersecting; sync(); }, { threshold: .03 });
  intersection.observe(canvas);
  canvas.addEventListener('pointermove', move, { passive: true });
  canvas.addEventListener('pointerleave', leave);
  reduced.addEventListener('change', sync);
  document.addEventListener('visibilitychange', sync);
  resize(); sync();
  const cleanup = () => {
    disposed = true;
    if (frame) cancelAnimationFrame(frame);
    resizeObserver.disconnect(); intersection.disconnect();
    canvas.removeEventListener('pointermove', move); canvas.removeEventListener('pointerleave', leave);
    reduced.removeEventListener('change', sync); document.removeEventListener('visibilitychange', sync);
  };
  // Keep the teardown function convenient while exposing the explicit UI toggle.
  cleanup.setPaused = (paused) => { manualPaused = Boolean(paused); sync(); };
  return cleanup;
}
