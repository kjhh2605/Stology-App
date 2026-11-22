/**
 * Stology hero: a Milky Way that crosses the whole hero and gathers into the logo's S, with one lavender star playing on it.
 * Canvas 2D, no libraries or WebGL context; one loop for the visible hero.
 * World units are fractions of the S height, centred on the S. The canvas covers the hero; `anchor` is the box the S sits in.
 */
const TAU = Math.PI * 2;
// Width / height of the app icon's S, which the centreline below was traced from.
const ASPECT = 848 / 953;
// Must match scripts/build-star-sheet.py.
const SHEET = { url: '/media/hero/star-sheet.webp', cell: 256, columns: 8, yawSteps: 48, blinkHalfRange: 7, fill: .93 };
// The S of the app icon as one smooth stroke in the icon's image space (0..1), from the top-right cap to the bottom-left cap.
const CENTRELINE = [
  [.93, .17], [.8, .168], [.6, .168], [.42, .168], [.25, .172], [.135, .21], [.085, .305], [.125, .395], [.245, .445], [.5, .4985],
  [.755, .552], [.875, .602], [.915, .692], [.865, .787], [.75, .825], [.58, .829], [.4, .829], [.2, .829], [.07, .829],
];
// Faint lanes that carry the galaxy across the rest of the hero: in from the upper right, out to the left behind the copy.
const INFLOW = [[2.05, -.62], [1.6, -.3], [1.22, .02]];
const OUTFLOW = [[-.3, .87], [-.85, 1], [-1.5, 1.02], [-2.2, .9], [-2.9, .82]];
// Half-width of the star band, and how thick it is where the band twists edge-on like the icon's folded ribbon.
const BAND = { half: .092, thickness: .42 };
// Star colours: white, lavender, indigo, pink, sky, warm. ODDS is the cumulative chance of each.
const TINTS = [[236, 231, 255], [181, 164, 255], [124, 104, 255], [255, 160, 216], [122, 204, 255], [255, 226, 180]];
const TINT_ODDS = [.3, .58, .74, .84, .95, 1];
// Haze colour along the S, from the rosy head through indigo to the blue tail.
const GRADE = [[0, [236, 156, 255]], [.28, [170, 122, 255]], [.52, [110, 86, 255]], [.76, [84, 126, 255]], [1, [96, 200, 255]]];
const FALLOFF = [[0, 1], [.2, .78], [.4, .45], [.6, .2], [.8, .06], [1, 0]];
// The page background (--bg in style.css): the sky is painted on top of it.
const BACKGROUND = [8, 9, 13];
// How the star rides the stream: [chance, speed, carve width, carve rate, shortest and longest stretch].
const RIDES = [
  { odds: .55, speed: .25, sway: .034, rate: 8.5, span: [.6, 1.7] },
  { odds: .78, speed: .1, sway: .05, rate: 13, span: [.28, .5] },
  { odds: 1, speed: .44, sway: .018, rate: 6, span: [.8, 1.6] },
];
const HERO_SIZE = .19, SPIN_TIME = .9, HOP_TIME = .9, TRAIL_LIFE = .85, REVEAL_END = 1.3;

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
const mix = (a, b, t) => a + (b - a) * t;
const ease = (rate, dt) => 1 - Math.exp(-rate * dt);
const smooth = (t) => t * t * (3 - 2 * t);
const ramp = (value, from, span) => smooth(clamp((value - from) / span, 0, 1));
// Fixed seeds keep the galaxy, the first frame and the reduced-motion still identical on every load.
const seeded = (seed) => () => { seed = (seed * 16807) % 2147483647; return (seed - 1) / 2147483646; };
const pickTint = (chance) => TINT_ODDS.findIndex((odd) => chance < odd);
const rgba = ([r, g, b], alpha) => `rgba(${r | 0},${g | 0},${b | 0},${alpha})`;
const hermite = (a, b, ma, mb, t) => (2 * t ** 3 - 3 * t * t + 1) * a + (t ** 3 - 2 * t * t + t) * ma + (3 * t * t - 2 * t ** 3) * b + (t ** 3 - t * t) * mb;

function grade(f) {
  for (let i = 1; i < GRADE.length; i++) {
    const [from, a] = GRADE[i - 1], [to, b] = GRADE[i];
    if (f <= to) return a.map((value, n) => mix(value, b[n], (f - from) / (to - from)));
  }
  return GRADE[GRADE.length - 1][1];
}

// The stream at even arc length: the lane in, the S between `head` and `tail`, and the lane out.
function buildStream() {
  const points = [...INFLOW, ...CENTRELINE, ...OUTFLOW].map(([u, v]) => [(u - .5) * ASPECT, v - .5]);
  const ext = [points[0], ...points, points[points.length - 1]], table = [];
  let length = 0, head = 0, tail = 0;
  const add = (x, y) => {
    if (table.length) length += Math.hypot(x - table[table.length - 1].x, y - table[table.length - 1].y);
    table.push({ x, y, s: length });
  };
  for (let i = 0; i < points.length - 1; i++) {
    const [a, b, c, d] = [ext[i], ext[i + 1], ext[i + 2], ext[i + 3]];
    for (let k = 0; k < 20; k++) {
      const t = k / 20;
      const at = (n) => .5 * (2 * b[n] + (c[n] - a[n]) * t + (2 * a[n] - 5 * b[n] + 4 * c[n] - d[n]) * t * t + (3 * b[n] - a[n] - 3 * c[n] + d[n]) * t * t * t);
      add(at(0), at(1));
      if (k === 0 && i === INFLOW.length) head = length;
      if (k === 0 && i === INFLOW.length + CENTRELINE.length - 1) tail = length;
    }
  }
  add(...points[points.length - 1]);

  const N = Math.round(length / .0055), step = length / (N - 1);
  const stream = { N, step, length, head, tail };
  for (const key of ['x', 'y', 'tx', 'ty', 'offA', 'offB', 'girth', 'env', 'dens']) stream[key] = new Float32Array(N);
  for (let i = 0, j = 0; i < N; i++) {
    const target = Math.min(i * step, length);
    while (j < table.length - 2 && table[j + 1].s <= target) j++;
    const a = table[j], b = table[j + 1], t = (target - a.s) / (b.s - a.s || 1);
    stream.x[i] = mix(a.x, b.x, t); stream.y[i] = mix(a.y, b.y, t);
  }
  for (let i = 0; i < N; i++) {
    const a = Math.max(0, i - 1), b = Math.min(N - 1, i + 1), dx = stream.x[b] - stream.x[a], dy = stream.y[b] - stream.y[a], span = Math.hypot(dx, dy) || 1;
    stream.tx[i] = dx / span; stream.ty[i] = dy / span;
  }
  // Across the band a star sits at n * offA + m * offB: the band turns half a twist over the diagonal of the S.
  // Outside the S the band widens, thins out and fades, so the lanes read as background.
  for (let i = 0; i < N; i++) {
    const s = i * step, inside = s >= head && s <= tail, f = clamp((s - head) / (tail - head), 0, 1);
    const beyond = inside ? 0 : s < head ? head - s : s - tail, reach = s < head ? head : length - tail;
    const angle = Math.PI * ramp(f, .37, .26), wide = BAND.half * (1 + 1.7 * ramp(beyond, 0, .9));
    stream.offA[i] = wide * Math.cos(angle); stream.offB[i] = wide * BAND.thickness * Math.sin(angle);
    stream.girth[i] = Math.hypot(stream.offA[i], stream.offB[i]);
    stream.env[i] = inside ? 1 - .44 * (1 - ramp(Math.min(s - head, tail - s), 0, .14)) : (.2 + .36 * Math.exp(-beyond / .18)) * ramp(reach - beyond, 0, reach * .55);
    stream.dens[i] = inside ? 1 : .3 + .7 * Math.exp(-beyond / .25);
  }
  return stream;
}

function makeSprite(size, paint) {
  const sprite = document.createElement('canvas');
  sprite.width = sprite.height = size;
  paint(sprite.getContext('2d'), size / 2);
  return sprite;
}
function radial(g, h, stops) {
  const gradient = g.createRadialGradient(h, h, 0, h, h, h);
  for (const [offset, colour] of stops) gradient.addColorStop(offset, colour);
  g.fillStyle = gradient; g.fillRect(0, 0, h * 2, h * 2);
}
const puffSprite = (tint) => makeSprite(128, (g, h) => radial(g, h, FALLOFF.map(([offset, k]) => [offset, rgba(tint, k)])));

function buildSprites() {
  return {
    dots: TINTS.map((tint) => makeSprite(32, (g, h) => radial(g, h, [[0, 'rgba(255,255,255,1)'], [.22, rgba(tint, .9)], [.5, rgba(tint, .22)], [1, rgba(tint, 0)]]))),
    puffs: TINTS.map(puffSprite),
    shade: puffSprite([20, 12, 78]),
    // A four-point glint for the brightest stars and the sparkles.
    flare: makeSprite(96, (g, h) => {
      radial(g, h, [[0, 'rgba(255,255,255,.95)'], [.12, 'rgba(226,218,255,.5)'], [.45, 'rgba(170,150,255,.1)'], [1, 'rgba(170,150,255,0)']]);
      const spike = g.createRadialGradient(h, h, 0, h, h, h);
      spike.addColorStop(0, 'rgba(255,255,255,1)'); spike.addColorStop(.5, 'rgba(232,226,255,.55)'); spike.addColorStop(1, 'rgba(232,226,255,0)');
      g.fillStyle = spike; g.beginPath(); g.moveTo(h, 0);
      g.quadraticCurveTo(h, h, h * 2, h); g.quadraticCurveTo(h, h, h, h * 2); g.quadraticCurveTo(h, h, 0, h); g.quadraticCurveTo(h, h, h, 0); g.fill();
    }),
  };
}

export function initHeroScene(canvas, { anchor = canvas, surface = canvas } = {}) {
  const ctx = canvas.getContext('2d', { alpha: true });
  if (!ctx) return () => {};

  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
  // sx/sy: on the canvas. wx/wy: in the world. near: how much of the stream is under it. shock: a press spreading out.
  const pointer = { x: 0, y: 0, tx: 0, ty: 0, sx: 0, sy: 0, wx: 0, wy: 0, vx: 0, vy: 0, inside: false, fresh: true, near: 0, shock: 0, shed: 0 };
  let width = 1200, height = 760, dpr = 1, unit = 430, compact = null;
  let frame = 0, active = true, disposed = false, manualPaused = false, clock = 0, last = 0;
  let sheet = null, sheetShown = 0, reveal = 0;
  const centre = { x: 900, y: 320 }, half = { w: .7, h: .7 }, roam = { l: -2, r: .8, t: -.6, b: .7 };
  const stream = buildStream(), sprites = buildSprites(), ride = { from: stream.head + .02, to: stream.tail - .06 };
  // Pre-rendered once per size: the wide dim light of the sky, the haze and dust on top of it,
  // and two layers of faint stars on the S that shimmer against each other.
  const haze = document.createElement('canvas'), sky = document.createElement('canvas'), specks = [document.createElement('canvas'), document.createElement('canvas')];
  const CELL = 6, MARGIN = 2;
  const edges = new Float32Array(1024), tinted = new Map();
  let flow = [], puffs = [], streaks = [];

  const random = seeded(2077);
  // What the star does next is different on every visit; everything else is seeded.
  const chance = Math.random, between = (min, max) => mix(min, max, chance());
  const twinkles = Array.from({ length: 54 }, () => ({ x: random(), y: random(), r: 1.4 + random() * 3.4, a: .25 + random() * .5, phase: random() * TAU, rate: .5 + random() * 1.4, flare: random() < .3, tint: pickTint(random()) }));
  const meteor = { wait: 3.5, life: 0, span: 1, x: 0, y: 0, vx: 0, vy: 0 };
  const hero = {
    mode: 'ride', style: RIDES[0], s: mix(stream.head, stream.tail, .47), until: mix(stream.head, stream.tail, .9), flight: null, u: 0, hover: 0, hoverSpan: 0,
    speed: RIDES[0].speed, x: 0, y: 0, nx: 0, ny: 0, angle: 0, dx: 0, dy: 0, vx: 0, vy: 0, n: 0, on: 1, ground: 1, amp: RIDES[0].sway, front: 1, depth: 1, scale: 1,
    carve: 0, yaw: 0, roll: 0, spin: 0, spinDir: 1, hop: 0, nextHop: 3.2, squash: 0, squashV: 0, blink: 0, nextBlink: 2.6, shed: 0, placed: false,
  };
  const trail = [], sparkles = [], rings = [];

  function populate() {
    const rnd = seeded(4391), share = stream.length / (stream.tail - stream.head), stars = Math.round((compact ? 520 : 980) * share);
    flow = Array.from({ length: stars }, (_, i) => {
      const n = rnd() + rnd() + rnd() - 1.5, glint = i < stars * .028;
      return {
        s: rnd() * stream.length, n, m: rnd() * 2 - 1, speed: (.05 + rnd() * .07) * (1 - .4 * Math.min(1, Math.abs(n))), rank: rnd(),
        size: glint ? 7 + rnd() * 9 : .9 + rnd() ** 3 * 3.1, alpha: glint ? .5 + rnd() * .4 : .28 + rnd() * .6, tint: pickTint(rnd()),
        phase: rnd() * TAU, rate: .6 + rnd() * 2.2, glint, side: 1, wake: 0, lit: 0, px: 0, py: 0, x: 0, y: 0, a: 0,
      };
    });
    puffs = Array.from({ length: Math.round((compact ? 28 : 46) * share) }, () => ({
      s: rnd() * stream.length, n: (rnd() * 2 - 1) * .7, m: rnd() * 2 - 1, speed: .025 + rnd() * .03, reach: 2.2 + rnd() * 2.6,
      alpha: .03 + rnd() * .045, tint: 1 + (rnd() * 4 | 0), phase: rnd() * TAU, x: 0, y: 0, r: 0, a: 0,
    }));
    streaks = Array.from({ length: compact ? 6 : 10 }, () => ({
      s: mix(stream.head, stream.tail, rnd()), n: (rnd() * 2 - 1) * .8, m: rnd() * 2 - 1, span: .28 + rnd() * .3, speed: .3 + rnd() * .22, alpha: .14 + rnd() * .16, weight: .8 + rnd() * .8,
    }));
  }

  function streamAt(s) {
    const at = clamp(s, 0, stream.length - stream.step) / stream.step, i = at | 0, t = at - i;
    return { x: mix(stream.x[i], stream.x[i + 1], t), y: mix(stream.y[i], stream.y[i + 1], t), tx: mix(stream.tx[i], stream.tx[i + 1], t), ty: mix(stream.ty[i], stream.ty[i + 1], t) };
  }
  const sFraction = (s) => (s - stream.head) / (stream.tail - stream.head);

  function emit(x, y, amount, force, size = 1, vx = 0, vy = 0) {
    for (let i = 0; i < amount && sparkles.length < 140; i++) {
      const a = random() * TAU, speed = force * (.25 + random() * .75), span = .55 + random() * .9, r = HERO_SIZE * size * .32 * Math.sqrt(random());
      sparkles.push({
        x: x + Math.cos(a) * r, y: y + Math.sin(a) * r, vx: Math.cos(a) * speed + vx, vy: Math.sin(a) * speed + vy,
        life: span, span, size: (.012 + random() * .024) * mix(.6, 1, size), tint: pickTint(random()), flare: random() < .3, phase: random() * TAU,
      });
    }
  }
  const shed = (amount, force) => emit(hero.dx, hero.dy, amount, force, hero.scale, hero.vx * .12, hero.vy * .12);
  function twirl(direction) {
    if (hero.spin > 0) return;
    hero.spin = 1;
    hero.spinDir = direction || (hero.vx < 0 ? -1 : 1);
  }
  // A hop with a full turn, eyes closed.
  function leap(direction) {
    if (hero.hop > 0) return;
    hero.hop = 1; hero.blink = HOP_TIME * .6;
    twirl(direction);
  }
  function touchDown() {
    hero.squashV = 2.6;
    shed(10, .12);
    if (hero.mode === 'ride') rings.push({ x: hero.x, y: hero.y, angle: hero.angle, flat: .45, reach: .2, life: .8, span: .8 });
  }

  // The deepest the star may come forward at a spot: over the copy it stays far away and small.
  const depthCap = (x, y) => mix(1, .5, ramp(Math.max(Math.abs(x) - half.w, Math.abs(y) - half.h), -.04, .3));
  function spot(far) {
    const box = far ? roam : { l: Math.max(roam.l, -.74), r: Math.min(roam.r, .74), t: Math.max(roam.t, -.68), b: Math.min(roam.b, .68) };
    return { x: between(box.l, box.r), y: between(box.t, box.b) };
  }
  // A smooth path through the points, leaving along `from` and arriving along `to`. Null if it turns too sharply or leaves the hero.
  function plan(points, from, to, strict = true) {
    // Each point has one direction; the pull along it is scaled to the stretch on either side, so uneven spacing does not kink the path.
    const last = points.length - 1, heading = points.map((p, i) => {
      if (i === 0) return from;
      if (i === last && to) return to;
      return aim(points[i - 1], points[Math.min(last, i + 1)]);
    });
    const flight = { x: [], y: [], depth: [], s: [], length: 0, i: 0, land: null };
    for (let i = 0; i < last; i++) {
      const a = points[i], b = points[i + 1], reach = Math.hypot(b.x - a.x, b.y - a.y);
      for (let k = i ? 1 : 0; k <= 16; k++) {
        const t = k / 16, n = flight.x.length;
        const x = hermite(a.x, b.x, heading[i].x * reach, heading[i + 1].x * reach, t), y = hermite(a.y, b.y, heading[i].y * reach, heading[i + 1].y * reach, t);
        if (n) flight.length += Math.hypot(x - flight.x[n - 1], y - flight.y[n - 1]);
        flight.x.push(x); flight.y.push(y); flight.depth.push(mix(a.depth, b.depth, smooth(t))); flight.s.push(flight.length);
      }
    }
    if (!strict) return flight;
    const count = flight.x.length;
    for (let i = 0; i < count; i++) {
      if (flight.x[i] < roam.l - .04 || flight.x[i] > roam.r + .04 || flight.y[i] < roam.t - .04 || flight.y[i] > roam.b + .04) return null;
      const a = Math.max(0, i - 3), b = Math.min(count - 1, i + 3);
      if (b - a < 4 || a === 0 || b === count - 1) continue;
      const ax = flight.x[a + 1] - flight.x[a - 1], ay = flight.y[a + 1] - flight.y[a - 1], bx = flight.x[b + 1] - flight.x[b - 1], by = flight.y[b + 1] - flight.y[b - 1];
      const turn = Math.abs(Math.atan2(ax * by - ay * bx, ax * bx + ay * by));
      if (turn > 1e-4 && (flight.s[b] - flight.s[a]) / turn < .035 + .05 * flight.depth[i]) return null;
    }
    return flight;
  }
  // Where a flight ends: a spot on the S, and the direction of the stream there.
  function landing(not) {
    for (let tries = 0; tries < 8; tries++) {
      const s = mix(ride.from, ride.to, between(.02, .82));
      if (not === undefined || Math.abs(s - not) > .35) {
        const p = streamAt(s);
        return { s, to: { x: p.tx, y: p.ty }, end: { x: p.x, y: p.y, depth: 1 } };
      }
    }
    return landing();
  }
  const waypoint = (p, deep) => ({ ...p, depth: Math.min(deep, depthCap(p.x, p.y)) });

  // Picks what the star does next. Each kind of flight is tried with fresh random points until one flies cleanly.
  function takeOff() {
    const grounded = hero.mode === 'ride', start = { x: hero.x, y: hero.y, depth: hero.depth };
    // Now and then it goes to see what the pointer is doing, unless it is already there.
    const beside = { x: clamp(pointer.wx - .13, roam.l, roam.r), y: clamp(pointer.wy - .13, roam.t, roam.b) };
    const curious = pointer.inside && Math.hypot(beside.x - start.x, beside.y - start.y) > .2 && chance() < .3;
    const kinds = curious ? ['visit'] : grounded ? ['arc', 'arc', 'arc', 'tour', 'tour', 'loop', 'loop', 'visit'] : ['arc', 'arc', 'tour'];
    // One kind is drawn first and given several goes, so a kind that rarely fits is not crowded out by the easy ones.
    const first = kinds[chance() * kinds.length | 0], order = [first, ...['arc', 'tour', 'visit'].filter((kind) => kind !== first)];
    for (let tries = 0; tries < order.length * 10; tries++) {
      const kind = order[tries / 10 | 0], points = [start];
      let heading = { x: Math.cos(hero.angle), y: Math.sin(hero.angle) }, to = null, land = null;
      // Off the stream towards `goal`: a swerve if it lies ahead, otherwise half a turn round to face it.
      const lift = (goal, deep) => {
        const dx = goal.x - start.x, dy = goal.y - start.y, side = heading.x * dy - heading.y * dx < 0 ? -1 : 1, r = between(.13, .2), reach = between(.22, .45), off = between(.06, .22) * side;
        const offsets = heading.x * dx + heading.y * dy > Math.hypot(dx, dy) * .25 ? [[reach, off]] : [[r, side * r], [-r * .2, side * r * 2]];
        return offsets.map(([along, across]) => waypoint({ x: start.x + heading.x * along - heading.y * across, y: start.y + heading.y * along + heading.x * across }, deep));
      };
      if (kind === 'loop') {
        // A loop-the-loop over the stream, coming down a little further on.
        const r = between(.12, .18), side = chance() < .5 ? -1 : 1, cx = start.x + heading.x * .1 - heading.y * side * r, cy = start.y + heading.y * .1 + heading.x * side * r;
        for (const turn of [80, 170, 260, 335]) {
          const a = turn * Math.PI / 180, along = Math.sin(a) * r, across = -Math.cos(a) * r * side;
          points.push({ x: cx + heading.x * along - heading.y * across, y: cy + heading.y * along + heading.x * across, depth: turn === 170 ? .88 : .95 });
        }
        const s = hero.s + between(.3, .5);
        if (s > ride.to) continue;
        const p = streamAt(s);
        land = s; to = { x: p.tx, y: p.ty }; points.push({ x: p.x, y: p.y, depth: 1 });
      } else if (kind === 'visit') {
        // Off to the pointer or to a quiet spot, to wait there a while.
        const goal = curious ? beside : spot(false);
        if (grounded) points.push(...lift(goal, between(.8, 1)));
        else { const mid = bend(start, goal); heading = aim(start, mid); points.push(waypoint(mid, between(.8, 1))); }
        points.push(waypoint(goal, between(.72, 1)));
      } else {
        const target = landing(grounded ? hero.s : undefined), deep = kind === 'tour' ? between(.42, .62) : between(.7, .95);
        if (kind === 'tour') {
          // A long way round, sometimes out over the copy.
          const stops = [spot(chance() < .55), ...(chance() < .45 ? [spot(chance() < .4)] : [])];
          if (grounded) points.push(...lift(stops[0], .8));
          else heading = aim(start, stops[0]);
          for (const stop of stops) points.push(waypoint(stop, deep));
        } else {
          // One arc over to another part of the stream.
          if (grounded) points.push(...lift(target.end, deep));
          else { const mid = bend(start, target.end); heading = aim(start, mid); points.push(waypoint(mid, deep)); }
        }
        points.push(target.end);
        land = target.s; to = target.to;
      }
      const flight = plan(points, heading, to);
      if (flight) { flight.land = land; return launch(flight); }
    }
    // Nothing fitted: keep riding if there is stream left, otherwise curve straight back to the S.
    if (grounded && hero.s < ride.to - .3) { hero.until = Math.min(ride.to, hero.s + between(.3, .6)); return; }
    const target = landing(), heading = grounded ? { x: Math.cos(hero.angle), y: Math.sin(hero.angle) } : aim(start, target.end);
    const flight = plan([start, target.end], heading, target.to, false);
    flight.land = target.s; launch(flight);
  }
  function aim(a, b) { const dx = b.x - a.x, dy = b.y - a.y, span = Math.hypot(dx, dy) || 1; return { x: dx / span, y: dy / span }; }
  // A point off to one side of the straight line, so a flight from rest still curves.
  function bend(a, b) { const side = between(-.28, .28); return { x: mix(a.x, b.x, .5) - (b.y - a.y) * side, y: mix(a.y, b.y, .5) + (b.x - a.x) * side }; }
  function launch(flight) {
    if (hero.mode === 'ride') { hero.squashV = -2.2; shed(10, .13); }
    hero.mode = 'fly'; hero.flight = flight; hero.u = 0;
  }
  function settle(s) {
    hero.mode = 'ride'; hero.s = s; hero.flight = null;
    const pick = chance();
    hero.style = RIDES.find((style) => pick < style.odds);
    hero.until = Math.min(ride.to, s + between(...hero.style.span));
    hero.nextHop = between(1.2, 5);
  }

  function stepHero(time, dt) {
    let base, landed = false;
    if (hero.mode === 'ride') {
      const here = streamAt(hero.s);
      // On the stream the star gathers speed downhill.
      hero.speed += (hero.style.speed * (1 + .5 * here.ty) - hero.speed) * ease(2, dt);
      hero.s += hero.speed * dt;
      hero.nextHop -= dt;
      if (hero.nextHop <= 0 && hero.style !== RIDES[1]) { leap(); hero.nextHop = chance() < .25 ? HOP_TIME + .15 : between(2.5, 7); }
      base = { ...streamAt(hero.s), depth: 1 };
      if (hero.s >= hero.until && hero.hop === 0) { hero.x = base.x; hero.y = base.y; takeOff(); }
    } else if (hero.mode === 'fly') {
      const flight = hero.flight, left = flight.length - hero.u;
      // Far from the viewer the star covers ground quickly. It slows into a spot where it means to stay.
      const cruise = mix(.5, .33, ramp(hero.depth, .5, .5));
      hero.speed += ((flight.land === null ? Math.min(cruise, left * 2.2 + .012) : cruise) - hero.speed) * ease(2.2, dt);
      hero.u = Math.min(flight.length, hero.u + hero.speed * dt);
      while (flight.i < flight.s.length - 2 && flight.s[flight.i + 1] < hero.u) flight.i++;
      const i = flight.i, t = clamp((hero.u - flight.s[i]) / (flight.s[i + 1] - flight.s[i] || 1), 0, 1), dx = flight.x[i + 1] - flight.x[i], dy = flight.y[i + 1] - flight.y[i], span = Math.hypot(dx, dy) || 1;
      base = { x: mix(flight.x[i], flight.x[i + 1], t), y: mix(flight.y[i], flight.y[i + 1], t), tx: dx / span, ty: dy / span, depth: mix(flight.depth[i], flight.depth[i + 1], t) };
      if (left < .004) {
        if (flight.land === null) { hero.mode = 'hover'; hero.hover = 0; hero.hoverSpan = between(1.6, 3.8); hero.speed = 0; } else { settle(flight.land); landed = true; }
      }
    } else {
      // Waiting in place: a slow drift that starts and ends at rest.
      hero.hover += dt;
      const sway = ramp(hero.hover, 0, .7) * ramp(hero.hoverSpan - hero.hover, 0, .7), flight = hero.flight, end = flight.x.length - 1;
      base = { x: flight.x[end] + Math.sin(hero.hover * .9) * .022 * sway, y: flight.y[end] + Math.sin(hero.hover * 1.3) * .03 * sway, tx: Math.cos(hero.angle), ty: Math.sin(hero.angle), depth: flight.depth[end] };
      if (hero.hover >= hero.hoverSpan) { hero.x = flight.x[end]; hero.y = flight.y[end]; takeOff(); }
    }
    if (hero.hop > 0) { hero.hop = Math.max(0, hero.hop - dt / HOP_TIME); landed ||= hero.hop === 0; }

    const riding = hero.mode === 'ride', air = Math.sin(Math.PI * hero.hop);
    hero.ground += ((riding ? 1 : 0) - hero.ground) * ease(6, dt);
    hero.on = hero.ground * (1 - air);
    // Carve from side to side across the stream; the swing dies away once the star is off it.
    hero.amp += ((riding ? hero.style.sway : 0) - hero.amp) * ease(2.5, dt);
    hero.carve += hero.speed * dt * (riding ? hero.style.rate : 8.5);
    const swing = Math.sin(hero.carve) * hero.amp, previous = { x: hero.dx, y: hero.dy };
    hero.n = swing / BAND.half;
    hero.x = base.x - base.ty * swing; hero.y = base.y + base.tx * swing; hero.angle = Math.atan2(base.ty, base.tx);
    hero.depth += (base.depth - hero.depth) * ease(riding ? 5 : 30, dt); hero.scale = hero.depth * (1 + air * .15);
    hero.dx = hero.x; hero.dy = hero.y - air * .085 - Math.sin(time * 1.7) * .006;
    if (!hero.placed) { hero.placed = true; previous.x = hero.dx; previous.y = hero.dy; }
    if (dt > 0) { const k = ease(10, dt); hero.vx += ((hero.dx - previous.x) / dt - hero.vx) * k; hero.vy += ((hero.dy - previous.y) / dt - hero.vy) * k; }
    if (landed) touchDown();
    hero.front += ((riding || hero.depth > .8 ? 1 : 0) - hero.front) * ease(5, dt);

    const pace = Math.hypot(hero.vx, hero.vy), waiting = hero.mode === 'hover';
    let yawTarget = waiting ? Math.sin(hero.hover * 1.5) * 32 : clamp(hero.vx / .2, -1, 1) * 40;
    if (pointer.inside) {
      const gap = Math.hypot(pointer.wx - hero.dx, pointer.wy - hero.dy);
      if (gap < .5) yawTarget = mix(yawTarget, clamp((pointer.wx - hero.dx) / .25, -1, 1) * 38, 1 - gap / .5);
    }
    hero.yaw += (yawTarget - hero.yaw) * ease(3.5, dt);
    // Nose down on the way down, and a lean into each turn.
    const lean = pace > .02 ? .5 * (hero.vy / pace) * Math.tanh(3 * hero.vx / pace) * Math.min(1, pace / .15) : 0;
    hero.roll += (lean + Math.sin(time * 1.3) * .035 - hero.roll) * ease(5, dt);
    hero.spin = Math.max(0, hero.spin - dt / SPIN_TIME);
    hero.squashV += (-180 * hero.squash - 14 * hero.squashV) * dt; hero.squash += hero.squashV * dt;
    hero.blink -= dt; hero.nextBlink -= dt;
    if (hero.nextBlink <= 0) { hero.blink = .14; hero.nextBlink = random() < .12 ? .32 : (waiting ? 1.2 : 2.2) + random() * 4.5; }

    for (const point of trail) point.age += dt;
    while (trail.length && trail[trail.length - 1].age > TRAIL_LIFE) trail.pop();
    if (trail.length < 250) trail.unshift({ x: hero.dx, y: hero.dy, age: 0, scale: hero.scale * Math.min(1, pace / .12) });
    hero.shed += dt * mix(24, 12, hero.ground) * clamp(pace / .25, .08, 1.7);
    while (hero.shed >= 1) { hero.shed--; shed(1, .045); }
  }

  function stepFlow(time, dt) {
    const { step, length, x, y, tx, ty, offA, offB, girth, env, dens } = stream, lift = ease(12, dt), sink = ease(1.3, dt), pull = ease(9, dt), release = ease(2.2, dt);
    // Every few seconds a surge of light runs down the S.
    const crest = stream.head + (time * .42) % (stream.tail - stream.head + 1.4) - .5, surge = (s) => Math.exp(-(((s - crest) / .2) ** 2));
    // Under the pointer the stars part and swirl like water round a stone, and follow it a little as it moves.
    const hover = pointer.inside, reach = .05 * (1 + pointer.shock * 1.6), push = .042 * (1 + pointer.shock * 3), swirl = .03;
    const dragX = clamp(pointer.vx * .05, -.07, .07), dragY = clamp(pointer.vy * .05, -.07, .07);
    let near = 0;
    for (const p of flow) {
      p.s += p.speed * dt; if (p.s >= length) p.s -= length;
      const at = p.s / step, i = at | 0, t = at - i, behind = hero.s - p.s;
      // The star parts the stars it has just passed.
      const wake = behind > -.05 && behind < .24 ? (1 - Math.max(0, behind) / .24) * hero.on * Math.max(0, 1 - Math.abs(p.n - hero.n) / 1.5) : 0;
      if (wake > 0 && p.wake < .02) p.side = p.n < hero.n ? -1 : 1;
      p.wake += (wake - p.wake) * (wake > p.wake ? lift : sink);
      const off = (p.n + p.side * p.wake * .5) * offA[i] + p.m * offB[i];
      const bx = mix(x[i], x[i + 1], t) - ty[i] * off, by = mix(y[i], y[i + 1], t) + tx[i] * off;
      let fx = 0, fy = 0, lit = 0;
      if (hover) {
        const dx = bx - pointer.wx, dy = by - pointer.wy, d2 = dx * dx + dy * dy;
        if (d2 < reach) {
          const gap = Math.sqrt(d2) || 1e-6;
          lit = (1 - d2 / reach) ** 2;
          fx = ((dx * push - dy * swirl) / gap + dragX) * lit; fy = ((dy * push + dx * swirl) / gap + dragY) * lit;
          if (lit * env[i] > near) near = lit * env[i];
        }
      }
      const k = lit > 0 ? pull : release;
      p.px += (fx - p.px) * k; p.py += (fy - p.py) * k;
      p.x = bx + p.px; p.y = by + p.py;
      p.lit += (lit - p.lit) * (lit > p.lit ? lift : sink);
      p.a = p.alpha * env[i] * ramp(dens[i] - p.rank, 0, .12) * (.7 + .3 * Math.sin(time * p.rate + p.phase)) * (1 + p.wake * 1.2 + p.lit * 1.6 + surge(p.s) * .9);
    }
    for (const p of puffs) {
      p.s += p.speed * dt; if (p.s >= length) p.s -= length;
      const at = p.s / step, i = at | 0, t = at - i, off = p.n * offA[i] + p.m * offB[i];
      p.x = mix(x[i], x[i + 1], t) - ty[i] * off; p.y = mix(y[i], y[i + 1], t) + tx[i] * off;
      p.r = p.reach * mix(BAND.half * .6, girth[i], .6); p.a = p.alpha * env[i] * (.72 + .28 * Math.sin(time * .4 + p.phase)) * (1 + surge(p.s) * .8);
    }
    for (const p of streaks) { p.s += p.speed * dt; if (p.s - p.span >= stream.tail) p.s = stream.head; }
    pointer.near += (near - pointer.near) * ease(8, dt);
  }

  function stepPointer(dt) {
    pointer.shock = Math.max(0, pointer.shock - dt / .7);
    if (!pointer.inside || dt <= 0) return;
    // Moving across the stream scatters stardust.
    const pace = Math.hypot(pointer.vx, pointer.vy);
    pointer.shed += dt * pointer.near * clamp(pace * 26, 1.5, 34);
    while (pointer.shed >= 1) { pointer.shed--; emit(pointer.wx, pointer.wy, 1, .05, .5, pointer.vx * .1, pointer.vy * .1); }
  }

  function update(time, dt) {
    stepHero(time, dt);
    stepFlow(time, dt);
    stepPointer(dt);
    for (let i = sparkles.length - 1; i >= 0; i--) {
      const s = sparkles[i], drag = Math.exp(-1.6 * dt);
      s.life -= dt; s.x += s.vx * dt; s.y += s.vy * dt; s.vx *= drag; s.vy = s.vy * drag + .03 * dt;
      if (s.life <= 0) sparkles.splice(i, 1);
    }
    for (let i = rings.length - 1; i >= 0; i--) { rings[i].life -= dt; if (rings[i].life <= 0) rings.splice(i, 1); }
    // Now and then a meteor crosses the far sky.
    if (meteor.life > 0) { meteor.life -= dt; meteor.x += meteor.vx * dt; meteor.y += meteor.vy * dt; }
    else if ((meteor.wait -= dt) <= 0) {
      const angle = between(2.5, 2.9), pace = between(420, 640);
      Object.assign(meteor, { wait: between(5, 12), span: between(.6, .95), x: between(.25, 1) * width, y: between(0, .45) * height, vx: Math.cos(angle) * pace, vy: Math.sin(angle) * pace });
      meteor.life = meteor.span;
    }
    reveal = Math.min(REVEAL_END, reveal + dt / 1.5);
    if (sheet) sheetShown = Math.min(1, sheetShown + dt / .6);
  }

  // Paints what never moves: the dim light of the whole sky, the graded haze of the S with its dark lanes, and dust.
  function renderBackdrop() {
    const scale = dpr * .5, rnd = seeded(911), { head, tail, step } = stream, first = Math.ceil(head / step), final = Math.floor(tail / step);
    const place = (i, n, m = 0) => {
      const off = n * stream.offA[i] + m * stream.offB[i];
      return [centre.x + (stream.x[i] - stream.ty[i] * off) * unit, centre.y + (stream.y[i] + stream.tx[i] * off) * unit];
    };

    // Wide, faint light is summed per cell at low resolution and dithered with noise.
    // Drawn as stacked faint sprites it bands, and the bands pick up the sprites' dither as a lattice.
    const w = Math.ceil(width / CELL) + MARGIN * 2 + 1, h = Math.ceil(height / CELL) + MARGIN * 2 + 1, light = new Float32Array(w * h * 3);
    const blob = (cx, cy, r, colour, strength) => {
      const x0 = Math.max(0, Math.floor((cx - r) / CELL) + MARGIN), x1 = Math.min(w - 1, Math.ceil((cx + r) / CELL) + MARGIN);
      const y0 = Math.max(0, Math.floor((cy - r) / CELL) + MARGIN), y1 = Math.min(h - 1, Math.ceil((cy + r) / CELL) + MARGIN);
      for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) {
        const k = 1 - (((x - MARGIN) * CELL - cx) ** 2 + ((y - MARGIN) * CELL - cy) ** 2) / (r * r);
        if (k <= 0) continue;
        const n = (y * w + x) * 3, a = strength * k * k * k;
        light[n] += colour[0] * a; light[n + 1] += colour[1] * a; light[n + 2] += colour[2] * a;
      }
    };
    // A glow around the S, wide clouds across the whole hero, and the haze of the lanes in and out.
    blob(centre.x, centre.y, unit * 1.05, [84, 62, 200], .2);
    for (let k = Math.round(clamp(width * height / 70000, 6, 22)); k > 0; k--) blob(rnd() * width, rnd() * height, unit * (.5 + rnd() * .8), [[70, 54, 170], [96, 62, 176], [50, 78, 170]][rnd() * 3 | 0], .032 + rnd() * .05);
    for (let i = 0; i < stream.N; i += 9) if (i < first || i > final) blob(...place(i, (rnd() - .5) * .8), stream.girth[i] * unit * 1.9, i < first ? GRADE[0][1] : GRADE[4][1], .055 * stream.env[i]);
    const image = new ImageData(w, h);
    for (let i = 0; i < w * h; i++) {
      for (let c = 0; c < 3; c++) image.data[i * 4 + c] = BACKGROUND[c] + light[i * 3 + c] + rnd() - .5;
      image.data[i * 4 + 3] = 255;
    }
    haze.width = w; haze.height = h;
    haze.getContext('2d').putImageData(image, 0, 0);

    sky.width = Math.max(1, Math.round(width * scale)); sky.height = Math.max(1, Math.round(height * scale));
    const g = sky.getContext('2d');
    g.setTransform(scale, 0, 0, scale, 0, 0);
    g.globalCompositeOperation = 'lighter';
    const stamp = (x, y, r, colour, alpha) => {
      const key = colour.map((value) => value >> 3).join();
      if (!tinted.has(key)) tinted.set(key, puffSprite(colour));
      g.globalAlpha = alpha; g.drawImage(tinted.get(key), x - r, y - r, r * 2, r * 2);
    };
    // The S, from a wide haze to the bright core.
    for (const [reach, alpha, pale] of [[2.1, .05, 0], [1.25, .09, .1], [.6, .12, .5]]) {
      for (let i = first; i <= final;) {
        const f = (i - first) / (final - first), r = Math.max(stream.girth[i], BAND.half * .45) * reach, colour = grade(f).map((value, n) => mix(value, TINTS[0][n], pale));
        stamp(...place(i, (rnd() - .5) * .3), r * unit, colour, alpha * stream.env[i]);
        i += Math.max(1, Math.round(r * .35 / step));
      }
    }
    // Clouds of indigo, pink and sky blue break up the even haze.
    for (let k = 0; k < 90; k++) {
      const i = first + Math.round(rnd() * (final - first));
      stamp(...place(i, (rnd() * 2 - 1) * 1.1, rnd() * 2 - 1), Math.max(stream.girth[i], BAND.half * .5) * unit * (.6 + rnd() * 1.2), TINTS[2 + (rnd() * 3 | 0)], (.07 + rnd() * .1) * stream.env[i]);
    }
    g.globalCompositeOperation = 'destination-out';
    for (let i = first; i <= final; i += 3) {
      const f = (i - first) / (final - first), n = .42 * Math.sin(f * 19 + 1.3) + .22 * Math.sin(f * 47);
      stamp(...place(i, n), BAND.half * unit * (.3 + rnd() * .3), [0, 0, 0], .13);
    }
    g.globalCompositeOperation = 'lighter'; g.globalAlpha = 1;
    // Scattered faint stars along the lanes, so the S is the bright middle of one long river, and dust everywhere else.
    for (let i = 0; i < stream.N; i += 9) {
      if (i >= first && i <= final) continue;
      for (let k = 0; k < 5; k++) {
        const [x, y] = place(Math.min(stream.N - 1, i + (rnd() * 9 | 0)), (rnd() + rnd() + rnd() - 1.5) * 1.5, rnd() * 2 - 1), size = .6 + rnd() * rnd() * 1.3;
        g.fillStyle = rgba(TINTS[pickTint(rnd())], (.25 + rnd() * .5) * stream.env[i]); g.fillRect(x - size / 2, y - size / 2, size, size);
      }
    }
    for (let k = Math.round(width * height / 3600); k > 0; k--) {
      const size = .6 + rnd() * rnd() * 1.5;
      g.fillStyle = `rgba(196,192,236,${.1 + rnd() * .42})`; g.fillRect(rnd() * width, rnd() * height, size, size);
    }

    // The faintest stars of the S, drawn sharp in a box around it.
    const box = Math.ceil(unit * 1.5);
    for (const layer of specks) {
      layer.width = layer.height = Math.max(1, Math.round(box * dpr));
      const s = layer.getContext('2d');
      s.setTransform(dpr, 0, 0, dpr, (box / 2 - centre.x) * dpr, (box / 2 - centre.y) * dpr);
      for (let k = compact ? 520 : 950; k > 0; k--) {
        const i = first + Math.round(rnd() * (final - first)), [x, y] = place(i, (rnd() + rnd() + rnd() - 1.5) * 1.35, rnd() * 2 - 1), size = .5 + rnd() * rnd() * 1.1;
        s.fillStyle = rgba(TINTS[pickTint(rnd())], (.18 + rnd() * .5) * stream.env[i]);
        s.fillRect(x - size / 2, y - size / 2, size, size);
      }
    }
  }

  // Where the centre of the S is on the canvas: it drifts a little with the pointer.
  function origin(time) {
    return { x: centre.x - pointer.x * 5, y: centre.y - pointer.y * 4 + Math.sin(time * .55) * unit * .006 };
  }

  function drawSky(o, time) {
    const x = o.x - centre.x, y = o.y - centre.y;
    ctx.globalCompositeOperation = 'source-over'; ctx.globalAlpha = 1;
    ctx.drawImage(haze, x - (MARGIN + .5) * CELL, y - (MARGIN + .5) * CELL, haze.width * CELL, haze.height * CELL);
    ctx.globalCompositeOperation = 'lighter';
    ctx.drawImage(sky, x, y, width, height);
    for (const mote of twinkles) {
      const r = mote.r * (mote.flare ? 2.2 : 1);
      ctx.globalAlpha = mote.a * (.5 + .5 * Math.sin(mote.phase + time * mote.rate));
      ctx.drawImage(mote.flare ? sprites.flare : sprites.dots[mote.tint], mote.x * width - r, mote.y * height - r, r * 2, r * 2);
    }
    if (meteor.life > 0) {
      const glow = Math.sin(Math.PI * meteor.life / meteor.span), tail = .2;
      const streak = ctx.createLinearGradient(meteor.x, meteor.y, meteor.x - meteor.vx * tail, meteor.y - meteor.vy * tail);
      streak.addColorStop(0, rgba(TINTS[0], .7 * glow)); streak.addColorStop(1, 'rgba(170,152,255,0)');
      ctx.globalAlpha = 1; ctx.strokeStyle = streak; ctx.lineWidth = 1.3; ctx.lineCap = 'round';
      ctx.beginPath(); ctx.moveTo(meteor.x, meteor.y); ctx.lineTo(meteor.x - meteor.vx * tail, meteor.y - meteor.vy * tail); ctx.stroke();
    }
  }

  function drawGalaxy(o, time) {
    const { step, head, tail } = stream, shown = ramp(reveal, 0, .9), revealed = reveal >= REVEAL_END;
    // While the scene opens, the stream pours in from the head of the S.
    const veil = (s) => (revealed ? 1 : ramp(reveal - clamp((s - head) / (tail - head), 0, 1), 0, .25));
    const k = clamp(unit / 430, .7, 1.15), shimmer = Math.sin(time * .45) * .4, box = Math.ceil(unit * 1.5);
    ctx.globalCompositeOperation = 'lighter';
    ctx.globalAlpha = shown * (.6 + shimmer); ctx.drawImage(specks[0], o.x - box / 2, o.y - box / 2, box, box);
    ctx.globalAlpha = shown * (.6 - shimmer); ctx.drawImage(specks[1], o.x - box / 2, o.y - box / 2, box, box);

    for (const p of puffs) {
      const r = p.r * unit, x = o.x + p.x * unit, y = o.y + p.y * unit;
      if (x < -r || x > width + r || y < -r || y > height + r) continue;
      ctx.globalAlpha = p.a * veil(p.s); ctx.drawImage(sprites.puffs[p.tint], x - r, y - r, r * 2, r * 2);
    }
    for (const p of flow) {
      const alpha = p.a * veil(p.s);
      if (alpha < .012) continue;
      const x = o.x + p.x * unit, y = o.y + p.y * unit, swell = 1 + p.wake * .5 + p.lit * .7;
      if (x < -20 || x > width + 20 || y < -20 || y > height + 20) continue;
      ctx.globalAlpha = Math.min(1, alpha);
      if (p.glint) { const r = p.size * k * swell * (.75 + .25 * Math.sin(time * p.rate * 1.7 + p.phase)); ctx.drawImage(sprites.flare, x - r, y - r, r * 2, r * 2); }
      else { const r = p.size * k * swell * 1.7; ctx.drawImage(sprites.dots[p.tint], x - r, y - r, r * 2, r * 2); }
    }
    // Thin lines of light that run ahead of the slow stars and show which way the stream flows.
    ctx.lineCap = 'round'; ctx.lineJoin = 'round';
    for (const p of streaks) {
      const front = Math.min(p.s, tail), back = Math.max(p.s - p.span, head);
      if (front - back < step * 2) continue;
      let first, final;
      ctx.beginPath();
      for (let n = 0; n <= 12; n++) {
        const i = (mix(back, front, n / 12) / step) | 0, off = p.n * stream.offA[i] + p.m * stream.offB[i];
        final = [o.x + (stream.x[i] - stream.ty[i] * off) * unit, o.y + (stream.y[i] + stream.tx[i] * off) * unit];
        if (n) ctx.lineTo(...final); else { ctx.moveTo(...final); first = final; }
      }
      const gradient = ctx.createLinearGradient(...first, ...final);
      gradient.addColorStop(0, 'rgba(170,152,255,0)'); gradient.addColorStop(1, rgba(TINTS[0], p.alpha * stream.env[(front / step) | 0] * veil(front)));
      ctx.globalAlpha = 1; ctx.strokeStyle = gradient; ctx.lineWidth = p.weight * k; ctx.stroke();
    }
  }

  // Light on the stream: under the pointer, under the star, and a ring from each landing or press.
  function drawContact(o) {
    ctx.globalCompositeOperation = 'lighter';
    if (pointer.near > .01) {
      const x = o.x + pointer.wx * unit, y = o.y + pointer.wy * unit, r = unit * .2, core = unit * .036;
      ctx.globalAlpha = Math.min(1, pointer.near * .5); ctx.drawImage(sprites.puffs[1], x - r, y - r, r * 2, r * 2);
      ctx.globalAlpha = Math.min(1, pointer.near * 1.2); ctx.drawImage(sprites.flare, x - core, y - core, core * 2, core * 2);
    }
    const r = unit * .25 * HERO_SIZE / .24;
    ctx.globalAlpha = .24 * hero.on * sheetShown;
    ctx.drawImage(sprites.puffs[1], o.x + hero.x * unit - r, o.y + hero.y * unit - r, r * 2, r * 2);
    for (const ring of rings) {
      const t = 1 - ring.life / ring.span, radius = unit * mix(.04, ring.reach, 1 - (1 - t) ** 3);
      ctx.save();
      ctx.translate(o.x + ring.x * unit, o.y + ring.y * unit); ctx.rotate(ring.angle); ctx.scale(1, ring.flat);
      ctx.beginPath(); ctx.arc(0, 0, radius, 0, TAU);
      ctx.restore();
      ctx.globalAlpha = 1; ctx.strokeStyle = rgba(TINTS[0], (1 - t) ** 1.5 * .55); ctx.lineWidth = mix(2.2, .4, t); ctx.stroke();
    }
  }

  // A ribbon of light behind the star: a wide violet veil and a thin bright core, each drawn as a strip that narrows and fades.
  function drawTrail(o) {
    const total = Math.min(trail.length, edges.length / 4);
    if (total < 3) return;
    ctx.globalCompositeOperation = 'lighter';
    for (const [spread, colour, strength] of [[.2, [142, 120, 255], .34], [.065, [242, 238, 255], .62]]) {
      for (let i = 0; i < total; i++) {
        const a = trail[Math.max(0, i - 1)], b = trail[Math.min(total - 1, i + 1)], point = trail[i];
        const dx = b.x - a.x, dy = b.y - a.y, span = Math.hypot(dx, dy) || 1e-6, wide = HERO_SIZE * spread * point.scale * (1 - point.age / TRAIL_LIFE) ** 1.3;
        edges[i * 4] = o.x + (point.x - dy / span * wide) * unit; edges[i * 4 + 1] = o.y + (point.y + dx / span * wide) * unit;
        edges[i * 4 + 2] = o.x + (point.x + dy / span * wide) * unit; edges[i * 4 + 3] = o.y + (point.y - dx / span * wide) * unit;
      }
      ctx.fillStyle = rgba(colour, 1);
      for (let i = 0, n = 4; i < total - 1; i++, n += 4) {
        ctx.globalAlpha = strength * (1 - trail[i].age / TRAIL_LIFE) ** 1.8 * sheetShown;
        ctx.beginPath(); ctx.moveTo(edges[n - 4], edges[n - 3]); ctx.lineTo(edges[n], edges[n + 1]); ctx.lineTo(edges[n + 2], edges[n + 3]); ctx.lineTo(edges[n - 2], edges[n - 1]); ctx.fill();
      }
    }
  }

  function drawHero(o, time, opacity) {
    const x = o.x + hero.dx * unit, y = o.y + hero.dy * unit, size = HERO_SIZE * unit * hero.scale / SHEET.fill;
    const step = Math.round((hero.yaw + hero.spinDir * 360 * smooth(1 - hero.spin) * (hero.spin > 0)) / (360 / SHEET.yawSteps));
    let index = ((step % SHEET.yawSteps) + SHEET.yawSteps) % SHEET.yawSteps;
    const signed = index > SHEET.yawSteps / 2 ? index - SHEET.yawSteps : index;
    if (hero.blink > 0 && Math.abs(signed) <= SHEET.blinkHalfRange) index = SHEET.yawSteps + signed + SHEET.blinkHalfRange;
    // A soft shadow keeps the pale star readable against the bright stream; it stays on the stream while the star hops.
    const sx = o.x + (hero.x + .012) * unit, sy = o.y + (hero.y + .045 * hero.scale) * unit, shade = size * mix(.62, .5, Math.sin(Math.PI * hero.hop));
    ctx.globalCompositeOperation = 'source-over';
    ctx.globalAlpha = .5 * hero.ground * opacity * sheetShown;
    ctx.drawImage(sprites.shade, sx - shade, sy - shade * .8, shade * 2, shade * 1.6);
    ctx.globalCompositeOperation = 'lighter';
    ctx.globalAlpha = (.3 + Math.sin(time * 2.4) * .05) * opacity * sheetShown;
    ctx.drawImage(sprites.puffs[1], x - size * .95, y - size * .95, size * 1.9, size * 1.9);
    ctx.globalCompositeOperation = 'source-over';
    const breath = 1 + Math.sin(time * 2.1) * .012;
    ctx.save();
    ctx.translate(x, y); ctx.rotate(hero.roll); ctx.scale((2 - breath) * (1 + hero.squash * .9), breath * (1 - hero.squash));
    ctx.globalAlpha = opacity * sheetShown;
    ctx.drawImage(sheet, index % SHEET.columns * SHEET.cell, Math.floor(index / SHEET.columns) * SHEET.cell, SHEET.cell, SHEET.cell, -size / 2, -size / 2, size, size);
    ctx.restore();
  }

  function drawSparkles(o, time) {
    ctx.globalCompositeOperation = 'lighter';
    for (const s of sparkles) {
      const glow = Math.sin(Math.PI * s.life / s.span), r = s.size * unit * (s.flare ? 1.5 : .8) * (.35 + .65 * glow), x = o.x + s.x * unit, y = o.y + s.y * unit;
      ctx.globalAlpha = Math.min(1, glow * 1.6) * (.72 + .28 * Math.sin(time * 9 + s.phase));
      ctx.drawImage(s.flare ? sprites.flare : sprites.dots[s.tint], x - r, y - r, r * 2, r * 2);
    }
  }

  function draw(time) {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.globalCompositeOperation = 'source-over'; ctx.globalAlpha = 1;
    ctx.clearRect(0, 0, width, height);
    const o = origin(time);
    drawSky(o, time);
    // Far away the star is behind the galaxy, close up in front of it. In between, the near star fades in over the far one.
    if (sheet && hero.front < .5) drawTrail(o);
    if (sheet && hero.front < .995) drawHero(o, time, 1);
    drawGalaxy(o, time);
    drawContact(o);
    if (sheet && hero.front >= .5) drawTrail(o);
    if (sheet && hero.front > .005) drawHero(o, time, hero.front);
    drawSparkles(o, time);
    ctx.globalCompositeOperation = 'source-over'; ctx.globalAlpha = 1;
  }

  function shouldRun() { return active && !document.hidden && !reduced.matches && !manualPaused && !disposed; }
  function tick(timestamp) {
    frame = 0;
    if (!shouldRun()) return;
    const dt = last ? Math.min((timestamp - last) / 1000, .05) : 0;
    clock += dt; last = timestamp;
    pointer.x += (pointer.tx - pointer.x) * .05;
    pointer.y += (pointer.ty - pointer.y) * .05;
    // The pointer in world units, and how fast it is moving there.
    const o = origin(clock), wx = (pointer.sx - o.x) / unit, wy = (pointer.sy - o.y) / unit;
    if (pointer.inside && !pointer.fresh && dt > 0) { const k = ease(14, dt); pointer.vx += ((wx - pointer.wx) / dt - pointer.vx) * k; pointer.vy += ((wy - pointer.wy) / dt - pointer.vy) * k; }
    else { pointer.vx = 0; pointer.vy = 0; }
    pointer.wx = wx; pointer.wy = wy; pointer.fresh = false;
    update(clock, dt);
    draw(clock);
    frame = requestAnimationFrame(tick);
  }
  function sync() {
    if (frame) cancelAnimationFrame(frame);
    frame = 0; last = 0;
    if (shouldRun()) frame = requestAnimationFrame(tick);
    else if (active && !document.hidden && !disposed) { reveal = REVEAL_END; sheetShown = sheet ? 1 : 0; draw(clock); }
  }
  function resize() {
    const rect = canvas.getBoundingClientRect(), box = anchor.getBoundingClientRect();
    width = Math.max(1, rect.width); height = Math.max(1, rect.height);
    // The canvas spans the hero, so its pixel count is capped rather than only its pixel ratio.
    dpr = Math.min(window.devicePixelRatio || 1, 2, Math.sqrt(5.2e6 / (width * height)));
    canvas.width = Math.round(width * dpr); canvas.height = Math.round(height * dpr);
    unit = Math.min(box.height * .7, box.width * .735);
    centre.x = box.left - rect.left + box.width / 2; centre.y = box.top - rect.top + box.height / 2;
    half.w = box.width / 2 / unit; half.h = box.height / 2 / unit;
    // Where the star may go: the whole hero, short of the edges and of the fade at the bottom.
    const pad = HERO_SIZE * .5;
    roam.l = (24 - centre.x) / unit + pad; roam.r = (width - 24 - centre.x) / unit - pad;
    roam.t = (24 - centre.y) / unit + pad; roam.b = (height * .84 - centre.y) / unit - pad;
    if (compact !== box.width < 450) { compact = box.width < 450; populate(); stepFlow(clock, 0); }
    renderBackdrop();
    draw(clock);
  }
  function locate(event) {
    const rect = canvas.getBoundingClientRect();
    pointer.sx = event.clientX - rect.left; pointer.sy = event.clientY - rect.top;
    pointer.tx = pointer.sx / width * 2 - 1; pointer.ty = pointer.sy / height * 2 - 1;
    if (!pointer.inside) { pointer.inside = true; pointer.fresh = true; }
  }
  function leave() { pointer.tx = 0; pointer.ty = 0; pointer.inside = false; }
  // Pressing the star makes it hop and twirl; pressing the stream sends a ripple through it.
  function poke(event) {
    locate(event);
    if (!shouldRun() || event.target.closest?.('a, button')) return;
    const o = origin(clock), wx = (pointer.sx - o.x) / unit, wy = (pointer.sy - o.y) / unit;
    if (Math.hypot(hero.dx - wx, hero.dy - wy) < HERO_SIZE * hero.scale * .6) { leap(wx < hero.dx ? 1 : -1); shed(8, .12); return; }
    pointer.shock = 1;
    rings.push({ x: wx, y: wy, angle: 0, flat: 1, reach: .17, life: .7, span: .7 });
    emit(wx, wy, 12, .16, .6);
  }
  function load(url) {
    const image = new Image();
    image.decoding = 'async'; image.src = url;
    return image.decode().then(() => image, () => null);
  }
  load(SHEET.url).then((image) => {
    if (!image || disposed) return;
    sheet = image;
    if (!shouldRun()) { sheetShown = 1; draw(clock); }
  });

  // Run the scene a moment ahead so the first frame, and the reduced-motion still, already has a wake behind the star.
  compact = anchor.getBoundingClientRect().width < 450;
  populate();
  for (let i = 0; i < 60; i++) { clock += 1 / 60; update(clock, 1 / 60); }
  reveal = 0;

  const resizeObserver = new ResizeObserver(resize);
  resizeObserver.observe(canvas);
  if (anchor !== canvas) resizeObserver.observe(anchor);
  const intersection = new IntersectionObserver(([entry]) => { active = entry.isIntersecting; sync(); }, { threshold: .03 });
  intersection.observe(canvas);
  surface.addEventListener('pointermove', locate, { passive: true });
  surface.addEventListener('pointerdown', poke, { passive: true });
  surface.addEventListener('pointerleave', leave);
  reduced.addEventListener('change', sync);
  document.addEventListener('visibilitychange', sync);
  resize(); sync();
  const cleanup = () => {
    disposed = true;
    if (frame) cancelAnimationFrame(frame);
    resizeObserver.disconnect(); intersection.disconnect();
    surface.removeEventListener('pointermove', locate); surface.removeEventListener('pointerdown', poke); surface.removeEventListener('pointerleave', leave);
    reduced.removeEventListener('change', sync); document.removeEventListener('visibilitychange', sync);
  };
  // Keep the teardown function convenient while exposing the explicit UI toggle.
  cleanup.setPaused = (paused) => { manualPaused = Boolean(paused); sync(); };
  return cleanup;
}
