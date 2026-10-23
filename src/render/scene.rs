//! Document + camera → world-space cached meshes, plus the screen-space overlay.

use super::draw;
use super::mesh::{Batch, MeshBuilder};
use super::tess::{Piece, Tess};
use crate::editor::{Editor, hit, snap::Guide};
use crate::geom::{Camera, Rect, V2, v2};
use crate::model::{BlendMode, Node};
use crate::model::{Color, Document, NodeId, NodeKind, TextAlign};
use crate::text;
use lyon::tessellation::{FillRule, LineCap};
use std::collections::HashMap;
use std::sync::Arc;

pub const CANVAS_BG: Color = Color::rgb(0xee, 0xee, 0xf0);
pub const GRID_DOT: Color = Color::rgb(0xcf, 0xcf, 0xd6);
/// The dark work surface and its grid (see [`Display::Dark`]).
pub const CANVAS_BG_DARK: Color = Color::rgb(0x18, 0x18, 0x1b);
pub const GRID_DOT_DARK: Color = Color::rgb(0x34, 0x34, 0x3a);
pub const ACCENT: Color = Color::rgb(0x2f, 0x6f, 0xf3);
pub const GUIDE: Color = Color::rgb(0xf2, 0x2f, 0x7a);

pub fn rgba(c: Color) -> [f32; 4] {
    c.to_f32(1.0)
}

// ---------------------------------------------------------------------------------- display

/// How the interactive canvas is shown: presentation only, never the document. `Light` draws
/// the document exactly as authored, as every export does (exports always use `Light`).
/// `Dark` draws a dark work surface and adapts each drawn colour to it: its OKLab lightness is
/// mirrored into a dark range while its hue, its chroma (as far as sRGB allows) and its alpha
/// are kept. Black text and strokes read light, white fills sit just above the surface, and a
/// red shape stays red. Image paints are never adapted (artwork stays as it is), and nothing
/// in the document, its history or its saves changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Light,
    Dark,
}

/// Adapted lightness range of [`Display::Dark`]: authored black (L 0) is shown at `HI`,
/// authored white (L 1) at `LO`, just above the dark surface (L ≈ 0.21).
const DARK_HI: f32 = 0.93;
const DARK_LO: f32 = 0.27;

impl Display {
    /// The display for an egui theme (the resolved appearance of the window or page).
    pub fn for_theme(theme: egui::Theme) -> Display {
        match theme {
            egui::Theme::Dark => Display::Dark,
            egui::Theme::Light => Display::Light,
        }
    }

    pub fn background(self) -> Color {
        match self {
            Display::Light => CANVAS_BG,
            Display::Dark => CANVAS_BG_DARK,
        }
    }

    pub fn grid_dot(self) -> Color {
        match self {
            Display::Light => GRID_DOT,
            Display::Dark => GRID_DOT_DARK,
        }
    }

    /// A drawn colour (straight-alpha sRGB) as shown. The alpha is never changed.
    pub fn color(self, c: [f32; 4]) -> [f32; 4] {
        match self {
            Display::Light => c,
            Display::Dark => {
                let [l, a, b] = oklab([c[0], c[1], c[2]]);
                let l = DARK_HI - l.clamp(0.0, 1.0) * (DARK_HI - DARK_LO);
                let [r, g, b] = from_oklab_in_gamut(l, a, b);
                [r, g, b, c[3]]
            }
        }
    }

    fn model_color(self, c: Color) -> Color {
        if self == Display::Light {
            return c;
        }
        let [r, g, b, _] = self.color(c.to_f32(1.0));
        let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        Color::rgba(byte(r), byte(g), byte(b), c.a)
    }

    /// Adapt a node's draw ops for display: solid fills, gradient stops (positions kept),
    /// strokes and glyphs. Image paints are left exactly as they are.
    pub fn ops(self, ops: &mut [draw::DrawOp]) {
        if self == Display::Light {
            return;
        }
        use draw::{DrawOp, PaintSpec};
        for op in ops {
            match op {
                DrawOp::Fill { paint, .. } => match paint {
                    PaintSpec::Solid(c) => *c = self.color(*c),
                    PaintSpec::Linear { stops, .. } | PaintSpec::Radial { stops, .. } => {
                        for s in stops.iter_mut() {
                            s.color = self.model_color(s.color);
                        }
                    }
                    PaintSpec::Image { .. } => {}
                },
                DrawOp::Stroke { color, .. } | DrawOp::Glyphs { color, .. } => {
                    *color = self.color(*color)
                }
            }
        }
    }
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB (0..1) to OKLab (Björn Ottosson's published matrices).
fn oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    let (r, g, b) = (to_linear(r), to_linear(g), to_linear(b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_oklab([l, a, b]: [f32; 3]) -> [f32; 3] {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        to_srgb(4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_),
        to_srgb(-1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_),
        to_srgb(-0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_),
    ]
}

/// OKLab to sRGB at lightness `l`, reducing chroma (never the hue) until it fits sRGB.
fn from_oklab_in_gamut(l: f32, a: f32, b: f32) -> [f32; 3] {
    let fits = |c: &[f32; 3]| c.iter().all(|v| (-0.001..=1.001).contains(v));
    let mut k = 1.0;
    let mut c = from_oklab([l, a, b]);
    for _ in 0..12 {
        if fits(&c) {
            break;
        }
        k *= 0.85;
        c = from_oklab([l, a * k, b * k]);
    }
    c.map(|v| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    })
}

/// What to draw and where.
pub struct View<'a> {
    pub doc: &'a Document,
    pub camera: Camera,
    /// Canvas size in logical px.
    pub viewport: V2,
    /// Canvas top-left in the render target, logical px.
    pub offset: V2,
}

impl View<'_> {
    fn to_screen(&self, p: V2) -> V2 {
        self.camera.world_to_screen(p) + self.offset
    }
    fn rect_to_screen(&self, r: &Rect) -> Rect {
        self.camera.world_rect_to_screen(r).translate(self.offset)
    }
    fn canvas_rect(&self) -> Rect {
        Rect::new(
            self.offset.x,
            self.offset.y,
            self.viewport.x,
            self.viewport.y,
        )
    }
}

/// The interactive canvas's background pattern (DESIGN.md): a view preference, never part of
/// the document, its history, its saves, snapping or any export. A plain surface (`None`) by
/// default; `Dots` is the zoom-adaptive dot grid; `Lines` the same spacing as thin lines, shown
/// to people as "Grid" (its stored name stays `lines`, so kept preferences still load).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Background {
    #[default]
    None,
    Dots,
    Lines,
}

impl Background {
    pub const ALL: [Background; 3] = [Background::None, Background::Dots, Background::Lines];

    /// The stored name (`none`, `dots` or `lines`).
    pub fn name(self) -> &'static str {
        match self {
            Background::None => "none",
            Background::Dots => "dots",
            Background::Lines => "lines",
        }
    }

    pub fn from_name(name: &str) -> Option<Background> {
        Background::ALL.into_iter().find(|b| b.name() == name)
    }

    /// The label every menu, the inspector and the palette show: None, Dots or Grid. A view
    /// pattern, never something on the Paper.
    pub fn label(self) -> &'static str {
        match self {
            Background::None => "None",
            Background::Dots => "Dots",
            Background::Lines => "Grid",
        }
    }
}

/// The old on/off grid switch: on is the dot grid.
impl From<bool> for Background {
    fn from(grid: bool) -> Background {
        if grid {
            Background::Dots
        } else {
            Background::None
        }
    }
}

/// Dot grid whose spacing adapts to the zoom so it never gets too dense or sparse.
pub fn grid(mb: &mut MeshBuilder, v: &View) {
    grid_dots(mb, v, GRID_DOT);
}

/// World spacing of the background pattern at zoom `z`: 20 world px, scaled by fives so it stays
/// between 14 and 70 screen px apart.
fn grid_spacing(z: f64) -> f64 {
    let mut spacing = 20.0;
    while spacing * z < 14.0 {
        spacing *= 5.0;
    }
    while spacing * z > 70.0 {
        spacing /= 5.0;
    }
    spacing
}

/// Line grid: the dot grid's spacing, as 1 px lines across the canvas.
fn grid_lines(mb: &mut MeshBuilder, v: &View, line: Color) {
    let spacing = grid_spacing(v.camera.zoom);
    let vis = v.camera.visible_world(v.viewport);
    let c = v.canvas_rect();
    // Lines cover more of the surface than dots: a lighter stroke keeps the same quiet weight.
    let [r, g, b, _] = rgba(line);
    let color = [r, g, b, 0.6];
    let x0 = (vis.x / spacing).floor() as i64;
    let x1 = (vis.right() / spacing).ceil() as i64;
    // Axis-aligned, so each line is a 1 px rectangle (no stroke tessellation per frame).
    for xi in x0..=x1 {
        let x = v.to_screen(v2(xi as f64 * spacing, 0.0)).x;
        mb.rect(Rect::new(x - 0.5, c.y, 1.0, c.h), color);
    }
    let y0 = (vis.y / spacing).floor() as i64;
    let y1 = (vis.bottom() / spacing).ceil() as i64;
    for yi in y0..=y1 {
        let y = v.to_screen(v2(0.0, yi as f64 * spacing)).y;
        mb.rect(Rect::new(c.x, y - 0.5, c.w, 1.0), color);
    }
}

fn grid_dots(mb: &mut MeshBuilder, v: &View, dot: Color) {
    let spacing = grid_spacing(v.camera.zoom);
    let vis = v.camera.visible_world(v.viewport);
    let x0 = (vis.x / spacing).floor() as i64;
    let y0 = (vis.y / spacing).floor() as i64;
    let x1 = (vis.right() / spacing).ceil() as i64;
    let y1 = (vis.bottom() / spacing).ceil() as i64;
    let color = rgba(dot);
    let r = 0.9;
    for yi in y0..=y1 {
        for xi in x0..=x1 {
            let s = v.to_screen(v2(xi as f64 * spacing, yi as f64 * spacing));
            mb.rect(Rect::new(s.x - r, s.y - r, 2.0 * r, 2.0 * r), color);
        }
    }
}

// ---------------------------------------------------------------------------------- world scene

/// One node's cached triangles plus where they are clipped and how they blend.
#[derive(Clone)]
pub struct Item {
    pub id: NodeId,
    pub pieces: Arc<Vec<Piece>>,
    /// World-space clip rectangle (from clipping frames).
    pub clip: Option<Rect>,
    pub blend: BlendMode,
}

struct Entry {
    node: Node,
    wr: Rect,
    opacity: f64,
    bucket: i32,
    pieces: Arc<Vec<Piece>>,
    used: bool,
}

/// Per-node tessellation cache. A node is re-tessellated only when it (or its world position,
/// inherited opacity or the zoom bucket) changes, so panning and idle frames do no work.
#[derive(Default)]
pub struct SceneCache {
    entries: HashMap<NodeId, Entry>,
    /// Item list of the previous frame and what it was built for (doc stamp, zoom bucket,
    /// culled world rect): reused while the view stays inside that rect.
    last: Option<(u64, i32, Rect, Vec<Item>)>,
    /// Tessellations performed during the last `items` call (for tests and profiling).
    pub last_misses: usize,
    /// How the items are shown (see [`Display`]); `Light`, the document as authored, by
    /// default, as for every export.
    display: Display,
}

/// Zoom buckets are powers of two: geometry built for 2^b is accurate up to 2^(b+1).
pub fn zoom_bucket(zoom: f64) -> i32 {
    zoom.max(1e-6).log2().floor() as i32
}

impl SceneCache {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last = None;
    }

    pub fn display(&self) -> Display {
        self.display
    }

    /// Show the items with `display` from now on. A change drops the cached meshes (they carry
    /// their shown colours); the document is not touched.
    pub fn set_display(&mut self, display: Display) {
        if self.display != display {
            self.display = display;
            self.clear();
        }
    }

    /// Items for `roots` (or the whole page) in paint order. `cull` skips subtrees outside a
    /// world rect.
    pub fn items(
        &mut self,
        doc: &Document,
        roots: Option<&[NodeId]>,
        zoom: f64,
        cull: Option<Rect>,
    ) -> Vec<Item> {
        let bucket = zoom_bucket(zoom);
        let tz = 2f64.powi(bucket);
        let mut tess = Tess::new(tz);
        let mut out = Vec::new();
        self.last_misses = 0;
        for e in self.entries.values_mut() {
            e.used = false;
        }
        let all = doc.root.clone();
        for id in roots.unwrap_or(&all) {
            let op = draw::inherited_opacity(doc, *id);
            let origin = doc.parent_origin(doc.get(*id).and_then(|n| n.parent));
            self.walk(
                doc, *id, origin, None, op, bucket, &mut tess, cull, &mut out,
            );
        }
        // Evict entries for deleted or long-unseen nodes when the cache grows large.
        let used = self.entries.values().filter(|e| e.used).count();
        if self.entries.len() > used * 4 + 2000 {
            self.entries.retain(|_, e| e.used);
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        doc: &Document,
        id: NodeId,
        origin: V2,
        clip: Option<Rect>,
        parent_opacity: f64,
        bucket: i32,
        tess: &mut Tess,
        cull: Option<Rect>,
        out: &mut Vec<Item>,
    ) {
        let Some(n) = doc.get(id) else { return };
        if !n.visible {
            return;
        }
        let wr = n.rect.translate(origin);
        let opacity = parent_opacity * n.style.opacity;
        let bounds = draw::draw_bounds(n, wr);
        let visible = cull.is_none_or(|c| c.intersects(&bounds))
            && clip.is_none_or(|c| c.intersects(&bounds));
        if visible && !n.is_group() {
            let hit = self.entries.get(&id).is_some_and(|e| {
                e.bucket == bucket && e.wr == wr && e.opacity == opacity && e.node == *n
            });
            if !hit {
                self.last_misses += 1;
                let mut d = draw::node_draw(n, wr, opacity);
                self.display.ops(&mut d.ops);
                let pieces = Arc::new(tess.ops(&d.ops));
                self.entries.insert(
                    id,
                    Entry {
                        node: n.clone(),
                        wr,
                        opacity,
                        bucket,
                        pieces,
                        used: true,
                    },
                );
            }
            let e = self.entries.get_mut(&id).unwrap();
            e.used = true;
            if !e.pieces.is_empty() {
                out.push(Item {
                    id,
                    pieces: e.pieces.clone(),
                    clip,
                    blend: n.style.blend,
                });
            }
        }
        if n.children.is_empty() {
            return;
        }
        let child_clip = match n.kind {
            NodeKind::Frame { clip: true } => {
                match clip.map_or(Some(wr), |c| c.intersection(&wr)) {
                    Some(c) => Some(c),
                    None => return,
                }
            }
            _ => clip,
        };
        if matches!(n.kind, NodeKind::Frame { clip: true })
            && cull.is_some_and(|c| !c.intersects(&wr))
        {
            return;
        }
        let o = wr.min();
        for c in &n.children {
            self.walk(doc, *c, o, child_clip, opacity, bucket, tess, cull, out);
        }
    }
}

/// Everything the GPU draws for the canvas: screen-space underlay (background, grid), the
/// world-space document, and the screen-space overlay (selection, guides, labels).
pub struct CanvasFrame {
    pub under: Vec<Batch>,
    pub items: Vec<Item>,
    pub over: Vec<Batch>,
    pub camera: Camera,
    /// Canvas top-left in the render target, logical px.
    pub offset: V2,
    /// Canvas rect in logical px (world items are scissored to it).
    pub viewport: Rect,
}

/// Draw text with its box's top-left at `origin` (screen px) at screen scale `z`.
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    mb: &mut MeshBuilder,
    t: &str,
    font_size: f64,
    wrap: Option<f64>,
    align: TextAlign,
    origin: V2,
    z: f64,
    color: [f32; 4],
) {
    let layout = text::layout(t, font_size, wrap, align);
    let px = font_size * z;
    if px < 2.5 {
        // Too small to read: draw "greeked" bars instead of glyphs.
        for l in &layout.lines {
            let lh = l.height * z;
            if l.width > 0.0 {
                let x = origin.x + l.carets[0] * z;
                mb.rect(
                    Rect::new(x, origin.y + l.top * z + lh * 0.3, l.width * z, lh * 0.4),
                    [color[0], color[1], color[2], color[3] * 0.35],
                );
            }
        }
        return;
    }
    for g in &layout.glyphs {
        let s = (g.size / text::face_of(g.face).units_per_em() as f64 * z) as f32;
        let m = text::glyph_mesh(g.face, g.gid);
        if m.indices.is_empty() {
            continue;
        }
        let o = origin + v2(g.x, g.baseline) * z;
        let (ox, oy) = (o.x as f32, o.y as f32);
        mb.triangles(
            m.vertices.iter().map(|v| [ox + v[0] * s, oy - v[1] * s]),
            &m.indices,
            color,
        );
    }
}

/// Interactive overlay: hover, selection, handles, marquee, guides, labels, caret.
pub fn overlay(mb: &mut MeshBuilder, ed: &Editor, offset: V2) {
    overlay_shown(mb, ed, offset, Display::Light);
}

/// [`overlay`] over a canvas shown with `display`: only the quiet frame-name labels change, so
/// they stay legible on the dark surface; the accent marks are the same in both.
fn overlay_shown(mb: &mut MeshBuilder, ed: &Editor, offset: V2, display: Display) {
    let label = match display {
        Display::Light => [0.42, 0.42, 0.46, 1.0],
        Display::Dark => [0.64, 0.64, 0.69, 1.0],
    };
    let v = View {
        doc: &ed.doc,
        camera: ed.camera,
        viewport: ed.viewport,
        offset,
    };
    let accent = rgba(ACCENT);
    let z = ed.camera.zoom;
    mb.set_clip(Some(v.canvas_rect()));

    // Frame name labels.
    for (id, r) in ed.frame_labels() {
        let n = ed.doc.get(id).unwrap();
        let selected = ed.selection.contains(&id);
        let color = if selected { accent } else { label };
        let r = r.translate(offset);
        let mut name = n.name.clone();
        let max_w = r.w;
        while !name.is_empty() && text::layout(&name, 12.0, None, TextAlign::Left).width > max_w {
            name.pop();
        }
        draw_text(
            mb,
            &name,
            12.0,
            None,
            TextAlign::Left,
            v2(r.x, r.y + 2.0),
            1.0,
            color,
        );
    }

    if let Some(h) = ed.hover.filter(|h| !ed.selection.contains(h))
        && let Some(wr) = ed.doc.world_rect(h)
    {
        outline_node(mb, &v, h, v.rect_to_screen(&wr), accent, 1.5);
    }

    for id in &ed.selection {
        if let Some(wr) = ed.doc.world_rect(*id) {
            outline_node(mb, &v, *id, v.rect_to_screen(&wr), accent, 1.0);
        }
    }

    if let Some(te) = &ed.text_edit {
        if let (Some(n), Some(wr)) = (ed.doc.get(te.id), ed.doc.world_rect(te.id))
            && let Some(l) = text::layout_node(n)
        {
            let o = v.to_screen(wr.min() + text::text_offset(n));
            if let Some(a) = te.anchor.filter(|a| *a != te.caret) {
                for (x, top, w, h) in l.selection_rects(a, te.caret) {
                    mb.rect(
                        Rect::new(o.x + x * z, o.y + top * z, w * z, h * z),
                        [accent[0], accent[1], accent[2], 0.25],
                    );
                }
            }
            let (x, top, h) = l.caret(te.caret);
            mb.rect(
                Rect::new(o.x + x * z - 0.75, o.y + top * z, 1.5, h * z),
                accent,
            );
        }
    } else if let Some(b) = ed.selection_bounds() {
        let sb = v.rect_to_screen(&b);
        if ed.selection.len() > 1 {
            mb.rect_outline(sb, 1.0, accent);
        }
        let tiny = sb.w.min(sb.h) < 24.0;
        for (hx, hy) in hit::HANDLES {
            if tiny && (hx == 0.5 || hy == 0.5) {
                continue;
            }
            let c = v2(sb.x + sb.w * hx, sb.y + sb.h * hy);
            let r = Rect::new(c.x - 4.0, c.y - 4.0, 8.0, 8.0);
            mb.rect(r, [1.0, 1.0, 1.0, 1.0]);
            mb.rect_outline(r, 1.0, accent);
        }
        // Size badge below the selection.
        let label = format!("{} × {}", fmt_num(b.w), fmt_num(b.h));
        let w = text::layout(&label, 11.0, None, TextAlign::Left).width;
        let pill = Rect::new(
            sb.center().x - w * 0.5 - 6.0,
            sb.bottom() + 8.0,
            w + 12.0,
            18.0,
        );
        mb.fill_path(
            &MeshBuilder::rounded_rect_path(pill, 4.0),
            accent,
            FillRule::NonZero,
        );
        draw_text(
            mb,
            &label,
            11.0,
            None,
            TextAlign::Left,
            v2(pill.x + 6.0, pill.y + 2.0),
            1.0,
            [1.0; 4],
        );
    }

    if let Some(m) = ed.marquee {
        let sm = v.rect_to_screen(&m);
        mb.rect(sm, [accent[0], accent[1], accent[2], 0.08]);
        mb.rect_outline(sm, 1.0, accent);
    }

    let guide = rgba(GUIDE);
    for g in ed.guides.iter().chain(&ed.measures) {
        match *g {
            Guide::V { x, y0, y1 } => {
                mb.line(v.to_screen(v2(x, y0)), v.to_screen(v2(x, y1)), 1.0, guide)
            }
            Guide::H { y, x0, x1 } => {
                mb.line(v.to_screen(v2(x0, y)), v.to_screen(v2(x1, y)), 1.0, guide)
            }
            Guide::Gap { a, b } => gap_guide(mb, &v, a, b, guide),
        }
    }
    vector_edit_overlay(mb, ed, &v, accent);
    whiteboard_overlay(mb, ed, &v, accent);
    mb.set_clip(None);
}

/// Arrow handles, bind targets, quick-connect knobs and the laser trail.
fn whiteboard_overlay(mb: &mut MeshBuilder, ed: &Editor, v: &View, accent: [f32; 4]) {
    let off = v.offset;
    if let Some(t) = ed.bind_hover
        && let Some(wr) = ed.doc.world_rect(t)
    {
        let n = ed.doc.get(t).unwrap();
        let path = draw::shape_path(n, wr.inflate(4.0 / v.camera.zoom))
            .unwrap_or_else(|| crate::vector::rounded_rect(wr, [0.0; 4]));
        let sp = path.map(|p| v.to_screen(p));
        mb.stroke_path(
            &sp.to_lyon(),
            3.0,
            [accent[0], accent[1], accent[2], 0.45],
            LineCap::Round,
        );
    }
    for (end, p) in ed.connector_handles() {
        let p = p + off;
        // Bend handles (2 and up: a straight arrow's middle, or a curve's interior points).
        if end >= 2 {
            mb.circle(p, 4.0, [accent[0], accent[1], accent[2], 0.5]);
        } else {
            mb.circle(p, 5.5, accent);
            mb.circle(p, 4.0, [1.0; 4]);
        }
    }
    for (_, p) in ed.quick_knobs() {
        let p = p + off;
        mb.circle(p, 8.0, [accent[0], accent[1], accent[2], 0.9]);
        mb.line(p - v2(4.0, 0.0), p + v2(4.0, 0.0), 1.6, [1.0; 4]);
        mb.line(p - v2(0.0, 4.0), p + v2(0.0, 4.0), 1.6, [1.0; 4]);
    }
    let trail = ed.laser_trail(ed.now_ms);
    if trail.len() >= 2 {
        let red = [0.98, 0.18, 0.2, 1.0];
        for w in trail.windows(2) {
            let (a, ka) = w[0];
            let (b, kb) = w[1];
            let width = 4.0 * (ka + kb) * 0.5;
            mb.line(v.to_screen(a), v.to_screen(b), width, red);
            mb.circle(v.to_screen(b), width * 0.5, red);
        }
        let (head, _) = *trail.last().unwrap();
        mb.circle(v.to_screen(head), 3.5, [1.0, 0.6, 0.6, 1.0]);
    } else if let Some((p, _)) = trail.first() {
        mb.circle(v.to_screen(*p), 3.0, [0.98, 0.18, 0.2, 1.0]);
    }
}

/// A measured distance: line with end ticks and a pill showing the length.
fn gap_guide(mb: &mut MeshBuilder, v: &View, a: V2, b: V2, color: [f32; 4]) {
    let (sa, sb) = (v.to_screen(a), v.to_screen(b));
    mb.line(sa, sb, 1.0, color);
    let n = (sb - sa).normalized().perp() * 4.0;
    mb.line(sa - n, sa + n, 1.0, color);
    mb.line(sb - n, sb + n, 1.0, color);
    let label = fmt_num(a.dist(b));
    let w = text::layout(&label, 11.0, None, TextAlign::Left).width;
    let c = (sa + sb) * 0.5;
    let horizontal = (sb.x - sa.x).abs() >= (sb.y - sa.y).abs();
    let pill = if horizontal {
        Rect::new(c.x - w * 0.5 - 4.0, c.y + 4.0, w + 8.0, 16.0)
    } else {
        Rect::new(c.x + 6.0, c.y - 8.0, w + 8.0, 16.0)
    };
    mb.fill_path(
        &MeshBuilder::rounded_rect_path(pill, 3.0),
        color,
        FillRule::NonZero,
    );
    draw_text(
        mb,
        &label,
        11.0,
        None,
        TextAlign::Left,
        v2(pill.x + 4.0, pill.y + 1.0),
        1.0,
        [1.0; 4],
    );
}

/// Anchors and handles while drawing with the pen or editing a path.
fn vector_edit_overlay(mb: &mut MeshBuilder, ed: &Editor, v: &View, accent: [f32; 4]) {
    let (id, selected, pen) = match (&ed.pen, &ed.path_edit) {
        (Some(p), _) => (p.id, None, true),
        (None, Some(pe)) => (pe.id, pe.selected, false),
        _ => return,
    };
    let (Some(n), Some(wr)) = (ed.doc.get(id), ed.doc.world_rect(id)) else {
        return;
    };
    let NodeKind::Path { subpaths, .. } = &n.kind else {
        return;
    };
    let s = |p: V2| v.to_screen(wr.min() + p);
    // Path outline in accent so it stays visible over any fill.
    let path = crate::vector::from_subpaths(wr.min(), subpaths).map(|p| v.to_screen(p));
    mb.stroke_path(&path.to_lyon(), 1.0, accent, LineCap::Round);
    for (si, sp) in subpaths.iter().enumerate() {
        let last = sp.points.len().saturating_sub(1);
        for (pi, pt) in sp.points.iter().enumerate() {
            let show_handles = selected == Some((si, pi)) || (pen && pi == last);
            if show_handles {
                for h in [pt.h_in, pt.h_out].into_iter().flatten() {
                    mb.line(s(pt.p), s(h), 1.0, accent);
                    mb.circle(s(h), 3.5, accent);
                    mb.circle(s(h), 2.0, [1.0; 4]);
                }
            }
            let c = s(pt.p);
            let r = Rect::new(c.x - 3.5, c.y - 3.5, 7.0, 7.0);
            let sel = selected == Some((si, pi));
            mb.rect(r, if sel { accent } else { [1.0; 4] });
            mb.rect_outline(r, 1.0, accent);
        }
    }
    // Rubber band from the last pen point to the pointer.
    if pen
        && !ed.is_busy()
        && let Some(last) = subpaths.first().and_then(|sp| sp.points.last())
    {
        mb.line(
            s(last.p),
            ed.pointer + v.offset,
            1.0,
            [accent[0], accent[1], accent[2], 0.6],
        );
    }
}

fn outline_node(mb: &mut MeshBuilder, v: &View, id: NodeId, sr: Rect, color: [f32; 4], w: f64) {
    let n = v.doc.get(id).unwrap();
    let z = v.camera.zoom;
    match &n.kind {
        NodeKind::Frame { .. } | NodeKind::Rect | NodeKind::Text { .. } | NodeKind::Group => {
            mb.rect_outline(sr, w, color)
        }
        NodeKind::Stroke { points, .. } if points.len() > 1 => {
            let pts: Vec<V2> = points.iter().map(|q| sr.min() + q.pos() * z).collect();
            mb.stroke_path(
                &MeshBuilder::polygon_path(&pts, false),
                w as f32,
                color,
                LineCap::Round,
            );
            mb.rect_outline(sr, 1.0, [color[0], color[1], color[2], 0.5]);
        }
        _ => {
            let wr = v.doc.world_rect(id).unwrap();
            if let Some(path) = draw::shape_path(n, wr) {
                let sp = path.map(|p| v.to_screen(p));
                mb.stroke_path(&sp.to_lyon(), w as f32, color, LineCap::Round);
            }
        }
    }
}

fn fmt_num(x: f64) -> String {
    let r = (x * 10.0).round() / 10.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r:.1}")
    }
}

/// Full interactive canvas: background pattern (`background`; `true`/`false` are the dot grid
/// or none), document, overlay, shown with the cache's [`Display`]. The pattern is built each
/// frame in screen space under the cached document items, so switching it touches no cache.
pub fn canvas(
    ed: &Editor,
    cache: &mut SceneCache,
    offset: V2,
    background: impl Into<Background>,
) -> CanvasFrame {
    let v = View {
        doc: &ed.doc,
        camera: ed.camera,
        viewport: ed.viewport,
        offset,
    };
    let display = cache.display();
    let mut under = MeshBuilder::default();
    under.set_clip(Some(v.canvas_rect()));
    under.rect(v.canvas_rect(), rgba(display.background()));
    match background.into() {
        Background::None => {}
        Background::Dots => grid_dots(&mut under, &v, display.grid_dot()),
        Background::Lines => grid_lines(&mut under, &v, display.grid_dot()),
    }
    let vis = ed.camera.visible_world(ed.viewport);
    let bucket = zoom_bucket(ed.camera.zoom);
    let items = match &cache.last {
        Some((stamp, b, cull, items))
            if *stamp == ed.doc_stamp && *b == bucket && cull.contains_rect(&vis) =>
        {
            cache.last_misses = 0;
            items.clone()
        }
        _ => {
            let cull = vis.inflate(vis.w.max(vis.h) * 0.5);
            let items = cache.items(&ed.doc, None, ed.camera.zoom, Some(cull));
            cache.last = Some((ed.doc_stamp, bucket, cull, items.clone()));
            items
        }
    };
    let mut over = MeshBuilder::default();
    overlay_shown(&mut over, ed, offset, display);
    CanvasFrame {
        under: under.finish(),
        items,
        over: over.finish(),
        camera: ed.camera,
        offset,
        viewport: v.canvas_rect(),
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;

    fn under_vertices(ed: &Editor, b: Background) -> usize {
        let f = canvas(ed, &mut SceneCache::default(), V2::ZERO, b);
        f.under.iter().map(|b| b.mesh.vertices.len()).sum()
    }

    #[test]
    fn none_is_the_plain_surface_and_dots_and_lines_are_underlay_only() {
        let mut ed = Editor::default();
        let r = Rect::new(100.0, 100.0, 50.0, 50.0);
        ed.add_node(Node::new(NodeId(0), NodeKind::Rect, r), None);
        assert_eq!(
            Background::default(),
            Background::None,
            "a fresh view is plain"
        );
        assert_eq!(
            under_vertices(&ed, Background::None),
            4,
            "the surface alone"
        );
        let dots = under_vertices(&ed, Background::Dots);
        let lines = under_vertices(&ed, Background::Lines);
        assert!(dots > 4 && lines > 4 && dots != lines, "{dots} {lines}");
        // The old switch: on is the dot grid, off is none.
        assert_eq!(under_vertices(&ed, Background::from(true)), dots);
        assert_eq!(Background::from(false), Background::None);
        // The document's own meshes are the same whatever the pattern.
        let items = |b: Background| {
            let f = canvas(&ed, &mut SceneCache::default(), V2::ZERO, b);
            f.items
                .iter()
                .map(|i| (i.id, i.pieces.len()))
                .collect::<Vec<_>>()
        };
        assert_eq!(items(Background::None), items(Background::Lines));
        assert_eq!(items(Background::None), items(Background::Dots));
        for b in Background::ALL {
            assert_eq!(Background::from_name(b.name()), Some(b));
        }
        assert_eq!(Background::from_name("grid"), None);
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;
    use crate::model::{Cap, Join, Stop};
    use crate::vector::VPath;
    use draw::{DrawOp, PaintSpec};

    fn lightness(c: [f32; 4]) -> f32 {
        oklab([c[0], c[1], c[2]])[0]
    }

    /// One op of each kind: a solid fill, a gradient, an image, a stroke and glyphs.
    fn sample_ops() -> Vec<DrawOp> {
        let path = VPath::new();
        vec![
            DrawOp::Fill {
                path: path.clone(),
                paint: PaintSpec::Solid([1.0, 1.0, 1.0, 1.0]),
                even_odd: false,
            },
            DrawOp::Fill {
                path: path.clone(),
                paint: PaintSpec::Linear {
                    p0: V2::ZERO,
                    p1: v2(1.0, 0.0),
                    stops: vec![
                        Stop {
                            pos: 0.0,
                            color: Color::WHITE,
                        },
                        Stop {
                            pos: 0.7,
                            color: Color::rgba(0, 0, 0, 128),
                        },
                    ],
                    opacity: 0.8,
                },
                even_odd: false,
            },
            DrawOp::Fill {
                path: path.clone(),
                paint: PaintSpec::Image {
                    asset: "artwork".into(),
                    place: Rect::new(0.0, 0.0, 10.0, 10.0),
                    tile: false,
                    opacity: 0.6,
                },
                even_odd: false,
            },
            DrawOp::Stroke {
                path,
                color: [0.0, 0.0, 0.0, 1.0],
                width: 2.0,
                cap: Cap::Round,
                join: Join::Round,
                dash: vec![],
            },
            DrawOp::Glyphs {
                face: 0,
                size: 12.0,
                color: [0.11, 0.11, 0.12, 0.75],
                glyphs: vec![(1, V2::ZERO)],
            },
        ]
    }

    #[test]
    fn light_shows_the_document_exactly_as_authored() {
        let ops = sample_ops();
        let mut shown = ops.clone();
        Display::Light.ops(&mut shown);
        assert_eq!(shown, ops);
        assert_eq!(
            Display::default(),
            Display::Light,
            "exports and caches start light"
        );
        assert_eq!(SceneCache::default().display(), Display::Light);
    }

    #[test]
    fn dark_mirrors_lightness_and_keeps_hue_family_and_alpha() {
        let d = Display::Dark;
        let surface = lightness(rgba(CANVAS_BG_DARK));
        let ink = d.color([0.0, 0.0, 0.0, 1.0]);
        assert!(
            lightness(ink) > 0.88,
            "black text and strokes read light: {ink:?}"
        );
        let paper = d.color([1.0, 1.0, 1.0, 0.5]);
        assert!(
            lightness(paper) > surface && lightness(paper) < 0.35,
            "white sits just above the dark surface: {paper:?}"
        );
        assert_eq!(paper[3], 0.5, "alpha is kept exactly");
        assert!(
            (paper[0] - paper[1]).abs() < 0.01 && (paper[1] - paper[2]).abs() < 0.01,
            "a neutral stays neutral: {paper:?}"
        );
        let (dark, light) = (d.color([0.2, 0.2, 0.2, 1.0]), d.color([0.8, 0.8, 0.8, 1.0]));
        assert!(
            lightness(dark) > lightness(light),
            "the lightness order is mirrored"
        );
        let red = d.color([0.9, 0.15, 0.15, 1.0]);
        assert!(
            red[0] > red[1] + 0.2 && red[0] > red[2] + 0.2,
            "red stays red: {red:?}"
        );
        let blue = d.color([0.15, 0.35, 0.95, 1.0]);
        assert!(
            blue[2] > blue[0] + 0.2 && blue[2] > blue[1],
            "blue stays blue: {blue:?}"
        );
        let green = d.color([0.2, 0.7, 0.3, 1.0]);
        assert!(
            green[1] > green[0] && green[1] > green[2],
            "green stays green: {green:?}"
        );
        for i in 0..=8 {
            for j in 0..=8 {
                for k in 0..=8 {
                    let c = [i as f32 / 8.0, j as f32 / 8.0, k as f32 / 8.0, 0.3];
                    let shown = d.color(c);
                    assert!(
                        shown
                            .iter()
                            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                        "{c:?} -> {shown:?}"
                    );
                    assert_eq!(shown, d.color(c), "deterministic");
                    assert_eq!(shown[3], 0.3);
                }
            }
        }
    }

    #[test]
    fn dark_adapts_fills_gradients_strokes_and_glyphs_but_never_images() {
        let ops = sample_ops();
        let mut shown = ops.clone();
        Display::Dark.ops(&mut shown);
        let PaintSpec::Solid(fill) = (match &shown[0] {
            DrawOp::Fill { paint, .. } => paint.clone(),
            _ => unreachable!(),
        }) else {
            panic!("a solid stays solid")
        };
        assert!(lightness(fill) < 0.35, "{fill:?}");
        match (&shown[1], &ops[1]) {
            (
                DrawOp::Fill {
                    paint: PaintSpec::Linear { stops, opacity, .. },
                    ..
                },
                DrawOp::Fill {
                    paint:
                        PaintSpec::Linear {
                            stops: authored,
                            opacity: o,
                            ..
                        },
                    ..
                },
            ) => {
                assert_eq!(opacity, o);
                assert_eq!(stops.len(), authored.len());
                for (s, a) in stops.iter().zip(authored) {
                    assert_eq!(s.pos, a.pos, "stop positions are kept");
                    assert_eq!(s.color.a, a.color.a, "stop alpha is kept");
                }
                assert!(
                    stops[0].color.r < 100,
                    "white stop shown dark: {:?}",
                    stops[0]
                );
                assert!(
                    stops[1].color.r > 200,
                    "black stop shown light: {:?}",
                    stops[1]
                );
            }
            _ => panic!("the gradient stays a gradient"),
        }
        assert_eq!(shown[2], ops[2], "image paints are never adapted");
        match &shown[3] {
            DrawOp::Stroke { color, width, .. } => {
                assert!(lightness(*color) > 0.88);
                assert_eq!(*width, 2.0);
            }
            _ => panic!(),
        }
        // The glyphs are near-black ink (OKLab L ≈ 0.23), not pure black: shown at the mirrored
        // lightness the contract gives it (DARK_HI - L * (DARK_HI - DARK_LO), ≈ 0.78), well
        // above the dark surface, still neutral, with the alpha and everything else kept.
        let ink = match &ops[4] {
            DrawOp::Glyphs { color, .. } => *color,
            _ => unreachable!(),
        };
        match &shown[4] {
            DrawOp::Glyphs {
                face,
                size,
                color,
                glyphs,
            } => {
                let want = DARK_HI - lightness(ink) * (DARK_HI - DARK_LO);
                assert!(
                    (lightness(*color) - want).abs() < 0.01,
                    "mirrored lightness {want}: {color:?}"
                );
                assert!(
                    lightness(*color) > lightness(ink) + 0.4,
                    "dark ink reads light: {color:?}"
                );
                assert!(
                    lightness(*color) - lightness(rgba(CANVAS_BG_DARK)) > 0.5,
                    "legible on the dark surface: {color:?}"
                );
                assert!(
                    (color[0] - color[1]).abs() < 0.02 && (color[2] - color[0]).abs() < 0.03,
                    "neutral ink stays neutral: {color:?}"
                );
                assert_eq!(color[3], 0.75, "alpha is kept exactly");
                assert_eq!(
                    (*face, *size, glyphs.clone()),
                    (0, 12.0, vec![(1, V2::ZERO)]),
                    "only the colour changes"
                );
            }
            _ => panic!("glyphs stay glyphs"),
        }
    }

    #[test]
    fn a_display_change_drops_cached_meshes_and_exports_stay_light() {
        let mut doc = Document::default();
        let id = doc.alloc_id();
        let mut n = Node::new(id, NodeKind::Rect, Rect::new(0.0, 0.0, 40.0, 30.0));
        n.style = crate::model::Style::with_fill(Color::rgb(0xe6, 0x4a, 0x3b));
        n.style.stroke = Some(Color::BLACK);
        n.style.stroke_width = 2.0;
        doc.nodes.insert(id, n);
        doc.root.push(id);
        let authored = doc.clone();
        let mut cache = SceneCache::default();
        let light = cache.items(&doc, None, 1.0, None);
        cache.set_display(Display::Dark);
        let dark = cache.items(&doc, None, 1.0, None);
        assert_eq!(cache.last_misses, 1, "re-tessellated for the new display");
        let colours = |items: &[Item]| -> Vec<[f32; 4]> {
            items
                .iter()
                .flat_map(|i| i.pieces.iter())
                .flat_map(|p| p.mesh.vertices.iter().map(|v| v.color))
                .collect()
        };
        assert_ne!(colours(&light), colours(&dark));
        cache.set_display(Display::Dark);
        cache.items(&doc, None, 1.0, None);
        assert_eq!(
            cache.last_misses, 0,
            "the same display again keeps the cache"
        );
        let export = SceneCache::default().items(&doc, None, 1.0, None);
        assert_eq!(
            colours(&export),
            colours(&light),
            "exports draw the document as authored"
        );
        assert_eq!(
            doc.nodes.get(&id),
            authored.nodes.get(&id),
            "the document is untouched"
        );
    }
}

/// A frame that draws only `roots` (or the page) with no chrome: exports and thumbnails.
pub fn export_frame(
    doc: &Document,
    roots: Option<&[NodeId]>,
    camera: Camera,
    size_px: V2,
    background: Option<Color>,
) -> CanvasFrame {
    let mut under = MeshBuilder::default();
    if let Some(bg) = background {
        under.rect(Rect::new(0.0, 0.0, size_px.x, size_px.y), rgba(bg));
    }
    let items = SceneCache::default().items(doc, roots, camera.zoom, None);
    CanvasFrame {
        under: under.finish(),
        items,
        over: Vec::new(),
        camera,
        offset: V2::ZERO,
        viewport: Rect::new(0.0, 0.0, size_px.x, size_px.y),
    }
}
