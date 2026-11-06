//! Knowledge graph view: force-directed layout, canvas drawing (edges, nodes, labels),
//! and the floating filter card and zoom pill.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, Corners, Edges, ElementId, Entity, FontWeight, Hsla, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Rgba,
    SharedString, Subscription, TextRun, Window, canvas, div, fill, font, point, prelude::*, px,
    size, transparent_black,
};

use super::UI_FONT;
use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::icons::icon;
use crate::ui::widgets::{LineInput, LineInputEvent};
use crate::vault::KnowledgeGraph;

/// Space kept free around the graph inside the canvas, in px.
const MARGIN: f32 = 48.0;
/// Floating filter card at the top-right: right inset, top inset, width and estimated height.
const CARD_RIGHT: f32 = 14.0;
const CARD_TOP: f32 = 14.0;
const CARD_W: f32 = 240.0;
const CARD_H: f32 = 300.0;
/// Clear space kept between the filter card and the fitted layout.
const CARD_MARGIN: f32 = 40.0;
/// Height kept free above the bottom edge for the isolated-note row.
const ISO_STRIP: f32 = 40.0;
/// Left edge of the isolated-note row, clear of the zoom pill (canvas-local px).
const ISO_ROW_MIN_X: f32 = 130.0;
/// Radius of an isolated note's dot, before the size slider.
const ISO_DOT_PX: f32 = 3.0;
/// Most-linked notes that always get a label.
const TOP_LINKED_LABELS: usize = 8;
const LABEL_PX: f32 = 12.0;
const LABEL_LINE_PX: f32 = 16.0;
const SIZE_MIN: f32 = 0.6;
const SIZE_MAX: f32 = 2.0;
const ZOOM_MIN: f32 = 0.35;
const ZOOM_MAX: f32 = 4.0;

#[derive(Clone, Copy)]
struct Drag {
    last: Point<Pixels>,
    start: Point<Pixels>,
    moved: bool,
}

struct LayoutCache {
    key: u64,
    norm: Vec<(f32, f32)>,
    isolated: Vec<bool>,
}

/// Graph view state. Layout is cached here; the rest is UI state.
pub struct GraphState {
    /// Search text (typed into the card's search box).
    pub filter: String,
    /// Tag chip that filters nodes, if any.
    pub tag: Option<String>,
    /// Show notes without any link (the `显示孤立笔记` switch).
    pub show_isolated: bool,
    /// Node size multiplier (slider, 0.6..2.0).
    pub size_factor: f32,
    /// Whether the search box takes typed text (drives the box's border colour).
    pub search_focused: bool,
    /// The search box's text field. `filter` mirrors its text.
    pub filter_input: Option<Entity<LineInput>>,
    /// Subscriptions of `filter_input`, kept alive here.
    pub filter_subs: Vec<Subscription>,
    /// Zoom factor applied to the layout spacing.
    pub zoom: f32,
    /// Pan offset in px, applied after zoom.
    pub pan: (f32, f32),
    /// Node index under the pointer.
    pub hovered: Option<usize>,
    drag: Option<Drag>,
    layout: RefCell<Option<LayoutCache>>,
    canvas_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    track_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl Default for GraphState {
    fn default() -> Self {
        Self {
            filter: String::new(),
            tag: None,
            show_isolated: true,
            size_factor: 1.0,
            search_focused: false,
            filter_input: None,
            filter_subs: Vec::new(),
            zoom: 1.0,
            pan: (0.0, 0.0),
            hovered: None,
            drag: None,
            layout: RefCell::new(None),
            canvas_bounds: Rc::new(Cell::new(None)),
            track_bounds: Rc::new(Cell::new(None)),
        }
    }
}

/// Deterministic starting positions on a golden-angle spiral inside the unit square.
fn initial_positions(n: usize) -> Vec<(f32, f32)> {
    (0..n)
        .map(|i| {
            let r = 0.4 * ((i as f32 + 0.5) / n as f32).sqrt();
            let theta = i as f32 * 2.399_963;
            (0.5 + r * theta.cos(), 0.5 + r * theta.sin())
        })
        .collect()
}

/// Force-directed layout (Fruchterman-Reingold style: pairwise repulsion, spring attraction
/// along edges, weak centre gravity, linear cooling). Deterministic: no randomness.
/// Returns one `(x, y)` per node, all within `[0, 1]`, with the larger extent spanning the whole range.
pub fn force_layout(n: usize, edges: &[(usize, usize)]) -> Vec<(f32, f32)> {
    match n {
        0 => return Vec::new(),
        1 => return vec![(0.5, 0.5)],
        _ => {}
    }
    let mut pos = initial_positions(n);
    let k = 1.0 / (n as f32).sqrt();
    let iters = (30_000 / n).clamp(60, 300);
    let t0 = 0.1f32;
    let gravity = 0.05f32;
    let mut disp = vec![(0.0f32, 0.0f32); n];

    for it in 0..iters {
        let temp = t0 * (1.0 - it as f32 / iters as f32) + 1e-4;
        disp.iter_mut().for_each(|d| *d = (0.0, 0.0));

        // Repulsion between every pair.
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let d2 = (dx * dx + dy * dy).max(1e-6);
                let f = k * k / d2;
                disp[i].0 += dx * f;
                disp[i].1 += dy * f;
                disp[j].0 -= dx * f;
                disp[j].1 -= dy * f;
            }
        }

        // Spring attraction along edges.
        for &(a, b) in edges {
            if a >= n || b >= n || a == b {
                continue;
            }
            let dx = pos[b].0 - pos[a].0;
            let dy = pos[b].1 - pos[a].1;
            let d = (dx * dx + dy * dy).sqrt();
            let f = d / k;
            disp[a].0 += dx * f;
            disp[a].1 += dy * f;
            disp[b].0 -= dx * f;
            disp[b].1 -= dy * f;
        }

        // Gravity towards the centre, then apply capped steps.
        for i in 0..n {
            let gx = (0.5 - pos[i].0) * gravity;
            let gy = (0.5 - pos[i].1) * gravity;
            let dx = disp[i].0 + gx;
            let dy = disp[i].1 + gy;
            let len = (dx * dx + dy * dy).sqrt();
            if len > 0.0 {
                let step = len.min(temp) / len;
                pos[i].0 = (pos[i].0 + dx * step).clamp(0.0, 1.0);
                pos[i].1 = (pos[i].1 + dy * step).clamp(0.0, 1.0);
            }
        }
    }

    // Normalise: the larger extent spans [0, 1], the smaller one is centred.
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in &pos {
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    }
    let (w, h) = (maxx - minx, maxy - miny);
    let span = w.max(h).max(1e-6);
    let ox = (span - w) / 2.0;
    let oy = (span - h) / 2.0;
    pos.iter()
        .map(|&(x, y)| ((x - minx + ox) / span, (y - miny + oy) / span))
        .collect()
}

/// Difference `a - b` of two pixel values as f32.
fn dx_from(a: Pixels, b: Pixels) -> f32 {
    f32::from(a) - f32::from(b)
}

fn with_alpha(c: Rgba, a: f32) -> Rgba {
    Rgba { a, ..c }
}

/// Axis-aligned rectangle in canvas-local px.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Rect {
    fn w(&self) -> f32 {
        self.x1 - self.x0
    }

    fn h(&self) -> f32 {
        self.y1 - self.y0
    }

    fn intersects(&self, o: &Rect) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
}

/// Area of the canvas covered by the floating filter card (canvas-local px).
fn card_rect(w: f32) -> Rect {
    Rect {
        x0: w - CARD_RIGHT - CARD_W,
        y0: 0.0,
        x1: w,
        y1: CARD_TOP + CARD_H,
    }
}

/// Rectangle the connected layout is fitted into: the canvas inside `MARGIN`, minus the card.
/// Takes the larger of the space left of the card and the space below it.
fn fit_rect(w: f32, h: f32) -> Rect {
    let full = Rect {
        x0: MARGIN,
        y0: MARGIN,
        x1: w - MARGIN,
        y1: h - MARGIN,
    };
    if full.w() <= 0.0 || full.h() <= 0.0 {
        return full;
    }
    let card = card_rect(w);
    let left = Rect {
        x1: full.x1.min(card.x0 - CARD_MARGIN),
        ..full
    };
    let below = Rect {
        y0: full.y0.max(card.y1 + CARD_MARGIN),
        ..full
    };
    let area = |r: &Rect| r.w().max(0.0) * r.h().max(0.0);
    let pick = if area(&left) >= area(&below) {
        left
    } else {
        below
    };
    // Too small to read (very narrow split pane): give up the card clearance.
    if pick.w() < 120.0 || pick.h() < 120.0 {
        full
    } else {
        pick
    }
}

/// Canvas-local positions before zoom and pan. Connected notes are the normalised force layout
/// stretched to fill `fit_rect`. Isolated notes sit in one row on its bottom edge.
fn place(norm: &[(f32, f32)], isolated: &[bool], w: f32, h: f32) -> Vec<(f32, f32)> {
    let r = fit_rect(w, h);
    let iso_total = isolated.iter().filter(|&&x| x).count();
    let conn = if iso_total > 0 {
        Rect {
            y1: (r.y1 - ISO_STRIP).max(r.y0),
            ..r
        }
    } else {
        r
    };
    // The normalised layout spans [0, 1] on both axes, so it fills `conn` exactly.
    // The row starts right of the zoom pill, which sits in the bottom-left corner.
    let row_x0 = r.x0.max(ISO_ROW_MIN_X);
    let step = (r.x1 - row_x0).max(1.0) / iso_total.max(1) as f32;
    let mut k = 0usize;
    let mut out = vec![(0.0f32, 0.0f32); norm.len()];
    for (i, p) in norm.iter().enumerate() {
        if isolated.get(i).copied().unwrap_or(false) {
            out[i] = (row_x0 + (k as f32 + 0.5) * step, r.y1);
            k += 1;
        } else {
            out[i] = (conn.x0 + p.0 * conn.w(), conn.y0 + p.1 * conn.h());
        }
    }
    out
}

/// Canvas-local position after zoom (about the canvas centre) and pan.
fn to_screen(p: (f32, f32), w: f32, h: f32, zoom: f32, pan: (f32, f32)) -> (f32, f32) {
    (
        w * 0.5 + (p.0 - w * 0.5) * zoom + pan.0,
        h * 0.5 + (p.1 - h * 0.5) * zoom + pan.1,
    )
}

/// Final node positions in canvas-local px for the current size, zoom and pan.
fn screen_positions(
    norm: &[(f32, f32)],
    isolated: &[bool],
    w: f32,
    h: f32,
    zoom: f32,
    pan: (f32, f32),
) -> Vec<(f32, f32)> {
    place(norm, isolated, w, h)
        .into_iter()
        .map(|p| to_screen(p, w, h, zoom, pan))
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum NodeKind {
    Plain,
    Dim,
    Linked,
    Hot,
    Selected,
}

#[derive(Clone, Copy)]
struct NodeDraw {
    idx: usize,
    radius: f32,
    kind: NodeKind,
}

#[derive(Clone)]
struct LabelDraw {
    idx: usize,
    text: String,
    emphasized: bool,
}

/// Everything the canvas paint needs, owned so the paint closure can be 'static.
#[derive(Clone)]
struct Scene {
    theme: ActiveTheme,
    zoom: f32,
    pan: (f32, f32),
    norm: Vec<(f32, f32)>,
    isolated: Vec<bool>,
    nodes: Vec<NodeDraw>,
    plain_edges: Vec<(usize, usize)>,
    hot_edges: Vec<(usize, usize)>,
    /// Sorted by priority, most important first.
    labels: Vec<LabelDraw>,
}

fn paint_scene(scene: &Scene, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let t = scene.theme;
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let sp = screen_positions(&scene.norm, &scene.isolated, w, h, scene.zoom, scene.pan);
    let card = card_rect(w);
    let at = |p: (f32, f32)| point(px(ox + p.0), px(oy + p.1));

    // Edges: plain first, then the ones touching the selected or hovered node.
    for (edges, color) in [
        (&scene.plain_edges, t.line),
        (&scene.hot_edges, with_alpha(t.tx3, 0.5)),
    ] {
        if edges.is_empty() {
            continue;
        }
        let mut pb = PathBuilder::stroke(px(1.0));
        for &(a, b) in edges {
            pb.move_to(at(sp[a]));
            pb.line_to(at(sp[b]));
        }
        if let Ok(path) = pb.build() {
            window.paint_path(path, Hsla::from(color));
        }
    }

    // Nodes; the selected one is painted last so its ring sits on top of its neighbours.
    for nd in &scene.nodes {
        if nd.kind == NodeKind::Selected {
            continue;
        }
        draw_node(window, &sp, nd, &t, ox, oy);
    }
    for nd in scene.nodes.iter().filter(|n| n.kind == NodeKind::Selected) {
        draw_node(window, &sp, nd, &t, ox, oy);
    }

    // Labels after edges, so lines never cross the text; a sheet-coloured plate sits under each.
    let mut placed: Vec<(f32, f32, f32, f32)> = Vec::new();
    for lab in &scene.labels {
        let (x, y) = sp[lab.idx];
        let r = scene
            .nodes
            .iter()
            .find(|n| n.idx == lab.idx)
            .map_or(4.0, |n| n.radius);
        let mut f = font(UI_FONT);
        if lab.emphasized {
            f.weight = FontWeight::SEMIBOLD;
        }
        let color: Hsla = if lab.emphasized { t.tx } else { t.tx2 }.into();
        let run = TextRun {
            len: lab.text.len(),
            font: f,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from(lab.text.clone()),
            px(LABEL_PX),
            &[run],
            None,
        );
        let tx = ox + x + r + 6.0;
        let ty = oy + y - LABEL_LINE_PX / 2.0;
        let text_w = f32::from(shaped.width);
        let rect = (tx - 4.0, ty - 2.0, text_w + 8.0, LABEL_LINE_PX + 4.0);
        let overlaps = placed.iter().any(|p| {
            rect.0 < p.0 + p.2
                && p.0 < rect.0 + rect.2
                && rect.1 < p.1 + p.3
                && p.1 < rect.1 + rect.3
        });
        let inside =
            rect.0 >= ox && rect.0 + rect.2 <= ox + w && rect.1 >= oy && rect.1 + rect.3 <= oy + h;
        let local = Rect {
            x0: rect.0 - ox,
            y0: rect.1 - oy,
            x1: rect.0 - ox + rect.2,
            y1: rect.1 - oy + rect.3,
        };
        let under_card = local.intersects(&card);
        if overlaps || !inside || under_card {
            continue;
        }
        placed.push(rect);
        window.paint_quad(
            fill(
                Bounds::new(point(px(rect.0), px(rect.1)), size(px(rect.2), px(rect.3))),
                Hsla::from(with_alpha(t.sheet, 0.85)),
            )
            .corner_radii(Corners::all(px(4.0))),
        );
        let _ = shaped.paint(point(px(tx), px(ty)), px(LABEL_LINE_PX), window, cx);
    }
}

fn draw_node(
    window: &mut Window,
    sp: &[(f32, f32)],
    nd: &NodeDraw,
    t: &ActiveTheme,
    ox: f32,
    oy: f32,
) {
    let (x, y) = sp[nd.idx];
    let (cx, cy) = (ox + x, oy + y);
    let r = nd.radius;
    if nd.kind == NodeKind::Selected {
        // Soft ring: 6px band from r+2 to r+8, ac at 35% alpha.
        let outer = r + 8.0;
        window.paint_quad(
            fill(
                Bounds::new(
                    point(px(cx - outer), px(cy - outer)),
                    size(px(outer * 2.0), px(outer * 2.0)),
                ),
                transparent_black(),
            )
            .corner_radii(Corners::all(px(outer)))
            .border_widths(Edges::all(px(6.0)))
            .border_color(Hsla::from(with_alpha(t.ac, 0.35))),
        );
    }
    let color = match nd.kind {
        NodeKind::Plain => with_alpha(t.tx3, 0.4),
        NodeKind::Dim => with_alpha(t.tx3, 0.12),
        NodeKind::Linked => with_alpha(t.ac, 0.7),
        NodeKind::Hot | NodeKind::Selected => t.ac,
    };
    window.paint_quad(
        fill(
            Bounds::new(
                point(px(cx - r), px(cy - r)),
                size(px(r * 2.0), px(r * 2.0)),
            ),
            Hsla::from(color),
        )
        .corner_radii(Corners::all(px(r))),
    );
}

/// Cheap hash of the node titles and edges; any change to either invalidates the layout.
fn graph_key(titles: &[&str], edges: &[(usize, usize)]) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    titles.hash(&mut h);
    edges.hash(&mut h);
    h.finish()
}

/// Normalised layout and isolated flags for the graph. Recomputed only when the titles or the
/// edges change, so zoom, pan, filters and selection reuse it.
fn layout_for(st: &GraphState, graph: &KnowledgeGraph) -> (Vec<(f32, f32)>, Vec<bool>) {
    let n = graph.nodes.len();
    let titles: Vec<&str> = graph.nodes.iter().map(|node| node.title.as_str()).collect();
    let pairs: Vec<(usize, usize)> = graph
        .edges
        .iter()
        .map(|e| (e.from_index, e.to_index))
        .filter(|&(a, b)| a != b && a < n && b < n)
        .collect();
    let key = graph_key(&titles, &pairs);
    let mut cache = st.layout.borrow_mut();
    if cache.as_ref().is_none_or(|c| c.key != key) {
        let mut degree = vec![0usize; n];
        for &(a, b) in &pairs {
            degree[a] += 1;
            degree[b] += 1;
        }
        let isolated: Vec<bool> = degree.iter().map(|&d| d == 0).collect();
        // Lay out only the connected notes; isolated ones are placed in a row by `place`.
        let mut compact = vec![usize::MAX; n];
        let mut count = 0usize;
        for i in 0..n {
            if !isolated[i] {
                compact[i] = count;
                count += 1;
            }
        }
        let compact_edges: Vec<(usize, usize)> = pairs
            .iter()
            .map(|&(a, b)| (compact[a], compact[b]))
            .collect();
        let connected = force_layout(count, &compact_edges);
        let mut norm = vec![(0.5f32, 0.5f32); n];
        for i in 0..n {
            if !isolated[i] {
                norm[i] = connected[compact[i]];
            }
        }
        *cache = Some(LayoutCache {
            key,
            norm,
            isolated,
        });
    }
    let c = cache.as_ref().expect("layout cache is filled above");
    (c.norm.clone(), c.isolated.clone())
}

/// Which nodes pass the tag and isolated-note filters.
fn visible_mask(app: &RustdianApp) -> Vec<bool> {
    let st = &app.graph_ui;
    app.vault
        .graph
        .nodes
        .iter()
        .map(|n| {
            if !st.show_isolated && n.link_count == 0 {
                return false;
            }
            match &st.tag {
                Some(tag) => app
                    .vault
                    .notes
                    .get(n.note_index)
                    .is_some_and(|note| note.tags.iter().any(|t| t == tag)),
                None => true,
            }
        })
        .collect()
}

/// Index of the node the active note belongs to.
fn selected_node(app: &RustdianApp) -> Option<usize> {
    let active = app.vault.active_note_index;
    app.vault
        .graph
        .nodes
        .iter()
        .position(|n| n.note_index == active)
}

/// Node under a window-space point, nearest first, within radius plus 4px.
fn node_at(app: &RustdianApp, pos: Point<Pixels>) -> Option<usize> {
    let bounds = app.graph_ui.canvas_bounds.get()?;
    let local = (
        f32::from(pos.x) - f32::from(bounds.origin.x),
        f32::from(pos.y) - f32::from(bounds.origin.y),
    );
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (norm, isolated) = layout_for(&app.graph_ui, &app.vault.graph);
    let positions = screen_positions(&norm, &isolated, w, h, app.graph_ui.zoom, app.graph_ui.pan);
    let visible = visible_mask(app);
    let max_lc = app
        .vault
        .graph
        .nodes
        .iter()
        .map(|n| n.link_count)
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let mut best: Option<(f32, usize)> = None;
    for (i, node) in app.vault.graph.nodes.iter().enumerate() {
        if !visible.get(i).copied().unwrap_or(false) {
            continue;
        }
        let Some(&(x, y)) = positions.get(i) else {
            continue;
        };
        let r = draw_radius(node.link_count, max_lc, app.graph_ui.size_factor);
        let d = ((x - local.0).powi(2) + (y - local.1).powi(2)).sqrt();
        if d <= r + 4.0 && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// Dot radius for drawing and hit tests: isolated notes are small fixed dots.
fn draw_radius(link_count: usize, max_lc: f32, size_factor: f32) -> f32 {
    if link_count == 0 {
        ISO_DOT_PX * size_factor
    } else {
        node_radius(link_count, max_lc, size_factor)
    }
}

fn node_radius(link_count: usize, max_lc: f32, size_factor: f32) -> f32 {
    (4.0 + 7.0 * (link_count as f32 / max_lc).sqrt()) * size_factor
}

/// Creates the search box's `LineInput` (IME-capable) and subscribes to it. The `filter` string
/// mirrors the input's text. Called once from `RustdianApp::new`.
pub fn install(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let input = cx.new(|cx| LineInput::new(cx, "搜索节点"));
    let sub = cx.subscribe(
        &input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            match ev {
                LineInputEvent::Changed(text) => this.graph_ui.filter = text.clone(),
                LineInputEvent::Submit(_) | LineInputEvent::Cancel => {
                    this.graph_ui.search_focused = false;
                    this.want_focus = true;
                }
            }
            cx.notify();
        },
    );
    app.graph_ui.filter_input = Some(input);
    app.graph_ui.filter_subs.push(sub);
}

impl RustdianApp {
    /// 2D knowledge graph: canvas with edges, nodes and labels, plus the filter card,
    /// zoom pill and counts pill. Works at any size (also inside a split pane).
    pub(crate) fn render_graph_view(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let st = &self.graph_ui;
        let graph = &self.vault.graph;
        let (norm, isolated) = layout_for(st, graph);
        let visible = visible_mask(self);
        let selected = selected_node(self);
        let hovered = st
            .hovered
            .filter(|&i| visible.get(i).copied().unwrap_or(false));
        let query = st.filter.trim().to_lowercase();
        let searching = !query.is_empty();
        let max_lc = graph
            .nodes
            .iter()
            .map(|n| n.link_count)
            .max()
            .unwrap_or(1)
            .max(1) as f32;

        // Neighbours of the selected node.
        let mut neighbour = vec![false; graph.nodes.len()];
        if let Some(sel) = selected {
            for e in &graph.edges {
                if e.from_index == sel && e.to_index < neighbour.len() {
                    neighbour[e.to_index] = true;
                }
                if e.to_index == sel && e.from_index < neighbour.len() {
                    neighbour[e.from_index] = true;
                }
            }
        }

        let matches: Vec<bool> = graph
            .nodes
            .iter()
            .map(|n| !searching || n.title.to_lowercase().contains(&query))
            .collect();

        let mut nodes = Vec::new();
        for (i, n) in graph.nodes.iter().enumerate() {
            if !visible[i] {
                continue;
            }
            let kind = if Some(i) == hovered {
                NodeKind::Hot
            } else if Some(i) == selected {
                NodeKind::Selected
            } else if neighbour[i] {
                NodeKind::Linked
            } else if !matches[i] {
                NodeKind::Dim
            } else {
                NodeKind::Plain
            };
            nodes.push(NodeDraw {
                idx: i,
                radius: draw_radius(n.link_count, max_lc, st.size_factor),
                kind,
            });
        }

        let mut plain_edges = Vec::new();
        let mut hot_edges = Vec::new();
        for e in &graph.edges {
            let (a, b) = (e.from_index, e.to_index);
            if a >= graph.nodes.len() || b >= graph.nodes.len() || !visible[a] || !visible[b] {
                continue;
            }
            let hot = Some(a) == selected
                || Some(b) == selected
                || Some(a) == hovered
                || Some(b) == hovered;
            if hot {
                hot_edges.push((a, b));
            } else {
                plain_edges.push((a, b));
            }
        }

        // Label candidates: selected, hovered, search matches, neighbours, most linked.
        let mut top: Vec<usize> = (0..graph.nodes.len()).filter(|&i| visible[i]).collect();
        top.sort_by(|&a, &b| {
            graph.nodes[b]
                .link_count
                .cmp(&graph.nodes[a].link_count)
                .then(a.cmp(&b))
        });
        let top: Vec<usize> = top
            .into_iter()
            .filter(|&i| graph.nodes[i].link_count > 0)
            .take(TOP_LINKED_LABELS)
            .collect();
        let mut candidates: Vec<(u8, usize, usize)> = Vec::new();
        for i in 0..graph.nodes.len() {
            if !visible[i] {
                continue;
            }
            let prio = if Some(i) == selected {
                0
            } else if Some(i) == hovered {
                1
            } else if searching && matches[i] {
                2
            } else if neighbour[i] {
                3
            } else if top.contains(&i) {
                4
            } else {
                continue;
            };
            candidates.push((prio, usize::MAX - graph.nodes[i].link_count, i));
        }
        candidates.sort();
        let labels: Vec<LabelDraw> = candidates
            .into_iter()
            .map(|(prio, _, i)| LabelDraw {
                idx: i,
                text: graph.nodes[i].title.clone(),
                emphasized: prio <= 1,
            })
            .collect();

        let scene = Scene {
            theme: t,
            zoom: st.zoom,
            pan: st.pan,
            norm,
            isolated,
            nodes,
            plain_edges,
            hot_edges,
            labels,
        };
        let canvas_cell = st.canvas_bounds.clone();
        let track_cell = st.track_bounds.clone();

        // Filter card contents.
        let search_focused = st.search_focused;
        let tags: Vec<String> = self.vault.all_tags().into_iter().take(8).collect();
        let active_tag = st.tag.clone();
        let show_isolated = st.show_isolated;
        let size_factor = st.size_factor;
        let slider_frac = (size_factor - SIZE_MIN) / (SIZE_MAX - SIZE_MIN);
        let node_count = graph.nodes.len();
        let edge_count = graph.edges.len();
        let empty = node_count == 0;

        let mut area = div()
            .id("graph_area")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.graph_ui.search_focused = false;
                    window.focus(&this.focus);
                    this.graph_ui.drag = Some(Drag {
                        last: ev.position,
                        start: ev.position,
                        moved: false,
                    });
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if ev.pressed_button == Some(MouseButton::Left) {
                    if let Some(mut drag) = this.graph_ui.drag {
                        let dx = f32::from(ev.position.x) - f32::from(drag.last.x);
                        let dy = f32::from(ev.position.y) - f32::from(drag.last.y);
                        let total = (dx_from(ev.position.x, drag.start.x))
                            .hypot(dx_from(ev.position.y, drag.start.y));
                        if total > 4.0 {
                            drag.moved = true;
                        }
                        this.graph_ui.pan.0 += dx;
                        this.graph_ui.pan.1 += dy;
                        drag.last = ev.position;
                        this.graph_ui.drag = Some(drag);
                        cx.notify();
                    }
                    return;
                }
                this.graph_ui.drag = None;
                let hit = node_at(this, ev.position);
                if hit != this.graph_ui.hovered {
                    this.graph_ui.hovered = hit;
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                    if let Some(drag) = this.graph_ui.drag.take()
                        && !drag.moved
                        && let Some(idx) = node_at(this, ev.position)
                    {
                        let title = this.vault.graph.nodes[idx].title.clone();
                        this.open_note_by_title(&title);
                    }
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        canvas_cell.set(Some(bounds));
                        paint_scene(&scene, bounds, window, cx);
                    },
                )
                .absolute()
                .size_full(),
            );
        if empty {
            area = area.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .text_color(t.tx3)
                    .child("还没有笔记链接。在笔记里写 [[双链]] 后，关系会出现在这里。"),
            );
        }

        // Search box.
        let search_box = div()
            .id("graph_search")
            .h(px(32.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if search_focused { t.ac_line } else { t.line })
            .bg(t.sheet)
            .text_color(t.tx3)
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.graph_ui.search_focused = true;
                    if let Some(input) = this.graph_ui.filter_input.clone() {
                        input.read(cx).focus(window);
                    }
                    cx.notify();
                }),
            )
            .child(icon("search", 15.0, t.tx3))
            .child(match &st.filter_input {
                Some(input) => {
                    input.update(cx, |i, cx| i.set_theme(t, cx));
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(input.clone())
                        .into_any_element()
                }
                None => div().into_any_element(),
            });

        // Tag chips.
        let mut chips = div().flex().flex_wrap().gap(px(6.0));
        for (i, tag) in tags.iter().enumerate() {
            let on = active_tag.as_deref() == Some(tag.as_str());
            let tag_value = tag.clone();
            chips = chips.child(
                div()
                    .id(ElementId::Name(SharedString::from(format!(
                        "graph_tag_{i}"
                    ))))
                    .h(px(22.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .bg(if on { t.ac_soft } else { t.hover })
                    .text_color(if on { t.ac } else { t.tx2 })
                    .child(SharedString::from(format!("#{tag}")))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.graph_ui.tag =
                                if this.graph_ui.tag.as_deref() == Some(tag_value.as_str()) {
                                    None
                                } else {
                                    Some(tag_value.clone())
                                };
                            cx.notify();
                        }),
                    ),
            );
        }

        // Isolated-notes switch.
        let switch = div()
            .id("graph_isolated")
            .relative()
            .w(px(32.0))
            .h(px(18.0))
            .rounded_full()
            .cursor_pointer()
            .bg(if show_isolated { t.ac } else { t.line })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.graph_ui.show_isolated = !this.graph_ui.show_isolated;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .left(px(if show_isolated { 16.0 } else { 2.0 }))
                    .size(px(14.0))
                    .rounded_full()
                    .bg(t.raised),
            );

        // Size slider: click or drag on the track.
        let track_set = move |this: &mut RustdianApp, x: f32| {
            if let Some(b) = this.graph_ui.track_bounds.get() {
                let frac = ((x - f32::from(b.origin.x)) / f32::from(b.size.width).max(1.0))
                    .clamp(0.0, 1.0);
                let v = SIZE_MIN + frac * (SIZE_MAX - SIZE_MIN);
                this.graph_ui.size_factor = (v * 10.0).round() / 10.0;
            }
        };
        let track_set_down = track_set;
        let track = div()
            .id("graph_slider")
            .relative()
            .h(px(4.0))
            .my(px(8.0))
            .rounded(px(2.0))
            .bg(t.line)
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    track_set_down(this, f32::from(ev.position.x));
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                if ev.pressed_button == Some(MouseButton::Left) {
                    track_set(this, f32::from(ev.position.x));
                    cx.notify();
                }
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, _, _| track_cell.set(Some(bounds)),
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(gpui::relative(slider_frac))
                    .rounded(px(2.0))
                    .bg(t.ac),
            )
            .child(
                div()
                    .absolute()
                    .top(px(-5.0))
                    .left(gpui::relative(slider_frac))
                    .ml(px(-7.0))
                    .size(px(14.0))
                    .rounded_full()
                    .bg(t.raised)
                    .border_2()
                    .border_color(t.ac),
            );

        let card = div()
            .absolute()
            .top(px(14.0))
            .right(px(14.0))
            .w(px(240.0))
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .rounded(px(12.0))
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .shadow_lg()
            // Without occlude, the canvas area below also gets the mouse-down: it takes focus back
            // from the search field and starts a pan. The card keeps its clicks to itself.
            .occlude()
            .child(search_box)
            .child(
                div()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.tx3)
                    .child("标签"),
            )
            .child(chips)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(12.5))
                    .text_color(t.tx)
                    .child("显示孤立笔记")
                    .child(switch),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(px(12.5))
                            .text_color(t.tx)
                            .child("节点大小")
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(t.tx2)
                                    .child(SharedString::from(format!("{:.1}x", size_factor))),
                            ),
                    )
                    .child(track),
            );

        // Zoom pill (bottom-left).
        let zoom_btn = |id: &'static str, name: &'static str| {
            div()
                .id(id)
                .w(px(32.0))
                .h(px(30.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon(name, 15.0, t.tx2))
        };
        let zoom_pill = div()
            .absolute()
            .left(px(14.0))
            .bottom(px(14.0))
            .flex()
            .p(px(3.0))
            .gap(px(2.0))
            .rounded_full()
            .bg(with_alpha(t.raised, 0.92))
            .border_1()
            .border_color(t.line)
            .shadow_lg()
            .occlude()
            .child(zoom_btn("graph_zoom_in", "plus").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.graph_ui.zoom = (this.graph_ui.zoom * 1.25).min(ZOOM_MAX);
                    cx.notify();
                }),
            ))
            .child(zoom_btn("graph_zoom_out", "minus").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.graph_ui.zoom = (this.graph_ui.zoom / 1.25).max(ZOOM_MIN);
                    cx.notify();
                }),
            ))
            .child(zoom_btn("graph_zoom_fit", "focus-2").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.graph_ui.zoom = 1.0;
                    this.graph_ui.pan = (0.0, 0.0);
                    cx.notify();
                }),
            ));

        // Counts pill (bottom-right).
        let counts = div()
            .absolute()
            .right(px(14.0))
            .bottom(px(14.0))
            .h(px(26.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded_full()
            .bg(with_alpha(t.raised, 0.92))
            .border_1()
            .border_color(t.line)
            .text_size(px(11.5))
            .text_color(t.tx3)
            .child(SharedString::from(format!(
                "{node_count} 个节点 · {edge_count} 条链接"
            )));

        div()
            .relative()
            .size_full()
            .bg(t.sheet)
            .overflow_hidden()
            .child(area)
            .child(card)
            .child(zoom_pill)
            .child(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mean_edge_ratio(pos: &[(f32, f32)], edges: &[(usize, usize)]) -> f32 {
        let n = pos.len();
        let d = |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        let mut all = 0.0;
        let mut cnt: f32 = 0.0;
        for i in 0..n {
            for j in (i + 1)..n {
                all += d(pos[i], pos[j]);
                cnt += 1.0;
            }
        }
        let mean_all = all / cnt.max(1.0);
        let mean_edge =
            edges.iter().map(|&(a, b)| d(pos[a], pos[b])).sum::<f32>() / edges.len().max(1) as f32;
        mean_edge / mean_all.max(1e-6)
    }

    fn test_graph(n: usize) -> Vec<(usize, usize)> {
        // Ring plus deterministic chords: every node has at least two edges.
        let mut edges: Vec<(usize, usize)> = (0..n).map(|i| (i, (i + 1) % n)).collect();
        for i in 0..n {
            let j = (i * 7 + 3) % n;
            if j != i && !edges.contains(&(i, j)) && !edges.contains(&(j, i)) {
                edges.push((i, j));
            }
        }
        edges
    }

    #[test]
    fn layout_is_finite_and_within_bounds() {
        for n in [0usize, 1, 2, 5, 40, 150] {
            let edges = test_graph(n.max(2));
            let pos = force_layout(n, &edges);
            assert_eq!(pos.len(), n);
            for (x, y) in pos {
                assert!(x.is_finite() && y.is_finite());
                assert!((0.0..=1.0).contains(&x), "x out of bounds: {x}");
                assert!((0.0..=1.0).contains(&y), "y out of bounds: {y}");
            }
        }
    }

    #[test]
    fn layout_is_deterministic() {
        let edges = test_graph(30);
        assert_eq!(force_layout(30, &edges), force_layout(30, &edges));
    }

    #[test]
    fn linked_nodes_end_closer_than_at_start() {
        let n = 30;
        let edges = test_graph(n);
        let start = initial_positions(n);
        let end = force_layout(n, &edges);
        let before = mean_edge_ratio(&start, &edges);
        let after = mean_edge_ratio(&end, &edges);
        assert!(
            after < before,
            "edges not contracted: before {before}, after {after}"
        );
    }

    /// Connected ring-with-chords of `conn` notes followed by `iso` isolated notes.
    fn sample(conn: usize, iso: usize) -> (Vec<(f32, f32)>, Vec<bool>) {
        let n = conn + iso;
        let pos = force_layout(conn, &test_graph(conn));
        let mut norm = vec![(0.5f32, 0.5f32); n];
        norm[..conn].copy_from_slice(&pos);
        let isolated = (0..n).map(|i| i >= conn).collect();
        (norm, isolated)
    }

    const SIZES: [(f32, f32); 3] = [(1000.0, 800.0), (1280.0, 860.0), (960.0, 640.0)];

    #[test]
    fn placed_nodes_stay_clear_of_the_filter_card() {
        for (w, h) in SIZES {
            let (norm, iso) = sample(24, 3);
            let card = card_rect(w);
            for (x, y) in place(&norm, &iso, w, h) {
                // Node radius is at most 22 px at the largest size slider value.
                let under = x > card.x0 - 22.0
                    && x < card.x1 + 22.0
                    && y > card.y0 - 22.0
                    && y < card.y1 + 22.0;
                assert!(!under, "node at ({x}, {y}) under the card on {w}x{h}");
            }
        }
    }

    #[test]
    fn fitted_layout_stays_inside_margins() {
        for (w, h) in SIZES {
            let (norm, iso) = sample(24, 3);
            let r = fit_rect(w, h);
            assert!(r.x0 >= MARGIN - 1e-3 && r.y0 >= MARGIN - 1e-3);
            assert!(r.x1 <= w - MARGIN + 1e-3 && r.y1 <= h - MARGIN + 1e-3);
            for (x, y) in place(&norm, &iso, w, h) {
                assert!(
                    x >= MARGIN - 1e-3 && x <= w - MARGIN + 1e-3,
                    "x {x} outside"
                );
                assert!(
                    y >= MARGIN - 1e-3 && y <= h - MARGIN + 1e-3,
                    "y {y} outside"
                );
            }
        }
    }

    #[test]
    fn fit_rect_does_not_touch_the_card() {
        for (w, h) in SIZES {
            assert!(!fit_rect(w, h).intersects(&card_rect(w)));
        }
    }

    #[test]
    fn isolated_notes_form_a_row_on_the_bottom_edge() {
        let (w, h) = (1000.0, 800.0);
        let (norm, iso) = sample(10, 4);
        let pos = place(&norm, &iso, w, h);
        let bottom = fit_rect(w, h).y1;
        let row = &pos[10..];
        assert!(row.iter().all(|p| (p.1 - bottom).abs() < 1e-3));
        assert!(row.windows(2).all(|p| p[0].0 < p[1].0));
    }

    #[test]
    fn layout_key_changes_with_titles_and_edges() {
        let base = graph_key(&["x", "y"], &[(0, 1)]);
        assert_eq!(base, graph_key(&["x", "y"], &[(0, 1)]));
        assert_ne!(base, graph_key(&["x", "z"], &[(0, 1)]));
        assert_ne!(base, graph_key(&["x", "y"], &[]));
    }

    #[test]
    fn radius_grows_with_link_count() {
        assert!(node_radius(10, 10.0, 1.0) > node_radius(0, 10.0, 1.0));
        assert!((node_radius(0, 10.0, 1.0) - 4.0).abs() < 1e-4);
        assert!((node_radius(10, 10.0, 1.0) - 11.0).abs() < 1e-4);
    }
}
