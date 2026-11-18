//! Deterministic 3D graph layout and camera math. No UI or render loop lives here.
use std::collections::{HashMap, HashSet};
use std::ops::{Add, Div, Mul, Neg, Sub};

pub const FOV_Y: f64 = 42.0 * std::f64::consts::PI / 180.0;
pub const NEAR: f64 = 0.5;
pub const FAR: f64 = 10_000.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
impl Vec3 {
    pub const ZERO: Self = Self::new(0., 0., 0.);
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    pub fn dot(self, rhs: Self) -> f64 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }
    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }
    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }
    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }
    pub fn normalized(self) -> Self {
        let n = self.length();
        if n > 1e-12 && n.is_finite() {
            self / n
        } else {
            Self::ZERO
        }
    }
    pub fn add(self, rhs: Self) -> Self {
        self + rhs
    }
    pub fn sub(self, rhs: Self) -> Self {
        self - rhs
    }
    pub fn mul(self, rhs: f64) -> Self {
        self * rhs
    }
    pub fn lerp(self, rhs: Self, t: f64) -> Self {
        self + (rhs - self) * t
    }
    pub fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}
impl Add for Vec3 {
    type Output = Self;
    fn add(self, r: Self) -> Self {
        Self::new(self.x + r.x, self.y + r.y, self.z + r.z)
    }
}
impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, r: Self) -> Self {
        Self::new(self.x - r.x, self.y - r.y, self.z - r.z)
    }
}
impl Mul<f64> for Vec3 {
    type Output = Self;
    fn mul(self, r: f64) -> Self {
        Self::new(self.x * r, self.y * r, self.z * r)
    }
}
impl Mul<Vec3> for f64 {
    type Output = Vec3;
    fn mul(self, r: Vec3) -> Vec3 {
        r * self
    }
}
impl Div<f64> for Vec3 {
    type Output = Self;
    fn div(self, r: f64) -> Self {
        Self::new(self.x / r, self.y / r, self.z / r)
    }
}
impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        self * -1.
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Projected {
    /// Logical view pixels, with bottom-left origin like AppKit's default NSView.
    pub x: f64,
    pub y: f64,
    /// Positive camera-space distance, used for label ordering and visibility.
    pub depth: f64,
    /// Pixels per world unit at this point, including perspective foreshortening.
    pub radius_scale: f64,
    /// Homogeneous Metal coordinates: depth is in [0,w], never OpenGL [-w,w].
    pub clip: [f32; 4],
}
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub yaw: f64,
    pub pitch: f64,
    pub distance: f64,
    pub pivot: Vec3,
    /// Projection-centre shift in logical pixels (+x right, +y up). It moves the
    /// pivot to the centre of the area that controls leave free.
    pub shift_x: f64,
    pub shift_y: f64,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.62,
            pitch: 0.30,
            distance: 800.,
            pivot: Vec3::ZERO,
            shift_x: 0.,
            shift_y: 0.,
        }
    }
}
impl Camera {
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let right = Vec3::new(self.yaw.cos(), 0., -self.yaw.sin());
        let toward = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        );
        let up = toward.cross(right).normalized();
        (right, up, toward)
    }
    pub fn position(&self) -> Vec3 {
        self.pivot + self.basis().2 * self.distance
    }
    pub fn eye(&self) -> Vec3 {
        self.position()
    }
    pub fn project(&self, world: Vec3, width: f64, height: f64) -> Option<Projected> {
        if !world.finite()
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.
            || height <= 0.
        {
            return None;
        }
        let (right, up, toward) = self.basis();
        let relative = world - self.position();
        let depth = -relative.dot(toward);
        if !depth.is_finite() || depth < NEAR || depth > FAR {
            return None;
        }
        let f = 1. / (FOV_Y * 0.5).tan();
        let aspect = width / height;
        let clip = [
            (relative.dot(right) * f / aspect + 2. * self.shift_x / width * depth) as f32,
            (relative.dot(up) * f + 2. * self.shift_y / height * depth) as f32,
            (FAR / (FAR - NEAR) * depth - NEAR * FAR / (FAR - NEAR)) as f32,
            depth as f32,
        ];
        let radius_scale = height * 0.5 * f / depth;
        Some(Projected {
            x: width * 0.5 + self.shift_x + relative.dot(right) * radius_scale,
            y: height * 0.5 + self.shift_y + relative.dot(up) * radius_scale,
            depth,
            radius_scale,
            clip,
        })
    }
    pub fn ray(&self, x: f64, y: f64, width: f64, height: f64) -> (Vec3, Vec3) {
        let (right, up, toward) = self.basis();
        let h = height.max(1.);
        let w = width.max(1.);
        let tan = (FOV_Y * 0.5).tan();
        let nx = ((x - self.shift_x) / w * 2. - 1.) * (w / h) * tan;
        let ny = ((y - self.shift_y) / h * 2. - 1.) * tan;
        (
            self.position(),
            (right * nx + up * ny - toward).normalized(),
        )
    }
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        // Yaw stays unwrapped so a damped follower never takes the long way round.
        self.yaw -= dx * 0.0042;
        self.pitch = (self.pitch + dy * 0.0036).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }
    /// Factors above one zoom in. Limits follow the graph size, like the web map,
    /// so a node can never fill the viewport.
    pub fn zoom(&mut self, factor: f64, min: f64, max: f64) {
        if factor.is_finite() && factor > 0. {
            self.distance = (self.distance / factor).clamp(min, max.max(min));
        }
    }
    /// Frame all supplied spheres, honoring the narrower vertical/horizontal FOV.
    /// A bounding-sphere fit cannot crop back/front nodes when the camera orbits.
    pub fn fit(&mut self, points: &[(Vec3, f64)], width: f64, height: f64) {
        let shift = (self.shift_x, self.shift_y);
        let valid: Vec<_> = points
            .iter()
            .filter(|(p, r)| p.finite() && r.is_finite())
            .collect();
        if valid.is_empty() {
            *self = Self::default();
            (self.shift_x, self.shift_y) = shift;
            return;
        }
        self.pivot = valid.iter().fold(Vec3::ZERO, |sum, (p, _)| sum + *p) / valid.len() as f64;
        let radius = valid
            .iter()
            .map(|(p, r)| (*p - self.pivot).length() + r.max(0.))
            .fold(20., f64::max);
        self.distance = fit_distance(radius, width, height);
        self.yaw = 0.62;
        self.pitch = 0.30;
    }
    /// Move the pivot on a camera-parallel plane, preserving perceived drag scale.
    pub fn screen_delta(&self, dx: f64, dy: f64, depth: f64, height: f64) -> Vec3 {
        let (right, up, _) = self.basis();
        let units = 2. * (FOV_Y * 0.5).tan() * depth / height.max(1.);
        (right * dx + up * dy) * units
    }
}

pub const PITCH_LIMIT: f64 = std::f64::consts::PI / 3.;

/// Tangent of the narrower half field of view of a viewport.
pub fn limiting_tan(width: f64, height: f64) -> f64 {
    let tan_v = (FOV_Y * 0.5).tan();
    tan_v.min(tan_v * width.max(1.) / height.max(1.))
}
/// Camera distance that frames a bounding sphere. sin() frames the sphere itself,
/// rather than only the flat plane through its centre.
pub fn fit_distance(radius: f64, width: f64, height: f64) -> f64 {
    radius.max(1.) / limiting_tan(width, height).atan().sin() * 1.18
}

const SETTLE_EPSILON: f64 = 0.0004;
pub const CAMERA_TRANSITION: f64 = 0.72;
pub fn ease_in_out_cubic(progress: f64) -> f64 {
    if progress < 0.5 {
        4. * progress.powi(3)
    } else {
        1. - (-2. * progress + 2.).powi(3) / 2.
    }
}
/// Camera follower ported from the web map: select/overview moves are a bounded
/// 720 ms ease, direct manipulation is a short exponential damping.
#[derive(Clone, Copy, Debug)]
pub struct Rig {
    pub now: Camera,
    pub goal: Camera,
    from: Camera,
    bounded: bool,
    elapsed: f64,
    pub min_distance: f64,
    pub max_distance: f64,
}
impl Default for Rig {
    fn default() -> Self {
        let camera = Camera::default();
        Self {
            now: camera,
            goal: camera,
            from: camera,
            bounded: false,
            elapsed: CAMERA_TRANSITION,
            min_distance: 40.,
            max_distance: 8000.,
        }
    }
}
impl Rig {
    pub fn set_limits(&mut self, min: f64, max: f64) {
        self.min_distance = min;
        self.max_distance = max.max(min);
        self.goal.distance = self
            .goal
            .distance
            .clamp(self.min_distance, self.max_distance);
    }
    pub fn set_shift(&mut self, x: f64, y: f64) {
        for camera in [&mut self.now, &mut self.goal, &mut self.from] {
            camera.shift_x = x;
            camera.shift_y = y;
        }
    }
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        self.bounded = false;
        self.goal.orbit(dx, dy);
    }
    pub fn zoom(&mut self, factor: f64) {
        self.bounded = false;
        self.goal.zoom(factor, self.min_distance, self.max_distance);
    }
    pub fn rotate(&mut self, radians: f64) {
        self.bounded = false;
        self.goal.yaw += radians;
    }
    /// Start a bounded move to `goal`; `animate=false` arrives immediately.
    pub fn travel(&mut self, mut goal: Camera, animate: bool) {
        goal.shift_x = self.now.shift_x;
        goal.shift_y = self.now.shift_y;
        goal.distance = goal.distance.clamp(self.min_distance, self.max_distance);
        // Take the short way round for yaw.
        let tau = std::f64::consts::TAU;
        self.now.yaw = goal.yaw + (self.now.yaw - goal.yaw + tau / 2.).rem_euclid(tau) - tau / 2.;
        self.goal = goal;
        self.from = self.now;
        self.elapsed = 0.;
        self.bounded = animate;
        if !animate {
            self.snap();
        }
    }
    pub fn snap(&mut self) {
        self.now = self.goal;
        self.bounded = false;
        self.elapsed = CAMERA_TRANSITION;
    }
    /// Returns true while the camera is still moving.
    pub fn update(&mut self, delta: f64, reduced_motion: bool) -> bool {
        if reduced_motion {
            self.snap();
            return false;
        }
        let mut moving = false;
        if self.bounded {
            self.elapsed = (self.elapsed + delta).min(CAMERA_TRANSITION);
            let progress = self.elapsed / CAMERA_TRANSITION;
            let k = ease_in_out_cubic(progress);
            self.now.yaw = self.from.yaw + (self.goal.yaw - self.from.yaw) * k;
            self.now.pitch = self.from.pitch + (self.goal.pitch - self.from.pitch) * k;
            self.now.distance = self.from.distance + (self.goal.distance - self.from.distance) * k;
            self.now.pivot = self.from.pivot.lerp(self.goal.pivot, k);
            self.bounded = progress < 1.;
            moving = self.bounded;
        } else {
            let damping = 1. - 0.0016f64.powf(delta.max(0.));
            self.now.yaw += (self.goal.yaw - self.now.yaw) * damping;
            self.now.pitch += (self.goal.pitch - self.now.pitch) * damping;
            self.now.distance += (self.goal.distance - self.now.distance) * damping;
            self.now.pivot = self.now.pivot.lerp(self.goal.pivot, damping);
        }
        let settled = (self.goal.yaw - self.now.yaw).abs() <= SETTLE_EPSILON
            && (self.goal.pitch - self.now.pitch).abs() <= SETTLE_EPSILON
            && (self.goal.distance - self.now.distance).abs() <= SETTLE_EPSILON * 100.
            && (self.goal.pivot - self.now.pivot).length_squared() <= SETTLE_EPSILON;
        if !moving && settled {
            self.now = self.goal;
        }
        moving || !settled
    }
}

/// Nearest positive ray/sphere intersection. A ray beginning inside a sphere
/// returns the exit surface; a sphere behind the ray is never selectable.
pub fn ray_sphere(origin: Vec3, direction: Vec3, center: Vec3, radius: f64) -> Option<f64> {
    if radius <= 0. || !radius.is_finite() {
        return None;
    }
    let oc = origin - center;
    let a = direction.dot(direction);
    if a < 1e-16 {
        return None;
    }
    let half_b = oc.dot(direction);
    let c = oc.dot(oc) - radius * radius;
    let discriminant = half_b * half_b - a * c;
    if discriminant < 0. || !discriminant.is_finite() {
        return None;
    }
    let root = discriminant.sqrt();
    let near = (-half_b - root) / a;
    let far = (-half_b + root) / a;
    if near >= 0. {
        Some(near)
    } else if far >= 0. {
        Some(far)
    } else {
        None
    }
}

fn hash_id(id: &str) -> u64 {
    id.bytes().fold(0xcbf29ce484222325u64, |v, b| {
        (v ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// Compute a stable 3D volume once; interaction never runs a perpetual physics
/// simulation. Sorting IDs/links makes the result independent of DB row order.
/// Local repulsion uses a spatial grid, while attraction traverses actual edges.
pub fn layout(ids: &[String], edges: &[(String, String)]) -> HashMap<String, Vec3> {
    let mut ids = ids.to_vec();
    ids.sort();
    ids.dedup();
    let n = ids.len();
    if n == 0 {
        return HashMap::new();
    }
    if n == 1 {
        return HashMap::from([(ids[0].clone(), Vec3::ZERO)]);
    }
    let index: HashMap<_, _> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mut links = HashSet::new();
    for (a, b) in edges {
        if let (Some(&a), Some(&b)) = (index.get(a.as_str()), index.get(b.as_str())) {
            if a != b {
                links.insert((a.min(b), a.max(b)));
            }
        }
    }
    let mut links: Vec<_> = links.into_iter().collect();
    links.sort_unstable();
    let radius = (n as f64).cbrt() * 84.;
    let golden = std::f64::consts::PI * (3. - 5f64.sqrt());
    let mut positions: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let y = 1. - 2. * (i as f64 + 0.5) / n as f64;
            let side = (1. - y * y).sqrt();
            let angle = i as f64 * golden + 0.37;
            let radial = 0.64 + 0.36 * ((hash_id(id) >> 32) as u32 as f64 / u32::MAX as f64).cbrt();
            Vec3::new(angle.cos() * side, y * 0.85, angle.sin() * side) * radius * radial
        })
        .collect();
    let initial = positions.clone();
    let cell = 100.;
    let key = |p: Vec3| {
        (
            (p.x / cell).floor() as i32,
            (p.y / cell).floor() as i32,
            (p.z / cell).floor() as i32,
        )
    };
    for _ in 0..64 {
        let mut grid = HashMap::<(i32, i32, i32), Vec<usize>>::new();
        for (i, p) in positions.iter().enumerate() {
            grid.entry(key(*p)).or_default().push(i);
        }
        let mut deltas = vec![Vec3::ZERO; n];
        for i in 0..n {
            let (x, y, z) = key(positions[i]);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(bucket) = grid.get(&(x + dx, y + dy, z + dz)) {
                            for &j in bucket {
                                if j <= i {
                                    continue;
                                }
                                let d = positions[j] - positions[i];
                                let length = d.length();
                                if length < 94. && length > 1e-8 {
                                    let force = d / length * (94. - length) * 0.22;
                                    deltas[i] = deltas[i] - force;
                                    deltas[j] = deltas[j] + force;
                                }
                            }
                        }
                    }
                }
            }
        }
        for &(a, b) in &links {
            let d = positions[b] - positions[a];
            let length = d.length();
            if length > 1e-8 {
                let force = d / length * (length - 138.) * 0.018;
                deltas[a] = deltas[a] + force;
                deltas[b] = deltas[b] - force;
            }
        }
        for i in 0..n {
            let force = deltas[i] + (initial[i] - positions[i]) * 0.024;
            let step = if force.length() > 7. {
                force.normalized() * 7.
            } else {
                force
            };
            positions[i] = positions[i] + step;
        }
    }
    let center = positions.iter().fold(Vec3::ZERO, |a, b| a + *b) / n as f64;
    ids.into_iter()
        .zip(positions.into_iter().map(|p| p - center))
        .collect()
}

fn fibonacci_direction(index: usize, total: usize) -> Vec3 {
    let golden = std::f64::consts::PI * (3. - 5f64.sqrt());
    let y = if total <= 1 {
        0.
    } else {
        1. - index as f64 / (total - 1) as f64 * 2.
    };
    let radius = (1. - y * y).max(0.).sqrt();
    let theta = golden * index as f64;
    Vec3::new(theta.cos() * radius, y, theta.sin() * radius)
}

pub struct FocusPlan {
    /// Radius the camera must frame: selected node plus its 1-hop shell.
    pub radius: f64,
    /// Number of 1-hop neighbours placed on the shell.
    #[allow(dead_code)]
    pub neighbors: usize,
}
/// Deterministic 3D ego layout around `selected`, ported from the web map.
/// 1-hop neighbours form a shallow camera-facing shell, 2-hop nodes a middle
/// shell and everything else an outer one; context nodes are then relaxed so
/// they never overlap. `unit` is one web world unit in local units.
pub fn focus_layout(
    base: &[Vec3],
    radii: &[f64],
    adjacency: &[Vec<usize>],
    visible: &[bool],
    selected: usize,
    basis: (Vec3, Vec3, Vec3),
    unit: f64,
    out: &mut [Vec3],
) -> FocusPlan {
    let count = base.len();
    let (right, up, toward) = basis;
    let mut tiers = vec![3u8; count];
    tiers[selected] = 0;
    let direct: Vec<usize> = adjacency[selected]
        .iter()
        .copied()
        .filter(|i| *i != selected && visible[*i])
        .collect();
    for &i in &direct {
        tiers[i] = 1;
    }
    for &i in &direct {
        for &j in &adjacency[i] {
            if visible[j] && tiers[j] == 3 {
                tiers[j] = 2;
            }
        }
    }
    let n = direct.len() as f64;
    let direct_shell = (8.5 + n * 0.58).clamp(10.5, 19.) * unit;
    let middle_shell = direct_shell + (8. + n * 0.18).min(13.) * unit;
    let outer_shell = middle_shell + 13. * unit;
    let origin = base[selected];
    for index in 0..count {
        if index == selected {
            out[index] = Vec3::ZERO;
            continue;
        }
        let mut direction = (base[index] - origin).normalized();
        if direction == Vec3::ZERO {
            direction = fibonacci_direction(index, count.max(2));
        }
        let shell = if tiers[index] == 2 {
            middle_shell
        } else {
            outer_shell
        };
        out[index] = direction * shell;
    }
    let mut largest = radii[selected];
    for (order, &index) in direct.iter().enumerate() {
        let angle = -std::f64::consts::FRAC_PI_2
            + order as f64 / direct.len().max(1) as f64 * std::f64::consts::TAU;
        let depth_ratio = ((order as f64 * 0.618_033_988_749_894_9).fract() - 0.5) * 0.42;
        let planar = direct_shell * (1. - depth_ratio * depth_ratio).max(0.72).sqrt();
        out[index] = right * (angle.cos() * planar)
            + up * (angle.sin() * planar)
            + toward * (direct_shell * depth_ratio);
        largest = largest.max(radii[index]);
    }
    relax_context(out, radii, visible, &tiers, unit);
    FocusPlan {
        radius: if direct.is_empty() {
            (6. * unit).max(radii[selected] * 4.)
        } else {
            direct_shell + largest + 2.8 * unit
        },
        neighbors: direct.len(),
    }
}
fn relax_context(positions: &mut [Vec3], radii: &[f64], visible: &[bool], tiers: &[u8], unit: f64) {
    let cell = 5. * unit;
    let key = |p: Vec3| {
        (
            (p.x / cell).floor() as i64,
            (p.y / cell).floor() as i64,
            (p.z / cell).floor() as i64,
        )
    };
    let count = positions.len();
    for _ in 0..3 {
        let mut grid = HashMap::<(i64, i64, i64), Vec<usize>>::new();
        for index in 0..count {
            if visible[index] {
                grid.entry(key(positions[index])).or_default().push(index);
            }
        }
        for index in 0..count {
            if !visible[index] {
                continue;
            }
            let (cx, cy, cz) = key(positions[index]);
            for ox in -1..=1 {
                for oy in -1..=1 {
                    for oz in -1..=1 {
                        let Some(bucket) = grid.get(&(cx + ox, cy + oy, cz + oz)) else {
                            continue;
                        };
                        for &other in bucket {
                            if other <= index {
                                continue;
                            }
                            let index_fixed = tiers[index] <= 1;
                            let other_fixed = tiers[other] <= 1;
                            if index_fixed && other_fixed {
                                continue;
                            }
                            let mut delta = positions[other] - positions[index];
                            let mut distance = delta.length();
                            let minimum = radii[index] + radii[other] + 1.15 * unit;
                            if distance >= minimum {
                                continue;
                            }
                            if distance < 1e-4 {
                                delta = fibonacci_direction(other, count.max(2));
                                distance = 1.;
                            }
                            let push = delta / distance * (minimum - distance);
                            let index_share = if index_fixed {
                                0.
                            } else if other_fixed {
                                1.
                            } else {
                                0.5
                            };
                            let other_share = if other_fixed {
                                0.
                            } else if index_fixed {
                                1.
                            } else {
                                0.5
                            };
                            positions[index] = positions[index] - push * index_share;
                            positions[other] = positions[other] + push * other_share;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-7, "{a} != {b}");
    }
    #[test]
    fn projection_is_perspective_and_metal_depth() {
        let c = Camera {
            yaw: 0.,
            pitch: 0.,
            distance: 800.,
            ..Camera::default()
        };
        let near = c.project(Vec3::new(0., 0., 200.), 800., 600.).unwrap();
        let far = c.project(Vec3::new(0., 0., -200.), 800., 600.).unwrap();
        assert!(near.radius_scale > far.radius_scale);
        close(near.x, 400.);
        close(near.y, 300.);
        assert!(near.clip[2] > 0. && near.clip[2] < near.clip[3]);
        assert!(far.clip[2] / far.clip[3] > near.clip[2] / near.clip[3]);
        assert!(c.project(Vec3::new(0., 0., 801.), 800., 600.).is_none());
    }
    #[test]
    fn ray_roundtrip_and_front_surface_order() {
        let c = Camera::default();
        let point = Vec3::new(42., -35., 60.);
        let p = c.project(point, 900., 650.).unwrap();
        let (o, d) = c.ray(p.x, p.y, 900., 650.);
        assert!((o + d * (point - o).dot(d) - point).length() < 1e-8);
        let front = o + d * 200.;
        let rear = o + d * 350.;
        assert!(ray_sphere(o, d, front, 20.).unwrap() < ray_sphere(o, d, rear, 20.).unwrap());
        assert!(ray_sphere(o, d, o - d * 20., 10.).is_none());
    }
    #[test]
    fn orbit_changes_two_axes_and_clamps_pitch() {
        let mut c = Camera::default();
        let before = c.position();
        c.orbit(100., 100.);
        assert!((c.position() - before).length() > 10.);
        assert!(c.yaw != 0.62 && c.pitch != 0.30);
        c.orbit(0., 1e10);
        close(c.pitch, std::f64::consts::PI / 3.);
        c.zoom(0., 100., 900.);
        assert!(c.distance.is_finite());
        c.zoom(1e9, 100., 900.);
        close(c.distance, 100.);
        c.zoom(1e-9, 100., 900.);
        close(c.distance, 900.);
    }
    #[test]
    fn safe_area_shift_moves_projection_and_ray_together() {
        let mut c = Camera::default();
        c.shift_x = 40.;
        c.shift_y = -25.;
        let pivot = c.project(c.pivot, 900., 650.).unwrap();
        close(pivot.x, 490.);
        close(pivot.y, 300.);
        close(pivot.clip[0] as f64 / pivot.clip[3] as f64, 80. / 900.);
        let point = Vec3::new(42., -35., 60.);
        let p = c.project(point, 900., 650.).unwrap();
        let (o, d) = c.ray(p.x, p.y, 900., 650.);
        assert!((o + d * (point - o).dot(d) - point).length() < 1e-8);
    }
    #[test]
    fn rig_damps_direct_input_and_bounds_transitions() {
        let mut rig = Rig::default();
        rig.set_limits(150., 900.);
        rig.zoom(1e9);
        close(rig.goal.distance, 150.);
        assert!(rig.now.distance > 700.);
        assert!(rig.update(1. / 60., false));
        assert!(rig.now.distance < 800. && rig.now.distance > 150.);
        for _ in 0..600 {
            rig.update(1. / 60., false);
        }
        close(rig.now.distance, 150.);
        assert!(!rig.update(1. / 60., false));
        // A bounded move takes 720 ms, and yaw goes the short way round.
        rig.now.yaw = 6.2;
        let goal = Camera::default();
        rig.travel(goal, true);
        assert!((rig.now.yaw - goal.yaw).abs() < std::f64::consts::PI);
        assert!(rig.update(0.36, false));
        close(rig.now.distance, (150. + goal.distance) / 2.);
        assert!(!rig.update(0.36, false));
        close(rig.now.yaw, goal.yaw);
        rig.zoom(2.);
        assert!(!rig.update(1. / 60., true));
        close(rig.now.distance, rig.goal.distance);
    }
    #[test]
    fn focus_layout_matches_web_shells_and_never_overlaps() {
        let count = 40;
        let ids = (0..count).map(|i| format!("n{i:02}")).collect::<Vec<_>>();
        let base_map = layout(&ids, &[]);
        let base = ids.iter().map(|id| base_map[id]).collect::<Vec<_>>();
        let radii = vec![15.; count];
        let mut adjacency = vec![Vec::new(); count];
        for i in 1..=6 {
            adjacency[0].push(i);
            adjacency[i].push(0);
        }
        adjacency[1].push(7);
        adjacency[7].push(1);
        let visible = vec![true; count];
        let camera = Camera::default();
        let mut out = vec![Vec3::ZERO; count];
        let plan = focus_layout(
            &base,
            &radii,
            &adjacency,
            &visible,
            0,
            camera.basis(),
            12.,
            &mut out,
        );
        assert_eq!(plan.neighbors, 6);
        let shell = (8.5f64 + 6. * 0.58).clamp(10.5, 19.) * 12.;
        close(plan.radius, shell + 15. + 2.8 * 12.);
        assert_eq!(out[0], Vec3::ZERO);
        for i in 1..=6 {
            assert!(
                (out[i].length() - shell).abs() < shell * 0.16,
                "{}",
                out[i].length()
            );
            // The shell is shallow: it faces the camera.
            assert!(out[i].dot(camera.basis().2).abs() <= shell * 0.2101);
        }
        // First neighbour sits straight below the selected node, as on the web.
        assert!(out[1].dot(camera.basis().1) < -shell * 0.9);
        assert!(out[7].length() > shell * 1.3 && out[8].length() > out[7].length() - 1.);
        for i in 0..count {
            for j in i + 1..count {
                if i <= 6 && j <= 6 {
                    continue;
                }
                assert!((out[i] - out[j]).length() > 30., "{i} {j}");
            }
        }
    }
    #[test]
    fn deterministic_layout_has_real_depth() {
        let ids = (0..8).map(|i| format!("node-{i}")).collect::<Vec<_>>();
        let edges = vec![
            (ids[0].clone(), ids[1].clone()),
            (ids[1].clone(), ids[3].clone()),
        ];
        let a = layout(&ids, &edges);
        let mut reversed = ids.clone();
        reversed.reverse();
        let b = layout(&reversed, &edges.iter().rev().cloned().collect::<Vec<_>>());
        assert_eq!(a, b);
        let points = ids.iter().map(|id| a[id]).collect::<Vec<_>>();
        let volume = (points[1] - points[0])
            .dot((points[2] - points[0]).cross(points[3] - points[0]))
            .abs();
        assert!(volume > 100.);
        assert!(points.iter().any(|p| p.z > 20.) && points.iter().any(|p| p.z < -20.));
    }
    #[test]
    fn fit_frames_all_spheres_at_narrow_and_wide_aspects() {
        let ids = (0..25).map(|i| i.to_string()).collect::<Vec<_>>();
        let positions = layout(&ids, &[]);
        for (width, height) in [(400., 900.), (1200., 500.)] {
            let mut c = Camera::default();
            let points = positions.values().map(|p| (*p, 20.)).collect::<Vec<_>>();
            c.fit(&points, width, height);
            for (p, r) in &points {
                let q = c.project(*p, width, height).unwrap();
                assert!(q.x - r * q.radius_scale > 0. && q.x + r * q.radius_scale < width);
                assert!(q.y - r * q.radius_scale > 0. && q.y + r * q.radius_scale < height);
            }
        }
    }
    #[test]
    fn screen_drag_preserves_camera_plane_depth() {
        let c = Camera::default();
        let d = c.screen_delta(30., -20., 300., 600.);
        close(d.dot(c.basis().2), 0.);
        let a = c.project(Vec3::ZERO, 800., 600.).unwrap();
        let b = c.project(d, 800., 600.).unwrap();
        close(b.x - a.x, 30. * 300. / c.distance);
        close(b.y - a.y, -20. * 300. / c.distance);
    }
}
