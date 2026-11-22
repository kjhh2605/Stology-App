//! Versioned, self-contained note diagrams. Coordinates are canvas points.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Write;

pub const LIMIT: usize = 1 << 20;
pub const WIDTH: f64 = 2400.;
pub const HEIGHT: f64 = 1800.;
const INKS: [&str; 4] = ["#1e1e2a", "#4F46E5", "#D6455D", "#1F8A5B"];
const FILLS: [&str; 4] = ["none", "#E7E5FF", "#FFF1C2", "#DDF3E4"];
pub type Point = [f64; 2];

pub fn valid_hex(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

// Version 1 stored palette indices and used the stroke for all labels.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredColor {
    Name(String),
    Legacy(usize),
}
fn read_color<'de, D: serde::Deserializer<'de>>(d: D, legacy: &[&str]) -> Result<String, D::Error> {
    match StoredColor::deserialize(d)? {
        StoredColor::Name(s) => Ok(s),
        StoredColor::Legacy(i) => legacy.get(i).map(|s| s.to_string())
            .ok_or_else(|| serde::de::Error::custom("지원하지 않는 색 번호입니다")),
    }
}
fn read_ink<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    read_color(d, &INKS)
}
fn read_fill<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    read_color(d, &FILLS)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Rect,
    Ellipse,
    Diamond,
    Arrow,
    Pen,
    Text,
}
impl Kind {
    pub fn shape(self) -> bool {
        matches!(self, Self::Rect | Self::Ellipse | Self::Diamond)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Head {
    #[default]
    None,
    Open,
    Filled,
}
impl Head {
    fn open() -> Self {
        Self::Open
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}
impl Side {
    pub const ALL: [Self; 4] = [Self::Top, Self::Right, Self::Bottom, Self::Left];
    fn normal(self) -> Point {
        match self {
            Self::Top => [0., -1.],
            Self::Right => [1., 0.],
            Self::Bottom => [0., 1.],
            Self::Left => [-1., 0.],
        }
    }
}

/// The strokes of one arrow: its line, broken around the label, and its heads.
pub struct ArrowShape {
    pub lines: Vec<Vec<Point>>,
    /// Whether the head is filled, then its left wing, tip and right wing.
    pub heads: Vec<(bool, [Point; 3])>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    pub id: u32,
    pub kind: Kind,
    pub a: Point,
    pub b: Point,
    pub points: Vec<Point>,
    pub from: Option<u32>,
    pub to: Option<u32>,
    pub text: String,
    pub font_size: f64,
    #[serde(deserialize_with = "read_ink")]
    pub ink: String,
    #[serde(deserialize_with = "read_fill")]
    pub fill: String,
    #[serde(default)]
    pub text_color: Option<String>,
    // Version 3 arrow options. A side pins that end to the middle of one side
    // of the attached shape; without it the end follows the line of centers.
    #[serde(default)]
    pub from_side: Option<Side>,
    #[serde(default)]
    pub to_side: Option<Side>,
    #[serde(default)]
    pub start_head: Head,
    #[serde(default = "Head::open")]
    pub end_head: Head,
    #[serde(default)]
    pub dashed: bool,
    #[serde(default)]
    pub elbow: bool,
}
impl Element {
    pub fn new(id: u32, kind: Kind, at: Point) -> Self {
        Self {
            id,
            kind,
            a: at,
            b: at,
            points: Vec::new(),
            from: None,
            to: None,
            text: String::new(),
            font_size: 15.,
            ink: INKS[0].into(),
            fill: "none".into(),
            text_color: Some(INKS[0].into()),
            from_side: None,
            to_side: None,
            start_head: Head::None,
            end_head: Head::Open,
            dashed: false,
            elbow: false,
        }
    }
    pub fn label_lines(&self) -> impl Iterator<Item = (&str, f64)> {
        let step = self.font_size * 1.25;
        let first = -(self.text.bytes().filter(|&b| b == b'\n').count() as f64) * step / 2.;
        self.text.split('\n').enumerate().map(move |(i, line)| (line, first + i as f64 * step))
    }
    pub fn label_color(&self) -> &str {
        self.text_color.as_deref().unwrap_or(&self.ink)
    }
    // SVG has no font metrics, so widths are estimated per character.
    fn line_width(&self, line: &str) -> f64 {
        let width: f64 = line
            .chars()
            .map(|c| if c.is_ascii() { 10. } else { 16. })
            .sum();
        width * self.font_size / 15.
    }
    pub fn center(&self) -> Point {
        [(self.a[0] + self.b[0]) / 2., (self.a[1] + self.b[1]) / 2.]
    }
    fn edge(&self, toward: Point) -> Point {
        let [x, y] = self.center();
        let (dx, dy) = (toward[0] - x, toward[1] - y);
        let (rx, ry) = (
            (self.b[0] - x).abs().max(0.01),
            (self.b[1] - y).abs().max(0.01),
        );
        let (u, v) = (dx.abs() / rx, dy.abs() / ry);
        let divisor = match self.kind {
            Kind::Ellipse => u.hypot(v),
            Kind::Diamond => u + v,
            _ => u.max(v),
        };
        if divisor == 0. {
            [x, y]
        } else {
            [x + dx / divisor, y + dy / divisor]
        }
    }
    /// The middle of one side of a shape's box.
    pub fn anchor(&self, side: Side) -> Point {
        let [x, y] = self.center();
        let [nx, ny] = side.normal();
        [x + nx * (self.b[0] - x).abs(), y + ny * (self.b[1] - y).abs()]
    }
    fn facing(&self, toward: Point) -> Side {
        let [x, y] = self.center();
        let (dx, dy) = (toward[0] - x, toward[1] - y);
        let (rx, ry) = (
            (self.b[0] - x).abs().max(0.01),
            (self.b[1] - y).abs().max(0.01),
        );
        match (dx.abs() / rx >= dy.abs() / ry, dx >= 0., dy >= 0.) {
            (true, true, _) => Side::Right,
            (true, false, _) => Side::Left,
            (false, _, true) => Side::Bottom,
            (false, _, false) => Side::Top,
        }
    }
    /// The visible path of an arrow, always at least two points.
    pub fn route(&self, items: &[Self]) -> Vec<Point> {
        let find = |id| items.iter().find(|v| Some(v.id) == id && v.kind.shape());
        let (from, to) = (find(self.from), find(self.to));
        let aim = |shape: Option<&Self>, side: Option<Side>, free: Point| match (shape, side) {
            (Some(s), Some(side)) => s.anchor(side),
            (Some(s), None) => s.center(),
            _ => free,
        };
        let aim_a = aim(from, self.from_side, self.a);
        let aim_b = aim(to, self.to_side, self.b);
        if !self.elbow {
            let end = |shape: Option<&Self>, side: Option<Side>, own: Point, other: Point| {
                match (shape, side) {
                    (Some(s), None) => s.edge(other),
                    _ => own,
                }
            };
            return vec![
                end(from, self.from_side, aim_a, aim_b),
                end(to, self.to_side, aim_b, aim_a),
            ];
        }
        let port = |shape: Option<&Self>, side: Option<Side>, own: Point, other: Point| match shape {
            Some(s) => {
                let side = side.unwrap_or_else(|| s.facing(other));
                (s.anchor(side), Some(side.normal()))
            }
            None => (own, None),
        };
        let (a, leave) = port(from, self.from_side, aim_a, aim_b);
        let (b, enter) = port(to, self.to_side, aim_b, aim_a);
        elbow(a, leave, b, enter)
    }
    pub fn ends(&self, items: &[Self]) -> (Point, Point) {
        let route = self.route(items);
        (route[0], route[route.len() - 1])
    }
    pub fn corners(&self, items: &[Self]) -> (Point, Point) {
        let route = if self.kind == Kind::Arrow {
            self.route(items)
        } else {
            vec![self.a, self.b]
        };
        let mut lo = route[0];
        let mut hi = route[0];
        for p in route.iter().chain(&self.points) {
            for i in 0..2 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        (lo, hi)
    }
    /// Where the label is centered: halfway along an arrow, else mid-box.
    pub fn label_point(&self, items: &[Self]) -> Point {
        if self.kind == Kind::Arrow {
            return midpoint(&self.route(items));
        }
        let (a, b) = self.corners(items);
        [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.]
    }
    /// `label` is the box the line must leave free for the arrow's text.
    pub fn arrow_shape(&self, items: &[Self], label: Option<(Point, Point)>) -> ArrowShape {
        let route = self.route(items);
        let n = route.len();
        let heads = [
            (self.start_head, route[0], route[1]),
            (self.end_head, route[n - 1], route[n - 2]),
        ]
        .into_iter()
        .filter(|(head, ..)| *head != Head::None)
        .map(|(head, tip, back)| {
            let [l, r] = Self::arrowhead(back, tip);
            (head == Head::Filled, [l, tip, r])
        })
        .collect();
        let lines = match label {
            Some((lo, hi)) => cut(&route, lo, hi),
            None => vec![route],
        };
        ArrowShape { lines, heads }
    }
    pub fn arrowhead(a: Point, b: Point) -> [Point; 2] {
        let angle = (b[1] - a[1]).atan2(b[0] - a[0]);
        [-0.45, 0.45].map(|d| {
            [
                b[0] - 11. * (angle + d).cos(),
                b[1] - 11. * (angle + d).sin(),
            ]
        })
    }
}
const STUB: f64 = 14.;
const JOG: f64 = 40.;
// A right-angled path that leaves and enters attached shapes head-on. `leave`
// and `enter` are the outward normals of the attached sides; a free end has none.
fn elbow(a: Point, leave: Option<Point>, b: Point, enter: Option<Point>) -> Vec<Point> {
    let stub = |p: Point, normal: Option<Point>, other: Point| {
        normal.map_or(p, |n| {
            let ahead = (other[0] - p[0]) * n[0] + (other[1] - p[1]) * n[1];
            let len = if ahead > 0. { STUB.min(ahead / 2.) } else { STUB };
            [p[0] + n[0] * len, p[1] + n[1] * len]
        })
    };
    let (p, q) = (stub(a, leave, b), stub(b, enter, a));
    let mid = [(p[0] + q[0]) / 2., (p[1] + q[1]) / 2.];
    let mut bends = [
        vec![[mid[0], p[1]], [mid[0], q[1]]],
        vec![[p[0], mid[1]], [q[0], mid[1]]],
        vec![[q[0], p[1]]],
        vec![[p[0], q[1]]],
    ];
    if (q[0] - p[0]).abs() < (q[1] - p[1]).abs() {
        bends.swap(0, 1);
        bends.swap(2, 3);
    }
    let best = bends
        .into_iter()
        .enumerate()
        .map(|(rank, bends)| {
            let mut path = vec![a, p];
            path.extend(bends);
            path.extend([q, b]);
            let (path, cost) = tidy(path);
            (path, cost + rank as f64 * 0.01)
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(path, _)| path)
        .unwrap_or_default();
    if best.len() < 2 {
        vec![a, b]
    } else {
        best
    }
}
// Drops repeated and in-line points, and scores the path: doubling back costs
// most, then each bend, then length.
fn tidy(path: Vec<Point>) -> (Vec<Point>, f64) {
    let mut out: Vec<Point> = Vec::new();
    let mut reversals = 0.;
    for p in path {
        if out.last().is_some_and(|q| (q[0] - p[0]).hypot(q[1] - p[1]) < 1e-6) {
            continue;
        }
        if let [.., u, v] = out[..] {
            let (ux, uy, vx, vy) = (v[0] - u[0], v[1] - u[1], p[0] - v[0], p[1] - v[1]);
            if (ux * vy - uy * vx).abs() < 1e-6 {
                if ux * vx + uy * vy < 0. {
                    reversals += 1.;
                }
                out.pop();
            }
        }
        out.push(p);
    }
    let length: f64 = out
        .windows(2)
        .map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]))
        .sum();
    let cost = reversals * 1e9 + out.len() as f64 * 1e5 + length;
    (out, cost)
}
// Halfway along the path. When that falls on a short jog between two longer
// runs, the label moves to the middle of the longest run instead.
fn midpoint(path: &[Point]) -> Point {
    let length = |s: &[Point]| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
    let along = |s: &[Point], t: f64| {
        [
            s[0][0] + (s[1][0] - s[0][0]) * t,
            s[0][1] + (s[1][1] - s[0][1]) * t,
        ]
    };
    let mut longest = &path[..2];
    for s in path.windows(2) {
        if length(s) > length(longest) {
            longest = s;
        }
    }
    let mut left = path.windows(2).map(length).sum::<f64>() / 2.;
    for s in path.windows(2) {
        let len = length(s);
        if left <= len {
            return if len < JOG && length(longest) > len {
                along(longest, 0.5)
            } else {
                along(s, if len > 0. { left / len } else { 0. })
            };
        }
        left -= len;
    }
    path[path.len() - 1]
}
/// The parts of a polyline outside the box `lo..hi`.
pub fn cut(line: &[Point], lo: Point, hi: Point) -> Vec<Vec<Point>> {
    let mut runs = Vec::new();
    let mut run: Vec<Point> = Vec::new();
    for s in line.windows(2) {
        let (p, q) = (s[0], s[1]);
        let at = |t: f64| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
        // The segment is inside the box for t0 < t < t1.
        let (mut t0, mut t1) = (0f64, 1f64);
        for k in 0..2 {
            let d = q[k] - p[k];
            if d == 0. {
                if p[k] < lo[k] || p[k] > hi[k] {
                    t1 = -1.;
                }
            } else {
                let (u, v) = ((lo[k] - p[k]) / d, (hi[k] - p[k]) / d);
                t0 = t0.max(u.min(v));
                t1 = t1.min(u.max(v));
            }
        }
        if t0 >= t1 {
            if run.is_empty() {
                run.push(p);
            }
            run.push(q);
            continue;
        }
        if t0 > 0. {
            if run.is_empty() {
                run.push(p);
            }
            run.push(at(t0));
        }
        if run.len() > 1 {
            runs.push(std::mem::take(&mut run));
        }
        run.clear();
        if t1 < 1. {
            run = vec![at(t1), q];
        }
    }
    if run.len() > 1 {
        runs.push(run);
    }
    runs
}

#[derive(Clone, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Diagram(pub Vec<Element>);
impl Diagram {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.0.len() <= 2000,
            "도형은 2,000개까지 저장할 수 있습니다"
        );
        let mut ids = HashSet::new();
        let shapes: HashSet<_> = self
            .0
            .iter()
            .filter(|e| e.kind.shape())
            .map(|e| e.id)
            .collect();
        for e in &self.0 {
            ensure!(ids.insert(e.id) && e.id < u32::MAX, "잘못된 도형 ID입니다");
            ensure!(
                valid_hex(&e.ink) && (e.fill == "none" || valid_hex(&e.fill)) && valid_hex(e.label_color()) && (8. ..=72.).contains(&e.font_size),
                "지원하지 않는 색입니다"
            );
            ensure!(
                e.text.len() <= 8192 && !e.text.chars().any(|c| c.is_control() && c != '\n'),
                "지원하지 않는 글자 또는 길이입니다"
            );
            ensure!(
                e.points.len() <= 32000 && (e.kind == Kind::Pen) == (!e.points.is_empty()),
                "잘못된 펜 데이터입니다"
            );
            ensure!(
                e.points
                    .iter()
                    .chain([&e.a, &e.b])
                    .all(|p| (0. ..=WIDTH).contains(&p[0]) && (0. ..=HEIGHT).contains(&p[1])),
                "캔버스 범위를 벗어났습니다"
            );
            for id in [e.from, e.to].into_iter().flatten() {
                ensure!(
                    e.kind == Kind::Arrow && shapes.contains(&id),
                    "잘못된 화살표 연결입니다"
                );
            }
            ensure!(
                (e.from_side.is_none() || e.from.is_some())
                    && (e.to_side.is_none() || e.to.is_some())
                    && (e.kind == Kind::Arrow || !(e.dashed || e.elbow)),
                "잘못된 화살표 설정입니다"
            );
        }
        Ok(())
    }
    pub fn bounds(&self) -> (Point, Point) {
        let mut lo = [WIDTH, HEIGHT];
        let mut hi = [0f64; 2];
        if self.0.is_empty() {
            return ([0., 0.], [80., 48.]);
        }
        for e in &self.0 {
            let (mut a, mut b) = e.corners(&self.0);
            if !e.text.is_empty() {
                let [cx, cy] = e.label_point(&self.0);
                for (line, offset) in e.label_lines() {
                let width = e.line_width(line);
                let left = if e.kind == Kind::Text {
                    a[0]
                } else {
                    cx - width / 2.
                };
                a[0] = a[0].min(left);
                b[0] = b[0].max(left + width);
                a[1] = a[1].min(cy + offset - e.font_size);
                b[1] = b[1].max(cy + offset + e.font_size);
                }
            }
            for i in 0..2 {
                lo[i] = lo[i].min(a[i] - 14.);
                hi[i] = hi[i].max(b[i] + 14.);
            }
        }
        (lo, hi)
    }
    pub fn has_content(&self) -> bool {
        self.0.iter().any(|e| e.kind != Kind::Text || !e.text.trim().is_empty())
    }
    pub fn svg(&self, title: &str) -> Result<String> {
        self.validate()?;
        ensure!(self.has_content(), "빈 다이어그램은 저장할 수 없습니다");
        let title = quick_xml::escape::escape(crate::document_title(title));
        let ([x, y], [right, bottom]) = self.bounds();
        let (w, h) = (right - x, bottom - y);
        let json = quick_xml::escape::escape(serde_json::to_string(self)?).into_owned();
        let mut out = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{x} {y} {w} {h}"><title>{title}</title><metadata><stology-diagram version="3">{json}</stology-diagram></metadata>"#
        );
        for e in &self.0 {
            let ([x, y], [r, b]) = e.corners(&self.0);
            let (w, h) = (r - x, b - y);
            let [cx, cy] = e.label_point(&self.0);
            let stroke = &e.ink;
            let fill = &e.fill;
            let text_color = e.label_color();
            write!(
                out,
                r#"<g stroke="{stroke}" stroke-width="1.7" fill="{fill}" stroke-linejoin="round" stroke-linecap="round">"#
            )?;
            match e.kind {
                Kind::Rect => write!(
                    out,
                    r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6"/>"#
                )?,
                Kind::Ellipse => write!(
                    out,
                    r#"<ellipse cx="{cx}" cy="{cy}" rx="{}" ry="{}"/>"#,
                    w / 2.,
                    h / 2.
                )?,
                Kind::Diamond => write!(
                    out,
                    r#"<polygon points="{cx},{y} {r},{cy} {cx},{b} {x},{cy}"/>"#
                )?,
                Kind::Arrow => {
                    let label = (!e.text.is_empty()).then(|| {
                        let half = e.text.split('\n').map(|l| e.line_width(l)).fold(0., f64::max) / 2. + 4.;
                        let rows = e.label_lines().count() as f64;
                        let tall = (rows * 1.25 - 0.25) * e.font_size / 2. + 2.;
                        ([cx - half, cy - tall], [cx + half, cy + tall])
                    });
                    let shape = e.arrow_shape(&self.0, label);
                    if !shape.lines.is_empty() {
                        let dash = if e.dashed { r#" stroke-dasharray="6 4""# } else { "" };
                        write!(out, r#"<path fill="none"{dash} d=""#)?;
                        for run in &shape.lines {
                            for (i, [x, y]) in run.iter().enumerate() {
                                write!(out, "{}{x} {y}", if i == 0 { 'M' } else { 'L' })?;
                            }
                        }
                        out.push_str("\"/>");
                    }
                    for (filled, [l, tip, r]) in shape.heads {
                        let (paint, close) = if filled { (stroke.as_str(), "Z") } else { ("none", "") };
                        write!(
                            out,
                            r#"<path fill="{paint}" d="M{} {}L{} {}L{} {}{close}"/>"#,
                            l[0], l[1], tip[0], tip[1], r[0], r[1]
                        )?;
                    }
                }
                Kind::Pen => {
                    out.push_str("<polyline fill=\"none\" points=\"");
                    for [x, y] in &e.points {
                        write!(out, "{x},{y} ")?;
                    }
                    out.push_str("\"/>");
                }
                Kind::Text => (),
            }
            out.push_str("</g>");
            if !e.text.is_empty() {
                let (pos, anchor) = if e.kind == Kind::Text {
                    (x, "start")
                } else {
                    (cx, "middle")
                };
                for (line, offset) in e.label_lines() {
                write!(
                    out,
                    r#"<text x="{pos}" y="{}" text-anchor="{anchor}" font-family="SUIT, sans-serif" font-size="{}" fill="{text_color}">{}</text>"#,
                    cy + offset + e.font_size / 3.,
                    e.font_size,
                    quick_xml::escape::escape(line)
                )?;
                }
            }
            ensure!(out.len() + 6 <= LIMIT, "다이어그램은 1 MiB 이하여야 합니다");
        }
        out.push_str("</svg>");
        Ok(out)
    }
    pub fn read(svg: &str) -> Result<(Self, String)> {
        ensure!(svg.len() <= LIMIT, "다이어그램 크기 초과");
        use quick_xml::{events::Event, Reader};
        let mut reader = Reader::from_str(svg);
        let mut stack = Vec::new();
        let mut model = None;
        let mut title = String::new();
        let mut root_seen = false;
        loop {
            match reader.read_event()? {
                Event::Start(tag) => {
                    let name = tag.name().as_ref().to_vec();
                    if stack.is_empty() {
                        ensure!(!root_seen && name == b"svg", "올바른 SVG가 아닙니다");
                        root_seen = true;
                    }
                    if stack.len() == 1 && name == b"title" {
                        let inner = reader.read_text(tag.name())?;
                        title = quick_xml::escape::unescape(&inner)?.into_owned();
                    } else if stack.len() == 2
                        && stack[0] == b"svg"
                        && stack[1] == b"metadata"
                        && name == b"stology-diagram"
                    {
                        ensure!(
                            model.is_none()
                                && tag
                                    .try_get_attribute("version")?
                                    .is_some_and(|a| matches!(a.value.as_ref(), b"1" | b"2" | b"3")),
                            "지원하지 않는 편집 데이터입니다"
                        );
                        let inner = reader.read_text(tag.name())?;
                        let diagram: Self =
                            serde_json::from_str(&quick_xml::escape::unescape(&inner)?)?;
                        diagram.validate()?;
                        model = Some(diagram);
                    } else {
                        stack.push(name);
                    }
                }
                Event::End(_) => {
                    stack.pop();
                }
                Event::DocType(_) => anyhow::bail!("지원하지 않는 SVG입니다"),
                Event::Eof => break,
                _ => (),
            }
        }
        ensure!(stack.is_empty(), "손상된 다이어그램입니다");
        Ok((model.context("편집 데이터를 읽을 수 없습니다")?, crate::document_title(&title).to_owned()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_and_attached_arrows_roundtrip() {
        let mut node = Element::new(1, Kind::Ellipse, [10., 20.]);
        node.b = [90., 80.];
        node.text = "한글 <&> \"".into();
        let mut arrow = Element::new(2, Kind::Arrow, [50., 50.]);
        arrow.b = [120., 50.];
        arrow.from = Some(1);
        let d = Diagram(vec![node, arrow]);
        let svg = d.svg("  용어 <A&B>  ").unwrap();
        assert_eq!(d.0[1].ends(&d.0).0, [90., 50.]);
        assert_eq!(Diagram::read(&svg).unwrap(), (d.clone(), "용어 <A&B>".into()));
        let legacy = svg.replace("<title>용어 &lt;A&amp;B&gt;</title>", "");
        assert_eq!(Diagram::read(&legacy).unwrap(), (d, "제목 없음".into()));
        assert!(Diagram::read(&svg.replace("version=\"3\"", "version=\"4\"")).is_err());
        assert!(Diagram::read(svg.trim_end_matches("</svg>")).is_err());
        assert!(Diagram::read("<svg/>").is_err());
    }
    fn boxed(id: u32, a: Point, b: Point) -> Element {
        let mut e = Element::new(id, Kind::Rect, a);
        e.b = b;
        e
    }
    fn linked(from: u32, to: u32) -> Element {
        let mut e = Element::new(9, Kind::Arrow, [0., 0.]);
        (e.from, e.to) = (Some(from), Some(to));
        e
    }
    #[test]
    fn version_2_arrows_keep_one_open_head_and_a_straight_solid_line() {
        let old = r##"<svg><metadata><stology-diagram version="2">[{"id":1,"kind":"arrow","a":[10,10],"b":[90,10],"points":[],"from":null,"to":null,"text":"","font_size":15,"ink":"#1e1e2a","fill":"none","text_color":null}]</stology-diagram></metadata></svg>"##;
        let (diagram, _) = Diagram::read(old).unwrap();
        let arrow = &diagram.0[0];
        assert_eq!((arrow.start_head, arrow.end_head), (Head::None, Head::Open));
        assert!(!arrow.dashed && !arrow.elbow && arrow.from_side.is_none());
        let svg = diagram.svg("").unwrap();
        assert!(svg.contains(r#"<path fill="none" d="M10 10L90 10"/>"#));
        assert_eq!(svg.matches("<path").count(), 2);
    }
    #[test]
    fn heads_dashes_and_label_gap_reach_the_svg() {
        let mut arrow = Element::new(1, Kind::Arrow, [0., 50.]);
        arrow.b = [256., 50.];
        arrow.text = "ab".into();
        (arrow.start_head, arrow.end_head, arrow.dashed) = (Head::Filled, Head::Filled, true);
        let diagram = Diagram(vec![arrow]);
        let svg = diagram.svg("").unwrap();
        // Two 10pt letters and 4pt of padding either side of the middle.
        assert!(svg.contains(r#"stroke-dasharray="6 4" d="M0 50L114 50M142 50L256 50""#));
        assert_eq!(svg.matches(r##"<path fill="#1e1e2a""##).count(), 2);
        assert_eq!(Diagram::read(&svg).unwrap().0, diagram);
        let shape = diagram.0[0].arrow_shape(&diagram.0, None);
        assert_eq!(shape.lines, vec![vec![[0., 50.], [256., 50.]]]);
        assert_eq!(shape.heads[0].1[1], [0., 50.]);
        assert!(shape.heads[0].1[0][0] > 0. && shape.heads[1].1[0][0] < 256.);
    }
    #[test]
    fn pinned_sides_separate_arrows_between_the_same_shapes() {
        let shapes = [boxed(1, [0., 0.], [100., 60.]), boxed(2, [200., 0.], [300., 60.])];
        let mut there = linked(1, 2);
        let mut back = linked(2, 1);
        let items = |arrow: &Element| [shapes[0].clone(), shapes[1].clone(), arrow.clone()];
        assert_eq!(there.route(&items(&there)), vec![[100., 30.], [200., 30.]]);
        assert_eq!(back.route(&items(&back)), vec![[200., 30.], [100., 30.]]);
        (there.from_side, there.to_side) = (Some(Side::Top), Some(Side::Top));
        (back.from_side, back.to_side) = (Some(Side::Bottom), Some(Side::Bottom));
        assert_eq!(there.route(&items(&there)), vec![[50., 0.], [250., 0.]]);
        assert_eq!(back.route(&items(&back)), vec![[250., 60.], [50., 60.]]);
        // One pinned end still aims the free-following end at the pin.
        there.to_side = None;
        assert_eq!(there.route(&items(&there))[1], [200., 22.5]);
        let mut loose = Element::new(9, Kind::Arrow, [0., 0.]);
        loose.to_side = Some(Side::Left);
        assert!(Diagram(vec![loose]).validate().is_err());
    }
    #[test]
    fn elbow_routes_leave_and_enter_shapes_head_on() {
        let right_angles = |path: &[Point]| {
            path.windows(2).all(|s| s[0][0] == s[1][0] || s[0][1] == s[1][1])
        };
        let near = [boxed(1, [0., 0.], [100., 60.]), boxed(2, [200., 100.], [300., 160.])];
        let mut arrow = linked(1, 2);
        arrow.elbow = true;
        let items = |shapes: &[Element; 2], arrow: &Element| {
            vec![shapes[0].clone(), shapes[1].clone(), arrow.clone()]
        };
        // Facing sides: out to the right, down the middle, into the left side.
        let path = arrow.route(&items(&near, &arrow));
        assert_eq!(path, vec![[100., 30.], [150., 30.], [150., 130.], [200., 130.]]);
        assert_eq!(arrow.label_point(&items(&near, &arrow)), [150., 80.]);
        // A short jog in the middle leaves the label on a longer run.
        let offset = [near[0].clone(), boxed(2, [20., 200.], [120., 260.])];
        let path = arrow.route(&items(&offset, &arrow));
        assert_eq!(path, vec![[50., 60.], [50., 130.], [70., 130.], [70., 200.]]);
        assert_eq!(arrow.label_point(&items(&offset, &arrow)), [50., 95.]);
        // A pinned top side is entered from above with a single bend.
        arrow.to_side = Some(Side::Top);
        let path = arrow.route(&items(&near, &arrow));
        assert_eq!(path, vec![[100., 30.], [250., 30.], [250., 100.]]);
        // Shapes in line need no bend at all.
        arrow.to_side = None;
        let level = [near[0].clone(), boxed(2, [200., 0.], [300., 60.])];
        assert_eq!(arrow.route(&items(&level, &arrow)), vec![[100., 30.], [200., 30.]]);
        // A target behind the pinned side is reached without doubling back.
        arrow.from_side = Some(Side::Left);
        let path = arrow.route(&items(&near, &arrow));
        assert!(right_angles(&path) && path[0] == [0., 30.] && path[1][0] < 0.);
        assert_eq!(arrow.corners(&items(&near, &arrow)), ([-14., 30.], [200., 130.]));
        // Free ends bend once.
        let mut free = Element::new(1, Kind::Arrow, [10., 10.]);
        free.b = [110., 60.];
        free.elbow = true;
        assert_eq!(free.route(&[]), vec![[10., 10.], [110., 10.], [110., 60.]]);
        free.b = [10., 10.];
        assert_eq!(free.route(&[]).len(), 2);
    }
    #[test]
    fn cut_removes_only_the_part_inside_the_box() {
        let line = [[0., 0.], [100., 0.], [100., 100.]];
        assert_eq!(cut(&line, [200., 200.], [300., 300.]), vec![line.to_vec()]);
        assert_eq!(
            cut(&line, [40., -5.], [60., 5.]),
            vec![vec![[0., 0.], [40., 0.]], vec![[60., 0.], [100., 0.], [100., 100.]]]
        );
        // A box over the corner removes the end of one segment and the start of the next.
        assert_eq!(
            cut(&line, [90., -10.], [110., 10.]),
            vec![vec![[0., 0.], [90., 0.]], vec![[100., 10.], [100., 100.]]]
        );
        assert!(cut(&line, [-10., -10.], [110., 110.]).is_empty());
        assert_eq!(cut(&line, [-10., -10.], [50., 10.]), vec![vec![[50., 0.], [100., 0.], [100., 100.]]]);
    }
    #[test]
    fn legacy_index_colors_and_custom_text_survive_reopening() {
        let old = r#"<svg><metadata><stology-diagram version="1">[{"id":1,"kind":"rect","a":[2,3],"b":[40,50],"points":[],"from":null,"to":null,"text":"기억","font_size":15,"ink":2,"fill":3}]</stology-diagram></metadata></svg>"#;
        let (mut diagram, _) = Diagram::read(old).unwrap();
        let shape = &mut diagram.0[0];
        assert_eq!(shape.label_color(), "#D6455D");
        assert_eq!(shape.fill, "#DDF3E4");
        shape.text_color = Some("#123ABC".into());
        let svg = diagram.svg("메모").unwrap();
        assert!(svg.contains("fill=\"#123ABC\">기억</text>"));
        assert_eq!(Diagram::read(&svg).unwrap().0, diagram);
        diagram.0[0].fill = "url(other.svg)".into();
        assert!(diagram.svg("").is_err());
    }
    #[test]
    fn rejects_invalid_coordinates_ids_and_sizes() {
        let mut e = Element::new(1, Kind::Rect, [0., 0.]);
        assert!(Diagram(vec![e.clone(); 2001]).svg("").is_err());
        e.a[0] = f64::NAN;
        assert!(Diagram(vec![e.clone()]).svg("").is_err());
        e.a[0] = 0.;
        e.kind = Kind::Arrow;
        e.from = Some(99);
        assert!(Diagram(vec![e]).svg("").is_err());
    }
}
