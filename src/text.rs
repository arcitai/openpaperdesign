//! Text layout and glyph outlines.
//!
//! Text is rendered as vector glyph outlines, so it stays crisp at every zoom level and
//! exports to SVG without font dependencies. The faces are bundled in `assets/fonts` (SIL OFL
//! 1.1, nothing taken from Paper): Inter, the default and the app's own chrome font, and Geist
//! Mono, each in Regular, Bold, Italic and Bold Italic ([`FontFamily`]). A layer's family
//! picks its faces, glyphs, advances and natural line metrics; a character Geist Mono lacks is
//! drawn from the matching Inter face (`glyph_in`). Nothing is loaded at run time.

use crate::geom::{V2, v2};
use crate::model::{CharStyle, Color, FontFamily, TextAlign, TextStyle};
use crate::vector::VPath;
use lyon::path::Path;
use lyon::tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub static FONT_BYTES: &[u8] = crate::theme::INTER_REGULAR;
static BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");
static ITALIC: &[u8] = include_bytes!("../assets/fonts/Inter-Italic.ttf");
static BOLD_ITALIC: &[u8] = include_bytes!("../assets/fonts/Inter-BoldItalic.ttf");
static MONO: &[u8] = include_bytes!("../assets/fonts/GeistMono-Regular.ttf");
static MONO_BOLD: &[u8] = include_bytes!("../assets/fonts/GeistMono-Bold.ttf");
static MONO_ITALIC: &[u8] = include_bytes!("../assets/fonts/GeistMono-Italic.ttf");
static MONO_BOLD_ITALIC: &[u8] = include_bytes!("../assets/fonts/GeistMono-BoldItalic.ttf");

/// Face index: bit 0 = bold, bit 1 = italic, the bits above = the family
/// ([`FontFamily::index`]). Inter's four are 0–3, as before families existed; Geist Mono's 4–7.
/// A glyph id means something only together with its face (the families number their glyphs
/// differently), so every cache and draw op keys glyphs by the whole face.
pub type FaceId = u8;

/// How many bundled faces there are (four per family).
pub const FACE_COUNT: usize = 4 * FontFamily::ALL.len();

/// Inter's face for these flags.
pub fn face_id(bold: bool, italic: bool) -> FaceId {
    face_id_in(FontFamily::Inter, bold, italic)
}

/// `family`'s face for these flags.
pub fn face_id_in(family: FontFamily, bold: bool, italic: bool) -> FaceId {
    family.index() << 2 | bold as u8 | (italic as u8) << 1
}

/// The family a face belongs to.
pub fn family_of(id: FaceId) -> FontFamily {
    FontFamily::ALL[(id >> 2) as usize]
}

/// The bundled font file of a face. Every [`FaceId`] made by [`face_id_in`] has one; another
/// number is a bug (it panics instead of drawing some other face).
pub fn face_bytes(id: FaceId) -> &'static [u8] {
    match id {
        0 => FONT_BYTES,
        1 => BOLD,
        2 => ITALIC,
        3 => BOLD_ITALIC,
        4 => MONO,
        5 => MONO_BOLD,
        6 => MONO_ITALIC,
        7 => MONO_BOLD_ITALIC,
        _ => panic!("no bundled face {id}"),
    }
}

pub fn face_of(id: FaceId) -> &'static ttf_parser::Face<'static> {
    static FACES: OnceLock<Vec<ttf_parser::Face<'static>>> = OnceLock::new();
    &FACES.get_or_init(|| {
        (0..FACE_COUNT as u8)
            .map(|i| ttf_parser::Face::parse(face_bytes(i), 0).expect("bundled font parses"))
            .collect()
    })[id as usize]
}

/// Inter Regular, the default face.
pub fn face() -> &'static ttf_parser::Face<'static> {
    face_of(0)
}

/// Natural line height of Inter text at `font_size` (see [`line_height_in`]).
pub fn line_height(font_size: f64) -> f64 {
    line_height_in(FontFamily::Inter, font_size)
}

/// Natural line height of `family`'s text at `font_size`: its regular face's ascender,
/// descender and line gap (at least 1.1 times the size). A layer whose line height is Auto has
/// lines this tall.
pub fn line_height_in(family: FontFamily, font_size: f64) -> f64 {
    let f = face_of(face_id_in(family, false, false));
    let upm = f.units_per_em() as f64;
    let h = (f.ascender() as f64 - f.descender() as f64 + f.line_gap() as f64) / upm;
    font_size * h.max(1.1)
}

/// Inter's ascent at `font_size` (see [`ascent_in`]).
pub fn ascent(font_size: f64) -> f64 {
    ascent_in(FontFamily::Inter, font_size)
}

/// `family`'s ascent (baseline below a line's top, before line-height leading) at `font_size`.
pub fn ascent_in(family: FontFamily, font_size: f64) -> f64 {
    let f = face_of(face_id_in(family, false, false));
    f.ascender() as f64 / f.units_per_em() as f64 * font_size
}

#[derive(Debug, Clone, Copy)]
pub struct PlacedGlyph {
    pub gid: u16,
    pub face: FaceId,
    pub size: f64,
    pub color: Color,
    /// Pen position of the glyph origin (baseline), relative to the text box.
    pub x: f64,
    pub baseline: f64,
    pub advance: f64,
    /// Char index into the source string this glyph comes from.
    pub char_index: usize,
    pub underline: bool,
    pub strike: bool,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub top: f64,
    pub height: f64,
    pub baseline: f64,
    pub width: f64,
    /// Char index range `[start, end)`; `end` excludes the line break.
    pub start: usize,
    pub end: usize,
    /// Left x of each char boundary in the line (len = end - start + 1).
    pub carets: Vec<f64>,
}

#[derive(Debug, Clone)]
pub struct TextLayout {
    pub glyphs: Vec<PlacedGlyph>,
    pub lines: Vec<Line>,
    pub width: f64,
    pub height: f64,
    pub font_size: f64,
}

/// The glyph drawn for `c` in `face` at `font_size`: the face it actually comes from, its glyph
/// id there and its advance. A character the face lacks comes from the matching Inter face
/// (same weight and slant) when Inter has it, so text that showed in Inter (a check mark, say)
/// still shows when its layer becomes Geist Mono, while every character Geist Mono has stays
/// Geist Mono. One neither has is the requested face's own `?` (or its empty glyph): no other
/// font is looked for.
fn glyph_in(face: FaceId, c: char, font_size: f64) -> (FaceId, u16, f64) {
    let own = face_of(face);
    let inter = face_id(face & 1 == 1, face & 2 == 2);
    let (face, gid) = match own.glyph_index(c) {
        Some(g) => (face, g.0),
        None => match face_of(inter).glyph_index(c) {
            Some(g) if inter != face => (inter, g.0),
            _ => (face, own.glyph_index('?').map_or(0, |g| g.0)),
        },
    };
    let f = face_of(face);
    let adv = f.glyph_hor_advance(ttf_parser::GlyphId(gid)).unwrap_or(0) as f64;
    (face, gid, adv / f.units_per_em() as f64 * font_size)
}

#[cfg(test)]
fn advance(c: char, font_size: f64) -> (u16, f64) {
    let (_, g, a) = glyph_in(0, c, font_size);
    (g, a)
}

/// Whether `family`'s own faces have a glyph for `c` (see `glyph_in` for one that hasn't).
pub fn has_glyph(family: FontFamily, c: char) -> bool {
    face_of(face_id_in(family, false, false))
        .glyph_index(c)
        .is_some()
}

/// Lay out plain text with optional wrapping at `max_width`.
pub fn layout(text: &str, font_size: f64, max_width: Option<f64>, align: TextAlign) -> TextLayout {
    layout_rich(
        text,
        font_size,
        Color::INK,
        &TextStyle::default(),
        max_width,
        align,
    )
}

/// Lay out text whose characters may each have their own weight, slant, size and color, all in
/// the layer's one family (`ts.family`): its faces, advances and natural line metrics.
pub fn layout_rich(
    text: &str,
    font_size: f64,
    color: Color,
    ts: &TextStyle,
    max_width: Option<f64>,
    align: TextAlign,
) -> TextLayout {
    let chars: Vec<char> = text.chars().collect();
    let styles: Vec<CharStyle> = (0..chars.len())
        .map(|i| ts.char_style(i, font_size, color))
        .collect();
    let ls = ts.letter_spacing;
    let family = ts.family;
    // Each character's face (its family's, or Inter's for one the family lacks), glyph and
    // advance: what wrapping, carets, hit testing and drawing all use.
    let adv: Vec<(FaceId, u16, f64)> = chars
        .iter()
        .zip(&styles)
        .map(|(c, s)| {
            let (f, g, a) = glyph_in(face_id_in(family, s.bold, s.italic), *c, s.size);
            (f, g, a + ls)
        })
        .collect();
    let lh_of = |size: f64| match ts.line_height {
        Some(m) => size * m,
        None => line_height_in(family, size),
    };
    // Break into lines: hard breaks on '\n', soft breaks at spaces when wrapping.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    while start <= chars.len() {
        let hard_end = chars[start..]
            .iter()
            .position(|c| *c == '\n')
            .map_or(chars.len(), |p| start + p);
        let mut line_start = start;
        if let Some(maxw) = max_width {
            let mut x = 0.0;
            let mut last_space: Option<usize> = None;
            let mut i = line_start;
            while i < hard_end {
                let a = adv[i].2;
                if chars[i] == ' ' {
                    last_space = Some(i);
                }
                if x + a > maxw && i > line_start {
                    let brk = match last_space {
                        Some(s) if s > line_start => s,
                        _ => i,
                    };
                    let skip_space = chars.get(brk) == Some(&' ');
                    ranges.push((line_start, brk));
                    line_start = if skip_space { brk + 1 } else { brk };
                    i = line_start;
                    x = 0.0;
                    last_space = None;
                    continue;
                }
                x += a;
                i += 1;
            }
        }
        ranges.push((line_start, hard_end));
        start = hard_end + 1;
    }

    let mut lines = Vec::new();
    let mut glyphs = Vec::new();
    let mut max_line_w: f64 = 0.0;
    let mut top = 0.0;
    for (s, e) in ranges.iter().copied() {
        // Line metrics from the largest size on the line (or the char before an empty line).
        let sizes = styles[s..e].iter().map(|c| c.size);
        let max_size = sizes
            .clone()
            .fold(None, |m: Option<f64>, z| Some(m.map_or(z, |m| m.max(z))))
            .unwrap_or_else(|| {
                styles
                    .get(s.saturating_sub(1))
                    .map_or(font_size, |c| c.size)
            });
        let lh = lh_of(max_size);
        let natural = line_height_in(family, max_size);
        let baseline = top + ascent_in(family, max_size) + (lh - natural) * 0.5;
        let mut x = 0.0;
        let mut carets = vec![0.0];
        for i in s..e {
            let st = styles[i];
            let (face, gid, a) = adv[i];
            glyphs.push(PlacedGlyph {
                gid,
                face,
                size: st.size,
                color: st.color,
                x,
                baseline,
                advance: a,
                char_index: i,
                underline: st.underline,
                strike: st.strike,
            });
            x += a;
            carets.push(x);
        }
        max_line_w = max_line_w.max(x);
        lines.push(Line {
            top,
            height: lh,
            baseline,
            width: x,
            start: s,
            end: e,
            carets,
        });
        top += lh;
    }
    let box_w = max_width.unwrap_or(max_line_w);
    if align != TextAlign::Left {
        let k = if align == TextAlign::Center { 0.5 } else { 1.0 };
        for line in &mut lines {
            let dx = (box_w - line.width) * k;
            for c in &mut line.carets {
                *c += dx;
            }
            for g in glyphs
                .iter_mut()
                .filter(|g| g.char_index >= line.start && g.char_index < line.end)
            {
                g.x += dx;
            }
        }
    }
    TextLayout {
        glyphs,
        height: top,
        lines,
        width: max_line_w,
        font_size,
    }
}

impl TextLayout {
    /// Caret (x, top, height) for a char index into the source string.
    pub fn caret(&self, index: usize) -> (f64, f64, f64) {
        let i = self.line_of(index);
        let l = &self.lines[i];
        let k = index.clamp(l.start, l.end) - l.start;
        (l.carets[k], l.top, l.height)
    }

    /// Index of the line holding char boundary `index`.
    pub fn line_of(&self, index: usize) -> usize {
        for (i, l) in self.lines.iter().enumerate() {
            let next_start = self.lines.get(i + 1).map(|n| n.start);
            if index >= l.start && index <= l.end && next_start.is_none_or(|ns| index < ns) {
                return i;
            }
        }
        self.lines.len() - 1
    }

    /// Highlight rectangles (x, top, w, h) for the char range `[a, b)`.
    pub fn selection_rects(&self, a: usize, b: usize) -> Vec<(f64, f64, f64, f64)> {
        let (a, b) = (a.min(b), a.max(b));
        let mut out = Vec::new();
        for l in &self.lines {
            let s = a.max(l.start);
            let e = b.min(l.end);
            if s > e || (s == e && !(a < l.start && b > l.end)) {
                continue;
            }
            let x0 = l.carets[s - l.start];
            let x1 = l.carets[e - l.start];
            let extra = if b > l.end && l.end < usize::MAX {
                4.0
            } else {
                0.0
            };
            out.push((x0, l.top, (x1 - x0 + extra).max(1.0), l.height));
        }
        out
    }

    /// Char index nearest to a local point.
    pub fn hit(&self, p: V2) -> usize {
        let li = self
            .lines
            .iter()
            .position(|l| p.y < l.top + l.height)
            .unwrap_or(self.lines.len() - 1);
        let l = &self.lines[li];
        let mut best = 0;
        for (i, x) in l.carets.iter().enumerate() {
            if (x - p.x).abs() < (l.carets[best] - p.x).abs() {
                best = i;
            }
        }
        l.start + best
    }
}

/// A glyph tessellated once in font units (y up).
pub struct GlyphMesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

pub fn glyph_mesh(face: FaceId, gid: u16) -> Arc<GlyphMesh> {
    type Cache = Mutex<HashMap<(FaceId, u16), Arc<GlyphMesh>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(m) = cache.lock().unwrap().get(&(face, gid)) {
        return m.clone();
    }
    let mesh = Arc::new(tessellate_glyph(face, gid));
    cache.lock().unwrap().insert((face, gid), mesh.clone());
    mesh
}

fn tessellate_glyph(face: FaceId, gid: u16) -> GlyphMesh {
    let mut b = LyonOutline {
        b: Path::builder(),
        open: false,
    };
    let f = face_of(face);
    f.outline_glyph(ttf_parser::GlyphId(gid), &mut b);
    if b.open {
        b.b.end(true);
    }
    let path = b.b.build();
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    // In the face's own units, so every family is tessellated equally finely.
    let tol = (f.units_per_em() as f64 / 2000.0) as f32;
    let _ = FillTessellator::new().tessellate_path(
        &path,
        &FillOptions::tolerance(tol).with_fill_rule(lyon::tessellation::FillRule::NonZero),
        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position().to_array()),
    );
    GlyphMesh {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

struct LyonOutline {
    b: lyon::path::path::Builder,
    open: bool,
}

impl ttf_parser::OutlineBuilder for LyonOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.b.end(true);
        }
        self.b.begin(lyon::math::point(x, y));
        self.open = true;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.b.line_to(lyon::math::point(x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.b
            .quadratic_bezier_to(lyon::math::point(x1, y1), lyon::math::point(x, y));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.b.cubic_bezier_to(
            lyon::math::point(x1, y1),
            lyon::math::point(x2, y2),
            lyon::math::point(x, y),
        );
    }
    fn close(&mut self) {
        self.b.end(true);
        self.open = false;
    }
}

/// Outline of a glyph placed at `origin` (baseline) with `font_size`, y down.
pub fn glyph_path(face: FaceId, gid: u16, origin: V2, font_size: f64) -> VPath {
    struct B {
        p: VPath,
        o: V2,
        s: f64,
    }
    impl B {
        fn t(&self, x: f32, y: f32) -> V2 {
            self.o + v2(x as f64 * self.s, -(y as f64) * self.s)
        }
    }
    impl ttf_parser::OutlineBuilder for B {
        fn move_to(&mut self, x: f32, y: f32) {
            let q = self.t(x, y);
            self.p.move_to(q);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            let q = self.t(x, y);
            self.p.line_to(q);
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            let (a, b) = (self.t(x1, y1), self.t(x, y));
            self.p.quad_to(a, b);
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            let (a, b, c) = (self.t(x1, y1), self.t(x2, y2), self.t(x, y));
            self.p.cubic_to(a, b, c);
        }
        fn close(&mut self) {
            self.p.close();
        }
    }
    let f = face_of(face);
    let mut b = B {
        p: VPath::new(),
        o: origin,
        s: font_size / f.units_per_em() as f64,
    };
    f.outline_glyph(ttf_parser::GlyphId(gid), &mut b);
    b.p
}

/// SVG path data for a glyph (regular face).
pub fn glyph_svg_path(gid: u16, origin: V2, font_size: f64) -> String {
    glyph_path(0, gid, origin, font_size).to_svg()
}

/// Inner padding of sticky notes.
pub const STICKY_PAD: f64 = 16.0;
/// Font size of an arrow's label.
pub const LABEL_SIZE: f64 = 16.0;

/// Where a node's text box starts inside the node. An arrow's label box is placed by
/// `connector::label_centre` (on the middle of its line, or beside it where it would hide the
/// arrow's ends or heads), in the arrow's own (node-local) space, with the backing padding
/// `render::draw::LABEL_PAD` counted in.
pub fn text_offset(n: &crate::model::Node) -> V2 {
    match &n.kind {
        crate::model::NodeKind::Sticky { .. } => v2(STICKY_PAD, STICKY_PAD),
        crate::model::NodeKind::Connector {
            points,
            route,
            start_head,
            end_head,
            label: Some(_),
            ..
        } => {
            let Some(l) = layout_node(n) else {
                return V2::ZERO;
            };
            let pad = crate::render::draw::LABEL_PAD;
            let size = v2(l.width.max(1.0) + 2.0 * pad, l.height + 2.0 * pad);
            let heads = [*start_head, *end_head];
            let stroke = n.style.stroke_width.max(0.5);
            match crate::connector::label_centre(points, *route, heads, stroke, size) {
                Some(c) => c - v2(l.width.max(1.0) * 0.5, l.height * 0.5),
                None => V2::ZERO,
            }
        }
        _ => V2::ZERO,
    }
}

/// Layout of a text node's (or sticky note's) content in its box, or of an arrow's label
/// (centred lines, unwrapped, in the arrow's stroke colour); None for other kinds and for an
/// arrow without a label.
pub fn layout_node(n: &crate::model::Node) -> Option<TextLayout> {
    if let crate::model::NodeKind::Connector {
        label: Some(label), ..
    } = &n.kind
    {
        let color = n.style.stroke.unwrap_or(Color::INK);
        return Some(layout_rich(
            label,
            LABEL_SIZE,
            color,
            &n.text_style,
            None,
            TextAlign::Center,
        ));
    }
    if let crate::model::NodeKind::Sticky { text, font_size } = &n.kind {
        // Shrink the font (down to 12) until the text fits, like Excalidraw's sticky notes.
        let w = (n.rect.w - 2.0 * STICKY_PAD).max(10.0);
        let h = (n.rect.h - 2.0 * STICKY_PAD).max(10.0);
        let mut size = *font_size;
        loop {
            let ratio = size / font_size;
            let mut ts = n.text_style.clone();
            for sp in &mut ts.spans {
                sp.size = sp.size.map(|z| z * ratio);
            }
            let l = layout_rich(text, size, Color::INK, &ts, Some(w), TextAlign::Left);
            if l.height <= h || size <= 12.0 {
                return Some(l);
            }
            size = (size - 2.0).max(12.0);
        }
    }
    let crate::model::NodeKind::Text {
        text,
        font_size,
        auto_width,
        align,
    } = &n.kind
    else {
        return None;
    };
    let color = n
        .style
        .fills
        .last()
        .map_or(Color::INK, |p| p.preview_color());
    Some(layout_rich(
        text,
        *font_size,
        color,
        &n.text_style,
        (!auto_width).then_some(n.rect.w),
        *align,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_basic_and_newlines() {
        let l = layout("Hello\nWorld!", 20.0, None, TextAlign::Left);
        assert_eq!(l.lines.len(), 2);
        assert!(l.width > 40.0 && l.width < 120.0, "{}", l.width);
        assert!((l.height - 2.0 * line_height(20.0)).abs() < 1e-9);
        assert_eq!(l.glyphs.len(), 11);
        assert_eq!(l.caret(0).0, 0.0);
        assert_eq!(l.caret(6).1, line_height(20.0));
        let empty = layout("", 20.0, None, TextAlign::Left);
        assert_eq!(empty.lines.len(), 1);
        assert_eq!(empty.caret(0), (0.0, 0.0, line_height(20.0)));
    }

    #[test]
    fn wrapping_respects_width() {
        let l = layout(
            "the quick brown fox jumps over the lazy dog",
            16.0,
            Some(80.0),
            TextAlign::Left,
        );
        assert!(l.lines.len() >= 3);
        for line in &l.lines {
            assert!(
                line.width <= 80.0 + 1e-6 || line.end - line.start == 1,
                "{line:?}"
            );
        }
    }

    #[test]
    fn hit_and_caret_agree() {
        let l = layout("abc def", 30.0, None, TextAlign::Left);
        for i in 0..=7 {
            let (x, top, h) = l.caret(i);
            assert_eq!(l.hit(v2(x, top + h * 0.5)), i);
        }
    }

    #[test]
    fn glyphs_tessellate() {
        let (gid, _) = advance('A', 10.0);
        let m = glyph_mesh(0, gid);
        assert!(!m.indices.is_empty());
        assert!(glyph_svg_path(gid, V2::ZERO, 10.0).starts_with('M'));
    }

    /// The family name and subfamily a bundled face's own `name` table records.
    fn names(id: FaceId) -> (String, String) {
        let f = face_of(id);
        let get = |which: u16| {
            f.names()
                .into_iter()
                .filter(|n| n.name_id == which)
                .find_map(|n| n.to_string())
                .unwrap_or_default()
        };
        (
            get(ttf_parser::name_id::FAMILY),
            get(ttf_parser::name_id::SUBFAMILY),
        )
    }

    #[test]
    fn every_bundled_face_is_the_family_and_style_its_id_says() {
        // Inter keeps the ids it had before families (0–3: what every cached glyph and draw op
        // of an existing document used).
        for (bold, italic, id) in [
            (false, false, 0),
            (true, false, 1),
            (false, true, 2),
            (true, true, 3),
        ] {
            assert_eq!(face_id(bold, italic), id);
            assert_eq!(face_id_in(FontFamily::Inter, bold, italic), id);
            assert_eq!(face_id_in(FontFamily::GeistMono, bold, italic), id + 4);
        }
        let style = ["Regular", "Bold", "Italic", "Bold Italic"];
        for id in 0..FACE_COUNT as u8 {
            let family = family_of(id);
            assert_eq!(FontFamily::ALL[(id >> 2) as usize], family);
            let (name, sub) = names(id);
            assert!(
                name.starts_with(family.name()),
                "face {id}'s own family record: {name}"
            );
            if family == FontFamily::GeistMono {
                // The supplied files' exact records (FONT-PROVENANCE.json).
                assert_eq!(name, "Geist Mono");
                assert_eq!(
                    sub,
                    style[(id & 3) as usize],
                    "face {id}'s own style record"
                );
            }
        }
        assert!(!names(0).0.contains("Mono") && names(4).0.contains("Mono"));
        // Distinct files: no family reuses another's bytes.
        for a in 0..FACE_COUNT as u8 {
            for b in a + 1..FACE_COUNT as u8 {
                assert_ne!(face_bytes(a), face_bytes(b), "faces {a} and {b}");
            }
        }
    }

    #[test]
    fn inter_layout_is_unchanged_by_families() {
        // A default style and an explicit Inter one lay out identically, glyph for glyph, with
        // Inter's faces (ids 0–3) only.
        let text = "Æble — øre ✓ ok\nsecond line";
        let plain = TextStyle {
            bold: true,
            ..Default::default()
        };
        let explicit = TextStyle {
            family: FontFamily::Inter,
            ..plain.clone()
        };
        let a = layout_rich(text, 20.0, Color::INK, &plain, Some(120.0), TextAlign::Left);
        let b = layout_rich(
            text,
            20.0,
            Color::INK,
            &explicit,
            Some(120.0),
            TextAlign::Left,
        );
        assert_eq!(format!("{:?}", a.glyphs), format!("{:?}", b.glyphs));
        assert_eq!(format!("{:?}", a.lines), format!("{:?}", b.lines));
        assert!(a.glyphs.iter().all(|g| g.face < 4));
        assert_eq!(line_height(20.0), line_height_in(FontFamily::Inter, 20.0));
        assert_eq!(ascent(20.0), ascent_in(FontFamily::Inter, 20.0));
    }

    #[test]
    fn geist_mono_lays_out_with_its_own_faces_advances_and_metrics() {
        let mono = TextStyle {
            family: FontFamily::GeistMono,
            ..Default::default()
        };
        let layout =
            |s: &str, ts: &TextStyle| layout_rich(s, 20.0, Color::INK, ts, None, TextAlign::Left);
        // Monospaced: "iiii" and "MMMM" are equally wide in Geist Mono, not in Inter.
        let (ni, nm) = (layout("iiii", &mono).width, layout("MMMM", &mono).width);
        assert!((ni - nm).abs() < 1e-9, "{ni} {nm}");
        let inter = TextStyle::default();
        let (ii, im) = (layout("iiii", &inter).width, layout("MMMM", &inter).width);
        assert!(im - ii > 10.0, "Inter is proportional: {ii} {im}");
        // Its glyphs come from its own faces (4–7), with its own glyph ids and outlines.
        let l = layout("Ag", &mono);
        assert!(
            l.glyphs
                .iter()
                .all(|g| family_of(g.face) == FontFamily::GeistMono)
        );
        let i = layout("Ag", &inter);
        let path = |g: &PlacedGlyph| glyph_path(g.face, g.gid, V2::ZERO, 20.0).to_svg();
        assert_ne!(path(&l.glyphs[0]), path(&i.glyphs[0]), "a different 'A'");
        assert!(
            !glyph_mesh(l.glyphs[0].face, l.glyphs[0].gid)
                .indices
                .is_empty()
        );
        // Bold and italic pick Geist Mono's bold and italic faces.
        let bold_italic = TextStyle {
            bold: true,
            italic: true,
            ..mono.clone()
        };
        assert_eq!(layout("A", &bold_italic).glyphs[0].face, 7);
        // Natural line metrics are Geist Mono's: an Auto line is its height, and the caret and
        // hit test use it.
        let lh = line_height_in(FontFamily::GeistMono, 20.0);
        assert_eq!(l.lines[0].height, lh);
        assert_eq!(l.caret(0).2, lh);
        assert_ne!(lh, line_height(20.0), "each family's own natural height");
        let two = layout("ab\ncd", &mono);
        assert_eq!(two.height, 2.0 * lh);
        assert_eq!(two.hit(v2(0.0, lh * 1.5)), 3);
        // A set line height still wins, in either family.
        let set = TextStyle {
            line_height: Some(2.0),
            ..mono.clone()
        };
        assert_eq!(layout("a", &set).lines[0].height, 40.0);
    }

    #[test]
    fn a_glyph_geist_mono_lacks_comes_from_the_matching_inter_face() {
        // The supplied Geist Mono faces have no U+2713 (FONT-COVERAGE.json); Inter has. In a
        // Geist Mono layer every letter it has stays Geist Mono, the check mark is Inter's own
        // glyph in the same weight and slant (its face, outline and advance throughout), and a
        // character neither has is Geist Mono's '?'. The text and family stay as stored.
        assert!(!has_glyph(FontFamily::GeistMono, '✓') && has_glyph(FontFamily::Inter, '✓'));
        let text = "Æøå✓ ok 中";
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let mono = TextStyle {
                family: FontFamily::GeistMono,
                bold,
                italic,
                ..Default::default()
            };
            let l = layout_rich(text, 20.0, Color::INK, &mono, None, TextAlign::Left);
            assert_eq!(l.glyphs.len(), text.chars().count());
            let own = face_id_in(FontFamily::GeistMono, bold, italic);
            let inter = face_id(bold, italic);
            for (g, c) in l.glyphs.iter().zip(text.chars()) {
                if c == '✓' {
                    assert_eq!(g.face, inter, "{c:?} from Inter's matching face");
                    let want = face_of(inter).glyph_index(c).unwrap().0;
                    assert_eq!(g.gid, want);
                    let a = glyph_in(inter, c, 20.0).2;
                    assert!((g.advance - a).abs() < 1e-9, "Inter's advance");
                    assert!(
                        glyph_path(g.face, g.gid, V2::ZERO, 20.0)
                            .to_svg()
                            .starts_with('M')
                    );
                    assert!(!glyph_mesh(g.face, g.gid).indices.is_empty());
                } else if has_glyph(FontFamily::GeistMono, c) {
                    assert_eq!(g.face, own, "{c:?} stays Geist Mono");
                    assert_eq!(g.gid, face_of(own).glyph_index(c).unwrap().0);
                } else if !has_glyph(FontFamily::Inter, c) {
                    let q = face_of(own).glyph_index('?').map_or(0, |q| q.0);
                    assert_eq!(
                        (g.face, g.gid),
                        (own, q),
                        "{c:?}: in neither, Geist Mono's '?'"
                    );
                }
            }
            // Carets follow the mixed advances exactly, and hit testing agrees with them.
            let line = &l.lines[0];
            for (k, g) in l.glyphs.iter().enumerate() {
                assert!((line.carets[k + 1] - line.carets[k] - g.advance).abs() < 1e-9);
                let (x, top, h) = l.caret(k);
                assert_eq!(l.hit(v2(x, top + h * 0.5)), k);
            }
            // The SVG writer's ops split by face, so the check mark is drawn with its own.
            let faces: std::collections::BTreeSet<FaceId> =
                l.glyphs.iter().map(|g| g.face).collect();
            let want: std::collections::BTreeSet<FaceId> = [own, inter].into_iter().collect();
            assert_eq!(faces, want);
        }
        // In an Inter layer nothing changes: Inter's own check mark, Inter's '?' for the rest.
        let l = layout(text, 20.0, None, TextAlign::Left);
        assert!(l.glyphs.iter().all(|g| g.face == 0));
    }
}
