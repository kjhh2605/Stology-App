// 지식지도 3D 그래프를 Canvas 2D로 다시 그린다. 배치·카메라·강조·이름표 규칙은 앱의
// app/ui/src/graph.rs와 graph_math.rs를 그대로 옮겼고, Metal 구체는 같은 조명식으로 미리 칠한 스프라이트로 대신한다.
// 깊이는 구체·연결선을 멀리서 가까이 순서로 칠해 근사한다. 장면이 쓰지 않는 관계 필터·단원 강조·개념 드래그·
// 연결선 선택과 같은 쌍의 곡선 연결(이 데이터에는 없다)은 옮기지 않았다.
// 좌표는 앱처럼 아래쪽이 원점인 논리 픽셀로 계산하고 그릴 때만 뒤집는다.

const NODE_RADIUS = 15;
const UNIT = NODE_RADIUS / 1.25;
const LAYOUT_DURATION = 0.72;
const FOV_Y = 42 * Math.PI / 180;
const NEAR = 0.5, FAR = 10000;
const PITCH_LIMIT = Math.PI / 3;
const NEAR_CLIP = 4;
const LABEL_FONT = 14, EYEBROW_FONT = 10.5, LABEL_PAD_X = 8, LABEL_HEIGHT = 24, EYEBROW_HEIGHT = 13, LABEL_MAX_TEXT = 152;
const ACTIVE = 0x4f46e5, DIM = 0xc9cede, EDGE = 0x8b93a8, EDGE_HIGHLIGHT = 0x5b53e8, INK = 0x1c1e2e;
const UNIT_COLORS = [0x16857b, 0xc77724, 0xbc5774, 0x3884b2, 0x83912e, 0xa16d44, 0x9c5ca0, 0x3b9470, 0xc26342, 0x657ba8];
const MAP_BACKGROUND = 'rgb(248,249,253)';
const LINK_LABELS = { kind_of: '종류', part_of: '구성', prerequisite: '선행', cause: '원인', contrast: '대조', example: '예시', applies: '적용', related: '관련' };
export const unitColor = position => '#' + UNIT_COLORS[position % UNIT_COLORS.length].toString(16).padStart(6, '0');

/* ---------- 벡터 ---------- */
const v3 = (x, y, z) => [x, y, z];
const add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const mul = (a, k) => [a[0] * k, a[1] * k, a[2] * k];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const len = a => Math.hypot(a[0], a[1], a[2]);
const norm = a => { const n = len(a); return n > 1e-12 && Number.isFinite(n) ? mul(a, 1 / n) : [0, 0, 0]; };
const lerp3 = (a, b, t) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
const ZERO = [0, 0, 0];
const smoothstep = (a, b, x) => { const t = Math.min(1, Math.max(0, (x - a) / (b - a))); return t * t * (3 - 2 * t); };
const nearFade = d => smoothstep(3 * NODE_RADIUS, 6 * NODE_RADIUS, d);
const easeInOutCubic = p => (p < .5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
const srgb = hex => [((hex >> 16) & 255) / 255, ((hex >> 8) & 255) / 255, (hex & 255) / 255];
const toLinear = c => c.map(v => (v <= .04045 ? v / 12.92 : Math.pow((v + .055) / 1.055, 2.4)));
const toSrgb = v => (v <= .0031308 ? v * 12.92 : 1.055 * Math.pow(Math.max(v, 0), 1 / 2.4) - .055);
const rgba = (c, a) => `rgba(${Math.round(c[0] * 255)},${Math.round(c[1] * 255)},${Math.round(c[2] * 255)},${a})`;

/* ---------- 카메라(graph_math.rs Camera·Rig) ---------- */
const defaultCamera = () => ({ yaw: .62, pitch: .30, distance: 800, pivot: ZERO, shiftX: 0, shiftY: 0 });
const copyCamera = c => ({ ...c, pivot: [...c.pivot] });
function basis(c) {
  const right = v3(Math.cos(c.yaw), 0, -Math.sin(c.yaw));
  const toward = v3(Math.sin(c.yaw) * Math.cos(c.pitch), Math.sin(c.pitch), Math.cos(c.yaw) * Math.cos(c.pitch));
  return [right, norm(cross(toward, right)), toward];
}
const eyeOf = c => add(c.pivot, mul(basis(c)[2], c.distance));
function project(c, world, w, h) {
  const [right, up, toward] = basis(c);
  const rel = sub(world, eyeOf(c));
  const depth = -dot(rel, toward);
  if (!(depth >= NEAR && depth <= FAR)) return null;
  const f = 1 / Math.tan(FOV_Y / 2);
  const scale = h * .5 * f / depth;
  return { x: w * .5 + c.shiftX + dot(rel, right) * scale, y: h * .5 + c.shiftY + dot(rel, up) * scale, depth, scale };
}
const limitingTan = (w, h) => { const t = Math.tan(FOV_Y / 2); return Math.min(t, t * Math.max(w, 1) / Math.max(h, 1)); };
const fitDistance = (r, w, h) => Math.max(r, 1) / Math.sin(Math.atan(limitingTan(w, h))) * 1.18;
function fitCamera(c, points, w, h) {
  if (!points.length) return { ...defaultCamera(), shiftX: c.shiftX, shiftY: c.shiftY };
  const pivot = mul(points.reduce((s, [p]) => add(s, p), ZERO), 1 / points.length);
  const radius = points.reduce((m, [p, r]) => Math.max(m, len(sub(p, pivot)) + Math.max(r, 0)), 20);
  return { ...c, pivot, distance: fitDistance(radius, w, h), yaw: .62, pitch: .30 };
}
const CAMERA_TRANSITION = .72;
class Rig {
  constructor() { this.now = defaultCamera(); this.goal = defaultCamera(); this.from = defaultCamera(); this.bounded = false; this.elapsed = CAMERA_TRANSITION; this.min = 40; this.max = 8000; }
  setLimits(min, max) { this.min = min; this.max = Math.max(max, min); this.goal.distance = Math.min(this.max, Math.max(this.min, this.goal.distance)); }
  setShift(x, y) { for (const c of [this.now, this.goal, this.from]) { c.shiftX = x; c.shiftY = y; } }
  orbit(dx, dy) { this.bounded = false; this.goal.yaw -= dx * .0042; this.goal.pitch = Math.min(PITCH_LIMIT, Math.max(-PITCH_LIMIT, this.goal.pitch + dy * .0036)); }
  travel(goal, animate) {
    goal = copyCamera(goal); goal.shiftX = this.now.shiftX; goal.shiftY = this.now.shiftY;
    goal.distance = Math.min(this.max, Math.max(this.min, goal.distance));
    const tau = Math.PI * 2;
    this.now.yaw = goal.yaw + (((this.now.yaw - goal.yaw + tau / 2) % tau) + tau) % tau - tau / 2;
    this.goal = goal; this.from = copyCamera(this.now); this.elapsed = 0; this.bounded = animate;
    if (!animate) this.snap();
  }
  snap() { this.now = copyCamera(this.goal); this.bounded = false; this.elapsed = CAMERA_TRANSITION; }
  update(dt, reduced) {
    if (reduced) { this.snap(); return false; }
    let moving = false;
    if (this.bounded) {
      this.elapsed = Math.min(CAMERA_TRANSITION, this.elapsed + dt);
      const p = this.elapsed / CAMERA_TRANSITION, k = easeInOutCubic(p);
      for (const key of ['yaw', 'pitch', 'distance']) this.now[key] = this.from[key] + (this.goal[key] - this.from[key]) * k;
      this.now.pivot = lerp3(this.from.pivot, this.goal.pivot, k);
      this.bounded = p < 1; moving = this.bounded;
    } else {
      const d = 1 - Math.pow(.0016, Math.max(dt, 0));
      for (const key of ['yaw', 'pitch', 'distance']) this.now[key] += (this.goal[key] - this.now[key]) * d;
      this.now.pivot = lerp3(this.now.pivot, this.goal.pivot, d);
    }
    const e = .0004;
    const settled = Math.abs(this.goal.yaw - this.now.yaw) <= e && Math.abs(this.goal.pitch - this.now.pitch) <= e && Math.abs(this.goal.distance - this.now.distance) <= e * 100 && dot(sub(this.goal.pivot, this.now.pivot), sub(this.goal.pivot, this.now.pivot)) <= e;
    if (!moving && settled) this.now = copyCamera(this.goal);
    return moving || !settled;
  }
}

/* ---------- 배치(graph_math.rs layout·focus_layout) ---------- */
function hashId(id) {
  let v = 0xcbf29ce484222325n;
  for (const b of new TextEncoder().encode(id)) v = BigInt.asUintN(64, (v ^ BigInt(b)) * 0x100000001b3n);
  return v;
}
const cellKey = (p, cell) => `${Math.floor(p[0] / cell)},${Math.floor(p[1] / cell)},${Math.floor(p[2] / cell)}`;
function neighbourCells(p, cell) {
  const x = Math.floor(p[0] / cell), y = Math.floor(p[1] / cell), z = Math.floor(p[2] / cell), out = [];
  for (let dx = -1; dx <= 1; dx++) for (let dy = -1; dy <= 1; dy++) for (let dz = -1; dz <= 1; dz++) out.push(`${x + dx},${y + dy},${z + dz}`);
  return out;
}
const byteOrder = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
export function layout(ids, edges) {
  const sorted = [...new Set(ids)].sort(byteOrder);
  const n = sorted.length;
  if (n === 0) return new Map();
  if (n === 1) return new Map([[sorted[0], ZERO]]);
  const index = new Map(sorted.map((id, i) => [id, i]));
  const pairs = new Set();
  for (const [a, b] of edges) { const i = index.get(a), j = index.get(b); if (i !== undefined && j !== undefined && i !== j) pairs.add(Math.min(i, j) * 100000 + Math.max(i, j)); }
  const links = [...pairs].sort((a, b) => a - b).map(k => [Math.floor(k / 100000), k % 100000]);
  const radius = Math.cbrt(n) * 84, golden = Math.PI * (3 - Math.sqrt(5));
  let positions = sorted.map((id, i) => {
    const y = 1 - 2 * (i + .5) / n, side = Math.sqrt(1 - y * y), angle = i * golden + .37;
    const radial = .64 + .36 * Math.cbrt(Number(hashId(id) >> 32n) / 0xffffffff);
    return mul(v3(Math.cos(angle) * side, y * .85, Math.sin(angle) * side), radius * radial);
  });
  const initial = positions.map(p => [...p]);
  const cell = 100;
  for (let iter = 0; iter < 64; iter++) {
    const grid = new Map();
    positions.forEach((p, i) => { const k = cellKey(p, cell); if (!grid.has(k)) grid.set(k, []); grid.get(k).push(i); });
    const deltas = positions.map(() => [0, 0, 0]);
    for (let i = 0; i < n; i++) {
      for (const k of neighbourCells(positions[i], cell)) {
        for (const j of grid.get(k) || []) {
          if (j <= i) continue;
          const d = sub(positions[j], positions[i]), l = len(d);
          if (l < 94 && l > 1e-8) { const f = mul(d, (94 - l) * .22 / l); deltas[i] = sub(deltas[i], f); deltas[j] = add(deltas[j], f); }
        }
      }
    }
    for (const [a, b] of links) {
      const d = sub(positions[b], positions[a]), l = len(d);
      if (l > 1e-8) { const f = mul(d, (l - 138) * .018 / l); deltas[a] = add(deltas[a], f); deltas[b] = sub(deltas[b], f); }
    }
    positions = positions.map((p, i) => { const f = add(deltas[i], mul(sub(initial[i], p), .024)); return add(p, len(f) > 7 ? mul(norm(f), 7) : f); });
  }
  const center = mul(positions.reduce((s, p) => add(s, p), ZERO), 1 / n);
  return new Map(sorted.map((id, i) => [id, sub(positions[i], center)]));
}
function fibonacciDirection(index, total) {
  const golden = Math.PI * (3 - Math.sqrt(5)), y = total <= 1 ? 0 : 1 - index / (total - 1) * 2, r = Math.sqrt(Math.max(0, 1 - y * y)), t = golden * index;
  return v3(Math.cos(t) * r, y, Math.sin(t) * r);
}
function focusLayout(base, radii, adjacency, visible, selected, [right, up, toward], out) {
  const count = base.length, tiers = new Array(count).fill(3);
  tiers[selected] = 0;
  const direct = adjacency[selected].filter(i => i !== selected && visible[i]);
  direct.forEach(i => { tiers[i] = 1; });
  for (const i of direct) for (const j of adjacency[i]) if (visible[j] && tiers[j] === 3) tiers[j] = 2;
  const n = direct.length;
  const directShell = Math.min(19, Math.max(10.5, 8.5 + n * .58)) * UNIT;
  const middleShell = directShell + Math.min(13, 8 + n * .18) * UNIT;
  const outerShell = middleShell + 13 * UNIT;
  const origin = base[selected];
  for (let i = 0; i < count; i++) {
    if (i === selected) { out[i] = ZERO; continue; }
    let dir = norm(sub(base[i], origin));
    if (dir[0] === 0 && dir[1] === 0 && dir[2] === 0) dir = fibonacciDirection(i, Math.max(count, 2));
    out[i] = mul(dir, tiers[i] === 2 ? middleShell : outerShell);
  }
  let largest = radii[selected];
  direct.forEach((i, order) => {
    const angle = -Math.PI / 2 + order / Math.max(n, 1) * Math.PI * 2;
    const depthRatio = ((order * .6180339887498949) % 1 - .5) * .42;
    const planar = directShell * Math.sqrt(Math.max(.72, 1 - depthRatio * depthRatio));
    out[i] = add(add(mul(right, Math.cos(angle) * planar), mul(up, Math.sin(angle) * planar)), mul(toward, directShell * depthRatio));
    largest = Math.max(largest, radii[i]);
  });
  relaxContext(out, radii, visible, tiers);
  return n ? directShell + largest + 2.8 * UNIT : Math.max(6 * UNIT, radii[selected] * 4);
}
function relaxContext(positions, radii, visible, tiers) {
  const cell = 5 * UNIT, count = positions.length;
  for (let iter = 0; iter < 3; iter++) {
    const grid = new Map();
    for (let i = 0; i < count; i++) if (visible[i]) { const k = cellKey(positions[i], cell); if (!grid.has(k)) grid.set(k, []); grid.get(k).push(i); }
    for (let i = 0; i < count; i++) {
      if (!visible[i]) continue;
      for (const k of neighbourCells(positions[i], cell)) {
        for (const j of grid.get(k) || []) {
          if (j <= i) continue;
          const iFixed = tiers[i] <= 1, jFixed = tiers[j] <= 1;
          if (iFixed && jFixed) continue;
          let d = sub(positions[j], positions[i]), dist = len(d);
          const minimum = radii[i] + radii[j] + 1.15 * UNIT;
          if (dist >= minimum) continue;
          if (dist < 1e-4) { d = fibonacciDirection(j, Math.max(count, 2)); dist = 1; }
          const push = mul(d, (minimum - dist) / dist);
          const iShare = iFixed ? 0 : jFixed ? 1 : .5, jShare = jFixed ? 0 : iFixed ? 1 : .5;
          positions[i] = sub(positions[i], mul(push, iShare));
          positions[j] = add(positions[j], mul(push, jShare));
        }
      }
    }
  }
}

/* ---------- 구체 스프라이트(graph_metal.rs sphere_f·halo_f) ---------- */
const SPRITE = 128;
const spriteCache = new Map();
function viewLights(camera) {
  const [r, u, t] = basis(camera);
  const light = (x, y, z) => { const l = norm(v3(x, y, z)); return [dot(l, r), dot(l, u), dot(l, t)]; };
  return { key: light(-42, 58, 46), fill: light(46, -18, 28), rim: light(12, 26, -62), sky: light(0, 1, 0) };
}
const LIGHTS = viewLights(defaultCamera());
function makeCanvas(w, h) {
  if (typeof OffscreenCanvas === 'function') return new OffscreenCanvas(w, h);
  const c = document.createElement('canvas'); c.width = w; c.height = h; return c;
}
/** 앱과 같은 조명식으로 칠한 구체. selected는 선택한 개념의 두 번째 하이라이트 */
function sphereSprite(albedo, selected) {
  const key = albedo.join(',') + (selected ? 's' : '');
  if (spriteCache.has(key)) return spriteCache.get(key);
  const canvas = makeCanvas(SPRITE, SPRITE), ctx = canvas.getContext('2d');
  const img = ctx.createImageData(SPRITE, SPRITE), R = SPRITE / 2 - 1, L = LIGHTS;
  const half = (a, b) => norm(add(a, b));
  const v = [0, 0, 1], keyHalf = half(L.key, v), fillHalf = half(L.fill, v);
  const sat = x => Math.min(1, Math.max(0, x));
  for (let py = 0; py < SPRITE; py++) {
    for (let px = 0; px < SPRITE; px++) {
      const x = (px + .5 - SPRITE / 2) / R, y = (SPRITE / 2 - py - .5) / R, r2 = x * x + y * y;
      const coverage = sat((1 - Math.sqrt(r2)) * R + .5);
      if (coverage <= 0) continue;
      const nz = Math.sqrt(Math.max(0, 1 - r2)), n = [x, y, nz];
      const sky = dot(n, L.sky) * .5 + .5;
      const ambient = [.42 + (.72 - .42) * sky, .43 + (.74 - .43) * sky, .53 + (.83 - .53) * sky];
      let k = sat((dot(n, L.key) + .3) / 1.3); k *= k;
      const fill = sat(dot(n, L.fill)), fillTint = [.074, .079, .10];
      const keyGloss = Math.pow(sat(dot(n, keyHalf)), selected ? 46 : 34);
      const coatGloss = selected ? Math.pow(sat(dot(n, keyHalf)), 220) : 0;
      const fillGloss = Math.pow(sat(dot(n, fillHalf)), 64);
      const sheen = Math.pow(1 - nz, 3.5) * sat(dot(n, L.rim) * .6 + .4);
      const o = (py * SPRITE + px) * 4;
      for (let c = 0; c < 3; c++) {
        let col = albedo[c] * (ambient[c] + .46 * k + fillTint[c] * fill) + albedo[c] * .05 * (selected ? 1 : 0);
        col += .24 * keyGloss + .42 * coatGloss + [.86, .90, 1][c] * .03 * fillGloss + [.62, .68, 1][c] * .12 * sheen;
        img.data[o + c] = Math.round(sat(toSrgb(sat(col))) * 255);
      }
      img.data[o + 3] = Math.round(coverage * 255);
    }
  }
  ctx.putImageData(img, 0, 0);
  spriteCache.set(key, canvas);
  return canvas;
}
function haloSprite() {
  if (spriteCache.has('halo')) return spriteCache.get('halo');
  const canvas = makeCanvas(SPRITE, SPRITE), ctx = canvas.getContext('2d'), img = ctx.createImageData(SPRITE, SPRITE);
  const tint = srgb(ACTIVE), R = SPRITE / 2;
  for (let py = 0; py < SPRITE; py++) for (let px = 0; px < SPRITE; px++) {
    const q = Math.hypot(px + .5 - R, py + .5 - R) / R;
    if (q >= 1) continue;
    const o = (py * SPRITE + px) * 4, a = .46 * Math.pow(1 - smoothstep(.4, 1, q), 1.6);
    img.data[o] = tint[0] * 255; img.data[o + 1] = tint[1] * 255; img.data[o + 2] = tint[2] * 255; img.data[o + 3] = Math.round(a * 255);
  }
  ctx.putImageData(img, 0, 0);
  spriteCache.set('halo', canvas);
  return canvas;
}

/* ---------- 지도 ---------- */
const normalizeText = s => String(s).replace(/\s/g, '').toLowerCase();
const approach = (now, goal, rate, eps) => (Math.abs(goal - now) > eps && rate < 1 ? now + (goal - now) * rate : goal);

/**
 * createAtlas(canvas, graph, options)
 * graph: { concepts:[{id,name,unit,aliases?}], links:[{s,o,kind,inferred,cond}] }
 * options: { width, height, pixelRatio, font, reduced, insets:{top,bottom} }
 */
export function createAtlas(canvas, graph, { width, height, pixelRatio = 2, font = 'SUIT, sans-serif', reduced = false, insets = { top: 48, bottom: 30 } } = {}) {
  const ctx = canvas.getContext('2d');
  canvas.width = Math.round(width * pixelRatio); canvas.height = Math.round(height * pixelRatio);
  const W = width, H = height, n = graph.concepts.length;
  const nodes = graph.concepts.map(c => ({ id: c.id, name: c.name, keys: [normalizeText(c.name), ...(c.aliases || []).map(normalizeText)], unit: c.unit, albedo: toLinear(srgb(UNIT_COLORS[c.unit % UNIT_COLORS.length])) }));
  const links = graph.links.map(l => ({ a: l.s, b: l.o, kind: l.kind, directed: !(l.kind === 'contrast' || l.kind === 'related'), inferred: !!l.inferred, label: (LINK_LABELS[l.kind] || '관련') + (l.cond ? ' · 조건' : '') }));
  const adjacency = nodes.map(() => []);
  for (const l of links) if (l.a !== l.b) { adjacency[l.a].push(l.b); adjacency[l.b].push(l.a); }
  adjacency.forEach((list, i) => { const uniq = [...new Set(list)].sort((x, y) => byteOrder(nodes[x].id, nodes[y].id)); adjacency[i] = uniq; nodes[i].degree = uniq.length; });
  const placed = layout(nodes.map(v => v.id), links.map(l => [nodes[l.a].id, nodes[l.b].id]));
  const base = nodes.map(v => placed.get(v.id));
  const bounds = base.reduce((m, p) => Math.max(m, len(p) + NODE_RADIUS), NODE_RADIUS);
  const dimLinear = toLinear(srgb(DIM));

  const s = {
    goal: base.map(p => [...p]), from: base.map(p => [...p]), now: base.map(p => [...p]), tween: null,
    scale: new Array(n).fill(NODE_RADIUS), scaleGoal: new Array(n).fill(NODE_RADIUS),
    dim: new Array(n).fill(0), dimGoal: new Array(n).fill(0), gloss: new Array(n).fill(0), glossGoal: new Array(n).fill(0),
    shown: new Array(n).fill(true), hits: new Array(n).fill(false), eyebrow: new Array(n).fill(''),
    stroke: links.map(() => ({ rgb: srgb(EDGE), alpha: 0, width: 1.25 })), strokeGoal: links.map(() => ({ rgb: srgb(EDGE), alpha: 0, width: 1.25 })),
    labelAlpha: new Array(n).fill(0), labelStyle: nodes.map(() => ['default', '']),
    halo: [[null, 0], [null, 0]], rig: new Rig(), selected: null, hovered: null, focus: null, focusRadius: 6 * UNIT, query: '',
    widths: new Map(),
  };
  s.rig.setShift(0, (insets.bottom - insets.top) / 2);
  const usable = () => [W, Math.max(200, H - insets.top - insets.bottom)];

  function updateLimits() {
    const [w, h] = usable();
    s.rig.setLimits(Math.max(9.6 * NODE_RADIUS, .34 * bounds), Math.max(3.1 * bounds, fitDistance(bounds + 22, w, h)));
  }
  function beginTween(animate) { if (animate && !reduced) s.tween = 0; else { s.now = s.goal.map(p => [...p]); s.tween = null; } }
  function frameAll(animate) {
    updateLimits();
    const [w, h] = usable();
    const points = base.map((p, i) => [p, NODE_RADIUS + 22]).filter((_, i) => s.shown[i]);
    s.rig.travel(fitCamera(s.rig.now, points, w, h), animate && !reduced);
  }
  function frameFocus(animate) {
    const [w, h] = usable();
    const desired = s.focusRadius * 1.22 / Math.max(.1, limitingTan(w, h));
    s.rig.max = Math.max(s.rig.max, desired / .82);
    const goal = copyCamera(s.rig.goal);
    goal.pivot = ZERO;
    goal.distance = Math.min(s.rig.max * .82, Math.max(s.rig.min, desired));
    s.rig.travel(goal, animate && !reduced);
  }
  function applyHighlight() {
    const focus = s.selected ?? s.hovered;
    const related = new Set(focus === null ? [] : adjacency[focus]);
    for (let i = 0; i < n; i++) {
      const isFocus = focus === i, isRelated = related.has(i);
      let d = 0;
      if (s.selected !== null) { if (!isFocus && !isRelated) d = .65; }
      else if (s.hovered !== null && !isFocus && !isRelated && !s.hits[i]) d = .55;
      s.dimGoal[i] = d;
      let scale = NODE_RADIUS;
      if (s.selected === i) scale *= 1.1;
      else if (s.hovered === i) scale *= 1.04;
      else if (isRelated) scale *= 1.07;
      else if (s.selected === null && s.hits[i]) scale *= 1.03;
      s.scaleGoal[i] = s.shown[i] ? scale : 0;
      s.glossGoal[i] = s.selected === i ? 1 : 0;
    }
    const names = nodes.map(() => []);
    if (s.selected !== null) {
      for (const l of links) {
        const other = l.a === s.selected && l.b !== s.selected ? l.b : l.b === s.selected && l.a !== s.selected ? l.a : null;
        if (other !== null && !names[other].includes(l.label)) names[other].push(l.label);
      }
    }
    s.eyebrow = names.map(v => v.join(' · '));
    links.forEach((l, i) => {
      const incident = focus !== null && (l.a === focus || l.b === focus);
      let stroke = { rgb: srgb(EDGE), alpha: focus !== null ? .3 : .62, width: 1.25 };
      if (incident) stroke = { rgb: srgb(EDGE_HIGHLIGHT), alpha: .95, width: 2 };
      if (!s.shown[l.a] || !s.shown[l.b]) stroke.alpha = 0;
      s.strokeGoal[i] = stroke;
    });
  }
  function refreshHits() { const q = normalizeText(s.query); s.hits = nodes.map(v => q !== '' && v.keys.some(k => k.includes(q))); }
  function focusOn(animate) {
    if (s.selected === null) return;
    const camBasis = basis(s.rig.now);
    s.from = s.now.map(p => [...p]);
    s.focusRadius = focusLayout(base, new Array(n).fill(NODE_RADIUS), adjacency, s.shown, s.selected, camBasis, s.goal);
    s.focus = s.selected;
    beginTween(animate);
    frameFocus(animate);
  }
  function overview(animate) { s.from = s.now.map(p => [...p]); s.goal = base.map(p => [...p]); s.focus = null; beginTween(animate); frameAll(animate); }

  function step(dt) {
    s.rig.update(dt, reduced);
    const steps = Math.min(3, Math.max(.4, dt * 60));
    const rate = perFrame => (reduced ? 1 : Math.min(1, perFrame * steps));
    let ease = null;
    if (s.tween !== null) {
      const elapsed = Math.min(LAYOUT_DURATION, s.tween + dt), p = reduced ? 1 : elapsed / LAYOUT_DURATION;
      ease = easeInOutCubic(p); s.tween = p < 1 ? elapsed : null;
    }
    for (let i = 0; i < n; i++) {
      if (ease !== null) s.now[i] = lerp3(s.from[i], s.goal[i], ease);
      else {
        const diff = sub(s.goal[i], s.now[i]);
        s.now[i] = reduced || len(diff) <= .002 * UNIT ? [...s.goal[i]] : add(s.now[i], mul(diff, rate(.2)));
      }
      s.scale[i] = approach(s.scale[i], s.scaleGoal[i], rate(.18), .0008 * UNIT);
      s.gloss[i] = approach(s.gloss[i], s.glossGoal[i], rate(.2), .002);
      s.dim[i] = approach(s.dim[i], s.dimGoal[i], rate(.2), .002);
    }
    links.forEach((l, i) => {
      const goal = s.strokeGoal[i], st = s.stroke[i];
      if (st.alpha < .01) { st.rgb = [...goal.rgb]; st.width = goal.width; }
      st.alpha = approach(st.alpha, goal.alpha, rate(.2), .004);
      st.width = approach(st.width, goal.width, rate(.2), .01);
      st.rgb = st.rgb.map((c, k) => approach(c, goal.rgb[k], rate(.2), .002));
    });
    [s.selected, s.hovered !== s.selected ? s.hovered : null].forEach((target, slot) => {
      const halo = s.halo[slot];
      if (target !== null && halo[0] !== target) { halo[0] = target; halo[1] = 0; }
      halo[1] = approach(halo[1], target !== null ? 1 : 0, rate(.22), .004);
    });
  }

  /* ----- 이름표(place_labels) ----- */
  const fontOf = (size, weight) => `${weight} ${size}px ${font}`;
  function fitted(text, size, weight) {
    const key = `${text}|${size}|${weight}`;
    if (s.widths.has(key)) return s.widths.get(key);
    ctx.font = fontOf(size, weight);
    let shown = text, width = ctx.measureText(shown).width;
    if (width > LABEL_MAX_TEXT) {
      const chars = [...text];
      let keep = chars.length;
      while (keep > 1 && width > LABEL_MAX_TEXT) { keep -= 1; shown = chars.slice(0, keep).join('').trimEnd() + '…'; width = ctx.measureText(shown).width; }
    }
    const out = [shown, width];
    s.widths.set(key, out);
    return out;
  }
  const position = i => s.now[i];
  function labelCandidate(i, focus) {
    if (focus === i) return [100, s.selected !== null ? 'selected' : 'hover', true];
    if (focus !== null) {
      if (adjacency[focus].includes(i)) return s.eyebrow[i] ? [95, 'relation', true] : [90, 'default', true];
      return s.hovered === i ? [96, 'hover', true] : [-1, 'default', false];
    }
    if (s.hits[i]) return [80, 'search', false];
    return [Math.min(12, nodes[i].degree), 'default', false];
  }
  function labelRect(i, tone, eyebrow) {
    const anchor = project(s.rig.now, add(position(i), v3(0, s.scale[i] * 1.45, 0)), W, H);
    if (!anchor) return null;
    const bold = tone === 'selected' || tone === 'relation';
    const [title, tw] = fitted(nodes[i].name, LABEL_FONT, bold ? 700 : 600);
    const [eb, ew] = eyebrow ? fitted(eyebrow, EYEBROW_FONT, 700) : ['', 0];
    const width = Math.ceil(Math.max(tw, ew) + LABEL_PAD_X * 2), height = LABEL_HEIGHT + (eyebrow ? EYEBROW_HEIGHT : 0);
    return { x: Math.round(anchor.x - width / 2), y: Math.round(anchor.y), w: width, h: height, title, eyebrow: eb };
  }
  const overlap = (a, b) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
  function placeLabels(dt, snap) {
    const focus = s.selected ?? s.hovered, eye = eyeOf(s.rig.now);
    const order = [];
    for (let i = 0; i < n; i++) {
      const c = labelCandidate(i, focus);
      if (c[0] >= 0 && s.shown[i] && s.scale[i] > 1 && nearFade(len(sub(position(i), eye))) >= .5) order.push([i, c]);
    }
    order.sort((a, b) => b[1][0] - a[1][0] || nodes[b[0]].degree - nodes[a[0]].degree || byteOrder(nodes[a[0]].name, nodes[b[0]].name));
    const budget = Math.max(order.filter(([, c]) => c[2]).length, 13);
    const safe = { x: 0, y: insets.bottom, w: W, h: H - insets.top - insets.bottom };
    const placedRects = [], goal = new Array(n).fill(false), rects = new Array(n).fill(null);
    for (const [i, [, tone, must]] of order) {
      if (placedRects.length >= budget) break;
      const eyebrow = tone === 'relation' ? s.eyebrow[i] : '';
      const r = labelRect(i, tone, eyebrow);
      if (!r) continue;
      const cx = r.x + r.w / 2, cy = r.y;
      if (cx < safe.x || cx > safe.x + safe.w || cy < safe.y || cy > safe.y + safe.h) continue;
      const padded = { x: r.x - 4, y: r.y - 3, w: r.w + 8, h: r.h + 6 };
      if (!must && placedRects.some(o => overlap(padded, o))) continue;
      placedRects.push(padded); goal[i] = true; s.labelStyle[i] = [tone, eyebrow]; rects[i] = r;
    }
    const rate = snap ? 1 : 1 - Math.exp(-dt / .045);
    const labels = [];
    for (let i = 0; i < n; i++) {
      s.labelAlpha[i] = approach(s.labelAlpha[i], goal[i] ? 1 : 0, rate, .01);
      if (s.labelAlpha[i] <= .01) continue;
      const [tone, eyebrow] = s.labelStyle[i];
      const r = rects[i] || labelRect(i, tone, eyebrow);
      if (r) labels.push({ ...r, tone, alpha: s.labelAlpha[i] });
    }
    return labels;
  }

  /* ----- 그리기 ----- */
  function silhouette(i) {
    const p = project(s.rig.now, position(i), W, H);
    if (!p) return null;
    const d = len(sub(position(i), eyeOf(s.rig.now))), r = s.scale[i];
    if (d <= r * 1.02) return null;
    return [p.x, p.y, r * p.scale * d / Math.sqrt(d * d - r * r)];
  }
  function clipToDisc(a, b, disc) {
    // b가 원 안에 있으면 a→b가 원과 만나는 점으로 자른다. a도 안이면 전부 가려진다.
    const inside = p => Math.hypot(p[0] - disc[0], p[1] - disc[1]) < disc[2];
    if (!inside(b)) return b;
    if (inside(a)) return null;
    const dx = b[0] - a[0], dy = b[1] - a[1], fx = a[0] - disc[0], fy = a[1] - disc[1];
    const qa = dx * dx + dy * dy, qb = 2 * (fx * dx + fy * dy), qc = fx * fx + fy * fy - disc[2] * disc[2];
    const t = Math.min(1, Math.max(0, (-qb - Math.sqrt(Math.max(0, qb * qb - 4 * qa * qc))) / (2 * qa)));
    return [a[0] + dx * t, a[1] + dy * t, a[2] + (b[2] - a[2]) * t];
  }
  function linkSegment(l, i) {
    const st = s.stroke[i];
    if (st.alpha < .01) return null;
    const p = position(l.a), q = position(l.b), rp = s.scale[l.a], rq = s.scale[l.b];
    if (rp < .5 || rq < .5) return null;
    const start = rp * .94, end = l.directed ? rq * 1.02 : rq * .94;
    const span = len(sub(q, p)), dir = norm(sub(q, p));
    if (span <= start + end + 1) return null;
    const cam = s.rig.now, eye = eyeOf(cam), toward = basis(cam)[2];
    const a3 = add(p, mul(dir, start)), b3 = sub(q, mul(dir, end));
    if (-dot(sub(a3, eye), toward) < NEAR_CLIP || -dot(sub(b3, eye), toward) < NEAR_CLIP) return null;
    const pa = project(cam, a3, W, H), pb = project(cam, b3, W, H);
    if (!pa || !pb) return null;
    let A = [pa.x, pa.y, pa.depth], B = [pb.x, pb.y, pb.depth];
    const da = silhouette(l.a), db = silhouette(l.b);
    if (da) { A = clipToDisc(B, A, da); if (!A) return null; }
    if (db) { B = clipToDisc(A, B, db); if (!B) return null; }
    return { A, B, st, link: l, depth: (A[2] + B[2]) / 2 };
  }
  const Y = y => H - y;
  function drawLink({ A, B, st, link }) {
    const alpha = st.alpha * nearFade(B[2]);
    ctx.strokeStyle = rgba(st.rgb, alpha); ctx.fillStyle = rgba(st.rgb, alpha);
    ctx.lineWidth = st.width;
    let [bx, by] = [B[0], B[1]];
    const total = Math.hypot(bx - A[0], by - A[1]);
    let arrow = null;
    if (link.directed) {
      const length = Math.min(Math.min(11, Math.max(7, st.width * 4.2)), total * .6);
      if (length >= 3) {
        const ux = (bx - A[0]) / total, uy = (by - A[1]) / total;
        arrow = { tip: [bx, by], u: [ux, uy], length };
        bx -= ux * (length - .5); by -= uy * (length - .5);
      }
    }
    ctx.setLineDash(link.inferred ? [7, 5] : []);
    ctx.beginPath(); ctx.moveTo(A[0], Y(A[1])); ctx.lineTo(bx, Y(by)); ctx.stroke();
    ctx.setLineDash([]);
    if (arrow) {
      const { tip, u, length } = arrow, back = [tip[0] - u[0] * length, tip[1] - u[1] * length], half = length * .42;
      ctx.beginPath();
      ctx.moveTo(tip[0], Y(tip[1]));
      ctx.lineTo(back[0] - u[1] * half, Y(back[1] + u[0] * half));
      ctx.lineTo(back[0] + u[1] * half, Y(back[1] - u[0] * half));
      ctx.closePath(); ctx.fill();
    }
  }
  function drawSphere(i, p, eye) {
    const r = s.scale[i] * p.scale * len(sub(position(i), eye)) / Math.sqrt(Math.max(1e-6, len(sub(position(i), eye)) ** 2 - s.scale[i] ** 2));
    const fade = nearFade(len(sub(position(i), eye)));
    const x = p.x, y = Y(p.y), size = r * 2 * SPRITE / (SPRITE - 2);
    ctx.globalAlpha = fade;
    ctx.drawImage(sphereSprite(nodes[i].albedo, false), x - size / 2, y - size / 2, size, size);
    if (s.dim[i] > .002) { ctx.globalAlpha = fade * s.dim[i]; ctx.drawImage(sphereSprite(dimLinear, false), x - size / 2, y - size / 2, size, size); }
    if (s.gloss[i] > .002) { ctx.globalAlpha = fade * s.gloss[i]; ctx.drawImage(sphereSprite(nodes[i].albedo, true), x - size / 2, y - size / 2, size, size); }
    ctx.globalAlpha = 1;
  }
  function drawHalo(slot, eye) {
    const [node, strength] = s.halo[slot];
    if (node === null || strength <= .004 || s.scale[node] < .05) return;
    const p = project(s.rig.now, position(node), W, H);
    if (!p) return;
    const [factor, opacity] = slot === 0 ? [2.3, .72] : [1.9, .42];
    const size = s.scale[node] * factor * 2 * p.scale;
    ctx.globalAlpha = opacity * strength * nearFade(len(sub(position(node), eye)));
    ctx.drawImage(haloSprite(), p.x - size / 2, Y(p.y) - size / 2, size, size);
    ctx.globalAlpha = 1;
  }
  /** 둥근 사각형 경로. roundRect가 없는 브라우저도 같은 모양이 되도록 직접 그린다 */
  function rounded(x, y, w, h, r) { ctx.beginPath(); ctx.moveTo(x + r, y); ctx.arcTo(x + w, y, x + w, y + h, r); ctx.arcTo(x + w, y + h, x, y + h, r); ctx.arcTo(x, y + h, x, y, r); ctx.arcTo(x, y, x + w, y, r); ctx.closePath(); }
  function pill(x, y, w, h, color, alpha) { ctx.fillStyle = rgba(color, alpha); rounded(x, y, w, h, 8); ctx.fill(); }
  function drawLabel(l) {
    const a = l.alpha, brand = srgb(ACTIVE), ink = srgb(INK), top = Y(l.y + l.h), mid = l.x + l.w / 2, titleY = Y(l.y) - 12;
    ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    if (l.tone === 'selected') {
      pill(l.x, top, l.w, l.h, brand, a);
      ctx.font = fontOf(LABEL_FONT, 700); ctx.fillStyle = rgba([1, 1, 1], a); ctx.fillText(l.title, mid, titleY);
    } else if (l.tone === 'relation') {
      pill(l.x, top, l.w, l.h, [1, 1, 1], .92 * a); pill(l.x, top, l.w, l.h, brand, .11 * a);
      if (l.eyebrow) { ctx.font = fontOf(EYEBROW_FONT, 700); ctx.fillStyle = rgba(brand, .82 * a); ctx.fillText(l.eyebrow, mid, top + 8.5); }
      ctx.font = fontOf(LABEL_FONT, 700); ctx.fillStyle = rgba(brand, a); ctx.fillText(l.title, mid, titleY);
    } else {
      pill(l.x, top, l.w, l.h, [1, 1, 1], .86 * a);
      if (l.tone === 'hover' || l.tone === 'search') {
        ctx.strokeStyle = l.tone === 'hover' ? rgba([.122, .133, .239], .16 * a) : rgba(brand, .48 * a);
        ctx.lineWidth = 1; rounded(l.x + .5, top + .5, l.w - 1, l.h - 1, 7.5); ctx.stroke();
      }
      ctx.font = fontOf(LABEL_FONT, 600); ctx.fillStyle = rgba(ink, a); ctx.fillText(l.title, mid, titleY);
    }
  }
  function draw(dt = 1 / 60, snap = false) {
    ctx.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
    ctx.fillStyle = MAP_BACKGROUND; ctx.fillRect(0, 0, W, H);
    const cam = s.rig.now, eye = eyeOf(cam), items = [];
    links.forEach((l, i) => { const seg = linkSegment(l, i); if (seg) items.push({ depth: seg.depth, draw: () => drawLink(seg) }); });
    for (let i = 0; i < n; i++) {
      if (s.scale[i] < .05) continue;
      const p = project(cam, position(i), W, H);
      if (!p || nearFade(len(sub(position(i), eye))) <= 0) continue;
      items.push({ depth: p.depth, draw: () => { s.halo.forEach(([node], slot) => { if (node === i) drawHalo(slot, eye); }); drawSphere(i, p, eye); } });
    }
    items.sort((a, b) => b.depth - a.depth);
    ctx.lineCap = 'butt';
    for (const it of items) it.draw();
    lastLabels = placeLabels(dt, snap);
    for (const l of lastLabels) drawLabel(l);
  }
  let lastLabels = [];

  /* ----- 공개 동작(graph.rs Public API) ----- */
  const api = {
    /** 처음 지도가 열릴 때 개념이 가운데서 펼쳐진다 */
    enter() { if (reduced) return; for (let i = 0; i < n; i++) { s.now[i] = mul(s.goal[i], .62); s.scale[i] = s.scaleGoal[i] * .25; } s.from = s.now.map(p => [...p]); s.tween = 0; s.labelAlpha.fill(0); },
    select(i) { if (s.selected === i) return; s.selected = i; s.hovered = null; applyHighlight(); focusOn(true); },
    /** 선택을 풀고 전체 보기로 돌아간다(카메라도 기본 각도로) */
    clear() { s.selected = null; s.hovered = null; applyHighlight(); overview(true); },
    hover(i) { if (s.hovered === i) return; s.hovered = i; applyHighlight(); },
    search(q) { s.query = q; refreshHits(); applyHighlight(); },
    /** 빈 곳 드래그. dx·dy는 화면 픽셀(오른쪽·아래가 +) */
    orbit(dx, dy) { s.rig.orbit(dx, dy); },
    /** 개념의 화면 위치와 반지름(캔버스 왼쪽 위 원점) */
    screenOf(i) { const p = project(s.rig.now, position(i), W, H); return p ? { x: p.x, y: Y(p.y), r: s.scale[i] * p.scale } : null; },
    /** 마지막으로 그린 이름표(캔버스 왼쪽 위 원점). 장면 구도 검사에 쓴다 */
    get labels() { return lastLabels.map(l => ({ x: l.x, y: H - l.y - l.h, w: l.w, h: l.h, title: l.title, tone: l.tone, alpha: l.alpha })); },
    neighbours(i) { return adjacency[i]; },
    tick(dt) { step(Math.min(.064, Math.max(0, dt))); draw(dt, false); },
    settle() { for (let k = 0; k < 60; k++) step(1 / 30); s.rig.snap(); s.now = s.goal.map(p => [...p]); s.tween = null; draw(1, true); },
  };
  applyHighlight();
  s.stroke = s.strokeGoal.map(st => ({ ...st, rgb: [...st.rgb] }));
  updateLimits();
  frameAll(false);
  return api;
}
