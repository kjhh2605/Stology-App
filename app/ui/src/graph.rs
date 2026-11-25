//! Perspective 3D knowledge graph.
//!
//! Interaction follows the web map (prod-ui `features/knowledge/engine`): damped
//! orbit and zoom with graph-relative limits, 720 ms select/overview moves, an
//! ego layout around the selected concept, eased colour/size highlights, halos
//! and a prioritised label pool. Analytic spheres and constant-width relations
//! are drawn by Metal with real depth; AppKit labels and picking share the same
//! camera projection. A display link runs only while something is moving.
#[path = "graph_math.rs"]
mod math;
#[path = "graph_metal.rs"]
mod metal;
use math::{ray_sphere, Camera, Rig, Vec3};
use metal::{FlatVertex, Metal, SphereInstance};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AnyThread, ClassType, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{
    MainThreadMarker, NSAffineTransform, NSDictionary, NSNotification, NSNotificationCenter,
    NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    f64::consts::TAU,
    time::Instant,
};

// Measurement helpers for the examples; the app itself does not call them.
#[allow(unused_imports)]
pub use metal::{silhouette_deviation, stroke_width, write_png, Silhouette};

const NODE_RADIUS: f64 = 15.;
/// One web-map world unit in local units (the web's mean node radius is 1.25).
const UNIT: f64 = NODE_RADIUS / 1.25;
const LAYOUT_DURATION: f64 = 0.72;
/// Pointer travel below this is a tap; above it, a drag.
const TAP_SLOP: f64 = 6.;
const DOUBLE_TAP: f64 = 0.28;
/// Relations are cut this far in front of the eye instead of at the near plane.
const NEAR_CLIP: f64 = 4.;
const AA_PAD: f64 = 1.;

// Palette of the web map's light theme.
const ACTIVE: u32 = 0x4f46e5;
const DIM: u32 = 0xc9cede;
const EDGE: u32 = 0x8b93a8;
const EDGE_HIGHLIGHT: u32 = 0x5b53e8;
const INK: u32 = 0x1c1e2e;

fn srgb(hex: u32) -> [f64; 3] {
    [
        ((hex >> 16) & 255) as f64 / 255.,
        ((hex >> 8) & 255) as f64 / 255.,
        (hex & 255) as f64 / 255.,
    ]
}

pub fn unit_color(position: usize) -> [f64; 3] {
    const COLORS: [u32; 10] = [
        0x16857b, 0xc77724, 0xbc5774, 0x3884b2, 0x83912e,
        0xa16d44, 0x9c5ca0, 0x3b9470, 0xc26342, 0x657ba8,
    ];
    srgb(COLORS[position % COLORS.len()])
}
fn to_linear(c: [f64; 3]) -> [f64; 3] {
    c.map(|v| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    })
}
fn mix3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
/// Nodes and relations dissolve before they can fill the view or meet the near
/// plane: fully visible from six radii away, gone at three.
fn near_fade(distance: f64) -> f64 {
    smoothstep(3. * NODE_RADIUS, 6. * NODE_RADIUS, distance)
}

struct Node {
    id: String,
    name: String,
    /// Normalised name and aliases for search.
    keys: Vec<String>,
    degree: usize,
    unit: Option<String>,
    color: [f64; 3],
}
struct Link {
    /// Index into `data["assertions"]`.
    assertion: usize,
    a: usize,
    b: usize,
    relation: String,
    directed: bool,
    inferred: bool,
    label: String,
    /// Signed lane among relations between the same pair; 0 is the straight one.
    lane: f64,
    order: usize,
    sign: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stroke {
    rgb: [f64; 3],
    alpha: f64,
    width: f64,
}
const HIDDEN_STROKE: Stroke = Stroke {
    rgb: [0.545, 0.576, 0.659],
    alpha: 0.,
    width: 1.25,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tone {
    Default,
    Selected,
    Hover,
    Search,
    Relation,
}
struct Drag {
    node: Option<usize>,
    offset: Vec3,
    last: NSPoint,
    moved: f64,
    /// Cancelled by leaving the map; ignored until the button is released.
    dead: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    Node(usize),
    Link(usize),
    Nothing,
}
/// Area covered by host controls, in logical pixels from each edge.
#[derive(Clone, Copy, Debug, Default)]
struct Insets {
    top: f64,
    right: f64,
    bottom: f64,
    left: f64,
}
struct State {
    data: Value,
    nodes: Vec<Node>,
    links: Vec<Link>,
    index: HashMap<String, usize>,
    adjacency: Vec<Vec<usize>>,
    /// Deterministic overview layout, and the layout currently aimed at.
    base: Vec<Vec3>,
    goal: Vec<Vec3>,
    from: Vec<Vec3>,
    now: Vec<Vec3>,
    tween: Option<f64>,
    scale: Vec<f64>,
    scale_goal: Vec<f64>,
    color: Vec<[f64; 3]>,
    color_goal: Vec<[f64; 3]>,
    gloss: Vec<f64>,
    gloss_goal: Vec<f64>,
    shown: Vec<bool>,
    hits: Vec<bool>,
    floating: Vec<bool>,
    float_strength: Vec<f64>,
    float_offset: Vec<Vec3>,
    float_clock: f64,
    float_basis: (Vec3, Vec3, Vec3),
    eyebrow: Vec<String>,
    stroke: Vec<Stroke>,
    stroke_goal: Vec<Stroke>,
    label_alpha: Vec<f64>,
    label_style: Vec<(Tone, String)>,
    labels_moving: bool,
    /// Selected and hover halo: node and eased strength.
    halo: [(Option<usize>, f64); 2],
    bounds: f64,
    rig: Rig,
    selected: Option<Value>,
    selected_node: Option<usize>,
    node_selection_pending: bool,
    selected_link: Option<usize>,
    /// Centre of the ego layout, if one is active.
    focus: Option<usize>,
    focus_radius: f64,
    hovered: Option<usize>,
    hovered_link: Option<usize>,
    filter: String,
    unit_focus: String,
    query: String,
    drag: Option<Drag>,
    last_tap: Option<Instant>,
    insets: Insets,
    entry_pending: bool,
    ticker: Option<Retained<AnyObject>>,
    last_step: Option<Instant>,
    last_draw: Option<Instant>,
    metal: Option<Metal>,
    overlay: Option<Retained<GraphView>>,
    last_bounds: NSSize,
    widths: HashMap<(String, u8), (String, f64)>,
    /// Counters for the on-screen self-test: display-link ticks, drawn frames
    /// and seconds of CPU spent advancing and composing them.
    ticks: u64,
    draws: u64,
    work: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            data: Value::Null,
            nodes: Vec::new(),
            links: Vec::new(),
            index: HashMap::new(),
            adjacency: Vec::new(),
            base: Vec::new(),
            goal: Vec::new(),
            from: Vec::new(),
            now: Vec::new(),
            tween: None,
            scale: Vec::new(),
            scale_goal: Vec::new(),
            color: Vec::new(),
            color_goal: Vec::new(),
            gloss: Vec::new(),
            gloss_goal: Vec::new(),
            shown: Vec::new(),
            hits: Vec::new(),
            floating: Vec::new(),
            float_strength: Vec::new(),
            float_offset: Vec::new(),
            float_clock: 0.,
            float_basis: Camera::default().basis(),
            eyebrow: Vec::new(),
            stroke: Vec::new(),
            stroke_goal: Vec::new(),
            label_alpha: Vec::new(),
            label_style: Vec::new(),
            labels_moving: false,
            halo: [(None, 0.); 2],
            bounds: NODE_RADIUS,
            rig: Rig::default(),
            selected: None,
            selected_node: None,
            node_selection_pending: false,
            selected_link: None,
            focus: None,
            focus_radius: 6. * UNIT,
            hovered: None,
            hovered_link: None,
            filter: String::new(),
            unit_focus: String::new(),
            query: String::new(),
            drag: None,
            last_tap: None,
            insets: Insets::default(),
            entry_pending: false,
            ticker: None,
            last_step: None,
            last_draw: None,
            metal: None,
            overlay: None,
            last_bounds: NSSize::new(800., 600.),
            widths: HashMap::new(),
            ticks: 0,
            draws: 0,
            work: 0.,
        }
    }
}
thread_local! {static STATE:RefCell<State>=RefCell::new(State::default());static UI_COMMAND:RefCell<Option<String>>=const{RefCell::new(None)};static BACKGROUND_MOTION:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
pub fn take_ui_command() -> Option<String> {
    UI_COMMAND.with(|c| c.borrow_mut().take())
}
fn command(c: &str) {
    UI_COMMAND.with(|v| *v.borrow_mut() = Some(c.to_owned()));
}

// ── Model ────────────────────────────────────────────────────────────────
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}
fn relation_label(data: &Value, a: &Value) -> String {
    let name = data["relations"]
        .as_array()
        .and_then(|r| r.iter().find(|r| r["id"] == a["relation"]))
        .and_then(|r| r["label"].as_str())
        .unwrap_or("관계");
    format!(
        "{}{}{}",
        if a["negated"] == true { "¬ " } else { "" },
        name.chars().take(24).collect::<String>(),
        if a["condition"].as_str().is_some_and(|c| !c.is_empty()) {
            " · 조건"
        } else {
            ""
        }
    )
}
/// Replace the graph and compute its deterministic overview layout.
fn load(s: &mut State, data: Value) {
    s.data = data;
    s.nodes.clear();
    s.links.clear();
    s.index.clear();
    let subject = s.data["layer"] == "subject";
    let default_color = to_linear(srgb(ACTIVE));
    let unit_colors: HashMap<&str, [f64; 3]> = s.data["units"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| {
            Some((
                u["id"].as_str()?,
                to_linear(unit_color(u["position"].as_u64()? as usize)),
            ))
        })
        .collect();
    if !subject {
        s.unit_focus.clear();
    }
    for c in s.data["concepts"].as_array().into_iter().flatten() {
        let Some(id) = c["id"].as_str() else {
            continue;
        };
        if s.index.contains_key(id) {
            continue;
        }
        let name = c["name"].as_str().unwrap_or("개념").to_owned();
        let mut keys = vec![normalize(&name)];
        keys.extend(
            c["aliases"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .map(normalize),
        );
        s.index.insert(id.to_owned(), s.nodes.len());
        let unit = if subject { c["unit"].as_str() } else { None };
        let color = unit
            .and_then(|id| unit_colors.get(id).copied())
            .unwrap_or(default_color);
        s.nodes.push(Node {
            id: id.to_owned(),
            name,
            keys,
            degree: 0,
            unit: unit.map(str::to_owned),
            color,
        });
    }
    if !s.nodes.iter().any(|node| node.unit.as_deref() == Some(s.unit_focus.as_str())) {
        s.unit_focus.clear();
    }
    let n = s.nodes.len();
    let mut pair_counts = HashMap::new();
    let mut endpoints = Vec::new();
    for (i, a) in s.data["assertions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let (Some(p), Some(q)) = (
            a["subject"].as_str().and_then(|v| s.index.get(v)).copied(),
            a["object"].as_str().and_then(|v| s.index.get(v)).copied(),
        ) else {
            continue;
        };
        *pair_counts.entry((p.min(q), p.max(q))).or_insert(0usize) += 1;
        endpoints.push((i, p, q));
    }
    let mut lanes = HashMap::new();
    for (i, p, q) in endpoints {
        let a = &s.data["assertions"][i];
        let pair = (p.min(q), p.max(q));
        let count = pair_counts[&pair];
        let order = lanes.entry(pair).or_insert(0usize);
        let lane = *order as f64 - (count - 1) as f64 / 2.;
        let directed = s.data["relations"]
            .as_array()
            .and_then(|r| r.iter().find(|r| r["id"] == a["relation"]))
            .and_then(|r| r["directed"].as_bool())
            .unwrap_or(true);
        s.links.push(Link {
            assertion: i,
            a: p,
            b: q,
            relation: a["relation"].as_str().unwrap_or("").to_owned(),
            directed,
            inferred: subject && a["basis"] == "inferred",
            label: relation_label(&s.data, a),
            lane,
            order: *order,
            sign: if s.nodes[p].id <= s.nodes[q].id {
                1.
            } else {
                -1.
            },
        });
        *order += 1;
    }
    refresh_adjacency(s);
    let ids = s.nodes.iter().map(|v| v.id.clone()).collect::<Vec<_>>();
    let pairs = s
        .links
        .iter()
        .map(|l| (ids[l.a].clone(), ids[l.b].clone()))
        .collect::<Vec<_>>();
    let layout = math::layout(&ids, &pairs);
    s.base = ids.iter().map(|id| layout[id]).collect();
    s.bounds = s
        .base
        .iter()
        .map(|p| p.length() + NODE_RADIUS)
        .fold(NODE_RADIUS, f64::max);
    let colors: Vec<_> = s.nodes.iter().map(|node| node.color).collect();
    s.goal = s.base.clone();
    s.from = s.base.clone();
    s.now = s.base.clone();
    s.tween = None;
    s.scale = vec![NODE_RADIUS; n];
    s.scale_goal = vec![NODE_RADIUS; n];
    s.color = colors.clone();
    s.color_goal = colors;
    s.gloss = vec![0.; n];
    s.gloss_goal = vec![0.; n];
    s.shown = vec![true; n];
    s.hits = vec![false; n];
    s.floating = vec![false; n];
    s.float_strength = vec![0.; n];
    s.float_offset = vec![Vec3::ZERO; n];
    s.eyebrow = vec![String::new(); n];
    s.stroke = vec![HIDDEN_STROKE; s.links.len()];
    s.stroke_goal = vec![HIDDEN_STROKE; s.links.len()];
    s.label_alpha = vec![0.; n];
    s.label_style = vec![(Tone::Default, String::new()); n];
    s.halo = [(None, 0.); 2];
    s.selected = None;
    s.selected_node = None;
    s.selected_link = None;
    s.focus = None;
    s.hovered = None;
    s.hovered_link = None;
    s.drag = None;
    refresh_hits(s);
    refresh_shown(s);
    apply_highlight(s);
    s.stroke = s.stroke_goal.clone();
    update_limits(s);
    frame_all(s, false);
}
fn refresh_adjacency(s: &mut State) {
    s.adjacency.clear();
    s.adjacency.resize_with(s.nodes.len(), Vec::new);
    for link in &s.links {
        if link.a != link.b {
            s.adjacency[link.a].push(link.b);
            s.adjacency[link.b].push(link.a);
        }
    }
    for (i, neighbors) in s.adjacency.iter_mut().enumerate() {
        neighbors.sort_unstable_by(|a, b| s.nodes[*a].id.cmp(&s.nodes[*b].id));
        neighbors.dedup();
        s.nodes[i].degree = neighbors.len();
    }
}
fn refresh_hits(s: &mut State) {
    let q = normalize(&s.query);
    s.hits = s
        .nodes
        .iter()
        .map(|node| !q.is_empty() && node.keys.iter().any(|k| k.contains(&q)))
        .collect();
}
/// A relation filter without a selected concept narrows the map to that relation.
fn refresh_shown(s: &mut State) {
    let n = s.nodes.len();
    if s.filter.is_empty() || s.selected_node.is_some() {
        s.shown = vec![true; n];
        return;
    }
    let mut shown = vec![false; n];
    for link in &s.links {
        if link.relation == s.filter {
            shown[link.a] = true;
            shown[link.b] = true;
        }
    }
    s.shown = shown;
}
fn link_visible(s: &State, link: &Link) -> bool {
    s.shown[link.a]
        && s.shown[link.b]
        && (s.filter.is_empty() || s.selected_node.is_some() || link.relation == s.filter)
}
/// Turn selection, hover, search and filter into colour, size and stroke goals.
fn apply_highlight(s: &mut State) {
    let n = s.nodes.len();
    let focus = s.selected_node.or(s.hovered);
    let related: HashSet<usize> = focus
        .map(|f| s.adjacency[f].iter().copied().collect())
        .unwrap_or_default();
    let ends = s.selected_link.map(|l| (s.links[l].a, s.links[l].b));
    let dim = to_linear(srgb(DIM));
    for i in 0..n {
        let is_focus = focus == Some(i);
        let is_related = related.contains(&i);
        let mut color = s.nodes[i].color;
        if s.selected_node.is_some() {
            if !is_focus && !is_related {
                color = mix3(color, dim, 0.65);
            }
        } else if let Some((a, b)) = ends {
            if i != a && i != b {
                color = mix3(color, dim, 0.65);
            }
        } else if s.hovered.is_some() && !is_focus && !is_related && !s.hits[i] {
            color = mix3(color, dim, 0.55);
        }
        if s.selected_node.is_none()
            && s.selected_link.is_none()
            && !s.unit_focus.is_empty()
            && s.nodes[i].unit.as_deref() != Some(s.unit_focus.as_str())
        {
            color = mix3(color, dim, 0.75);
        }
        s.color_goal[i] = color;
        let mut scale = NODE_RADIUS;
        if s.selected_node == Some(i) {
            scale *= 1.1;
        } else if s.hovered == Some(i) {
            scale *= 1.04;
        } else if is_related {
            scale *= 1.07;
        } else if s.selected_node.is_none() && s.hits[i] {
            scale *= 1.03;
        }
        s.scale_goal[i] = if s.shown[i] { scale } else { 0. };
        s.gloss_goal[i] = if s.selected_node == Some(i) { 1. } else { 0. };
    }
    // Relation names ride on the neighbour's label; a chosen relation also lets
    // its neighbours float.
    let mut names = vec![Vec::<&str>::new(); n];
    s.floating = vec![false; n];
    if let Some(center) = s.selected_node {
        for link in &s.links {
            if !link_visible(s, link) {
                continue;
            }
            let other = if link.a == center && link.b != center {
                link.b
            } else if link.b == center && link.a != center {
                link.a
            } else {
                continue;
            };
            if !s.filter.is_empty() && link.relation != s.filter {
                continue;
            }
            if !names[other].contains(&link.label.as_str()) {
                names[other].push(&link.label);
            }
            if !s.filter.is_empty() {
                s.floating[other] = true;
            }
        }
    }
    s.eyebrow = names.into_iter().map(|v| v.join(" · ")).collect();
    let highlight = srgb(EDGE_HIGHLIGHT);
    for (i, link) in s.links.iter().enumerate() {
        let incident = focus.is_some_and(|f| link.a == f || link.b == f);
        let matches = s.filter.is_empty() || link.relation == s.filter;
        let mut stroke = Stroke {
            rgb: srgb(EDGE),
            alpha: if focus.is_some() || s.selected_link.is_some() {
                0.3
            } else {
                0.62
            },
            width: 1.25,
        };
        if (incident && (s.selected_node.is_none() || matches)) || s.hovered_link == Some(i) {
            stroke = Stroke {
                rgb: highlight,
                alpha: 0.95,
                width: 2.,
            };
        }
        if s.selected_link == Some(i) {
            stroke = Stroke {
                rgb: srgb(ACTIVE),
                alpha: 1.,
                width: 2.5,
            };
        }
        if s.selected_node.is_none()
            && s.selected_link.is_none()
            && !s.unit_focus.is_empty()
            && (s.nodes[link.a].unit.as_deref() != Some(s.unit_focus.as_str())
                || s.nodes[link.b].unit.as_deref() != Some(s.unit_focus.as_str()))
        {
            stroke.alpha *= 0.3;
        }
        if !link_visible(s, link) {
            stroke.alpha = 0.;
        }
        s.stroke_goal[i] = stroke;
    }
}
fn selection_for(s: &State, node: usize) -> Option<Value> {
    let id = s.nodes[node].id.as_str();
    let c = s.data["concepts"]
        .as_array()?
        .iter()
        .find(|c| c["id"].as_str() == Some(id))?;
    let a = s.data["assertions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["subject"].as_str() == Some(id) || a["object"].as_str() == Some(id))
        .cloned()
        .collect::<Vec<_>>();
    let evidence = a
        .iter()
        .flat_map(|a| a["evidence"].as_array().cloned().unwrap_or_default())
        .collect::<Vec<_>>();
    Some(json!({"kind":"concept","concept":c,"assertions":a,"evidence":evidence}))
}
fn selected_id(s: &State) -> Option<&str> {
    s.selected_node.map(|i| s.nodes[i].id.as_str())
}

// ── Layout and camera goals ──────────────────────────────────────────────
fn dimensions(s: &State) -> NSSize {
    s.overlay
        .as_ref()
        .map(|v| v.bounds().size)
        .filter(|size| size.width > 0. && size.height > 0.)
        .unwrap_or(s.last_bounds)
}
/// Viewport left free by host controls.
fn usable(s: &State) -> (f64, f64) {
    let size = dimensions(s);
    (
        (size.width - s.insets.left - s.insets.right).max(200.),
        (size.height - s.insets.top - s.insets.bottom).max(200.),
    )
}
fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}
/// Motion needs a view to drive it; headless use arrives at every goal at once.
fn animated(s: &State) -> bool {
    s.overlay.is_some() && !reduce_motion()
}
fn live(s: &State) -> bool {
    let Some(view) = &s.overlay else {
        return false;
    };
    let visible = view.window().is_some_and(|w| {
        w.isVisible() && w.occlusionState().contains(NSWindowOcclusionState::Visible)
    });
    visible
        && (BACKGROUND_MOTION.get()
            || MainThreadMarker::new()
                .is_some_and(|m| NSApplication::sharedApplication(m).isActive()))
}
fn update_limits(s: &mut State) {
    let (w, h) = usable(s);
    let min = (9.6 * NODE_RADIUS).max(0.34 * s.bounds);
    let max = (3.1 * s.bounds).max(math::fit_distance(s.bounds + 22., w, h));
    s.rig.set_limits(min, max);
}
fn begin_tween(s: &mut State, animate: bool) {
    if animate && animated(s) {
        s.tween = Some(0.);
    } else {
        s.now = s.goal.clone();
        s.tween = None;
    }
}
/// Frame every shown concept from the default overview direction.
fn frame_all(s: &mut State, animate: bool) {
    update_limits(s);
    let (w, h) = usable(s);
    let points = (0..s.nodes.len())
        .filter(|i| s.shown[*i])
        .filter(|i| s.unit_focus.is_empty() || s.nodes[*i].unit.as_deref() == Some(s.unit_focus.as_str()))
        .map(|i| (s.base[i], NODE_RADIUS + 22.))
        .collect::<Vec<_>>();
    let mut camera = Camera::default();
    camera.fit(&points, w, h);
    let animate = animate && animated(s);
    s.rig.travel(camera, animate);
}
fn overview(s: &mut State, animate: bool) {
    s.from = s.now.clone();
    s.goal = s.base.clone();
    s.focus = None;
    begin_tween(s, animate);
    frame_all(s, animate);
}
fn frame_focus(s: &mut State, animate: bool) {
    let (w, h) = usable(s);
    let desired = s.focus_radius * 1.22 / math::limiting_tan(w, h).max(0.1);
    // Small graphs have a short overview distance; never let that crop the shell.
    s.rig.max_distance = s.rig.max_distance.max(desired / 0.82);
    let mut goal = s.rig.goal;
    goal.pivot = Vec3::ZERO;
    goal.distance = desired.clamp(s.rig.min_distance, s.rig.max_distance * 0.82);
    let animate = animate && animated(s);
    s.rig.travel(goal, animate);
}
/// Ego layout around the selected concept, facing the camera as it is now.
fn focus_on(s: &mut State, animate: bool) {
    let Some(center) = s.selected_node else {
        return;
    };
    let basis = s.rig.now.basis();
    let radii = vec![NODE_RADIUS; s.nodes.len()];
    s.from = s.now.clone();
    let plan = math::focus_layout(
        &s.base,
        &radii,
        &s.adjacency,
        &s.shown,
        center,
        basis,
        UNIT,
        &mut s.goal,
    );
    s.focus = Some(center);
    s.focus_radius = plan.radius;
    s.float_basis = basis;
    begin_tween(s, animate);
    frame_focus(s, animate);
}
fn select_node(s: &mut State, node: usize) {
    s.node_selection_pending = true;
    s.entry_pending = false;
    if s.selected_node == Some(node) {
        return;
    }
    s.selected_node = Some(node);
    s.selected_link = None;
    s.selected = selection_for(s, node);
    s.hovered = None;
    s.hovered_link = None;
    s.drag = None;
    refresh_shown(s);
    apply_highlight(s);
    focus_on(s, true);
}
/// Deselect and return to the overview; nothing happens if nothing is selected.
fn clear_selection(s: &mut State) {
    if s.selected_node.is_none() && s.selected_link.is_none() && s.focus.is_none() {
        return;
    }
    reset_view(s);
}
/// Drops the selection, hover and drag, and anything waiting on entry.
fn forget_selection(s: &mut State) {
    s.entry_pending = false;
    s.selected = None;
    s.selected_node = None;
    s.selected_link = None;
    s.hovered = None;
    s.hovered_link = None;
    s.drag = None;
}
fn reset_view(s: &mut State) {
    forget_selection(s);
    s.unit_focus.clear();
    refresh_shown(s);
    apply_highlight(s);
    overview(s, true);
}

// ── Animation ────────────────────────────────────────────────────────────
fn position(s: &State, i: usize) -> Vec3 {
    s.now[i] + s.float_offset[i]
}
fn approach(now: &mut f64, goal: f64, rate: f64, epsilon: f64) -> bool {
    let diff = goal - *now;
    if diff.abs() > epsilon && rate < 1. {
        *now += diff * rate;
        true
    } else {
        *now = goal;
        false
    }
}
/// Advance every eased quantity by `dt`. Returns true while anything still moves.
fn step(s: &mut State, dt: f64) -> bool {
    let reduced = !animated(s);
    let mut moving = s.rig.update(dt, reduced);
    let steps = (dt * 60.).clamp(0.4, 3.);
    let rate = |per_frame: f64| {
        if reduced {
            1.
        } else {
            (per_frame * steps).min(1.)
        }
    };
    let mut ease = None;
    if let Some(elapsed) = s.tween {
        let elapsed = (elapsed + dt).min(LAYOUT_DURATION);
        let progress = if reduced {
            1.
        } else {
            elapsed / LAYOUT_DURATION
        };
        ease = Some(math::ease_in_out_cubic(progress));
        s.tween = (progress < 1.).then_some(elapsed);
        moving |= progress < 1.;
    }
    let dragged = s.drag.as_ref().and_then(|d| d.node.map(|n| (n, d.offset)));
    for i in 0..s.nodes.len() {
        let drag = dragged
            .filter(|(n, _)| *n == i)
            .map(|(_, o)| o)
            .unwrap_or(Vec3::ZERO);
        if let Some(k) = ease {
            s.now[i] = s.from[i].lerp(s.goal[i], k) + drag;
        } else {
            let goal = s.goal[i] + drag;
            let diff = goal - s.now[i];
            // The dragged node tracks the pointer exactly; a released one eases home.
            if reduced || dragged.is_some_and(|(n, _)| n == i) || diff.length() <= 0.002 * UNIT {
                s.now[i] = goal;
            } else {
                s.now[i] = s.now[i] + diff * rate(0.2);
                moving = true;
            }
        }
        moving |= approach(&mut s.scale[i], s.scale_goal[i], rate(0.18), 0.0008 * UNIT);
        moving |= approach(&mut s.gloss[i], s.gloss_goal[i], rate(0.2), 0.002);
        for c in 0..3 {
            moving |= approach(&mut s.color[i][c], s.color_goal[i][c], rate(0.2), 0.002);
        }
    }
    for i in 0..s.links.len() {
        let goal = s.stroke_goal[i];
        let stroke = &mut s.stroke[i];
        // An invisible relation takes its next colour at once, then fades in.
        if stroke.alpha < 0.01 {
            stroke.rgb = goal.rgb;
            stroke.width = goal.width;
        }
        moving |= approach(&mut stroke.alpha, goal.alpha, rate(0.2), 0.004);
        moving |= approach(&mut stroke.width, goal.width, rate(0.2), 0.01);
        for c in 0..3 {
            moving |= approach(&mut stroke.rgb[c], goal.rgb[c], rate(0.2), 0.002);
        }
    }
    let targets = [
        s.selected_node,
        s.hovered.filter(|h| Some(*h) != s.selected_node),
    ];
    for (slot, target) in targets.into_iter().enumerate() {
        let (node, strength) = &mut s.halo[slot];
        if target.is_some() && *node != target {
            *node = target;
            *strength = 0.;
        }
        moving |= approach(strength, target.is_some() as u8 as f64, rate(0.22), 0.004);
    }
    // Gentle bob of the neighbours a chosen relation points at.
    if reduced {
        s.float_strength.fill(0.);
        s.float_offset.fill(Vec3::ZERO);
    } else {
        s.float_clock = (s.float_clock + dt) % 3600.;
        let (right, up, toward) = s.float_basis;
        for i in 0..s.nodes.len() {
            let target = (s.floating[i] && s.shown[i]) as u8 as f64;
            moving |= approach(&mut s.float_strength[i], target, rate(0.28), 0.0008);
            let strength = s.float_strength[i];
            if strength <= 0.0008 {
                s.float_offset[i] = Vec3::ZERO;
                continue;
            }
            let phase = (i as f64 * 2.399963229728653) % TAU;
            let period = 2.8 + ((i * 37) % 17) as f64 / 17. * 1.6;
            let amplitude = (0.12 + ((i * 29) % 13) as f64 / 13. * 0.12) * UNIT * strength;
            let wave = (s.float_clock * TAU / period + phase).sin();
            let sway = (s.float_clock * TAU / (period * 1.37) + phase * 0.61).sin();
            let depth = (s.float_clock * TAU / (period * 1.81) + phase * 1.19).sin();
            s.float_offset[i] = up * (amplitude * wave)
                + right * (amplitude * 0.32 * sway)
                + toward * (amplitude * 0.18 * depth);
            moving |= target > 0.;
        }
    }
    moving
}
/// Arrive at every goal immediately.
fn settle(s: &mut State) {
    s.rig.snap();
    s.tween = None;
    let dragged = s.drag.as_ref().and_then(|d| d.node.map(|n| (n, d.offset)));
    s.now = s.goal.clone();
    if let Some((node, offset)) = dragged {
        s.now[node] = s.now[node] + offset;
    }
    s.scale = s.scale_goal.clone();
    s.color = s.color_goal.clone();
    s.gloss = s.gloss_goal.clone();
    s.stroke = s.stroke_goal.clone();
    s.float_strength.fill(0.);
    s.float_offset.fill(Vec3::ZERO);
    s.halo = [
        (s.selected_node, s.selected_node.is_some() as u8 as f64),
        (
            s.hovered.filter(|h| Some(*h) != s.selected_node),
            s.hovered.is_some_and(|h| Some(h) != s.selected_node) as u8 as f64,
        ),
    ];
}
fn stop_loop(s: &mut State) {
    if let Some(ticker) = s.ticker.take() {
        // NSTimer and CADisplayLink both stop, and release their target, here.
        let _: () = unsafe { msg_send![&*ticker, invalidate] };
    }
    s.last_step = None;
}
unsafe fn start_loop(s: &mut State) {
    if s.ticker.is_some() {
        return;
    }
    let Some(view) = &s.overlay else {
        return;
    };
    // A view display link follows the refresh rate of the display the window is
    // on, and suspends itself while the view is not visible.
    let has_link: bool =
        msg_send![&**view, respondsToSelector: sel!(displayLinkWithTarget:selector:)];
    let ticker: Option<Retained<AnyObject>> = if has_link {
        let link: Option<Retained<AnyObject>> =
            msg_send![&**view, displayLinkWithTarget: &**view, selector: sel!(animationTick:)];
        if let Some(link) = &link {
            let run_loop = NSRunLoop::mainRunLoop();
            let _: () = msg_send![&**link, addToRunLoop: &*run_loop, forMode: NSRunLoopCommonModes];
        }
        link
    } else {
        let timer: Retained<AnyObject> = msg_send![NSTimer::class(), scheduledTimerWithTimeInterval: 1.0f64 / 60. target: &**view selector: sel!(animationTick:) userInfo: std::ptr::null::<AnyObject>() repeats: true];
        Some(timer)
    };
    s.ticker = ticker;
    s.last_step = None;
}
/// Call after any state change: animate towards the new goals when the map is on
/// screen, otherwise arrive at once. Then request one redraw.
fn kick() {
    let view = STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        if animated(&s) && live(&s) {
            unsafe { start_loop(&mut s) }
        } else {
            stop_loop(&mut s);
            settle(&mut s);
        }
        s.overlay.clone()
    });
    if let Some(view) = view {
        view.setNeedsDisplay(true);
    }
}

// ── Relation geometry ────────────────────────────────────────────────────
fn bisect(f: impl Fn(f64) -> f64, mut lo: f64, mut hi: f64, target: f64) -> f64 {
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if f(mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.
}
/// World-space centre line of a relation, from the surface of its subject to the
/// surface of its object. Straight relations need two points: a straight line
/// stays straight under perspective.
fn link_path(s: &State, link: &Link) -> Option<Vec<Vec3>> {
    let (p, q) = (position(s, link.a), position(s, link.b));
    let (rp, rq) = (s.scale[link.a], s.scale[link.b]);
    if rp < 0.5 || rq < 0.5 {
        return None;
    }
    // The line starts just inside the sphere so no gap can open; an arrowhead
    // stops just outside so its tip touches the surface.
    let start = rp * 0.94;
    let end = if link.directed { rq * 1.02 } else { rq * 0.94 };
    let sample = |at: &dyn Fn(f64) -> Vec3, count: usize| -> Option<Vec<Vec3>> {
        if (at(0.5) - p).length() <= start || (at(0.5) - q).length() <= end {
            return None;
        }
        let t0 = bisect(|t| (at(t) - p).length(), 0., 0.5, start);
        let t1 = bisect(|t| -(at(t) - q).length(), 0.5, 1., -end);
        Some(
            (0..=count)
                .map(|i| at(t0 + (t1 - t0) * i as f64 / count as f64))
                .collect(),
        )
    };
    // Loops and bowed lanes open towards the viewer, so parallel relations fan
    // out on screen from any direction instead of hiding behind one another.
    let (right, up, toward) = s.rig.now.basis();
    if link.a == link.b {
        let radius = 48. + link.order as f64 * 24.;
        return sample(
            &|t: f64| {
                p + right * ((TAU * t).sin() * radius) + up * (radius - (TAU * t).cos() * radius)
            },
            40,
        );
    }
    let span = (q - p).length();
    let dir = (q - p).normalized();
    if link.lane == 0. {
        if span <= start + end + 1. {
            return None;
        }
        return Some(vec![p + dir * start, q - dir * end]);
    }
    let mut normal = dir.cross(toward);
    if normal.length() < 0.05 {
        // Seen end-on, any sideways direction will do.
        normal = dir.cross(up);
    }
    // The same lane keeps its side whichever end the relation starts from.
    let normal = normal.normalized() * link.sign;
    let control = (p + q) * 0.5 + normal * (link.lane * 92.);
    sample(
        &|t: f64| p * (1. - t).powi(2) + control * (2. * (1. - t) * t) + q * t * t,
        24,
    )
}
#[derive(Clone, Copy, Debug)]
struct ScreenPoint {
    x: f64,
    y: f64,
    /// Normalised device depth, linear across the screen.
    z: f64,
    /// Camera-space distance.
    depth: f64,
}
/// A visible stretch of a relation on screen.
struct Run {
    points: Vec<ScreenPoint>,
    /// Whether it still begins at the subject and ends at the object.
    from_start: bool,
    to_end: bool,
}
/// Project a world polyline, cutting it where it comes too close to the eye.
fn project_path(camera: &Camera, world: &[Vec3], w: f64, h: f64) -> Vec<Run> {
    let eye = camera.position();
    let toward = camera.basis().2;
    let depth = |p: Vec3| -(p - eye).dot(toward);
    let project = |p: Vec3| {
        camera.project(p, w, h).map(|v| ScreenPoint {
            x: v.x,
            y: v.y,
            z: (v.clip[2] / v.clip[3]) as f64,
            depth: v.depth,
        })
    };
    let push = |run: &mut Vec<ScreenPoint>, p: Vec3| {
        if let Some(v) = project(p) {
            run.push(v);
        }
    };
    let mut runs = Vec::new();
    let mut points: Vec<ScreenPoint> = Vec::new();
    let mut from_start = false;
    for (i, pair) in world.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        let (da, db) = (depth(a), depth(b));
        let cut = |target: f64| a.lerp(b, (target - da) / (db - da));
        match (da >= NEAR_CLIP, db >= NEAR_CLIP) {
            (true, true) => {
                if points.is_empty() {
                    from_start = i == 0;
                    push(&mut points, a);
                }
                push(&mut points, b);
            }
            (true, false) => {
                if points.is_empty() {
                    from_start = i == 0;
                    push(&mut points, a);
                }
                push(&mut points, cut(NEAR_CLIP));
                runs.push(Run {
                    points: std::mem::take(&mut points),
                    from_start,
                    to_end: false,
                });
            }
            (false, true) => {
                from_start = false;
                push(&mut points, cut(NEAR_CLIP));
                push(&mut points, b);
            }
            (false, false) => {}
        }
        if i + 2 == world.len() && points.len() > 1 {
            runs.push(Run {
                points: std::mem::take(&mut points),
                from_start,
                to_end: db >= NEAR_CLIP,
            });
        }
    }
    runs.retain(|run| run.points.len() > 1);
    runs
}
/// Outline of a sphere on screen: centre and radius in logical pixels.
type Disc = (f64, f64, f64);
fn silhouette(camera: &Camera, center: Vec3, radius: f64, w: f64, h: f64) -> Option<Disc> {
    let p = camera.project(center, w, h)?;
    let distance = (center - camera.position()).length();
    if distance <= radius * 1.02 {
        return None;
    }
    let grow = distance / (distance * distance - radius * radius).sqrt();
    Some((p.x, p.y, radius * p.radius_scale * grow))
}
/// End a run where it meets a sphere's outline, so a relation never draws across
/// the face of the concept it attaches to and its arrowhead always shows.
fn clip_to_outline(points: &mut Vec<ScreenPoint>, disc: Disc, at_end: bool) {
    if !at_end {
        points.reverse();
    }
    let inside = |p: &ScreenPoint| (p.x - disc.0).hypot(p.y - disc.1) < disc.2;
    let mut resolved = !points.last().is_some_and(inside);
    while !resolved && points.len() >= 2 {
        let (a, b) = (points[points.len() - 2], points[points.len() - 1]);
        if inside(&a) {
            points.pop();
            continue;
        }
        // Where a→b crosses the outline.
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let (fx, fy) = (a.x - disc.0, a.y - disc.1);
        let qa = dx * dx + dy * dy;
        let qb = 2. * (fx * dx + fy * dy);
        let qc = fx * fx + fy * fy - disc.2 * disc.2;
        let root = (qb * qb - 4. * qa * qc).max(0.).sqrt();
        let t = ((-qb - root) / (2. * qa)).clamp(0., 1.);
        *points.last_mut().unwrap() = ScreenPoint {
            x: a.x + dx * t,
            y: a.y + dy * t,
            z: a.z + (b.z - a.z) * t,
            depth: 1. / ((1. - t) / a.depth + t / b.depth),
        };
        resolved = true;
    }
    // Wholly over the face: nothing of it shows.
    if !resolved {
        points.clear();
    }
    if !at_end {
        points.reverse();
    }
}
fn ndc(p: (f64, f64), z: f64, w: f64, h: f64) -> [f32; 3] {
    [
        (p.0 / w * 2. - 1.) as f32,
        (p.1 / h * 2. - 1.) as f32,
        z as f32,
    ]
}
/// Constant-width screen-space ribbon with mitred joins and analytic edges.
fn ribbon(
    out: &mut Vec<FlatVertex>,
    run: &[ScreenPoint],
    half: f64,
    rgb: [f64; 3],
    alpha: f64,
    w: f64,
    h: f64,
    cap_end: bool,
    dashed: bool,
) {
    let points = run
        .iter()
        .enumerate()
        .filter(|(i, p)| *i == 0 || (p.x - run[i - 1].x).hypot(p.y - run[i - 1].y) > 0.05)
        .map(|(_, p)| *p)
        .collect::<Vec<_>>();
    if points.len() < 2 {
        return;
    }
    let direction = |a: &ScreenPoint, b: &ScreenPoint| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let l = dx.hypot(dy).max(1e-9);
        (dx / l, dy / l)
    };
    let total: f64 = points
        .windows(2)
        .map(|p| (p[1].x - p[0].x).hypot(p[1].y - p[0].y))
        .sum();
    let reach = half + AA_PAD;
    let mut travelled = 0.;
    let mut sides = Vec::with_capacity(points.len());
    for i in 0..points.len() {
        let before = (i > 0).then(|| direction(&points[i - 1], &points[i]));
        let after = (i + 1 < points.len()).then(|| direction(&points[i], &points[i + 1]));
        let (tx, ty) = match (before, after) {
            (Some(a), Some(b)) => {
                let (x, y) = (a.0 + b.0, a.1 + b.1);
                let l = x.hypot(y);
                if l < 1e-6 {
                    a
                } else {
                    (x / l, y / l)
                }
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => (1., 0.),
        };
        let (nx, ny) = (-ty, tx);
        // Widen the join so the ribbon keeps its width through a bend.
        let segment = before.or(after).unwrap_or((1., 0.));
        let miter = 1. / (nx * -segment.1 + ny * segment.0).abs().max(0.5);
        if i > 0 {
            travelled += (points[i].x - points[i - 1].x).hypot(points[i].y - points[i - 1].y);
        }
        let cap = if cap_end { total - travelled } else { 1000. };
        let fade = near_fade(points[i].depth);
        let color = [
            rgb[0] as f32,
            rgb[1] as f32,
            rgb[2] as f32,
            (alpha * fade) as f32,
        ];
        let vertex = |side: f64| FlatVertex {
            position: ndc(
                (
                    points[i].x + nx * reach * miter * side,
                    points[i].y + ny * reach * miter * side,
                ),
                points[i].z,
                w,
                h,
            ),
            distance: [
                (half - side * reach) as f32,
                (half + side * reach) as f32,
                cap as f32,
            ],
            color,
        };
        sides.push((travelled, vertex(1.), vertex(-1.)));
    }
    for pair in sides.windows(2) {
        let ((start, l0, r0), (end, l1, r1)) = (pair[0], pair[1]);
        if !dashed {
            out.extend([l0, r0, l1, r0, r1, l1]);
            continue;
        }
        let mut dash = (start / 12.).floor() * 12.;
        while dash < end {
            let a = start.max(dash);
            let b = end.min(dash + 7.);
            // Two halves keep both cap distances linear without extra buffers.
            let middle = dash + 3.5;
            for (a, b) in [(a, b.min(middle)), (a.max(middle), b)] {
                if b <= a {
                    continue;
                }
                let vertex = |p: FlatVertex, q: FlatVertex, at: f64| {
                    let t = ((at - start) / (end - start)) as f32;
                    let mut v = FlatVertex {
                        position: std::array::from_fn(|i| p.position[i] + (q.position[i] - p.position[i]) * t),
                        distance: std::array::from_fn(|i| p.distance[i] + (q.distance[i] - p.distance[i]) * t),
                        color: std::array::from_fn(|i| p.color[i] + (q.color[i] - p.color[i]) * t),
                    };
                    v.distance[2] = v.distance[2].min((at - dash).min(dash + 7. - at) as f32);
                    v
                };
                let (l0, r0, l1, r1) = (
                    vertex(l0, l1, a), vertex(r0, r1, a),
                    vertex(l0, l1, b), vertex(r0, r1, b),
                );
                out.extend([l0, r0, l1, r0, r1, l1]);
            }
            dash += 12.;
        }
    }
}
/// Anti-aliased screen-space triangle: grown by one pixel, with the distance to
/// each original edge carried to the fragment stage.
fn triangle(
    out: &mut Vec<FlatVertex>,
    points: [(f64, f64, f64); 3],
    rgb: [f64; 3],
    alpha: f64,
    w: f64,
    h: f64,
) {
    // Inside-positive distance to the edge opposite each vertex.
    let mut edges = [(0., 0., 0.); 3];
    for k in 0..3 {
        let (a, b, c) = (points[(k + 1) % 3], points[(k + 2) % 3], points[k]);
        let (mut nx, mut ny) = (-(b.1 - a.1), b.0 - a.0);
        let l = nx.hypot(ny);
        if l < 1e-6 {
            return;
        }
        nx /= l;
        ny /= l;
        let mut offset = -(nx * a.0 + ny * a.1);
        if nx * c.0 + ny * c.1 + offset < 0. {
            nx = -nx;
            ny = -ny;
            offset = -offset;
        }
        edges[k] = (nx, ny, offset);
    }
    let color = [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, alpha as f32];
    for k in 0..3 {
        // The grown corner lies one pixel outside both edges that meet here.
        let (e1, e2) = (edges[(k + 1) % 3], edges[(k + 2) % 3]);
        let det = e1.0 * e2.1 - e1.1 * e2.0;
        if det.abs() < 1e-9 {
            return;
        }
        let (c1, c2) = (-AA_PAD - e1.2, -AA_PAD - e2.2);
        let x = (c1 * e2.1 - c2 * e1.1) / det;
        let y = (e1.0 * c2 - e2.0 * c1) / det;
        let d = |e: (f64, f64, f64)| (e.0 * x + e.1 * y + e.2) as f32;
        out.push(FlatVertex {
            position: ndc((x, y), points[k].2, w, h),
            distance: [d(edges[0]), d(edges[1]), d(edges[2])],
            color,
        });
    }
}
/// Shorten a run by `length` pixels from its end, returning the removed tip and
/// the new end point where an arrowhead's base sits.
fn trim_end(run: &mut Vec<ScreenPoint>, length: f64) -> Option<(ScreenPoint, ScreenPoint)> {
    let tip = *run.last()?;
    let mut remaining = length;
    while run.len() >= 2 {
        let (a, b) = (run[run.len() - 2], run[run.len() - 1]);
        let segment = (b.x - a.x).hypot(b.y - a.y);
        if segment > remaining {
            let t = 1. - remaining / segment;
            let base = ScreenPoint {
                x: a.x + (b.x - a.x) * t,
                y: a.y + (b.y - a.y) * t,
                z: a.z + (b.z - a.z) * t,
                depth: 1. / ((1. - t) / a.depth + t / b.depth),
            };
            *run.last_mut().unwrap() = base;
            return Some((tip, base));
        }
        remaining -= segment;
        run.pop();
    }
    None
}
/// Refine a projected curve until no piece is longer than a few pixels, so a
/// bowed relation stays smooth however far the camera zooms in. The added points
/// lie on a Catmull-Rom spline through the projected samples.
fn smooth(points: &[ScreenPoint]) -> Vec<ScreenPoint> {
    if points.len() < 3 {
        return points.to_vec();
    }
    // Beyond either end the curve is continued by reflection.
    let last = points.len() as isize - 1;
    let at = |i: isize| {
        let mirror = |end: ScreenPoint, inner: ScreenPoint| ScreenPoint {
            x: 2. * end.x - inner.x,
            y: 2. * end.y - inner.y,
            ..end
        };
        if i < 0 {
            mirror(points[0], points[1])
        } else if i > last {
            mirror(points[last as usize], points[last as usize - 1])
        } else {
            points[i as usize]
        }
    };
    let mut out = Vec::with_capacity(points.len() * 2);
    out.push(points[0]);
    for i in 0..points.len() - 1 {
        let (p0, p1, p2, p3) = (
            at(i as isize - 1),
            points[i],
            points[i + 1],
            at(i as isize + 2),
        );
        let pieces = ((p2.x - p1.x).hypot(p2.y - p1.y) / 6.)
            .ceil()
            .clamp(1., 32.) as usize;
        for k in 1..=pieces {
            let t = k as f64 / pieces as f64;
            let spline = |a: f64, b: f64, c: f64, d: f64| {
                0.5 * (2. * b
                    + (c - a) * t
                    + (2. * a - 5. * b + 4. * c - d) * t * t
                    + (3. * b - a - 3. * c + d) * t * t * t)
            };
            out.push(ScreenPoint {
                x: spline(p0.x, p1.x, p2.x, p3.x),
                y: spline(p0.y, p1.y, p2.y, p3.y),
                z: p1.z + (p2.z - p1.z) * t,
                depth: 1. / ((1. - t) / p1.depth + t / p2.depth),
            });
        }
    }
    out
}
fn run_length(run: &[ScreenPoint]) -> f64 {
    run.windows(2)
        .map(|p| (p[1].x - p[0].x).hypot(p[1].y - p[0].y))
        .sum()
}
fn link_geometry(
    out: &mut Vec<FlatVertex>,
    camera: &Camera,
    world: &[Vec3],
    directed: bool,
    dashed: bool,
    stroke: Stroke,
    outlines: (Option<Disc>, Option<Disc>),
    w: f64,
    h: f64,
) {
    for run in project_path(camera, world, w, h) {
        let mut points = smooth(&run.points);
        if let (true, Some(disc)) = (run.from_start, outlines.0) {
            clip_to_outline(&mut points, disc, false);
        }
        if let (true, Some(disc)) = (run.to_end, outlines.1) {
            clip_to_outline(&mut points, disc, true);
        }
        if points.len() < 2 {
            continue;
        }
        let mut arrow = None;
        if directed && run.to_end {
            // Constant on screen, and never longer than the visible line.
            let length = (stroke.width * 4.2)
                .clamp(7., 11.)
                .min(run_length(&points) * 0.6);
            if length >= 3. {
                arrow = trim_end(&mut points, length - 0.5).map(|v| (v, length));
            }
        }
        ribbon(
            out,
            &points,
            stroke.width / 2.,
            stroke.rgb,
            stroke.alpha,
            w,
            h,
            arrow.is_some(),
            dashed,
        );
        if let Some(((tip, base), length)) = arrow {
            let (dx, dy) = (tip.x - base.x, tip.y - base.y);
            let l = dx.hypot(dy).max(1e-6);
            let (dx, dy) = (dx / l, dy / l);
            let back = (tip.x - dx * length, tip.y - dy * length);
            let half = length * 0.42;
            triangle(
                out,
                [
                    (tip.x, tip.y, tip.z),
                    (back.0 - dy * half, back.1 + dx * half, base.z),
                    (back.0 + dy * half, back.1 - dx * half, base.z),
                ],
                stroke.rgb,
                stroke.alpha * near_fade(tip.depth),
                w,
                h,
            );
        }
    }
}

// ── Picking ──────────────────────────────────────────────────────────────
/// Front-most concept under a point, with its camera-space depth.
fn hit_node(s: &State, p: NSPoint, size: NSSize) -> Option<(usize, f64)> {
    let camera = &s.rig.now;
    let (origin, dir) = camera.ray(p.x, p.y, size.width, size.height);
    let eye = camera.position();
    let toward = camera.basis().2;
    (0..s.nodes.len())
        .filter(|i| s.shown[*i] && s.scale[*i] > 1.)
        .filter_map(|i| {
            let center = position(s, i);
            camera.project(center, size.width, size.height)?;
            if near_fade((center - eye).length()) < 0.5 {
                return None;
            }
            ray_sphere(origin, dir, center, s.scale[i]).map(|t| (i, -dir.dot(toward) * t))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}
fn segment_distance(p: NSPoint, a: &ScreenPoint, b: &ScreenPoint) -> (f64, f64) {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let l = dx * dx + dy * dy;
    let t = if l < 1e-8 {
        0.
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / l).clamp(0., 1.)
    };
    (
        ((p.x - a.x - t * dx).powi(2) + (p.y - a.y - t * dy).powi(2)).sqrt(),
        1. / ((1. - t) / a.depth + t / b.depth),
    )
}
/// Front-most visible relation near a point. A relation competes with the sphere
/// under the pointer, so only one drawn in front of it can be picked.
fn hit_link(s: &State, p: NSPoint, size: NSSize, node_depth: Option<f64>) -> Option<usize> {
    let camera = &s.rig.now;
    let (w, h) = (size.width, size.height);
    let focal = h / (2. * (math::FOV_Y / 2.).tan());
    s.links
        .iter()
        .enumerate()
        .filter(|(i, link)| link_visible(s, link) && s.stroke_goal[*i].alpha > 0.)
        .filter_map(|(i, link)| {
            let world = link_path(s, link)?;
            project_path(camera, &world, w, h)
                .iter()
                .flat_map(|run| {
                    run.points
                        .windows(2)
                        .map(|v| segment_distance(p, &v[0], &v[1]))
                })
                .filter(|(d, z)| {
                    let tolerance = if node_depth.is_some() {
                        (focal * 0.85 / z + 2.).clamp(2., TAP_SLOP)
                    } else {
                        TAP_SLOP
                    };
                    d.is_finite()
                        && z.is_finite()
                        && *d < tolerance
                        && near_fade(*z) >= 0.5
                        && node_depth.is_none_or(|n| *z - 0.85 < n)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, z)| (i, z))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}
fn pick(s: &State, p: NSPoint, size: NSSize) -> Pick {
    let node = hit_node(s, p, size);
    if let Some(link) = hit_link(s, p, size, node.map(|(_, depth)| depth)) {
        Pick::Link(link)
    } else if let Some((node, _)) = node {
        Pick::Node(node)
    } else {
        Pick::Nothing
    }
}

// ── Labels ───────────────────────────────────────────────────────────────
const LABEL_FONT: f64 = 14.;
const EYEBROW_FONT: f64 = 10.5;
const LABEL_PAD_X: f64 = 8.;
const LABEL_HEIGHT: f64 = 24.;
const EYEBROW_HEIGHT: f64 = 13.;
const LABEL_MAX_TEXT: f64 = 152.;
const WEIGHT_SEMIBOLD: f64 = 0.3;
const WEIGHT_BOLD: f64 = 0.4;

#[cfg(not(test))]
fn measure(text: &str, size: f64, weight: f64) -> f64 {
    unsafe {
        let font = crate::font::weighted(size, weight);
        let attrs = NSDictionary::from_slices(&[NSFontAttributeName], &[&*font as &AnyObject]);
        NSString::from_str(text)
            .sizeWithAttributes(Some(&attrs))
            .width
    }
}
/// Unit tests run without AppKit text metrics.
#[cfg(test)]
fn measure(text: &str, size: f64, _weight: f64) -> f64 {
    text.chars()
        .map(|c| if c.is_ascii() { 0.56 } else { 1. })
        .sum::<f64>()
        * size
}
/// Text shortened with an ellipsis to fit `LABEL_MAX_TEXT`, and its real width.
fn fitted(
    cache: &mut HashMap<(String, u8), (String, f64)>,
    text: &str,
    size: f64,
    weight: f64,
) -> (String, f64) {
    let style = (size * 2.) as u8 + if weight > 0.35 { 128 } else { 0 };
    if let Some(hit) = cache.get(&(text.to_owned(), style)) {
        return hit.clone();
    }
    let mut shown = text.to_owned();
    let mut width = measure(&shown, size, weight);
    if width > LABEL_MAX_TEXT {
        let chars = text.chars().collect::<Vec<_>>();
        let mut keep = chars.len();
        while keep > 1 && width > LABEL_MAX_TEXT {
            keep -= 1;
            shown = chars[..keep]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_owned()
                + "…";
            width = measure(&shown, size, weight);
        }
    }
    if cache.len() > 4096 {
        cache.clear();
    }
    cache.insert((text.to_owned(), style), (shown.clone(), width));
    (shown, width)
}
struct LabelDraw {
    rect: NSRect,
    title: String,
    eyebrow: String,
    tone: Tone,
    alpha: f64,
}
struct Composition {
    spheres: Vec<SphereInstance>,
    halos: Vec<SphereInstance>,
    flat: Vec<FlatVertex>,
    labels: Vec<LabelDraw>,
}
fn overlap(a: NSRect, b: NSRect) -> bool {
    a.origin.x < b.origin.x + b.size.width
        && a.origin.x + a.size.width > b.origin.x
        && a.origin.y < b.origin.y + b.size.height
        && a.origin.y + a.size.height > b.origin.y
}
/// Label of one concept: priority, tone and whether it must show even if it
/// overlaps. Negative priority hides it. Mirrors the web map's label pool.
fn label_candidate(s: &State, i: usize, focus: Option<usize>) -> (i32, Tone, bool) {
    if let Some((a, b)) = s.selected_link.map(|l| (s.links[l].a, s.links[l].b)) {
        if s.selected_node.is_none() {
            return if i == a || i == b {
                (90, Tone::Default, true)
            } else if s.hovered == Some(i) {
                (96, Tone::Hover, true)
            } else {
                (-1, Tone::Default, false)
            };
        }
    }
    if focus == Some(i) {
        let tone = if s.selected_node.is_some() {
            Tone::Selected
        } else {
            Tone::Hover
        };
        return (100, tone, true);
    }
    if let Some(f) = focus {
        if s.adjacency[f].contains(&i) {
            return if s.eyebrow[i].is_empty() {
                (90, Tone::Default, true)
            } else {
                (95, Tone::Relation, true)
            };
        }
        // Beyond the web map: a dimmed concept still names itself under the pointer.
        return if s.hovered == Some(i) {
            (96, Tone::Hover, true)
        } else {
            (-1, Tone::Default, false)
        };
    }
    if s.hits[i] {
        return (80, Tone::Search, false);
    }
    (s.nodes[i].degree.min(12) as i32, Tone::Default, false)
}
fn label_rect(
    s: &mut State,
    i: usize,
    tone: Tone,
    eyebrow: &str,
    size: NSSize,
) -> Option<(NSRect, String, String)> {
    let p = position(s, i);
    // Anchored above the node in the world, like the web map.
    let anchor = s.rig.now.project(
        p + Vec3::new(0., s.scale[i] * 1.45, 0.),
        size.width,
        size.height,
    )?;
    let weight = if matches!(tone, Tone::Selected | Tone::Relation) {
        WEIGHT_BOLD
    } else {
        WEIGHT_SEMIBOLD
    };
    let name = s.nodes[i].name.clone();
    let (title, title_width) = fitted(&mut s.widths, &name, LABEL_FONT, weight);
    let (eyebrow, eyebrow_width) = if eyebrow.is_empty() {
        (String::new(), 0.)
    } else {
        fitted(&mut s.widths, eyebrow, EYEBROW_FONT, WEIGHT_BOLD)
    };
    let width = title_width.max(eyebrow_width) + LABEL_PAD_X * 2.;
    let height = LABEL_HEIGHT
        + if eyebrow.is_empty() {
            0.
        } else {
            EYEBROW_HEIGHT
        };
    Some((
        NSRect::new(
            NSPoint::new((anchor.x - width / 2.).round(), anchor.y.round()),
            NSSize::new(width.ceil(), height),
        ),
        title,
        eyebrow,
    ))
}
fn place_labels(s: &mut State, size: NSSize, dt: f64, snap: bool) -> Vec<LabelDraw> {
    let n = s.nodes.len();
    let focus = s.selected_node.or(s.hovered);
    let eye = s.rig.now.position();
    let mut order = (0..n)
        .map(|i| (i, label_candidate(s, i, focus)))
        .filter(|(i, c)| {
            c.0 >= 0
                && s.shown[*i]
                && s.scale[*i] > 1.
                && near_fade((position(s, *i) - eye).length()) >= 0.5
        })
        .collect::<Vec<_>>();
    order.sort_by(|a, b| {
        b.1 .0
            .cmp(&a.1 .0)
            .then(s.nodes[b.0].degree.cmp(&s.nodes[a.0].degree))
            .then(s.nodes[a.0].name.cmp(&s.nodes[b.0].name))
    });
    let required = order.iter().filter(|(_, c)| c.2).count();
    let budget = required.max(13);
    let safe = NSRect::new(
        NSPoint::new(s.insets.left, s.insets.bottom),
        NSSize::new(
            size.width - s.insets.left - s.insets.right,
            size.height - s.insets.top - s.insets.bottom,
        ),
    );
    let mut placed: Vec<NSRect> = Vec::new();
    let mut goal = vec![false; n];
    let mut rects: Vec<Option<(NSRect, String, String)>> = (0..n).map(|_| None).collect();
    for (i, (_, tone, must)) in order {
        if placed.len() >= budget {
            break;
        }
        let eyebrow = if tone == Tone::Relation {
            s.eyebrow[i].clone()
        } else {
            String::new()
        };
        let Some((rect, title, eyebrow_text)) = label_rect(s, i, tone, &eyebrow, size) else {
            continue;
        };
        // Keep labels out from under host controls.
        let (cx, cy) = (rect.origin.x + rect.size.width / 2., rect.origin.y);
        if cx < safe.origin.x
            || cx > safe.origin.x + safe.size.width
            || cy < safe.origin.y
            || cy > safe.origin.y + safe.size.height
        {
            continue;
        }
        let padded = NSRect::new(
            NSPoint::new(rect.origin.x - 4., rect.origin.y - 3.),
            NSSize::new(rect.size.width + 8., rect.size.height + 6.),
        );
        if !must && placed.iter().any(|other| overlap(padded, *other)) {
            continue;
        }
        placed.push(padded);
        goal[i] = true;
        s.label_style[i] = (tone, eyebrow);
        rects[i] = Some((rect, title, eyebrow_text));
    }
    let rate = if snap { 1. } else { 1. - (-dt / 0.045).exp() };
    let mut labels = Vec::new();
    s.labels_moving = false;
    for i in 0..n {
        let target = goal[i] as u8 as f64;
        s.labels_moving |= approach(&mut s.label_alpha[i], target, rate, 0.01);
        if s.label_alpha[i] <= 0.01 {
            continue;
        }
        // A label on its way out keeps the look it last had.
        let (tone, eyebrow) = s.label_style[i].clone();
        let Some((rect, title, eyebrow)) = rects[i]
            .take()
            .or_else(|| label_rect(s, i, tone, &eyebrow, size))
        else {
            continue;
        };
        labels.push(LabelDraw {
            rect,
            title,
            eyebrow,
            tone,
            alpha: s.label_alpha[i],
        });
    }
    // The hovered or selected relation names itself at the middle of its line.
    for (link, tone) in [
        (
            s.hovered_link.filter(|l| Some(*l) != s.selected_link),
            Tone::Hover,
        ),
        (s.selected_link, Tone::Selected),
    ] {
        let Some(link) = link else {
            continue;
        };
        let Some(world) = link_path(s, &s.links[link]) else {
            continue;
        };
        let Some(mid) = s
            .rig
            .now
            .project(world[world.len() / 2], size.width, size.height)
        else {
            continue;
        };
        if world.len() == 2 {
            // Two points: the middle of the straight line, not its far end.
            let Some(other) = s.rig.now.project(world[0], size.width, size.height) else {
                continue;
            };
            let label = s.links[link].label.clone();
            labels.push(link_label(
                s,
                ((mid.x + other.x) / 2., (mid.y + other.y) / 2.),
                &label,
                tone,
                size,
            ));
        } else {
            let label = s.links[link].label.clone();
            labels.push(link_label(s, (mid.x, mid.y), &label, tone, size));
        }
    }
    labels
}
fn link_label(s: &mut State, at: (f64, f64), label: &str, tone: Tone, size: NSSize) -> LabelDraw {
    let weight = if tone == Tone::Selected {
        WEIGHT_BOLD
    } else {
        WEIGHT_SEMIBOLD
    };
    let (title, width) = fitted(&mut s.widths, label, LABEL_FONT, weight);
    let width = (width + LABEL_PAD_X * 2.).ceil();
    LabelDraw {
        rect: NSRect::new(
            NSPoint::new(
                (at.0 - width / 2.)
                    .clamp(8., (size.width - width - 8.).max(8.))
                    .round(),
                (at.1 + 7.)
                    .clamp(8., (size.height - LABEL_HEIGHT - 8.).max(8.))
                    .round(),
            ),
            NSSize::new(width, LABEL_HEIGHT),
        ),
        title,
        eyebrow: String::new(),
        tone,
        alpha: 1.,
    }
}
/// Everything one image needs, from the current eased state.
fn compose(s: &mut State, size: NSSize, dt: f64, snap: bool) -> Composition {
    let (w, h) = (size.width, size.height);
    let camera = s.rig.now;
    let eye = camera.position();
    let n = s.nodes.len();
    let mut spheres = Vec::with_capacity(n);
    for i in 0..n {
        if s.scale[i] < 0.05 {
            continue;
        }
        let p = position(s, i);
        let fade = near_fade((p - eye).length());
        if fade <= 0. {
            continue;
        }
        spheres.push(SphereInstance {
            center: [p.x as f32, p.y as f32, p.z as f32],
            radius: s.scale[i] as f32,
            color: [
                s.color[i][0] as f32,
                s.color[i][1] as f32,
                s.color[i][2] as f32,
                fade as f32,
            ],
            params: [s.gloss[i] as f32, 0., 0., 0.],
        });
    }
    let mut halos = Vec::new();
    let tint = srgb(ACTIVE);
    for (slot, (size_factor, opacity)) in [(2.3, 0.72), (1.9, 0.42)].into_iter().enumerate() {
        let (Some(node), strength) = s.halo[slot] else {
            continue;
        };
        if strength <= 0.004 || node >= n || s.scale[node] < 0.05 {
            continue;
        }
        let p = position(s, node);
        halos.push(SphereInstance {
            center: [p.x as f32, p.y as f32, p.z as f32],
            radius: (s.scale[node] * size_factor) as f32,
            color: [
                tint[0] as f32,
                tint[1] as f32,
                tint[2] as f32,
                (opacity * strength * near_fade((p - eye).length())) as f32,
            ],
            params: [0.; 4],
        });
    }
    let mut flat = Vec::new();
    for (i, link) in s.links.iter().enumerate() {
        let stroke = s.stroke[i];
        if stroke.alpha < 0.01 {
            continue;
        }
        if let Some(world) = link_path(s, link) {
            let outline = |node: usize| silhouette(&camera, position(s, node), s.scale[node], w, h);
            link_geometry(
                &mut flat,
                &camera,
                &world,
                link.directed,
                link.inferred,
                stroke,
                (outline(link.a), outline(link.b)),
                w,
                h,
            );
        }
    }
    let labels = place_labels(s, size, dt, snap);
    Composition {
        spheres,
        halos,
        flat,
        labels,
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────
unsafe fn text(
    label: &str,
    p: (f64, f64),
    size: f64,
    weight: f64,
    color: [f64; 3],
    alpha: f64,
    center: bool,
) {
    let label = NSString::from_str(label);
    let font = crate::font::weighted(size, weight);
    let ink = NSColor::colorWithSRGBRed_green_blue_alpha(color[0], color[1], color[2], alpha);
    let attrs = NSDictionary::from_slices(
        &[NSFontAttributeName, NSForegroundColorAttributeName],
        &[&*font as &AnyObject, &*ink as &AnyObject],
    );
    let width = label.sizeWithAttributes(Some(&attrs)).width;
    label.drawAtPoint_withAttributes(
        NSPoint::new(if center { p.0 - width / 2. } else { p.0 }, p.1),
        Some(&attrs),
    );
}
unsafe fn fill_pill(rect: NSRect, color: [f64; 3], alpha: f64) {
    NSColor::colorWithSRGBRed_green_blue_alpha(color[0], color[1], color[2], alpha).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 8., 8.).fill();
}
unsafe fn stroke_pill(rect: NSRect, color: [f64; 3], alpha: f64) {
    let inset = NSRect::new(
        NSPoint::new(rect.origin.x + 0.5, rect.origin.y + 0.5),
        NSSize::new(rect.size.width - 1., rect.size.height - 1.),
    );
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(inset, 7.5, 7.5);
    path.setLineWidth(1.);
    NSColor::colorWithSRGBRed_green_blue_alpha(color[0], color[1], color[2], alpha).setStroke();
    path.stroke();
}
/// Solid backplates, never glass: a label must stay legible over any sphere.
unsafe fn draw_label(label: &LabelDraw) {
    let a = label.alpha;
    let brand = srgb(ACTIVE);
    let ink = srgb(INK);
    let middle = label.rect.origin.x + label.rect.size.width / 2.;
    let baseline = label.rect.origin.y + 3.;
    match label.tone {
        Tone::Selected => {
            fill_pill(label.rect, brand, a);
            text(
                &label.title,
                (middle, baseline),
                LABEL_FONT,
                WEIGHT_BOLD,
                [1.; 3],
                a,
                true,
            );
        }
        Tone::Relation => {
            fill_pill(label.rect, [1.; 3], 0.92 * a);
            fill_pill(label.rect, brand, 0.11 * a);
            if !label.eyebrow.is_empty() {
                text(
                    &label.eyebrow,
                    (middle, baseline + LABEL_HEIGHT - 5.),
                    EYEBROW_FONT,
                    WEIGHT_BOLD,
                    brand,
                    0.82 * a,
                    true,
                );
            }
            text(
                &label.title,
                (middle, baseline),
                LABEL_FONT,
                WEIGHT_BOLD,
                brand,
                a,
                true,
            );
        }
        tone => {
            fill_pill(label.rect, [1.; 3], 0.86 * a);
            match tone {
                Tone::Hover => stroke_pill(label.rect, [0.122, 0.133, 0.239], 0.16 * a),
                Tone::Search => stroke_pill(label.rect, brand, 0.48 * a),
                _ => {}
            }
            text(
                &label.title,
                (middle, baseline),
                LABEL_FONT,
                WEIGHT_SEMIBOLD,
                ink,
                a,
                true,
            );
        }
    }
}
unsafe fn draw_overlay(s: &State, labels: &[LabelDraw], size: NSSize) {
    for label in labels {
        draw_label(label);
    }
    if s.nodes.is_empty() {
        text(
            "자료에서 시작하는 3D 지식지도",
            (size.width / 2., size.height / 2. + 10.),
            22.,
            WEIGHT_SEMIBOLD,
            [0.2, 0.2, 0.32],
            1.,
            true,
        );
        text(
            "자료를 추가하면 개념과 관계가 공간에 펼쳐집니다.",
            (size.width / 2., size.height / 2. - 22.),
            14.,
            0.,
            [0.47, 0.48, 0.56],
            1.,
            true,
        );
    }
    text(
        "드래그 · 3D 회전   스크롤/핀치 · 줌   개념 드래그 · 놓으면 복귀   더블클릭 · 전체 보기",
        (18., 14.),
        10.,
        0.,
        [0.47, 0.48, 0.58],
        1.,
        false,
    );
}
/// The projection centre sits in the middle of the area host controls leave free.
fn apply_insets(s: &mut State) {
    s.rig.set_shift(
        (s.insets.left - s.insets.right) / 2.,
        (s.insets.bottom - s.insets.top) / 2.,
    );
}
// A requested frame updates CPU labels and GPU geometry together even when
// accessibility invokes a visible background window. Only the display link
// stops while inactive; no timer or redraw runs just because the app idles.
unsafe fn draw(b: NSRect) {
    if b.size.width <= 0. || b.size.height <= 0. {
        return;
    }
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        s.last_bounds = b.size;
        apply_insets(&mut s);
        let on_screen = animated(&s) && live(&s);
        if s.entry_pending && on_screen {
            // Concepts open out from the centre the first time the map appears.
            s.entry_pending = false;
            for i in 0..s.nodes.len() {
                s.now[i] = s.goal[i] * 0.62;
                s.scale[i] = s.scale_goal[i] * 0.25;
            }
            s.from = s.now.clone();
            s.tween = Some(0.);
            s.label_alpha.fill(0.);
            start_loop(&mut s);
        }
        let now = Instant::now();
        let dt = s
            .last_draw
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(1. / 60.)
            .min(0.064);
        s.last_draw = Some(now);
        let composition = compose(&mut s, b.size, dt, !on_screen);
        s.draws += 1;
        s.work += now.elapsed().as_secs_f64();
        if s.labels_moving && on_screen {
            start_loop(&mut s);
        }
        if let Some(renderer) = &s.metal {
            let visible = s
                .overlay
                .as_ref()
                .and_then(|v| v.window())
                .is_some_and(|w| {
                    w.isVisible() && w.occlusionState().contains(NSWindowOcclusionState::Visible)
                });
            if visible {
                renderer.draw(&metal::Scene {
                    camera: &s.rig.now,
                    size: (b.size.width, b.size.height),
                    spheres: &composition.spheres,
                    halos: &composition.halos,
                    flat: &composition.flat,
                });
            }
        } else {
            text(
                "Metal 3D 렌더러를 시작하지 못했습니다",
                (b.size.width / 2., b.size.height / 2.),
                17.,
                0.,
                [0.5, 0.18, 0.2],
                1.,
                true,
            );
            if let Some(error) = metal::last_error() {
                text(
                    &error.chars().take(100).collect::<String>(),
                    (b.size.width / 2., b.size.height / 2. - 26.),
                    11.,
                    0.,
                    [0.5, 0.25, 0.25],
                    1.,
                    true,
                );
            }
        }
        draw_overlay(&s, &composition.labels, b.size);
    });
}

// ── Input ────────────────────────────────────────────────────────────────
fn set_hover(s: &mut State, target: Pick) -> bool {
    let (node, link) = match target {
        Pick::Node(i) => (Some(i), None),
        Pick::Link(i) => (None, Some(i)),
        Pick::Nothing => (None, None),
    };
    if s.hovered == node && s.hovered_link == link {
        return false;
    }
    s.hovered = node;
    s.hovered_link = link;
    apply_highlight(s);
    true
}
/// The pointer left the map. A concept being dragged goes home, and the rest of
/// that press does nothing: releasing it is neither a drag nor a tap. An orbit
/// in progress simply continues, since AppKit keeps delivering the drag.
fn leave(s: &mut State) {
    if let Some(drag) = &mut s.drag {
        if drag.node.take().is_some() {
            drag.offset = Vec3::ZERO;
            drag.dead = true;
        }
    }
    set_hover(s, Pick::Nothing);
}
/// Returns true when the press that just ended was a tap.
fn release(s: &mut State) -> bool {
    // Releasing a dragged concept lets it ease back to its place.
    s.drag
        .take()
        .is_some_and(|drag| !drag.dead && drag.moved < TAP_SLOP)
}
fn tap(s: &mut State, target: Pick) {
    match target {
        Pick::Link(_) => clear_selection(s),
        Pick::Node(node) => select_node(s, node),
        Pick::Nothing => clear_selection(s),
    }
}
fn drag_node(s: &mut State, dx: f64, dy: f64, size: NSSize) {
    let Some(node) = s.drag.as_ref().and_then(|d| d.node) else {
        return;
    };
    let camera = s.rig.now;
    if let Some(projected) = camera.project(s.now[node], size.width, size.height) {
        // A delta in the camera plane at the node's own depth keeps the grabbed
        // point exactly under the pointer.
        let offset = camera.screen_delta(dx, dy, projected.depth, size.height);
        s.now[node] = s.now[node] + offset;
        if let Some(drag) = &mut s.drag {
            drag.offset = drag.offset + offset;
        }
    }
}
/// Zoom from a scroll delta in pixels; scrolling up zooms in, as on the web map.
fn scroll_zoom(pixels: f64) -> f64 {
    (pixels * 0.0016).exp()
}
define_class!(
    #[unsafe(super=NSView)]
    #[thread_kind=MainThreadOnly]
    #[ivars=()]
    struct GraphView;
    unsafe impl NSObjectProtocol for GraphView {}
    impl GraphView {
        #[unsafe(method(graphActivated:))]
        fn graph_activated(&self, _notification: &NSNotification) {
            kick();
        }
        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            false
        }
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _r: NSRect) {
            unsafe { draw(self.bounds()) }
        }
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking(&self) {
            unsafe {
                for area in self.trackingAreas().iter() {
                    self.removeTrackingArea(&area);
                }
                let area = NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    self.bounds(),
                    NSTrackingAreaOptions::MouseMoved
                        | NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveInKeyWindow
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(self),
                    None,
                );
                self.addTrackingArea(&area);
            }
        }
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let size = self.bounds().size;
            let (changed, over) = STATE.with(|cell| {
                let mut s = cell.borrow_mut();
                if s.drag.is_some() {
                    return (false, false);
                }
                let target = pick(&s, p, size);
                (set_hover(&mut s, target), target != Pick::Nothing)
            });
            if over {
                NSCursor::pointingHandCursor().set()
            } else {
                NSCursor::openHandCursor().set()
            }
            if changed {
                kick();
            }
        }
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            STATE.with(|cell| {
                let mut s = cell.borrow_mut();
                leave(&mut s);
            });
            kick();
        }
        #[unsafe(method(deleteGraphNode:))]
        fn delete_graph_node(&self, item:&NSMenuItem) {
            if let Some(id)=item.representedObject().and_then(|o|o.downcast::<NSString>().ok()) {
                command(&format!("delete_node:{id}"));
            }
        }
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event:&NSEvent) {
            let point=self.convertPoint_fromView(event.locationInWindow(),None);
            let id=STATE.with(|s|{let s=s.borrow();hit_node(&s,point,self.bounds().size).map(|(i,_)|s.nodes[i].id.clone())});
            if let Some(id)=id {unsafe{
                let menu=NSMenu::new(self.mtm());
                let item=NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(self.mtm()),&NSString::from_str("삭제…"),Some(sel!(deleteGraphNode:)),&NSString::from_str(""));
                item.setTarget(Some(self));
                item.setRepresentedObject(Some(&NSString::from_str(&id)));
                menu.addItem(&item);
                NSMenu::popUpContextMenu_withEvent_forView(&menu,event,self);
            }}
        }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
            }
            NSCursor::closedHandCursor().set();
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let size = self.bounds().size;
            STATE.with(|cell| {
                let mut s = cell.borrow_mut();
                let node = hit_node(&s, p, size).map(|(i, _)| i);
                s.drag = Some(Drag {
                    node,
                    offset: Vec3::ZERO,
                    last: p,
                    moved: 0.,
                    dead: false,
                });
            });
        }
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let size = self.bounds().size;
            STATE.with(|cell| {
                let mut s = cell.borrow_mut();
                let Some(drag) = &mut s.drag else {
                    return;
                };
                let (dx, dy) = (p.x - drag.last.x, p.y - drag.last.y);
                drag.moved += dx.abs() + dy.abs();
                drag.last = p;
                if drag.dead {
                    return;
                }
                if drag.node.is_some() {
                    drag_node(&mut s, dx, dy, size);
                } else {
                    s.rig.orbit(dx, -dy);
                }
            });
            kick();
        }
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let size = self.bounds().size;
            STATE.with(|cell| {
                let mut s = cell.borrow_mut();
                if !release(&mut s) {
                    return;
                }
                let now = Instant::now();
                let double = s
                    .last_tap
                    .is_some_and(|t| now.duration_since(t).as_secs_f64() < DOUBLE_TAP);
                if double {
                    s.last_tap = None;
                    reset_view(&mut s);
                } else {
                    s.last_tap = Some(now);
                    let target = hit_node(&s, p, size)
                        .map(|(i, _)| Pick::Node(i))
                        .unwrap_or(Pick::Nothing);
                    tap(&mut s, target);
                }
            });
            NSCursor::openHandCursor().set();
            kick();
        }
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            // A wheel notch arrives in lines; browsers count one as 40 pixels.
            let pixels = if event.hasPreciseScrollingDeltas() {
                event.scrollingDeltaY()
            } else {
                event.deltaY() * 40.
            };
            zoom(scroll_zoom(pixels));
        }
        #[unsafe(method(magnifyWithEvent:))]
        fn magnify(&self, event: &NSEvent) {
            zoom((1. + event.magnification()).max(0.05));
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let key = event
                .charactersIgnoringModifiers()
                .map(|s| s.to_string())
                .unwrap_or_default();
            match key.as_str() {
                "0" | "r" | "R" => {
                    fit();
                    command("overview")
                }
                "+" | "=" => zoom(1. / 0.86),
                "-" | "_" => zoom(1. / 1.16),
                _ => {
                    if event.keyCode() == 53 {
                        if selection().is_none() {
                            command("hide_nodes");
                        }
                        select("");
                    }
                }
            }
        }
        #[unsafe(method(animationTick:))]
        fn animation_tick(&self, _sender: &AnyObject) {
            STATE.with(|cell| {
                let mut s = cell.borrow_mut();
                if !animated(&s) || !live(&s) {
                    // Hidden, occluded or in the background: arrive and stop.
                    stop_loop(&mut s);
                    settle(&mut s);
                    return;
                }
                let now = Instant::now();
                let dt = s
                    .last_step
                    .map(|t| now.duration_since(t).as_secs_f64())
                    .unwrap_or(1. / 60.)
                    .min(0.064);
                s.last_step = Some(now);
                s.ticks += 1;
                let moving = step(&mut s, dt) | s.labels_moving;
                s.work += now.elapsed().as_secs_f64();
                if !moving {
                    stop_loop(&mut s);
                }
            });
            self.setNeedsDisplay(true);
        }
    }
);

// ── Public API ───────────────────────────────────────────────────────────
pub unsafe fn create(frame: NSRect, data: Value) -> Retained<NSView> {
    dispose();
    let mtm = MainThreadMarker::new().unwrap();
    let container = NSView::initWithFrame(NSView::alloc(mtm), frame);
    container.setClipsToBounds(true);
    container.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let bounds = NSRect::new(NSPoint::new(0., 0.), frame.size);
    let renderer = Metal::new(bounds);
    if let Some(m) = &renderer {
        container.addSubview(&m.view);
    }
    let allocated = GraphView::alloc(mtm).set_ivars(());
    let overlay: Retained<GraphView> = msg_send![super(allocated),initWithFrame:bounds];
    overlay.setClipsToBounds(true);
    overlay.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    container.addSubview(&overlay);
    let notifications = NSNotificationCenter::defaultCenter();
    for name in [
        NSApplicationDidBecomeActiveNotification,
        NSWindowDidChangeOcclusionStateNotification,
    ] {
        notifications.addObserver_selector_name_object(
            &overlay,
            sel!(graphActivated:),
            Some(name),
            None,
        );
    }
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        s.metal = renderer;
        s.overlay = Some(overlay);
        s.last_bounds = frame.size;
        // The host draws its search and relation controls over the top edge and
        // the gesture hint sits at the bottom.
        s.insets = Insets {
            top: 48.,
            right: 0.,
            bottom: 30.,
            left: 0.,
        };
        apply_insets(&mut s);
        load(&mut s, data);
        s.entry_pending = true;
    });
    kick();
    container
}
/// Area of the map covered by host controls, in logical pixels from each edge.
/// The graph centre and its labels keep out of it.
#[allow(dead_code)]
pub fn set_safe_area(top: f64, right: f64, bottom: f64, left: f64) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        s.insets = Insets {
            top: top.max(0.),
            right: right.max(0.),
            bottom: bottom.max(0.),
            left: left.max(0.),
        };
        apply_insets(&mut s);
        if s.selected_node.is_some() {
            frame_focus(&mut s, true);
        } else {
            frame_all(&mut s, true);
        }
    });
    kick();
}
pub fn update(data: Value) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        stop_loop(&mut s);
        load(&mut s, data);
        s.entry_pending = true;
    });
    kick();
}
/// Route graph shortcuts to the interactive overlay, never its plain container.
/// Release the state borrow before AppKit changes responders and invokes callbacks.
pub fn focus_input() -> bool {
    let overlay = STATE.with(|cell| cell.borrow().overlay.clone());
    overlay.is_some_and(|view| {
        view.window()
            .is_some_and(|window| window.makeFirstResponder(Some(&*view)))
    })
}

pub fn take_node_selection() -> bool {
    STATE.with(|s| std::mem::take(&mut s.borrow_mut().node_selection_pending))
}

pub fn selection() -> Option<Value> {
    STATE.with(|s| s.borrow().selected.clone())
}
/// Select a concept and focus it; an empty id deselects and returns to the
/// overview. Selecting what is already selected does nothing.
pub fn select(id: &str) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        if id.is_empty() {
            clear_selection(&mut s);
        } else if let Some(node) = s.index.get(id).copied() {
            select_node(&mut s, node);
        }
    });
    kick();
}
/// Highlight matching concepts. Typing never selects or moves the camera.
pub fn search(query: &str) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        s.query = query.into();
        refresh_hits(&mut s);
        apply_highlight(&mut s);
    });
    kick();
}
pub fn filter(relation: &str) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        s.filter = relation.into();
        refresh_shown(&mut s);
        apply_highlight(&mut s);
        if s.selected_node.is_none() {
            // The shown subset changed: frame it.
            frame_all(&mut s, true);
        }
    });
    kick();
}
pub fn focus_unit(unit: &str) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        if !unit.is_empty()
            && !s.nodes.iter().any(|node| node.unit.as_deref() == Some(unit))
        {
            return;
        }
        s.unit_focus = unit.into();
        forget_selection(&mut s);
        refresh_shown(&mut s);
        apply_highlight(&mut s);
        overview(&mut s, true);
    });
    kick();
}

pub fn fit() {
    STATE.with(|cell| reset_view(&mut cell.borrow_mut()));
    kick();
}
/// Lay the selected concept's neighbourhood out again, facing the camera as it is now.
pub fn focus() {
    STATE.with(|cell| focus_on(&mut cell.borrow_mut(), true));
    kick();
}
pub fn rotate(radians: f64) {
    STATE.with(|cell| cell.borrow_mut().rig.rotate(radians));
    kick();
}
/// Factors above one zoom in.
pub fn zoom(factor: f64) {
    if !factor.is_finite() || factor <= 0. {
        return;
    }
    STATE.with(|cell| cell.borrow_mut().rig.zoom(factor));
    kick();
}
pub fn dispose() {
    UI_COMMAND.with(|c| *c.borrow_mut() = None);
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        stop_loop(&mut s);
        if let Some(overlay) = &s.overlay {
            unsafe {
                NSNotificationCenter::defaultCenter().removeObserver(overlay);
            }
        }
        *s = State::default();
    });
}
#[allow(dead_code)]
pub fn metal_available() -> bool {
    STATE.with(|s| s.borrow().metal.is_some())
}
#[allow(dead_code)]
pub unsafe fn metal_smoke() -> Result<metal::SmokeReport, String> {
    metal::smoke_readback()
}

/// Preserve a workspace camera and focused layout while showing source evidence.
pub fn snapshot_view() -> Value {
    STATE.with(|cell| {
        let s = cell.borrow();
        let camera = s.rig.goal;
        let positions = s
            .nodes
            .iter()
            .zip(&s.goal)
            .map(|(node, p)| (node.id.clone(), json!([p.x, p.y, p.z])))
            .collect::<serde_json::Map<_, _>>();
        json!({"yaw":camera.yaw.rem_euclid(TAU),"pitch":camera.pitch,"distance":camera.distance,"pivot":[camera.pivot.x,camera.pivot.y,camera.pivot.z],"positions":positions,"selected":selected_id(&s),"query":s.query,"filter":s.filter})
    })
}
fn saved_vec(v: &Value) -> Option<Vec3> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    let p = Vec3::new(a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?);
    if p.finite() && p.length() < 100000. {
        Some(p)
    } else {
        None
    }
}
pub fn restore_view(value: &Value) {
    if !value.is_object() {
        return;
    }
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        stop_loop(&mut s);
        s.entry_pending = false;
        s.drag = None;
        if let Some(q) = value["query"].as_str() {
            s.query = q.to_owned();
        }
        if let Some(f) = value["filter"].as_str() {
            s.filter = f.to_owned();
        }
        s.selected_link = None;
        s.selected_node = value["selected"]
            .as_str()
            .and_then(|id| s.index.get(id).copied());
        s.selected = s.selected_node.and_then(|node| selection_for(&s, node));
        refresh_hits(&mut s);
        refresh_shown(&mut s);
        apply_highlight(&mut s);
        // Recompute the ego layout for its radius and limits, then put every
        // concept exactly where it was saved.
        if s.selected_node.is_some() {
            focus_on(&mut s, false);
        } else {
            overview(&mut s, false);
        }
        if let Some(positions) = value["positions"].as_object() {
            for (id, p) in positions {
                if let (Some(i), Some(p)) = (s.index.get(id).copied(), saved_vec(p)) {
                    s.goal[i] = p;
                }
            }
        }
        let mut camera = s.rig.goal;
        if let Some(y) = value["yaw"].as_f64().filter(|v| v.is_finite()) {
            camera.yaw = y.rem_euclid(TAU);
        }
        if let Some(p) = value["pitch"].as_f64().filter(|v| v.is_finite()) {
            camera.pitch = p.clamp(-math::PITCH_LIMIT, math::PITCH_LIMIT);
        }
        if let Some(d) = value["distance"].as_f64().filter(|v| v.is_finite()) {
            camera.distance = d;
        }
        if let Some(p) = saved_vec(&value["pivot"]) {
            camera.pivot = p;
        }
        s.rig.travel(camera, false);
        s.float_basis = camera.basis();
        settle(&mut s);
    });
    kick();
}

// ── Offscreen capture (quality checks and documentation; opens no window) ─
/// Arrive at every animation goal now. Captures use this instead of waiting.
#[allow(dead_code)]
pub fn settle_now() {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        stop_loop(&mut s);
        s.entry_pending = false;
        settle(&mut s);
    });
}
/// Hover a concept as the pointer would. For captures only.
#[allow(dead_code)]
pub fn hover_for_capture(id: &str) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        let target = s
            .index
            .get(id)
            .copied()
            .map(Pick::Node)
            .unwrap_or(Pick::Nothing);
        set_hover(&mut s, target);
        settle(&mut s);
    });
}
/// Hover a relation by assertion id. For captures only.
#[allow(dead_code)]
pub fn hover_relation_for_capture(assertion_id: &str) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        let link = s
            .links
            .iter()
            .position(|l| s.data["assertions"][l.assertion]["id"].as_str() == Some(assertion_id));
        if let Some(link) = link {
            set_hover(&mut s, Pick::Link(link));
        }
        settle(&mut s);
    });
}
/// Render the current map, with its labels, exactly as the screen pipeline would.
/// Returns BGRA rows (top first) of `width*scale` by `height*scale` pixels.
#[allow(dead_code)]
pub unsafe fn capture(width: f64, height: f64, scale: f64) -> Result<Vec<u8>, String> {
    let size = NSSize::new(width, height);
    let (pw, ph) = ((width * scale) as usize, (height * scale) as usize);
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        s.last_bounds = size;
        apply_insets(&mut s);
        let composition = compose(&mut s, size, 0., true);
        let renderer = s.metal.as_ref().ok_or("Metal 렌더러가 없습니다")?;
        let mut pixels = renderer.offscreen(
            &metal::Scene {
                camera: &s.rig.now,
                size: (width, height),
                spheres: &composition.spheres,
                halos: &composition.halos,
                flat: &composition.flat,
            },
            pw,
            ph,
        )?;
        let rep = metal::rgba_bitmap(pw, ph).ok_or("라벨 비트맵 생성 실패")?;
        std::ptr::write_bytes(rep.bitmapData(), 0, rep.bytesPerRow() as usize * ph);
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)
            .ok_or("라벨 그래픽 컨텍스트 생성 실패")?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&context));
        let transform = NSAffineTransform::transform();
        transform.scaleBy(scale);
        transform.concat();
        draw_overlay(&s, &composition.labels, size);
        context.flushGraphics();
        NSGraphicsContext::restoreGraphicsState_class();
        // Premultiplied RGBA labels over the BGRA map.
        let (data, row) = (rep.bitmapData(), rep.bytesPerRow() as usize);
        for y in 0..ph {
            for x in 0..pw {
                let src = data.add(y * row + x * 4);
                let alpha = *src.add(3) as u32;
                if alpha == 0 {
                    continue;
                }
                let dst = &mut pixels[(y * pw + x) * 4..(y * pw + x) * 4 + 4];
                for (d, c) in [(0usize, 2usize), (1, 1), (2, 0)] {
                    dst[d] = (*src.add(c) as u32 + dst[d] as u32 * (255 - alpha) / 255).min(255) as u8;
                }
            }
        }
        Ok(pixels)
    })
}
/// Set the camera directly. For captures only.
#[allow(dead_code)]
pub fn camera_for_capture(yaw: f64, pitch: f64, distance: Option<f64>) {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        let mut camera = s.rig.goal;
        camera.yaw = yaw;
        camera.pitch = pitch;
        s.rig.travel(camera, false);
        if let Some(distance) = distance {
            // Through the same limits as a wheel or pinch.
            let factor = s.rig.goal.distance / distance;
            s.rig.zoom(factor);
        }
        settle(&mut s);
    });
}
#[allow(dead_code)]
pub fn camera_distance() -> f64 {
    STATE.with(|cell| cell.borrow().rig.goal.distance)
}
/// Let the map animate in a visible window of an app that is not frontmost.
/// The on-screen self-test uses it so it never has to take keyboard focus.
#[allow(dead_code)]
pub fn allow_background_motion(allow: bool) {
    BACKGROUND_MOTION.set(allow);
}
/// Eased state as numbers, for the on-screen self-test.
#[allow(dead_code)]
pub fn debug_snapshot(node: &str) -> Value {
    STATE.with(|cell| {
        let s = cell.borrow();
        let size = dimensions(&s);
        let i = s.index.get(node).copied();
        let screen = i.and_then(|i| s.rig.now.project(position(&s, i), size.width, size.height));
        let presented = s
            .metal
            .as_ref()
            .map(|m| m.statistics().presented)
            .unwrap_or(0);
        json!({
            "running": s.ticker.is_some(),
            "ticks": s.ticks,
            "draws": s.draws,
            "presented": presented,
            "work": s.work,
            "distance": s.rig.now.distance,
            "goal_distance": s.rig.goal.distance,
            "yaw": s.rig.now.yaw,
            "pivot": s.rig.now.pivot.length(),
            "tween": s.tween,
            "selected": selected_id(&s),
            "kind": s.selected.as_ref().map(|v| v["kind"].clone()),
            "hovered": s.hovered.map(|i| s.nodes[i].id.clone()),
            "hovered_link": s.hovered_link,
            "x": screen.map(|p| p.x),
            "y": screen.map(|p| p.y),
            "scale": i.map(|i| s.scale[i]),
            "color": i.map(|i| s.color[i][0]),
            "home": i.map(|i| (s.now[i] - s.goal[i]).length()),
            "floating": i.map(|i| s.float_offset[i].length()),
            "label": i.map(|i| s.label_alpha[i]),
            "halo": s.halo[0].1,
            "hover_halo": s.halo[1].1,
        })
    })
}
#[allow(dead_code)]
pub unsafe fn probe_sphere(size: usize, radius_px: f64) -> Result<Vec<u8>, String> {
    metal::probe_sphere(size, radius_px)
}
/// One straight relation through the pivot at a given camera distance, for
/// checking that its width on screen does not change with zoom.
#[allow(dead_code)]
pub unsafe fn probe_relation(size: usize, distance: f64, width: f64) -> Result<Vec<u8>, String> {
    let renderer = metal::probe_renderer()?;
    let camera = Camera {
        yaw: 0.,
        pitch: 0.,
        distance,
        ..Camera::default()
    };
    let mut flat = Vec::new();
    link_geometry(
        &mut flat,
        &camera,
        &[Vec3::new(-400., 0., 0.), Vec3::new(400., 0., 0.)],
        false,
        false,
        Stroke {
            rgb: srgb(INK),
            alpha: 1.,
            width,
        },
        (None, None),
        size as f64,
        size as f64,
    );
    renderer.offscreen(
        &metal::Scene {
            camera: &camera,
            size: (size as f64, size as f64),
            spheres: &[],
            halos: &[],
            flat: &flat,
        },
        size,
        size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> State {
        let mut s = State::default();
        load(
            &mut s,
            json!({"concepts":[{"id":"a","name":"광 합 성","aliases":["Photosynthesis"],"definition":"빛"},{"id":"b","name":"엽록체"},{"id":"c","name":"ATP"},{"id":"d","name":"세포"}],"relations":[{"id":"r","label":"생성","directed":true},{"id":"s","label":"포함","directed":false}],"assertions":[{"id":"e1","subject":"a","object":"b","relation":"r","negated":true,"condition":"빛이 있을 때","evidence":[{"source_id":"source1","quote":"빛이 있을 때"}]},{"id":"e2","subject":"b","object":"a","relation":"r","evidence":[{"source_id":"source2"}]},{"id":"e3","subject":"c","object":"d","relation":"s","evidence":[]},{"id":"e4","subject":"a","object":"a","relation":"r","evidence":[]}]}),
        );
        s
    }
    fn subject_sample() -> State {
        let mut data = sample().data;
        data["layer"] = json!("subject");
        data["units"] = json!([
            {"id":"second","position":1},
            {"id":"first","position":0}
        ]);
        data["concepts"][0]["unit"] = json!("first");
        data["concepts"][1]["unit"] = json!("second");
        data["assertions"][2]["basis"] = json!("inferred");
        data["assertions"][2]["reason"] = json!("앞 개념으로 뒤 개념을 설명할 수 있어요");
        let mut s = State::default();
        load(&mut s, data);
        s
    }

    #[test]
    fn subject_units_use_position_colors_and_leave_unassigned_nodes_unchanged() {
        let s = subject_sample();
        assert_eq!(s.color_goal[node(&s, "a")], to_linear(unit_color(0)));
        assert_eq!(s.color_goal[node(&s, "b")], to_linear(unit_color(1)));
        assert_ne!(s.color_goal[node(&s, "a")], s.color_goal[node(&s, "b")]);
        assert_eq!(s.color_goal[node(&s, "c")], to_linear(srgb(ACTIVE)));
        assert_eq!(unit_color(0), unit_color(10));
    }

    #[test]
    fn legacy_and_material_layers_ignore_subject_fields() {
        let old = sample();
        assert_eq!(old.nodes.len(), 4);
        assert_eq!(old.links.len(), 4);
        assert!(old.nodes.iter().all(|node| node.color == to_linear(srgb(ACTIVE))));
        assert!(old.links.iter().all(|link| !link.inferred && link_visible(&old, link)));
        for layer in [Value::Null, json!("material")] {
            let mut data = subject_sample().data;
            data["layer"] = layer;
            let mut s = State::default();
            load(&mut s, data);
            assert_eq!(s.base, old.base);
            assert_eq!(s.color_goal, old.color_goal);
            assert_eq!(s.stroke_goal, old.stroke_goal);
            assert!(s.nodes.iter().all(|node| node.unit.is_none()));
            assert!(s.links.iter().all(|link| !link.inferred));
        }
    }

    #[test]
    fn unit_focus_frames_members_and_yields_to_concept_selection() {
        STATE.with(|cell| *cell.borrow_mut() = subject_sample());
        focus_unit("first");
        STATE.with(|cell| {
            let s = cell.borrow();
            let a = node(&s, "a");
            let b = node(&s, "b");
            assert!((s.rig.goal.pivot - s.base[a]).length() < 1e-6);
            assert_eq!(s.color_goal[a], s.nodes[a].color);
            assert_ne!(s.color_goal[b], s.nodes[b].color);
        });
        select("b");
        STATE.with(|cell| {
            let s = cell.borrow();
            let b = node(&s, "b");
            assert_eq!(s.selected_node, Some(b));
            assert_eq!(s.focus, Some(b));
            assert_eq!(s.color_goal[b], s.nodes[b].color);
        });
        focus_unit("");
        STATE.with(|cell| {
            let s = cell.borrow();
            assert!(s.unit_focus.is_empty() && s.selected_node.is_none());
            assert!(s.nodes.iter().enumerate().all(|(i, node)| s.color_goal[i] == node.color));
        });
        STATE.with(|cell| *cell.borrow_mut() = State::default());
    }

    #[test]
    fn inferred_ribbons_have_screen_space_gaps_without_changing_arrowheads() {
        let camera = Camera {
            yaw: 0.,
            pitch: 0.,
            distance: 600.,
            ..Camera::default()
        };
        let world = [Vec3::new(-200., 0., 0.), Vec3::new(200., 0., 0.)];
        let stroke = Stroke { rgb: srgb(EDGE), alpha: 1., width: 2. };
        let mut solid = Vec::new();
        let mut dashed = Vec::new();
        link_geometry(&mut solid, &camera, &world, true, false, stroke, (None, None), 800., 600.);
        link_geometry(&mut dashed, &camera, &world, true, true, stroke, (None, None), 800., 600.);
        assert!(dashed.len() > solid.len());
        for (solid, dashed) in solid.iter().rev().take(3).zip(dashed.iter().rev()) {
            assert_eq!(solid.position, dashed.position);
            assert_eq!(solid.color, dashed.color);
        }
        let x = |v: &FlatVertex| (v.position[0] as f64 + 1.) * 400.;
        let segments = &dashed[..dashed.len() - 3];
        assert!(segments.chunks_exact(6).zip(segments.chunks_exact(6).skip(1)).any(|(a, b)| {
            let end = a.iter().map(x).fold(f64::MIN, f64::max);
            let start = b.iter().map(x).fold(f64::MAX, f64::min);
            start - end > 4.9
        }));
    }

    fn size() -> NSSize {
        NSSize::new(800., 600.)
    }
    fn node(s: &State, id: &str) -> usize {
        s.index[id]
    }
    fn link(s: &State, id: &str) -> usize {
        s.links
            .iter()
            .position(|l| s.data["assertions"][l.assertion]["id"] == id)
            .unwrap()
    }
    fn place(s: &mut State, id: &str, p: Vec3) {
        let i = node(s, id);
        s.now[i] = p;
        s.goal[i] = p;
    }
    fn look(s: &mut State, distance: f64) {
        s.rig.set_limits(1., 1e6);
        s.rig.travel(
            Camera {
                yaw: 0.,
                pitch: 0.,
                distance,
                ..Camera::default()
            },
            false,
        );
    }
    fn screen(s: &State, id: &str) -> NSPoint {
        let p = s.rig.now.project(s.now[node(s, id)], 800., 600.).unwrap();
        NSPoint::new(p.x, p.y)
    }

    #[test]
    fn alias_search_highlights_without_selecting_or_moving() {
        let mut s = sample();
        let camera = s.rig.goal;
        s.query = "PHOTO synthesis".into();
        refresh_hits(&mut s);
        apply_highlight(&mut s);
        assert!(s.hits[node(&s, "a")] && !s.hits[node(&s, "b")]);
        assert!(s.selected.is_none() && s.selected_node.is_none());
        assert_eq!(s.rig.goal.distance, camera.distance);
        assert!((s.scale_goal[node(&s, "a")] - NODE_RADIUS * 1.03).abs() < 1e-9);
        s.query = "광합성".into();
        refresh_hits(&mut s);
        assert!(s.hits[node(&s, "a")]);
        // The selection payload still carries every assertion and its evidence.
        let selection = selection_for(&s, node(&s, "a")).unwrap();
        assert_eq!(selection["assertions"].as_array().unwrap().len(), 3);
        assert_eq!(selection["evidence"][0]["source_id"], "source1");
        assert_eq!(selection["assertions"][0]["negated"], true);
        assert!(s.links[link(&s, "e1")].label.contains("조건"));
    }
    #[test]
    fn ray_pick_tracks_nearest_depth_after_orbit() {
        let mut s = sample();
        for id in ["c", "d"] {
            place(&mut s, id, Vec3::new(0., 5000., 0.));
        }
        place(&mut s, "a", Vec3::new(0., 0., 50.));
        place(&mut s, "b", Vec3::new(0., 0., -50.));
        look(&mut s, 500.);
        let center = NSPoint::new(400., 300.);
        assert_eq!(hit_node(&s, center, size()).unwrap().0, node(&s, "a"));
        let mut turned = s.rig.goal;
        turned.yaw = std::f64::consts::PI;
        s.rig.travel(turned, false);
        assert_eq!(hit_node(&s, center, size()).unwrap().0, node(&s, "b"));
    }
    #[test]
    fn relation_filter_narrows_the_overview_but_not_a_selection() {
        let mut s = sample();
        s.filter = "r".into();
        refresh_shown(&mut s);
        apply_highlight(&mut s);
        settle(&mut s);
        let c = node(&s, "c");
        assert!(!s.shown[c] && s.scale[c] == 0.);
        let p = screen(&s, "c");
        assert_ne!(hit_node(&s, p, size()).map(|v| v.0), Some(c));
        assert_eq!(s.stroke_goal[link(&s, "e3")].alpha, 0.);
        let a = node(&s, "a");
        select_node(&mut s, a);
        assert!(s.shown[c]);
        // The chosen relation names and floats its neighbours; others stay lit.
        let b = node(&s, "b");
        assert!(s.floating[b] && !s.floating[c]);
        assert_eq!(s.eyebrow[b], "¬ 생성 · 조건 · 생성");
        assert_eq!(s.color_goal[b], to_linear(srgb(ACTIVE)));
        assert_ne!(s.color_goal[c], to_linear(srgb(ACTIVE)));
    }
    #[test]
    fn parallel_reverse_and_self_relations_stay_apart_and_pickable() {
        let mut s = sample();
        place(&mut s, "a", Vec3::new(-120., 0., 0.));
        place(&mut s, "b", Vec3::new(120., 0., 0.));
        place(&mut s, "c", Vec3::new(-120., -260., 0.));
        place(&mut s, "d", Vec3::new(120., -260., 0.));
        look(&mut s, 900.);
        // Look down a little: the two lanes bow apart in depth.
        let mut tilted = s.rig.goal;
        tilted.yaw = 0.3;
        tilted.pitch = 0.5;
        s.rig.travel(tilted, false);
        let paths = s
            .links
            .iter()
            .map(|l| link_path(&s, l).unwrap())
            .collect::<Vec<_>>();
        let (e1, e2, e3, e4) = (
            link(&s, "e1"),
            link(&s, "e2"),
            link(&s, "e3"),
            link(&s, "e4"),
        );
        // Two relations between the same pair bow apart; the single one is straight.
        assert!((paths[e1][12] - paths[e2][12]).length() > 40.);
        assert_eq!(paths[e3].len(), 2);
        assert!(paths[e4].len() > 30);
        for (i, path) in paths.iter().enumerate() {
            let l = &s.links[i];
            // Lines start just inside the subject and end at the object's surface.
            assert!(((path[0] - s.now[l.a]).length() - NODE_RADIUS * 0.94).abs() < 0.01);
            let end = if l.directed { 1.02 } else { 0.94 };
            assert!(
                ((*path.last().unwrap() - s.now[l.b]).length() - NODE_RADIUS * end).abs() < 0.01
            );
            let mid = s.rig.now.project(path[path.len() / 2], 800., 600.).unwrap();
            let at = if path.len() == 2 {
                let start = s.rig.now.project(path[0], 800., 600.).unwrap();
                NSPoint::new((mid.x + start.x) / 2., (mid.y + start.y) / 2.)
            } else {
                NSPoint::new(mid.x, mid.y)
            };
            assert_eq!(pick(&s, at, size()), Pick::Link(i));
        }
    }
    #[test]
    fn parallel_relations_fan_out_on_screen_from_every_direction() {
        let mut s = sample();
        place(&mut s, "a", Vec3::new(-120., 0., 0.));
        place(&mut s, "b", Vec3::new(120., 0., 0.));
        let (e1, e2, e4) = (link(&s, "e1"), link(&s, "e2"), link(&s, "e4"));
        for (yaw, pitch) in [(0., 0.), (0.9, 0.4), (1.4, -0.9), (3.1, 1.0), (4.6, 0.2)] {
            look(&mut s, 900.);
            let mut camera = s.rig.goal;
            camera.yaw = yaw;
            camera.pitch = pitch;
            s.rig.travel(camera, false);
            let middle = |i: usize| {
                let path = link_path(&s, &s.links[i]).unwrap();
                s.rig.now.project(path[path.len() / 2], 800., 600.).unwrap()
            };
            let (m1, m2) = (middle(e1), middle(e2));
            // 46 world units apart, facing the camera: about 40 px at this distance.
            assert!(
                (m1.x - m2.x).hypot(m1.y - m2.y) > 35.,
                "yaw {yaw} pitch {pitch}"
            );
            // A self relation stays an open loop above its concept.
            let top = middle(e4);
            let center = s.rig.now.project(s.now[node(&s, "a")], 800., 600.).unwrap();
            assert!(top.y - center.y > 60. && (top.x - center.x).abs() < 3.);
        }
    }
    #[test]
    fn released_drag_returns_and_focus_keeps_context() {
        let mut s = sample();
        let a = node(&s, "a");
        let home = s.goal[a];
        s.drag = Some(Drag {
            node: Some(a),
            offset: Vec3::ZERO,
            last: NSPoint::new(0., 0.),
            moved: 0.,
            dead: false,
        });
        drag_node(&mut s, 90., 20., size());
        assert!((s.now[a] - home).length() > 10.);
        assert_eq!(s.goal[a], home);
        // Still held: stepping keeps the node under the pointer.
        let held = s.now[a];
        step(&mut s, 1. / 60.);
        assert!((s.now[a] - held).length() < 1e-9);
        s.drag = None;
        settle(&mut s);
        assert_eq!(s.now[a], home);
        select_node(&mut s, a);
        assert_eq!(s.now[a], Vec3::ZERO);
        assert_eq!(s.rig.goal.pivot, Vec3::ZERO);
        let b = node(&s, "b");
        assert!(s.now[b].length() > 100.);
        assert!(s.now[node(&s, "c")].length() > s.now[b].length());
        assert!(s.ticker.is_none());
    }
    #[test]
    fn leaving_the_map_cancels_a_concept_drag_but_never_taps() {
        let mut s = sample();
        let a = node(&s, "a");
        select_node(&mut s, a);
        let b = node(&s, "b");
        let home = s.goal[b];
        let press = |s: &mut State, node: Option<usize>| {
            s.drag = Some(Drag {
                node,
                offset: Vec3::ZERO,
                last: NSPoint::new(0., 0.),
                moved: 0.,
                dead: false,
            })
        };
        // A press and release without movement is a tap.
        press(&mut s, None);
        assert!(release(&mut s));
        // Dragging a concept out of the map sends it home; the release that
        // follows must not count as a tap and deselect.
        press(&mut s, Some(b));
        drag_node(&mut s, 3., 0., size());
        leave(&mut s);
        settle(&mut s);
        assert_eq!(s.now[b], home);
        assert!(!release(&mut s));
        assert_eq!(selected_id(&s), Some("a"));
        // An orbit keeps going outside the map and is not a tap either.
        press(&mut s, None);
        s.drag.as_mut().unwrap().moved = 40.;
        leave(&mut s);
        assert!(s.drag.as_ref().is_some_and(|d| !d.dead));
        assert!(!release(&mut s));
    }
    #[test]
    fn offcenter_grab_moves_exact_screen_delta_without_jumping() {
        let mut s = sample();
        let a = node(&s, "a");
        let before = s.rig.now.project(s.now[a], 800., 600.).unwrap();
        s.drag = Some(Drag {
            node: Some(a),
            offset: Vec3::ZERO,
            last: NSPoint::new(0., 0.),
            moved: 0.,
            dead: false,
        });
        // A pointer may press at centre+7px; only its next delta moves the centre.
        drag_node(&mut s, 1., -2., size());
        let after = s.rig.now.project(s.now[a], 800., 600.).unwrap();
        assert!((after.x - before.x - 1.).abs() < 1e-8);
        assert!((after.y - before.y + 2.).abs() < 1e-8);
        assert!((after.depth - before.depth).abs() < 1e-8);
    }
    #[test]
    fn taps_follow_the_web_map() {
        let mut s = sample();
        // An empty tap with nothing selected leaves an orbited camera alone.
        s.rig.orbit(120., 40.);
        s.rig.snap();
        let orbited = s.rig.goal;
        tap(&mut s, Pick::Nothing);
        assert_eq!(s.rig.goal.yaw, orbited.yaw);
        assert_eq!(s.rig.goal.pitch, orbited.pitch);
        let a = node(&s, "a");
        tap(&mut s, Pick::Node(a));
        assert_eq!(selected_id(&s), Some("a"));
        assert_eq!(s.selected.as_ref().unwrap()["kind"], "concept");
        // Focus keeps the viewing direction and centres the selection.
        assert_eq!(s.rig.goal.yaw, orbited.yaw);
        assert_eq!(s.focus, Some(a));
        // Tapping the selected concept again changes nothing.
        let goal = s.goal.clone();
        s.rig.orbit(30., 0.);
        s.rig.snap();
        tap(&mut s, Pick::Node(a));
        assert_eq!(s.goal, goal);
        // A relation cannot be selected and behaves like empty space.
        let e2 = link(&s, "e2");
        tap(&mut s, Pick::Link(e2));
        assert!(s.selected.is_none());
        tap(&mut s, Pick::Node(a));
        // An empty tap now deselects and returns to the default overview.
        tap(&mut s, Pick::Nothing);
        assert!(s.selected.is_none() && s.focus.is_none());
        assert_eq!(s.goal, s.base);
        assert!((s.rig.goal.yaw - 0.62).abs() < 1e-9 && (s.rig.goal.pitch - 0.30).abs() < 1e-9);
    }
    #[test]
    fn zoom_is_limited_by_the_graph_and_scroll_up_zooms_in() {
        let mut s = sample();
        let overview = s.rig.goal.distance;
        assert!(scroll_zoom(10.) > 1. && scroll_zoom(-10.) < 1.);
        assert!((scroll_zoom(100.) - (0.16f64).exp()).abs() < 1e-12);
        for _ in 0..400 {
            s.rig.zoom(scroll_zoom(120.));
        }
        // Never closer than 9.6 node radii: a concept cannot fill the view.
        assert!((s.rig.goal.distance - 9.6 * NODE_RADIUS).abs() < 1e-9);
        let pixels = 600. / 2. / (math::FOV_Y / 2.).tan() * NODE_RADIUS / s.rig.goal.distance;
        assert!(pixels < 600. * 0.14);
        for _ in 0..400 {
            s.rig.zoom(scroll_zoom(-120.));
        }
        // The overview is the farthest view.
        assert!((s.rig.goal.distance - overview.max(3.1 * s.bounds)).abs() < 1e-6);
    }
    #[test]
    fn highlights_ease_and_settle() {
        let mut s = sample();
        let (a, b, c) = (node(&s, "a"), node(&s, "b"), node(&s, "c"));
        s.hovered = Some(a);
        apply_highlight(&mut s);
        let lit = to_linear(srgb(ACTIVE));
        assert_eq!(s.color_goal[a], lit);
        assert_eq!(s.color_goal[b], lit);
        assert_eq!(s.color_goal[c], mix3(lit, to_linear(srgb(DIM)), 0.55));
        assert!((s.scale_goal[a] - NODE_RADIUS * 1.04).abs() < 1e-9);
        assert!((s.scale_goal[b] - NODE_RADIUS * 1.07).abs() < 1e-9);
        assert_eq!(s.stroke_goal[link(&s, "e1")].width, 2.);
        assert_eq!(s.stroke_goal[link(&s, "e3")].alpha, 0.3);
        // Goals are not reached in one jump once motion is allowed; headless
        // state arrives at once.
        assert_ne!(s.color[c], s.color_goal[c]);
        assert!(!step(&mut s, 1. / 60.));
        assert_eq!(s.color[c], s.color_goal[c]);
        assert_eq!(s.halo[1], (Some(a), 1.));
    }
    #[test]
    fn labels_follow_priority_tone_and_safe_area() {
        let mut s = sample();
        let (a, b, c) = (node(&s, "a"), node(&s, "b"), node(&s, "c"));
        let labels = place_labels(&mut s, size(), 0., true);
        assert_eq!(labels.len(), 4);
        assert!(labels
            .iter()
            .all(|l| l.tone == Tone::Default && l.alpha == 1.));
        // The backplate is sized from the text, and sits above its concept.
        let named = labels.iter().find(|l| l.title == "엽록체").unwrap();
        assert!(
            (named.rect.size.width - (measure("엽록체", LABEL_FONT, WEIGHT_SEMIBOLD) + 16.).ceil())
                .abs()
                < 1e-9
        );
        assert!(named.rect.origin.y > screen(&s, "b").y);
        select_node(&mut s, a);
        let labels = place_labels(&mut s, size(), 0., true);
        let tone = |title: &str| labels.iter().find(|l| l.title == title).map(|l| l.tone);
        assert_eq!(tone("광 합 성"), Some(Tone::Selected));
        assert_eq!(tone("엽록체"), Some(Tone::Relation));
        assert_eq!(tone("ATP"), None);
        assert!(labels.iter().any(|l| l.eyebrow.contains("생성")));
        assert_eq!(label_candidate(&s, c, Some(a)), (-1, Tone::Default, false));
        s.hovered = Some(c);
        assert_eq!(label_candidate(&s, c, Some(a)), (96, Tone::Hover, true));
        // A label whose anchor falls under host controls is not drawn.
        s.insets.top = 590.;
        assert!(place_labels(&mut s, size(), 0., true).is_empty());
        let _ = b;
    }
    #[test]
    fn long_names_are_shortened_to_fit() {
        let mut cache = HashMap::new();
        let long = "아주 긴 개념 이름이 라벨 최대 너비를 넘는 경우의 처리";
        let (shown, width) = fitted(&mut cache, long, LABEL_FONT, WEIGHT_SEMIBOLD);
        assert!(shown.ends_with('…') && width <= LABEL_MAX_TEXT);
        assert_eq!(
            fitted(&mut cache, "ATP", LABEL_FONT, WEIGHT_SEMIBOLD).0,
            "ATP"
        );
    }
    #[test]
    fn ribbon_keeps_its_width_and_arrow_touches_the_object() {
        let camera = Camera {
            yaw: 0.,
            pitch: 0.,
            distance: 600.,
            ..Camera::default()
        };
        let world = [Vec3::new(-200., 0., 0.), Vec3::new(200., 0., 0.)];
        let stroke = Stroke {
            rgb: [0.; 3],
            alpha: 1.,
            width: 2.,
        };
        let mut flat = Vec::new();
        link_geometry(
            &mut flat,
            &camera,
            &world,
            true,
            false,
            stroke,
            (None, None),
            800.,
            600.,
        );
        // One quad for the line and one triangle for the arrowhead.
        assert_eq!(flat.len(), 9);
        let y = |v: &FlatVertex| (v.position[1] as f64 + 1.) / 2. * 600.;
        let x = |v: &FlatVertex| (v.position[0] as f64 + 1.) / 2. * 800.;
        // Coverage reaches zero exactly one pixel outside a 2-point-wide line.
        for v in &flat[..6] {
            assert!(((y(v) - 300.).abs() - 2.).abs() < 1e-3);
            assert!((v.distance[0].min(v.distance[1]) + 1.).abs() < 1e-4);
        }
        let tip = camera.project(world[1], 800., 600.).unwrap().x;
        let reach = flat[6..].iter().map(x).fold(f64::MIN, f64::max);
        assert!(reach >= tip && reach < tip + 3.5, "{reach} {tip}");
        // Twice as far away, the line is exactly as wide on screen.
        let far = Camera {
            distance: 1200.,
            ..camera
        };
        let mut distant = Vec::new();
        link_geometry(
            &mut distant,
            &far,
            &world,
            false,
            false,
            stroke,
            (None, None),
            800.,
            600.,
        );
        for v in &distant {
            assert!(((y(v) - 300.).abs() - 2.).abs() < 1e-3);
        }
    }
    #[test]
    fn relation_ends_at_the_outline_of_the_concept_it_attaches_to() {
        let camera = Camera {
            yaw: 0.,
            pitch: 0.,
            distance: 600.,
            ..Camera::default()
        };
        // The object sits at the pivot; the relation arrives from in front of it,
        // so its 3D end projects onto the face of the sphere.
        let target = Vec3::ZERO;
        let world = [Vec3::new(-200., 40., 150.), Vec3::new(-6., 2., 14.)];
        let disc = silhouette(&camera, target, 15., 800., 600.).unwrap();
        assert!((disc.0 - 400.).abs() < 1e-9 && (disc.1 - 300.).abs() < 1e-9);
        let exact = 300. / (math::FOV_Y / 2.).tan() * 15. / (600f64 * 600. - 225.).sqrt();
        assert!((disc.2 - exact).abs() < 1e-9);
        let mut run = project_path(&camera, &world, 800., 600.).remove(0).points;
        let inside_before = (run[1].x - disc.0).hypot(run[1].y - disc.1);
        assert!(inside_before < disc.2 * 0.5);
        clip_to_outline(&mut run, disc, true);
        let end = run.last().unwrap();
        assert!(((end.x - disc.0).hypot(end.y - disc.1) - disc.2).abs() < 1e-6);
        // Still in front of the sphere there, so it is drawn right up to the rim.
        assert!(end.depth < 600.);
        // A relation wholly over the face draws nothing.
        let mut hidden = project_path(
            &camera,
            &[Vec3::new(2., 0., 300.), Vec3::new(0., 2., 16.)],
            800.,
            600.,
        )
        .remove(0)
        .points;
        clip_to_outline(&mut hidden, disc, true);
        assert!(hidden.is_empty());
    }
    #[test]
    fn bowed_relation_is_refined_to_a_few_pixels_per_piece() {
        let point = |x: f64, y: f64| ScreenPoint {
            x,
            y,
            z: 0.5,
            depth: 100.,
        };
        // A quarter circle of radius 400 px sampled only nine times.
        let coarse = (0..9)
            .map(|i| {
                let a = std::f64::consts::FRAC_PI_2 * i as f64 / 8.;
                point(400. * a.cos(), 400. * a.sin())
            })
            .collect::<Vec<_>>();
        let fine = smooth(&coarse);
        assert!(fine.len() > 100);
        for pair in fine.windows(2) {
            assert!((pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y) <= 10.);
        }
        // The refined points follow the circle instead of cutting its corners.
        // Left as sampled, each chord would cut the arc by almost two pixels.
        let sagitta = 400. * (1. - (std::f64::consts::FRAC_PI_2 / 16.).cos());
        assert!(sagitta > 1.9);
        let off = |p: &ScreenPoint| (p.x.hypot(p.y) - 400.).abs();
        let step = std::f64::consts::FRAC_PI_2 / 8.;
        let interior = fine
            .iter()
            .filter(|p| (step..=step * 7.).contains(&p.y.atan2(p.x)))
            .map(off)
            .fold(0., f64::max);
        assert!(interior < 0.1, "{interior}");
        // Only the two end pieces, which meet a sphere, are approximate.
        assert!(fine.iter().map(off).fold(0., f64::max) < sagitta);
        // Straight relations are left alone.
        assert_eq!(smooth(&coarse[..2]).len(), 2);
    }
    #[test]
    fn relation_is_cut_before_the_eye_instead_of_wrapping() {
        let camera = Camera {
            yaw: 0.,
            pitch: 0.,
            distance: 300.,
            ..Camera::default()
        };
        // From in front of the camera to well behind it.
        let world = [Vec3::new(0., 0., 0.), Vec3::new(40., 0., 600.)];
        let runs = project_path(&camera, &world, 800., 600.);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].from_start && !runs[0].to_end);
        let end = runs[0].points.last().unwrap();
        assert!((end.depth - NEAR_CLIP).abs() < 1e-6);
        assert!(project_path(&camera, &[world[1], Vec3::new(0., 0., 900.)], 800., 600.).is_empty());
        // Close to the eye a relation fades out rather than slicing the view.
        assert_eq!(near_fade(3. * NODE_RADIUS), 0.);
        assert_eq!(near_fade(6. * NODE_RADIUS), 1.);
    }
    #[test]
    fn projected_edge_depth_is_perspective_correct() {
        let point = |x: f64, depth: f64| ScreenPoint {
            x,
            y: 0.,
            z: 0.,
            depth,
        };
        let (distance, depth) =
            segment_distance(NSPoint::new(50., 0.), &point(0., 100.), &point(100., 1000.));
        assert!(distance < 1e-8);
        assert!((depth - 181.818181818).abs() < 1e-6);
    }
    #[test]
    fn visible_edge_in_front_of_node_can_be_picked() {
        let mut s = sample();
        s.data["assertions"] = json!([{"id":"front","subject":"a","object":"b","relation":"r","evidence":[{"source_id":"kept"}]}]);
        let data = s.data.clone();
        load(&mut s, data);
        place(&mut s, "a", Vec3::new(-130., 0., 100.));
        place(&mut s, "b", Vec3::new(130., 0., 100.));
        place(&mut s, "c", Vec3::ZERO);
        place(&mut s, "d", Vec3::new(0., 150., 0.));
        look(&mut s, 650.);
        let p = NSPoint::new(400., 300.);
        assert_eq!(hit_node(&s, p, size()).unwrap().0, node(&s, "c"));
        assert_eq!(pick(&s, p, size()), Pick::Link(0));
        // Clicks use node hit testing even when a relation is drawn in front.
        let target = hit_node(&s, p, size()).map(|(i, _)| Pick::Node(i)).unwrap_or(Pick::Nothing);
        tap(&mut s, target);
        assert_eq!(s.selected_node, Some(node(&s, "c")));
        tap(&mut s, Pick::Nothing);
        place(&mut s, "a", Vec3::new(-130., 0., -100.));
        place(&mut s, "b", Vec3::new(130., 0., -100.));
        place(&mut s, "c", Vec3::ZERO);
        place(&mut s, "d", Vec3::new(0., 150., 0.));
        look(&mut s, 650.);
        assert_eq!(pick(&s, p, size()), Pick::Node(node(&s, "c")));
    }
    #[test]
    fn saved_view_round_trips_selection_camera_and_layout() {
        STATE.with(|cell| *cell.borrow_mut() = sample());
        select("a");
        rotate(0.4);
        zoom(1.3);
        settle_now();
        let saved = snapshot_view();
        let (goal, camera) = STATE.with(|cell| {
            let s = cell.borrow();
            (s.goal.clone(), s.rig.goal)
        });
        STATE.with(|cell| *cell.borrow_mut() = sample());
        restore_view(&saved);
        STATE.with(|cell| {
            let s = cell.borrow();
            assert_eq!(selected_id(&s), Some("a"));
            assert_eq!(s.goal, goal);
            assert_eq!(s.now, goal);
            assert!((s.rig.now.yaw - camera.yaw.rem_euclid(TAU)).abs() < 1e-9);
            assert!((s.rig.now.distance - camera.distance).abs() < 1e-9);
        });
        STATE.with(|cell| *cell.borrow_mut() = State::default());
    }
}
