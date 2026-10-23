//! The left panel's Theme view (DESIGN.md, revision 20): the canvas's design system, scoped to
//! what the editor actually supports. It is not the app's appearance (System, Light, Dark),
//! which stays in the All Projects menu.
//!
//! Laid out for short surfaces too (320 × 240): what is useful first, then one status row each
//! for what isn't, with the reasons in tooltips and a folded Details section.
//!
//! - Text styles and colours on the shown Paper: every distinct one its layers use, read from
//!   the document as the layout draws it. Choosing one selects those layers, so the Design
//!   panel's Text, Fill or Stroke section edits them together through the existing coalesced,
//!   undoable edits. Choosing a row writes nothing.
//! - A colour's swatch changes that colour as the Paper's shared value (owner #14/#10 group 6:
//!   token editing that reflects the actual supported operation). Through the selection alone
//!   it couldn't be: layers sharing a colour seldom share their whole fills and strokes, so the
//!   Design panel showed Mixed and offered only to replace them. The change reaches exactly the
//!   solid fills and strokes that had the colour when it began, on unlocked layers, as one
//!   coalesced undo step per picker interaction through the same transactions (instances get
//!   the inspector's overrides), so the canvas's save, conflict and Undo are the existing ones.
//!   No named value or store is created; gradients and text-run colours are left alone.
//! - A text style's "Aa" button changes that style (its size, or its face among its family's
//!   four) wherever the Paper's text uses it (the same reason: selecting its layers edits their
//!   whole text, not only the characters in that style). It reaches exactly the characters that
//!   had it when the change began, on unlocked text layers: a layer whose every character has it
//!   changes its own size or face, as the Design panel's whole-layer edits do; otherwise a
//!   range style covers exactly those characters (`TextStyle::apply_range`, keeping every
//!   other value a range has). One editor use is one undo step, laid out again by the existing
//!   text layout; instances get overrides. No named style is created.
//! - The same editor shows and sets the style's font family (revision 23: Inter or Geist Mono,
//!   the bundled families the renderer draws), and its layers' line height (a percentage of
//!   the size, or Auto, the font's own) and letter spacing (px), the whole-layer metrics the
//!   renderer draws and the Design panel's LH and LS already edit (revision 22). The model has
//!   all three per layer only, so they are offered only when every one of the style's layers
//!   has the style throughout; a style that is only part of some layer's text says so
//!   (`RANGE_METRICS`) and nothing changes. Differing values show as Mixed, never a made-up
//!   common one; an entry that isn't a supported value (text, infinities, beyond the Design
//!   panel's bounds) is dropped and changes nothing. The same preview admission, locks,
//!   one-step Undo and autosave hold as the size and face.
//! - Fonts: the bundled families (Inter and Geist Mono, four faces each) and which of them the
//!   Paper's text uses. Read-only; a style's family is set from its Aa editor.
//! - Named tokens and the project's `DESIGN.md` / `design.md`: not available. The document has
//!   no token store; this view has no admitted project folder (OPD never infers one), and the
//!   guide reading and editing itself isn't built (R19–R21). Both are status rows with their
//!   reason, never a decorative list or a promise.

use super::{inspector, section};
use crate::editor::Editor;
use crate::model::{CharStyle, Color, FontFamily, NodeId, NodeKind, Paint};
use crate::theme;
use egui::{RichText, Ui};
use std::collections::{HashMap, HashSet};

/// A text style a Paper uses: its size in hundredths of a point (so equal sizes group), bold,
/// italic and the layer's font family (two styles alike but for their family are two styles:
/// they are drawn with different faces).
pub type StyleKey = (i64, bool, bool, FontFamily);

/// How many colours the list shows before saying how many more there are.
const MAX_COLOURS: usize = 24;

/// Every distinct text style the shown Paper's text layers use, their ranges included (the
/// styles the layout draws, as the Design panel reads them), largest first, each with the
/// layers that use it in paint order.
pub fn text_styles(ed: &Editor) -> Vec<(StyleKey, Vec<NodeId>)> {
    let mut out: Vec<(StyleKey, Vec<NodeId>)> = Vec::new();
    for id in ed.doc.paint_order() {
        let Some(n) = ed.doc.get(id) else {
            continue;
        };
        if !matches!(n.kind, NodeKind::Text { .. }) {
            continue;
        }
        for c in inspector::char_styles(n) {
            let key = style_key(c, n.text_style.family);
            match out.iter_mut().find(|(k, _)| *k == key) {
                Some((_, ids)) if !ids.contains(&id) => ids.push(id),
                Some(_) => {}
                None => out.push((key, vec![id])),
            }
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

/// The style key of a character's resolved style in a layer of `family`.
fn style_key(c: CharStyle, family: FontFamily) -> StyleKey {
    ((c.size * 100.0).round() as i64, c.bold, c.italic, family)
}

/// The bundled families the shown Paper's text layers use, each with how many layers.
fn families_in_use(ed: &Editor) -> Vec<(FontFamily, usize)> {
    FontFamily::ALL
        .into_iter()
        .map(|f| {
            let n = ed
                .doc
                .nodes
                .values()
                .filter(|n| matches!(n.kind, NodeKind::Text { .. }) && n.text_style.family == f)
                .count();
            (f, n)
        })
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// Every distinct solid fill and stroke colour the shown Paper's layers use, the most used
/// first, each with its layers in paint order.
pub fn colours(ed: &Editor) -> Vec<(Color, Vec<NodeId>)> {
    let mut out: Vec<(Color, Vec<NodeId>)> = Vec::new();
    for id in ed.doc.paint_order() {
        let Some(n) = ed.doc.get(id) else {
            continue;
        };
        let solids = n.style.fills.iter().filter_map(|p| match p {
            Paint::Solid { color } => Some(*color),
            _ => None,
        });
        for c in solids.chain(n.style.stroke) {
            match out.iter_mut().find(|(k, _)| *k == c) {
                Some((_, ids)) if !ids.contains(&id) => ids.push(id),
                Some(_) => {}
                None => out.push((c, vec![id])),
            }
        }
    }
    out.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then_with(|| a.0.to_hex().cmp(&b.0.to_hex()))
    });
    out
}

/// A text style as people read it: "48 · Bold", "13.5 · Italic"; another family than Inter
/// after it, "14 · Regular · Geist Mono".
pub fn style_label((size, bold, italic, family): StyleKey) -> String {
    let pt = size as f64 / 100.0;
    let size = if pt.fract() == 0.0 {
        format!("{pt:.0}")
    } else {
        format!("{pt}")
    };
    let face = match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    };
    if family.is_default() {
        format!("{size} · {face}")
    } else {
        format!("{size} · {face} · {}", family.name())
    }
}

/// The Theme view, under the left panel's switch.
pub fn panel(ui: &mut Ui, ed: &mut Editor) {
    // Drawn this frame: a metric being typed here is still its field's ([`begin_frame`]).
    ui.ctx().data_mut(|d| d.insert_temp(drawn_key(), true));
    // What is useful comes first (the Paper's own styles and colours, each selecting its
    // layers), then one short status row per thing that isn't, and the reasons folded away.
    egui::ScrollArea::vertical()
        .id_salt("theme-view")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            used_text_styles(ui, ed);
            gap(ui);
            used_colours(ui, ed);
            gap(ui);
            status(ui, ed);
            gap(ui);
            details(ui);
        });
}

fn gap(ui: &mut Ui) {
    ui.add_space(10.0);
}

/// A short secondary line.
fn note(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::SECONDARY)
            .color(theme::pal(ui).secondary),
    );
}

/// A section header with its explanation as a tooltip.
fn header(ui: &mut Ui, title: &str, tip: &str) {
    let _ = ui
        .scope(|ui| section(ui, title))
        .response
        .on_hover_text(tip);
}

fn used_text_styles(ui: &mut Ui, ed: &mut Editor) {
    header(
        ui,
        "Text styles",
        "The sizes, faces and font families this Paper's text uses. Choose one to select that \
         text; its Aa button changes that style wherever this Paper uses it.",
    );
    let key = restyle_key();
    let mut active: Option<Restyle> = ui.ctx().data(|d| d.get_temp(key));
    if let Some(a) = active.take_if(|a| a.stamp != ed.doc_stamp) {
        // The document changed under it (see `Restyle`): that use ends here and its editor
        // closes, so a further change there can't reach ranges of another document.
        egui::Popup::close_id(ui.ctx(), a.popup);
        ed.history.seal();
    }
    let styles = text_styles(ed);
    if styles.is_empty() {
        note(ui, "No text on this Paper.");
    }
    let links = inheritors(ed);
    // The metric typed last frame and not entered, and whether its editor's fields are drawn
    // this frame (else it has gone, see below).
    let prior = ui.ctx().data(|d| d.get_temp::<MetricDraft>(draft_key()));
    let mut prior_drawn = false;
    let mut changed = None;
    let mut open = None;
    let mut drafting = None;
    for (style, ids) in styles {
        let (row, slot) = value_row(ui, true, &style_label(style), ids.len());
        if row.clicked() {
            select(ed, ids.clone());
        }
        if ids.iter().all(|id| ed.doc.is_effectively_locked(*id)) {
            style_glyph(ui, slot, theme::pal(ui).tertiary);
            continue;
        }
        // A locked instance inherits it (R67): no editor, the reason said instead. (The style
        // being changed was checked when its change began.)
        let in_use = active.as_ref().is_some_and(|a| a.now == style);
        if !in_use
            && !links.is_empty()
            && reaches_locked(ed, &links, &style_groups(ed, &style_targets(ed, style)))
        {
            let id = key.with(("refused", style));
            let paint = |ui: &Ui, r| style_glyph(ui, r, theme::pal(ui).tertiary);
            if refused_slot(ui, id, slot, INHERITED_STYLE, paint) {
                ed.status = INHERITED_STYLE.into();
            }
            continue;
        }
        // The editor's identity: the style its row had when the change began, so it stays open
        // as the row's style (and place in the list) changes; a row still left with that style
        // (locked layers) gets another.
        let salt = match &active {
            Some(a) if a.now == style => ("style", a.from),
            Some(a) if a.from == style => ("kept", style),
            _ => ("style", style),
        };
        // Its line height and letter spacing, as its targets have them: the frozen ones while
        // its use is under way (layers coming into the style meanwhile aren't its), else the
        // ones it has now.
        let shown = match &active {
            Some(a) if in_use => metrics(ed, &a.targets),
            _ => metrics(ed, &style_targets(ed, style)),
        };
        let frame = style_editor(ui, key.with(salt), slot, style, ids.len(), &shown);
        let popup = frame.popup;
        if frame.drawn && prior.as_ref().is_some_and(|d| d.popup == popup) {
            prior_drawn = true;
        }
        if let Some(change) = frame.change {
            changed = Some((style, change, popup));
        }
        if frame.open {
            open = Some(style);
        }
        if let Some((metric, text)) = frame.typing {
            drafting = Some((style, metric, text, popup));
        }
    }
    // A metric typed and not entered whose editor's fields weren't drawn this frame went
    // without its field losing the focus: another style's Aa opened its editor in its place
    // (only one is open at a time; an earlier row's closes this one before it is drawn), or its
    // row lost its editor. It is finished as that focus loss would finish it, into the use under
    // way, so it and that use's earlier changes are one step when it ends below (R71). Escape,
    // which closes the editor, still drops it.
    if let Some(d) = prior.filter(|_| !prior_drawn)
        && !ui.input(|i| i.key_pressed(egui::Key::Escape))
    {
        finish_draft(ui.ctx(), ed, &links, &mut active, &d);
    }
    if let Some((style, change, popup)) = changed {
        apply_style_change(ui.ctx(), ed, &links, &mut active, style, change, popup);
    } else if active.as_ref().is_some_and(|a| open != Some(a.now)) {
        // Its editor closed (or its style left the list): that interaction is one step.
        ed.history.seal();
        active = None;
    }
    ui.ctx().data_mut(|d| match active {
        Some(a) => {
            d.insert_temp(key, a);
        }
        None => {
            d.remove::<Restyle>(key);
        }
    });
    // A metric being typed is kept with the document it is for, so a frame that no longer
    // draws its field finishes it (above, or [`begin_frame`] when Theme isn't drawn); none
    // being typed now (entered, Escaped, finished) leaves none.
    ui.ctx().data_mut(|d| match drafting {
        Some((style, metric, text, popup)) => {
            let stamp = ed.doc_stamp;
            d.insert_temp(
                draft_key(),
                MetricDraft {
                    style,
                    metric,
                    text,
                    popup,
                    stamp,
                },
            );
        }
        None => {
            d.remove::<MetricDraft>(draft_key());
        }
    });
}

/// Apply `change` to the text style `style` (its editor's popup `popup`), continuing the use
/// under way (`active`) or beginning one: a later change of a use is tried on a preview, a
/// first one admitted whole by [`fresh_restyle`]; a font family, line height or letter spacing
/// needs every target to be a whole layer. A refused change applies nothing, closes the editor
/// and says why in the status line. The one path for the editor's fields and for a metric
/// finished after its field went away ([`finish_draft`]).
fn apply_style_change(
    ctx: &egui::Context,
    ed: &mut Editor,
    links: &HashMap<NodeId, Vec<NodeId>>,
    active: &mut Option<Restyle>,
    style: StyleKey,
    change: StyleChange,
    popup: egui::Id,
) {
    // Why a change is refused: a metric for a style only part of some layer has, or a locked
    // layer it would reach.
    let mut reason = INHERITED_STYLE;
    let next = match active.take() {
        Some(a) if a.now == style => {
            // Each later change of a use is tried on a copy too: one that would reach a locked
            // layer isn't applied, and the use ends (its earlier changes stay, one step). A
            // family or metric needs every frozen target to be a whole layer.
            if change.whole_layer() && !metrics(ed, &a.targets).eligible {
                reason = RANGE_METRICS;
                ed.history.seal();
                None
            } else if restyle_reaches_locked(ed, &a, change) {
                ed.history.seal();
                None
            } else {
                Some(a)
            }
        }
        other => {
            if other.is_some() {
                ed.history.seal();
            }
            let fresh = fresh_restyle(ed, links, style, popup, change);
            if fresh.is_none()
                && change.whole_layer()
                && !metrics(ed, &style_targets(ed, style)).eligible
            {
                reason = RANGE_METRICS;
            }
            fresh
        }
    };
    match next {
        Some(mut r) => {
            restyle(ed, &r, change);
            let (size, b, i, family) = r.now;
            r.now = match change {
                StyleChange::Size(v) => ((v * 100.0).round() as i64, b, i, family),
                StyleChange::Face { bold, italic } => (size, bold, italic, family),
                StyleChange::Family(f) => (size, b, i, f),
                // A layer metric isn't part of the style's key: the row stays where it is.
                StyleChange::LineHeight(_) | StyleChange::LetterSpacing(_) => r.now,
            };
            r.stamp = ed.doc_stamp;
            *active = Some(r);
        }
        None => {
            // Refused before it applies (see `reaches_locked`, `restyle_reaches_locked`,
            // `RANGE_METRICS`).
            egui::Popup::close_id(ctx, popup);
            ed.status = reason.into();
        }
    }
}

/// Which of a style's paragraph metrics a field edits.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Metric {
    LineHeight,
    LetterSpacing,
}

/// A line height or letter spacing typed into a style's editor and not yet entered (no Enter,
/// no focus moved within the editor): its style, which metric, the text, the editor's popup and
/// the document stamp it was typed against. Kept in egui's memory only while its field is drawn
/// with the keyboard, so a frame that stops drawing it (the left panel switched to Design,
/// panels collapsed, the UI hidden: [`begin_frame`]; another style's editor opened in its
/// place: `used_text_styles`) can finish it as losing focus would; before, the field simply
/// vanished and the entry was lost (R171 panel finding, R71 Aa finding). Ephemeral UI
/// state, never a value store: it applies only to the document it was typed against.
#[derive(Clone)]
struct MetricDraft {
    style: StyleKey,
    metric: Metric,
    text: String,
    popup: egui::Id,
    stamp: u64,
}

fn draft_key() -> egui::Id {
    egui::Id::new("opd-theme-metric-draft")
}

/// Marks a frame in which the Theme view was drawn (so its fields were too).
fn drawn_key() -> egui::Id {
    egui::Id::new("opd-theme-drawn")
}

/// Whether a metric typed into Theme waits to be finished for the document as it is now: an
/// unfinished UI edit the browser host guards (no clean reload or leave over it).
pub fn metric_draft_pending(ctx: &egui::Context, ed: &Editor) -> bool {
    ctx.data(|d| d.get_temp::<MetricDraft>(draft_key()))
        .is_some_and(|d| d.stamp == ed.doc_stamp)
}

/// At the start of each frame: if the Theme view wasn't drawn in the last one, a metric typed
/// there and not entered is finished as its field losing the focus would finish it
/// ([`finish_metric_draft`]), as the hidden renames are finished.
pub fn begin_frame(ctx: &egui::Context, ed: &mut Editor) {
    let drawn = ctx
        .data_mut(|d| d.remove_temp::<bool>(drawn_key()))
        .unwrap_or(false);
    if !drawn {
        finish_metric_draft(ctx, ed);
    }
}

/// A typed metric whose Theme view is gone, finished ([`finish_draft`]). Its editor isn't
/// shown any more, so its use ends here: one undo step, and the autosave free again.
fn finish_metric_draft(ctx: &egui::Context, ed: &mut Editor) {
    let Some(d) = ctx.data(|m| m.get_temp::<MetricDraft>(draft_key())) else {
        return;
    };
    ctx.data_mut(|m| m.remove::<MetricDraft>(draft_key()));
    if d.stamp != ed.doc_stamp {
        return;
    }
    let mut active: Option<Restyle> = ctx
        .data(|m| m.get_temp(restyle_key()))
        .filter(|a: &Restyle| a.stamp == ed.doc_stamp);
    finish_draft(ctx, ed, &inheritors(ed), &mut active, &d);
    if active.is_some() {
        ed.history.seal();
    }
    ctx.data_mut(|m| m.remove::<Restyle>(restyle_key()));
    egui::Popup::close_id(ctx, d.popup);
}

/// A typed metric `d` whose field is gone, finished as Enter or a click elsewhere in its editor
/// would finish it, continuing the use under way (`active`): a valid value applied through
/// [`apply_style_change`] (the same preview admission, frozen targets, whole layers and locks;
/// a refusal says why), an unchanged one or an entry that isn't a value applying nothing. Only
/// for the document it was typed against: after any other change of it (a reload or discard,
/// another Paper, Undo, an agent's edit) it is dropped, never applied to the new one. The
/// caller ends the use.
fn finish_draft(
    ctx: &egui::Context,
    ed: &mut Editor,
    links: &HashMap<NodeId, Vec<NodeId>>,
    active: &mut Option<Restyle>,
    d: &MetricDraft,
) {
    if d.stamp != ed.doc_stamp {
        return;
    }
    let change = match d.metric {
        Metric::LineHeight => parse_line_height(&d.text).map(StyleChange::LineHeight),
        Metric::LetterSpacing => parse_letter_spacing(&d.text).map(StyleChange::LetterSpacing),
    };
    if let Some(change) = change {
        let targets = match &*active {
            Some(a) if a.now == d.style => a.targets.clone(),
            _ => style_targets(ed, d.style),
        };
        let shown = metrics(ed, &targets);
        let unchanged = match change {
            StyleChange::LineHeight(lh) => shown.line_height == Some(lh),
            StyleChange::LetterSpacing(ls) => shown.letter_spacing == Some(ls),
            StyleChange::Size(_) | StyleChange::Face { .. } | StyleChange::Family(_) => false,
        };
        if !unchanged {
            apply_style_change(ctx, ed, links, active, d.style, change, d.popup);
        }
    }
}

/// A text style being changed from its row's editor, from the first change until the editor
/// closes: the style it had (`from`), the one it has now, and the exact character ranges that
/// had `from` then (frozen, so styles meeting on the way never spread it). `stamp` and `popup`
/// as for [`Recolour`]: any other change of the document ends it, and a host can tell it is
/// under way ([`restyling`]).
#[derive(Clone)]
struct Restyle {
    from: StyleKey,
    now: StyleKey,
    targets: Vec<StyleTarget>,
    stamp: u64,
    popup: egui::Id,
}

/// One unlocked text layer's characters in a style: all of them (`whole`: the change goes to
/// the layer's own size or face, as the Design panel's whole-layer edits do), or these char
/// ranges `[a, b)` (the change is a range style over exactly them, as a selection's is).
#[derive(Clone)]
struct StyleTarget {
    id: NodeId,
    whole: bool,
    ranges: Vec<(usize, usize)>,
}

/// One change from a text style's editor: its size, or its face (its family's four), which a
/// range can take; or what only a whole text layer has: its font family (`TextStyle::family`)
/// or a paragraph metric (`line_height`, a multiple of the size, `None` for the font's own,
/// shown as Auto; and `letter_spacing`, px added after each character), as the Design panel's
/// Font, LH and LS edit them.
#[derive(Clone, Copy, PartialEq)]
enum StyleChange {
    Size(f64),
    Face { bold: bool, italic: bool },
    Family(FontFamily),
    LineHeight(Option<f64>),
    LetterSpacing(f64),
}

impl StyleChange {
    /// A whole layer's own value (not something a range of characters can have): its family or
    /// a paragraph metric.
    fn whole_layer(self) -> bool {
        matches!(
            self,
            StyleChange::Family(_) | StyleChange::LineHeight(_) | StyleChange::LetterSpacing(_)
        )
    }
}

/// Why a style's font family, line height and letter spacing can't be changed from Theme (the
/// editor's note, and the status line if it is tried anyway).
pub const RANGE_METRICS: &str = "Font family, line height and letter spacing belong to a whole \
                                 text layer, and only part of some layer's text is in this \
                                 style. Set them per layer in the Design panel.";

/// Line height and letter spacing as a style's editor shows them, read from its target layers
/// (`None`: they differ, Mixed). `eligible` when every target is a whole text layer: only then
/// can Theme change them, or the targets' family.
struct Metrics {
    line_height: Option<Option<f64>>,
    letter_spacing: Option<f64>,
    eligible: bool,
}

/// [`Metrics`] of `targets` (each a whole layer or ranges of one).
fn metrics(ed: &Editor, targets: &[StyleTarget]) -> Metrics {
    let nodes: Vec<crate::model::Node> = targets
        .iter()
        .filter_map(|t| ed.doc.get(t.id).cloned())
        .collect();
    // The Design panel's own reading: one value only when every layer has it.
    let line_height = inspector::uniform(&nodes, |n| n.text_style.line_height);
    let letter_spacing = inspector::uniform(&nodes, |n| n.text_style.letter_spacing);
    Metrics {
        line_height,
        letter_spacing,
        eligible: !targets.is_empty() && targets.iter().all(|t| t.whole),
    }
}

/// A number as the editor shows it: at most two decimals, no trailing zeros.
fn short_number(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// A line height as the editor shows it: Auto, or a percentage of the size (as the Design
/// panel's LH shows it).
fn line_height_text(lh: Option<f64>) -> String {
    match lh {
        None => "Auto".into(),
        Some(m) => format!("{}%", short_number(m * 100.0)),
    }
}

/// A line height as entered: "Auto" (or 0, as in the Design panel) for the font's own, or a
/// percentage of the size up to 1000 (a "%" may follow). `None` for anything else (text,
/// infinities, out of range): nothing is changed then.
fn parse_line_height(s: &str) -> Option<Option<f64>> {
    let t = s.trim().to_ascii_lowercase();
    if t == "auto" {
        return Some(None);
    }
    let n: f64 = t.trim_end_matches('%').trim().parse().ok()?;
    if !n.is_finite() || !(0.0..=1000.0).contains(&n) {
        return None;
    }
    Some((n > 0.0).then_some(n / 100.0))
}

/// A letter spacing as the editor shows it, in px.
fn letter_spacing_text(ls: f64) -> String {
    format!("{} px", short_number(ls))
}

/// A letter spacing as entered: px from -50 to 200, the Design panel's bounds (a "px" may
/// follow). `None` for anything else: nothing is changed then.
fn parse_letter_spacing(s: &str) -> Option<f64> {
    let t = s.trim().to_ascii_lowercase();
    let n: f64 = t.trim_end_matches("px").trim().parse().ok()?;
    (n.is_finite() && (-50.0..=200.0).contains(&n)).then_some(n)
}

/// The faces Theme offers: each bundled family's four, as the renderer draws them.
const FACES: [(&str, bool, bool); 4] = [
    ("Regular", false, false),
    ("Bold", true, false),
    ("Italic", false, true),
    ("Bold Italic", true, true),
];

fn restyle_key() -> egui::Id {
    egui::Id::new("opd-theme-restyle")
}

/// Whether a text style change from Theme is still under way (as [`recolouring`]).
pub fn restyling(ctx: &egui::Context, ed: &Editor) -> bool {
    ctx.data(|d| d.get_temp::<Restyle>(restyle_key()))
        .is_some_and(|a| a.stamp == ed.doc_stamp && egui::Popup::is_id_open(ctx, a.popup))
}

/// Whether a value change from Theme (a colour or a text style) is under way: its whole use is
/// one undo step, so every host holds its autosave meanwhile (the browser's
/// `WebState::save_snapshot`, the native `Session::autosave_due_ui`; see [`recolouring`]).
pub fn editing(ctx: &egui::Context, ed: &Editor) -> bool {
    recolouring(ctx, ed) || restyling(ctx, ed)
}

/// Why Theme won't change a colour here (the swatch's tooltip, and the status line when it is
/// chosen anyway). See [`reaches_locked`].
pub const INHERITED_COLOUR: &str = "A locked instance inherits this colour from its component \
                                    and would change with it. Unlock that instance to change \
                                    the colour here.";
/// Why Theme won't change a text style here (as [`INHERITED_COLOUR`]).
pub const INHERITED_STYLE: &str = "A locked instance inherits this text style or its text box \
                                   from its component and would change with it. Unlock that \
                                   instance to change the style here.";

/// Every instance layer, by the layer it inherits from (`InstanceLink::main`).
fn inheritors(ed: &Editor) -> HashMap<NodeId, Vec<NodeId>> {
    let mut out: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for n in ed.doc.nodes.values() {
        if let Some(l) = &n.instance {
            out.entry(l.main).or_default().push(n.id);
        }
    }
    out
}

/// Whether a Theme change of each `(layer, override group)` in `changed` would also change an
/// effectively locked layer (R67). Committing an edit inside a main component keeps every
/// instance of it in sync (`Editor::fixup` → `sync_node` → `copy_props`), whatever the
/// instance's lock: a layer that inherits the group from a changed one (not overriding it)
/// takes the new value, and so on through instances of instances. Theme's targets skip locked
/// layers, so such a change is refused before anything changes, rather than reaching a locked
/// layer behind its back; ordinary component editing elsewhere is unchanged. `links` is
/// [`inheritors`].
fn reaches_locked(
    ed: &Editor,
    links: &HashMap<NodeId, Vec<NodeId>>,
    changed: &[(NodeId, &'static str)],
) -> bool {
    if links.is_empty() {
        return false;
    }
    let direct: HashSet<NodeId> = changed.iter().map(|(id, _)| *id).collect();
    let in_component = |id: NodeId| {
        std::iter::once(id)
            .chain(ed.doc.ancestors(id))
            .any(|a| ed.doc.get(a).is_some_and(|n| n.component))
    };
    let mut queue: Vec<(NodeId, &str)> = changed.to_vec();
    let mut seen: HashSet<(NodeId, &str)> = HashSet::new();
    while let Some((id, key)) = queue.pop() {
        // Only an edit inside a main component is synced to its instances.
        if !seen.insert((id, key)) || !in_component(id) {
            continue;
        }
        for &dep in links.get(&id).into_iter().flatten() {
            let Some(d) = ed.doc.get(dep) else {
                continue;
            };
            let overrides = d
                .instance
                .as_ref()
                .is_some_and(|l| l.overrides.iter().any(|o| o == key));
            // A layer changed directly is one of the change's own (unlocked) targets.
            if overrides || direct.contains(&dep) {
                continue;
            }
            if ed.doc.is_effectively_locked(dep) {
                return true;
            }
            queue.push((dep, key));
        }
    }
    false
}

/// A colour's places as the override groups they change.
fn colour_groups(places: &[Place]) -> Vec<(NodeId, &'static str)> {
    let mut out = Vec::new();
    for p in places {
        if !p.fills.is_empty() {
            out.push((p.id, "fills"));
        }
        if p.stroke {
            out.push((p.id, "stroke"));
        }
    }
    out
}

/// A text style's targets as the override group they change.
fn style_groups(ed: &Editor, targets: &[StyleTarget]) -> Vec<(NodeId, &'static str)> {
    let mut out = Vec::new();
    for t in targets {
        // Its font, and its text box: `restyle` lays the text out again (`fit_text`), and
        // instances copy a box's width and height unless they override `size` (R68: a locked
        // instance overriding only `font` still took the main's new box).
        out.push((t.id, "font"));
        out.push((t.id, "size"));
        // A group around it takes new bounds, and an auto-layout frame reflows (and may hug):
        // their boxes and places change too, up to the component itself.
        for a in ed.doc.ancestors(t.id) {
            let Some(n) = ed.doc.get(a) else {
                continue;
            };
            if n.is_group() || n.layout.is_some() {
                out.push((a, "size"));
                out.push((a, "position"));
            }
            if n.component {
                break;
            }
        }
    }
    out
}

/// A new text style use beginning with `change`, or `None` when Theme refuses it, before
/// anything of the live editor changes. Its whole first step is tried on a preview first
/// (`Editor::preview`, R69): ending an open text edit (which commits it, and may remove an
/// emptied text and so reflow its frame), the targets found after that (for a line height or
/// letter spacing, every one a whole layer, [`RANGE_METRICS`]), the inheritance
/// foresight ([`reaches_locked`] on [`style_groups`]) and the change with all its follow-ups;
/// it is refused if any effectively locked layer of the live document would differ. Only an
/// admitted use touches the live editor, by the same steps (the text edit ended, the targets
/// found again); the caller then applies the change.
fn fresh_restyle(
    ed: &mut Editor,
    links: &HashMap<NodeId, Vec<NodeId>>,
    style: StyleKey,
    popup: egui::Id,
    change: StyleChange,
) -> Option<Restyle> {
    let mut probe = ed.preview();
    probe.end_text_edit();
    let probe_links;
    let links = if ed.text_edit.is_some() {
        probe_links = inheritors(&probe);
        &probe_links
    } else {
        links
    };
    let r = Restyle {
        from: style,
        now: style,
        targets: style_targets(&probe, style),
        stamp: 0,
        popup,
    };
    // A family, line height or letter spacing belongs to whole layers only: never a part of a
    // subset.
    if change.whole_layer() && !metrics(&probe, &r.targets).eligible {
        return None;
    }
    if reaches_locked(&probe, links, &style_groups(&probe, &r.targets)) {
        return None;
    }
    restyle(&mut probe, &r, change);
    if locked_differ(ed, &probe) {
        return None;
    }
    ed.end_text_edit();
    Some(Restyle {
        targets: style_targets(ed, style),
        ..r
    })
}

/// Whether applying `change` to `r`'s targets would change any effectively locked layer: the
/// change applied to a preview of the shown Paper (`Editor::preview`: its document and open
/// transaction) through the very same code (`restyle`: its range edits, `fit_text`,
/// overrides and the commit's fixup, which keeps component instances, groups, auto layout and
/// bound arrows in step), so every derived effect counts, not only the inherited values
/// [`reaches_locked`] foresees for a row (R68). Nothing of the live editor is touched.
fn restyle_reaches_locked(ed: &Editor, r: &Restyle, change: StyleChange) -> bool {
    let mut probe = ed.preview();
    restyle(&mut probe, r, change);
    locked_differ(ed, &probe)
}

/// Whether any of `ed`'s effectively locked layers differs (or is gone) in `probe`.
fn locked_differ(ed: &Editor, probe: &Editor) -> bool {
    ed.doc
        .nodes
        .keys()
        .any(|id| ed.doc.is_effectively_locked(*id) && probe.doc.get(*id) != ed.doc.get(*id))
}

/// Whether `edit`, a whole-layer typography change of the text layers `ids` from the Design
/// panel (its Font), would change any effectively locked layer, found as Theme finds it for
/// its own family (R67–R69): the inheritance foresight ([`reaches_locked`] on the layers' font
/// and text box, [`style_groups`]) and `edit` itself tried on a preview (`Editor::preview`:
/// the document, open transaction and text edit as they stand, so an open draft's commit and
/// every fit, override and fixup the edit brings count), compared by [`locked_differ`]. A
/// locked layer among `ids` counts too. Nothing of the live editor is touched.
pub(super) fn text_edit_reaches_locked(
    ed: &Editor,
    ids: &[NodeId],
    edit: impl FnOnce(&mut Editor),
) -> bool {
    let targets: Vec<StyleTarget> = ids
        .iter()
        .map(|&id| StyleTarget {
            id,
            whole: true,
            ranges: Vec::new(),
        })
        .collect();
    if reaches_locked(ed, &inheritors(ed), &style_groups(ed, &targets)) {
        return true;
    }
    let mut probe = ed.preview();
    edit(&mut probe);
    locked_differ(ed, &probe)
}

/// A row's swatch or "Aa" whose change Theme refuses (`reason`): drawn quietly by `paint`, the
/// reason as its tooltip and accessible description. Choosing it changes nothing; it returns
/// whether it was chosen, so the caller says the reason in the status line.
fn refused_slot(
    ui: &mut Ui,
    id: egui::Id,
    slot: egui::Rect,
    reason: &str,
    paint: impl FnOnce(&Ui, egui::Rect),
) -> bool {
    let mut child = ui.new_child(egui::UiBuilder::new().id(id).max_rect(slot));
    let (rect, resp) = child.allocate_exact_size(slot.size(), egui::Sense::click());
    paint(&child, rect);
    theme::focus_ring(&child, rect, &resp, false);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, reason));
    resp.on_hover_text(reason).clicked()
}

/// Where the shown Paper's unlocked text layers have characters in `style`, resolved as the
/// layout draws them (`TextStyle::char_style`, the runs between their ranges' edges, as
/// [`text_styles`] lists them).
fn style_targets(ed: &Editor, style: StyleKey) -> Vec<StyleTarget> {
    let mut out = Vec::new();
    for id in ed.doc.paint_order() {
        let Some(n) = ed.doc.get(id) else {
            continue;
        };
        let NodeKind::Text {
            text, font_size, ..
        } = &n.kind
        else {
            continue;
        };
        if ed.doc.is_effectively_locked(id) {
            continue;
        }
        let family = n.text_style.family;
        let at = |i: usize| style_key(n.text_style.char_style(i, *font_size, Color::INK), family);
        let len = text.chars().count();
        if len == 0 {
            // No characters: the layer's own style is the one listed.
            if at(0) == style {
                out.push(StyleTarget {
                    id,
                    whole: true,
                    ranges: Vec::new(),
                });
            }
            continue;
        }
        let mut cuts: Vec<usize> = [0, len]
            .into_iter()
            .chain(n.text_style.spans.iter().flat_map(|s| [s.start, s.end]))
            .filter(|i| *i <= len)
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if at(a) != style {
                continue;
            }
            match ranges.last_mut() {
                Some(r) if r.1 == a => r.1 = b,
                _ => ranges.push((a, b)),
            }
        }
        if !ranges.is_empty() {
            let whole = ranges == [(0, len)];
            out.push(StyleTarget { id, whole, ranges });
        }
    }
    out
}

/// Give a restyle's targets `change`, in one coalesced transaction per editor use (sealed when
/// it closes): a whole layer's own size or face (its ranges' values for it dropped, as the
/// Design panel's whole-layer edits drop them), or a range style over exactly its ranges
/// (`TextStyle::apply_range`, which keeps every other value a range has: colour, underline,
/// strike); or a whole layer's own font family, line height or letter spacing, its ranges
/// untouched (only ever whole layers: see [`RANGE_METRICS`]). Text layers are laid out again
/// (`Editor::fit_text`: width kept unless it grows to fit its text); edits inside instances
/// become overrides, under the Design panel's field names.
fn restyle(ed: &mut Editor, r: &Restyle, change: StyleChange) {
    let ids: Vec<NodeId> = r.targets.iter().map(|t| t.id).collect();
    // The Design panel's own field names, so instance overrides are the same ones.
    let field = match change {
        StyleChange::Size(_) => "font size",
        StyleChange::Face { .. } => "font",
        StyleChange::Family(_) => "font family",
        StyleChange::LineHeight(_) => "line height",
        StyleChange::LetterSpacing(_) => "letter spacing",
    };
    let key = format!("theme text style {}:{ids:?}", style_label(r.from));
    ed.begin_coalescing("Change text style", Some(&key));
    {
        let mut tx = ed.tx();
        for t in &r.targets {
            let Some(n) = tx.node_mut(t.id) else {
                continue;
            };
            if t.whole {
                match change {
                    StyleChange::Size(v) => {
                        if let NodeKind::Text { font_size, .. } = &mut n.kind {
                            *font_size = v;
                        }
                        for sp in &mut n.text_style.spans {
                            sp.size = None;
                        }
                        n.text_style.normalize();
                    }
                    StyleChange::Face { bold, italic } => {
                        n.text_style.bold = bold;
                        n.text_style.italic = italic;
                        for sp in &mut n.text_style.spans {
                            sp.bold = None;
                            sp.italic = None;
                        }
                        n.text_style.normalize();
                    }
                    // The layer's own family or metric, as the Design panel's Font, LH and LS
                    // set it; its ranges (sizes, faces, colours, marks) are left exactly as
                    // they are, and its text too.
                    StyleChange::Family(f) => n.text_style.family = f,
                    StyleChange::LineHeight(lh) => n.text_style.line_height = lh,
                    StyleChange::LetterSpacing(ls) => n.text_style.letter_spacing = ls,
                }
            } else if change.whole_layer() {
                // Never reached: a family or metric is refused unless every target is a whole
                // layer (`RANGE_METRICS`), so no range is ever given one.
            } else {
                // Only the value that changes: a face from Regular to Bold leaves italic as the
                // characters have it, without writing it into their ranges.
                let (_, was_bold, was_italic, _) = r.now;
                for &(a, b) in &t.ranges {
                    n.text_style.apply_range(a, b, |sp| match change {
                        StyleChange::Size(v) => sp.size = Some(v),
                        StyleChange::Face { bold, italic } => {
                            if bold != was_bold {
                                sp.bold = Some(bold);
                            }
                            if italic != was_italic {
                                sp.italic = Some(italic);
                            }
                        }
                        StyleChange::Family(_)
                        | StyleChange::LineHeight(_)
                        | StyleChange::LetterSpacing(_) => {}
                    });
                }
            }
        }
    }
    ed.mark_overrides(&ids, field);
    for id in &ids {
        ed.fit_text(*id);
    }
    ed.commit();
}

/// A text style's "Aa" glyph in its row's swatch slot.
fn style_glyph(ui: &Ui, slot: egui::Rect, color: egui::Color32) {
    ui.painter().text(
        slot.center(),
        egui::Align2::CENTER_CENTER,
        "Aa",
        egui::FontId::proportional(theme::SECONDARY),
        color,
    );
}

/// A text style's editor: its "Aa" button at `slot`, taking no room in the row (the row stays
/// the selecting target around it), and the small popup it opens: the style's size, its
/// family's four faces and the bundled families, nothing the renderer can't draw, then its
/// layers' line height and letter spacing (`shown`: a value, Auto or Mixed; the family and
/// metrics unavailable, with the reason, when the style is only part of some layer's text).
/// `id` is the button's own identity, independent of the row's place.
fn style_editor(
    ui: &mut Ui,
    id: egui::Id,
    slot: egui::Rect,
    style: StyleKey,
    count: usize,
    shown: &Metrics,
) -> EditorFrame {
    let mut child = ui.new_child(egui::UiBuilder::new().id(id).max_rect(slot));
    let popup = id.with("popup");
    let open = egui::Popup::is_id_open(child.ctx(), popup);
    let (rect, resp) = child.allocate_exact_size(slot.size(), egui::Sense::click());
    let p = theme::pal(&child);
    let lit = open || resp.hovered() || resp.has_focus();
    if lit {
        child.painter().rect_filled(rect, 3.0, p.hover);
    }
    style_glyph(&child, rect, if lit { p.text } else { p.secondary });
    theme::focus_ring(&child, rect, &resp, false);
    let label = style_label(style);
    let name = format!("Change {label}");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    let layers = if count == 1 { "layer" } else { "layers" };
    let resp = resp.on_hover_text(format!(
        "Change {label} wherever this Paper's text uses it ({count} {layers}), as one undo \
         step. Locked layers keep theirs."
    ));
    let (_, bold, italic, family) = style;
    let mut change = None;
    let mut typing = None;
    let mut drawn = false;
    egui::Popup::menu(&resp)
        .id(popup)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            drawn = true;
            ui.set_min_width(180.0);
            let mut size = style.0 as f64 / 100.0;
            let r = ui.add_sized(
                egui::vec2(100.0, theme::FIELD_H),
                egui::DragValue::new(&mut size)
                    .speed(0.5)
                    .range(1.0..=1000.0)
                    .max_decimals(2)
                    .prefix("Size "),
            );
            if r.changed() && (size * 100.0).round() as i64 != style.0 {
                change = Some(StyleChange::Size(size));
            }
            ui.horizontal_wrapped(|ui| {
                for (face, b, i) in FACES {
                    let on = (b, i) == (bold, italic);
                    if ui.add(egui::Button::selectable(on, face)).clicked() && !on {
                        change = Some(StyleChange::Face { bold: b, italic: i });
                    }
                }
            });
            // The bundled families, the style's own selected: a whole layer's value, so only
            // offered when every layer in the style has it throughout (the note below says why
            // not otherwise).
            if let Some(f) = family_choice(ui, family, shown.eligible) {
                change = Some(StyleChange::Family(f));
            }
            ui.add_space(4.0);
            // The whole layers' paragraph metrics: a value, Auto, or Mixed (they differ; a value
            // entered then sets it for all of them). Nothing is read into a common number.
            let lh = shown.line_height.map_or(String::new(), line_height_text);
            let (entered, draft) =
                metric_field(ui, "Line height", &lh, "Auto or %", shown.eligible);
            if let Some(lh) = entered.as_deref().and_then(parse_line_height)
                && shown.line_height != Some(lh)
            {
                change = Some(StyleChange::LineHeight(lh));
            }
            typing = draft.map(|t| (Metric::LineHeight, t));
            let ls = shown
                .letter_spacing
                .map_or(String::new(), letter_spacing_text);
            let (entered, draft) = metric_field(ui, "Letter spacing", &ls, "px", shown.eligible);
            if let Some(ls) = entered.as_deref().and_then(parse_letter_spacing)
                && shown.letter_spacing != Some(ls)
            {
                change = Some(StyleChange::LetterSpacing(ls));
            }
            if let Some(t) = draft {
                typing = Some((Metric::LetterSpacing, t));
            }
            if !shown.eligible {
                note(ui, RANGE_METRICS);
            }
        });
    EditorFrame {
        change,
        open: egui::Popup::is_id_open(ui.ctx(), popup),
        popup,
        typing,
        drawn,
    }
}

/// The bundled font families as one row of choices, `now` selected (unavailable, in the
/// tertiary colour, unless `enabled`). Returns another family chosen. Each choice is an
/// ordinary focusable button with its family's name, so it is reached and pressed from the
/// keyboard like the faces above it.
pub(super) fn family_choice(ui: &mut Ui, now: FontFamily, enabled: bool) -> Option<FontFamily> {
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for f in FontFamily::ALL {
            let on = f == now;
            let b = egui::Button::selectable(on, f.name());
            let r = ui.add_enabled(enabled, b);
            let r = if f.is_monospace() {
                r.on_hover_text("Monospaced, for code, data and labels. Bundled (SIL OFL 1.1).")
            } else {
                r.on_hover_text("The default family. Bundled (SIL OFL 1.1).")
            };
            if r.clicked() && !on {
                chosen = Some(f);
            }
        }
    });
    chosen
}

/// What one style's editor did this frame.
struct EditorFrame {
    /// A finished change to the style.
    change: Option<StyleChange>,
    /// Whether its popup is open after this frame.
    open: bool,
    popup: egui::Id,
    /// A metric being typed (not yet entered) and its text.
    typing: Option<(Metric, String)>,
    /// Whether the popup's fields were drawn this frame. A field that was being typed in and
    /// isn't drawn has gone without a focus-loss frame (another style's editor replaced it).
    drawn: bool,
}

/// One paragraph metric in a style's editor: its label and a text field showing `shown` (a
/// value or Auto; empty, with "Mixed" as its hint, when the layers differ; `hint` names what
/// it takes). Returns what was entered once the field is finished (Enter, or the focus moving
/// on) and differs from what it showed (Escape drops it), and the text being typed while the
/// field has the keyboard. Until it is finished nothing changes; while it isn't focused it
/// shows the layers' value again, so a rejected entry just goes back.
fn metric_field(
    ui: &mut Ui,
    label: &str,
    shown: &str,
    hint: &str,
    enabled: bool,
) -> (Option<String>, Option<String>) {
    let id = ui.make_persistent_id(("theme-metric", label));
    ui.horizontal(|ui| {
        let p = theme::pal(ui);
        let colour = if enabled { p.secondary } else { p.tertiary };
        let size = egui::vec2(84.0, theme::FIELD_H);
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        let name = ui
            .allocate_ui_with_layout(size, layout, |ui| {
                ui.label(RichText::new(label).size(theme::BODY).color(colour))
            })
            .inner;
        // A typed draft only while the field has the keyboard (a popup closed meanwhile leaves
        // none behind): otherwise it shows the layers' value.
        let field_id = id.with("field");
        let focused = ui.ctx().memory(|m| m.has_focus(field_id));
        let mut draft: String = focused
            .then(|| ui.ctx().data(|d| d.get_temp(id)))
            .flatten()
            .unwrap_or_else(|| shown.to_owned());
        if !enabled && draft.is_empty() {
            // Not editable here: say what the layers have, as text.
            draft = "Mixed".to_owned();
        }
        let hint = if shown.is_empty() { "Mixed" } else { hint };
        let field = egui::TextEdit::singleline(&mut draft)
            .id(field_id)
            .hint_text(hint)
            .desired_width(84.0);
        let r = ui.add_enabled(enabled, field).labelled_by(name.id);
        let mut entered = None;
        let mut typing = None;
        if r.has_focus() {
            ui.ctx().data_mut(|d| d.insert_temp(id, draft.clone()));
            typing = (draft.trim() != shown).then_some(draft);
        } else {
            if r.lost_focus()
                && !ui.input(|i| i.key_pressed(egui::Key::Escape))
                && draft.trim() != shown
            {
                entered = Some(draft);
            }
            ui.ctx().data_mut(|d| d.remove::<String>(id));
        }
        (entered, typing)
    })
    .inner
}

/// A colour being changed from its swatch, from the first change until its picker closes: the
/// colour it had (`from`), the one it has now, and the exact places that had `from` then. Kept
/// in egui's memory, so the picker stays open while its row moves or meets another colour, and
/// the change never spreads to layers that only come to share the colour on the way. `stamp`
/// is the document's (`Editor::doc_stamp`) right after its own last change: any other change
/// of the document since (a reload or discard, another Paper shown, Undo, an agent's edit)
/// ends it and closes its picker, so its places never reach another document's layers that
/// happen to reuse their ids. `popup` is its picker's popup id, so a host can tell whether the
/// use is still under way ([`recolouring`]).
#[derive(Clone)]
struct Recolour {
    from: Color,
    now: Color,
    places: Vec<Place>,
    stamp: u64,
    popup: egui::Id,
}

/// Where the colour change under way is kept in egui's memory: one fixed id, so a host asks
/// about it too ([`recolouring`]).
fn recolour_key() -> egui::Id {
    egui::Id::new("opd-theme-recolour")
}

/// Whether a colour change from Theme is still under way: changed at least once, its picker
/// still open, and the document as its last change left it. Its whole picker use is meant to
/// be one undo step, and a save's acknowledgement seals the history (`Editor::mark_saved`, so
/// the saved state stays a history boundary): the browser host holds its autosave while this
/// is true (run159: three autosaves while typing 1, 5, 0 into R split one use into three
/// steps). It ends when the picker closes (or isn't shown) or the document changes otherwise.
pub fn recolouring(ctx: &egui::Context, ed: &Editor) -> bool {
    ctx.data(|d| d.get_temp::<Recolour>(recolour_key()))
        .is_some_and(|a| a.stamp == ed.doc_stamp && egui::Popup::is_id_open(ctx, a.popup))
}

/// One unlocked layer's places of a colour: which of its fills are that solid colour, and
/// whether its stroke is.
#[derive(Clone)]
struct Place {
    id: NodeId,
    fills: Vec<usize>,
    stroke: bool,
}

/// Where the shown Paper's unlocked layers use `c` as a solid fill or stroke.
fn places(ed: &Editor, c: Color) -> Vec<Place> {
    let mut out = Vec::new();
    for id in ed.doc.paint_order() {
        let Some(n) = ed.doc.get(id) else {
            continue;
        };
        if ed.doc.is_effectively_locked(id) {
            continue;
        }
        let fills: Vec<usize> = n
            .style
            .fills
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p, Paint::Solid { color } if *color == c))
            .map(|(i, _)| i)
            .collect();
        let stroke = n.style.stroke == Some(c);
        if !fills.is_empty() || stroke {
            out.push(Place { id, fills, stroke });
        }
    }
    out
}

/// Give `places` the colour `to`, in one coalesced transaction per picker interaction (sealed
/// when its picker closes), as the inspector's own edits are: same history, Undo and save.
fn recolour(ed: &mut Editor, r: &Recolour, to: Color) {
    let fills: Vec<NodeId> = r
        .places
        .iter()
        .filter(|p| !p.fills.is_empty())
        .map(|p| p.id)
        .collect();
    let strokes: Vec<NodeId> = r.places.iter().filter(|p| p.stroke).map(|p| p.id).collect();
    let ids: Vec<NodeId> = r.places.iter().map(|p| p.id).collect();
    let key = format!("theme colour {}:{ids:?}", r.from.to_hex());
    ed.begin_coalescing("Change colour", Some(&key));
    {
        let mut tx = ed.tx();
        for p in &r.places {
            let Some(n) = tx.node_mut(p.id) else {
                continue;
            };
            for &i in &p.fills {
                if let Some(Paint::Solid { color }) = n.style.fills.get_mut(i) {
                    *color = to;
                }
            }
            if p.stroke && n.style.stroke.is_some() {
                n.style.stroke = Some(to);
            }
        }
    }
    // Edits inside instances become overrides, as in the Design panel.
    ed.mark_overrides(&fills, "fills");
    ed.mark_overrides(&strokes, "stroke");
    ed.commit();
}

fn used_colours(ui: &mut Ui, ed: &mut Editor) {
    header(
        ui,
        "Colours",
        "The solid fill and stroke colours this Paper uses. Choose one to select its layers; \
         change its swatch to change that colour wherever this Paper uses it.",
    );
    let key = recolour_key();
    let mut active: Option<Recolour> = ui.ctx().data(|d| d.get_temp(key));
    if let Some(a) = active.take_if(|a| a.stamp != ed.doc_stamp) {
        // The document changed under it (see `Recolour`): that use ends here, and its picker
        // closes, so a further drag there can't reach places of another document.
        let child = ui.new_child(egui::UiBuilder::new().id(key.with(("colour", a.from))));
        egui::Popup::close_id(ui.ctx(), child.auto_id_with("popup"));
        ed.history.seal();
    }
    let colours = colours(ed);
    let more = colours.len().saturating_sub(MAX_COLOURS);
    if colours.is_empty() {
        note(ui, "No solid colours on this Paper.");
    }
    let links = inheritors(ed);
    let mut changed = None;
    let mut open = None;
    for (c, ids) in colours.into_iter().take(MAX_COLOURS) {
        let (row, swatch) = value_row(ui, true, &c.to_hex(), ids.len());
        if row.clicked() {
            select(ed, ids.clone());
        }
        if ids.iter().all(|id| ed.doc.is_effectively_locked(*id)) {
            locked_swatch(ui, swatch, c);
            continue;
        }
        // A locked instance inherits it (R67): no picker, the reason said instead. (The colour
        // being changed was checked when its change began.)
        let in_use = active.as_ref().is_some_and(|a| a.now == c);
        if !in_use
            && !links.is_empty()
            && reaches_locked(ed, &links, &colour_groups(&places(ed, c)))
        {
            let id = key.with(("refused", c));
            let paint = |ui: &Ui, r| locked_swatch(ui, r, c);
            if refused_slot(ui, id, swatch, INHERITED_COLOUR, paint) {
                ed.status = INHERITED_COLOUR.into();
            }
            continue;
        }
        // The picker's identity: the colour its row had when the change began, so it stays
        // open as the row's colour (and place in the list) changes; a row still left with that
        // colour (locked layers) gets another.
        let salt = match &active {
            Some(a) if a.now == c => ("colour", a.from),
            Some(a) if a.from == c => ("kept", c),
            _ => ("colour", c),
        };
        let (to, is_open, popup) = picker(ui, key.with(salt), swatch, c, ids.len());
        if let Some(to) = to {
            changed = Some((c, to, popup));
        }
        if is_open {
            open = Some(c);
        }
    }
    if let Some((c, to, popup)) = changed {
        let r = match active.take() {
            Some(a) if a.now == c => Some(a),
            other => {
                if other.is_some() {
                    ed.history.seal();
                }
                if reaches_locked(ed, &links, &colour_groups(&places(ed, c))) {
                    // Refused before anything changes (see `reaches_locked`).
                    egui::Popup::close_id(ui.ctx(), popup);
                    ed.status = INHERITED_COLOUR.into();
                    None
                } else {
                    ed.end_text_edit();
                    Some(Recolour {
                        from: c,
                        now: c,
                        places: places(ed, c),
                        stamp: 0,
                        popup,
                    })
                }
            }
        };
        if let Some(mut r) = r {
            recolour(ed, &r, to);
            r.now = to;
            r.stamp = ed.doc_stamp;
            active = Some(r);
        }
    } else if active.as_ref().is_some_and(|a| open != Some(a.now)) {
        // Its picker closed (or its colour left the list): that interaction is one step.
        ed.history.seal();
        active = None;
    }
    ui.ctx().data_mut(|d| match active {
        Some(a) => {
            d.insert_temp(key, a);
        }
        None => {
            d.remove::<Recolour>(key);
        }
    });
    if more > 0 {
        note(ui, &format!("And {more} more."));
    }
}

/// A colour's swatch as the Design panel's colour button, at `swatch` without taking room in
/// the row (the row stays the selecting target around it): the change, if any, whether its
/// picker is open, and the picker's popup id. `id` is the button's own identity, independent
/// of the row's place.
fn picker(
    ui: &mut Ui,
    id: egui::Id,
    swatch: egui::Rect,
    c: Color,
    count: usize,
) -> (Option<Color>, bool, egui::Id) {
    let mut child = ui.new_child(egui::UiBuilder::new().id(id).max_rect(swatch));
    child.spacing_mut().interact_size = swatch.size();
    // The button's popup is named from the next automatic id, as egui's colour button names it.
    let popup = child.auto_id_with("popup");
    let mut c2 = super::c32(c);
    let r = child.color_edit_button_srgba(&mut c2);
    let to = super::from32(c2);
    let changed = (r.changed() && to != c).then_some(to);
    let hex = c.to_hex();
    let name = format!("Change {hex}");
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, true, &name));
    let layers = if count == 1 { "layer" } else { "layers" };
    let _ = r.on_hover_text(format!(
        "Change {hex} wherever this Paper uses it as a solid fill or stroke ({count} {layers}), \
         as one undo step. Locked layers keep theirs."
    ));
    (changed, egui::Popup::is_id_open(child.ctx(), popup), popup)
}

/// The swatch of a colour only locked layers use (or one a locked instance inherits): shown,
/// not changeable here.
fn locked_swatch(ui: &Ui, swatch: egui::Rect, c: Color) {
    let edge = egui::Stroke::new(1.0, theme::pal(ui).control_border);
    ui.painter()
        .rect(swatch, 3.0, super::c32(c), edge, egui::StrokeKind::Inside);
}

/// The Fonts status row's value and tooltip: the bundled families, and which of them the shown
/// Paper's text layers use (`in_use`, from [`families_in_use`]).
pub fn fonts_status(in_use: &[(FontFamily, usize)]) -> (String, String) {
    let value = FontFamily::ALL.map(|f| f.name()).join(" · ");
    let used = if in_use.is_empty() {
        "No text on this Paper.".to_owned()
    } else {
        let list: Vec<String> = in_use
            .iter()
            .map(|(f, n)| {
                let layers = if *n == 1 { "layer" } else { "layers" };
                format!("{} ({n} {layers})", f.name())
            })
            .collect();
        format!("This Paper's text uses {}.", list.join(" and "))
    };
    let tip = format!(
        "Bundled with the app (SIL OFL 1.1), each in Regular, Bold, Italic and Bold Italic. The \
         canvas draws these only; a character Geist Mono lacks is drawn in Inter. {used} A \
         style's Aa button, or the Design panel's Font, sets a layer's family."
    );
    (value, tip)
}

/// One status row each: the fonts the canvas draws, and the two things Theme can't do here.
fn status(ui: &mut Ui, ed: &Editor) {
    header(
        ui,
        "System",
        "What this canvas's design system has, and what it hasn't.",
    );
    let (value, tip) = fonts_status(&families_in_use(ed));
    status_row(ui, "Fonts", &value, &tip);
    status_row(
        ui,
        "Named tokens",
        "Not available",
        "This editor has no named tokens: the document keeps literal values only.",
    );
    status_row(
        ui,
        "DESIGN.md",
        "Not connected",
        "No project folder is admitted to this view, and reading or editing a project's \
         DESIGN.md or design.md isn't built here.",
    );
}

/// The reasons behind the status rows, folded until wanted.
fn details(ui: &mut Ui) {
    egui::CollapsingHeader::new(RichText::new("Details").size(theme::SECONDARY))
        .id_salt("theme-details")
        .default_open(false)
        .icon(theme::disclosure)
        .show(ui, |ui| {
            note(
                ui,
                "Fonts: the canvas draws the bundled Inter and Geist Mono faces only; no system \
                 or uploaded font can be chosen. A family belongs to a whole text layer.",
            );
            note(
                ui,
                "Tokens: there are no named values to pick or apply; the Paper's own \
                 styles and colours are listed above; a colour's swatch, or a style's Aa \
                 button, changes it wherever this Paper uses it.",
            );
            note(
                ui,
                "Design source: this view has no admitted project folder and never infers one \
                 from the current folder, a picked folder, the project list or this canvas. \
                 Reading or editing a project's guide also isn't built yet.",
            );
            note(
                ui,
                "Appearance (System, Light or Dark) is separate, in the All Projects menu.",
            );
        });
}

/// A label and its state, the explanation as the row's tooltip; not a control.
fn status_row(ui: &mut Ui, label: &str, value: &str, tip: &str) {
    let p = theme::pal(ui);
    let size = egui::vec2(ui.available_width(), theme::FIELD_H);
    let (row, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(row.intersect(ui.clip_rect()));
    painter.text(
        egui::pos2(row.left() + 6.0, row.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(theme::BODY),
        p.text,
    );
    painter.text(
        egui::pos2(row.right() - 6.0, row.center().y),
        egui::Align2::RIGHT_CENTER,
        value,
        egui::FontId::proportional(theme::SECONDARY),
        p.secondary,
    );
    let _ = resp.on_hover_text(tip);
}

/// One chosen value: room for a colour swatch (its rectangle is returned, for the caller to
/// draw), its label and how many layers use it. The whole row is the target, with the hover
/// fill and the focus ring of the other rows.
fn value_row(ui: &mut Ui, swatch: bool, label: &str, count: usize) -> (egui::Response, egui::Rect) {
    let p = theme::pal(ui);
    let size = egui::vec2(ui.available_width(), theme::FIELD_H);
    let (row, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(row, theme::RADIUS, p.hover);
    }
    let mut x = row.left() + 6.0;
    let swatch_rect =
        egui::Rect::from_center_size(egui::pos2(x + 7.0, row.center().y), egui::vec2(14.0, 14.0));
    if swatch {
        x += 22.0;
    }
    let painter = ui.painter().with_clip_rect(row.intersect(ui.clip_rect()));
    painter.text(
        egui::pos2(x, row.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(theme::BODY),
        p.text,
    );
    painter.text(
        egui::pos2(row.right() - 6.0, row.center().y),
        egui::Align2::RIGHT_CENTER,
        count.to_string(),
        egui::FontId::proportional(theme::SECONDARY),
        p.secondary,
    );
    theme::focus_ring(ui, row, &resp, false);
    let layers = if count == 1 { "layer" } else { "layers" };
    let tip = format!("Select the {count} {layers} that use {label}");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    (resp.on_hover_text(tip), swatch_rect)
}

/// Select `ids` (as the palette's Select layer does): the Design panel then shows, and edits,
/// what they share. A selection only: nothing in the document changes.
fn select(ed: &mut Editor, ids: Vec<NodeId>) {
    ed.end_text_edit();
    ed.selection = ids;
}

/// Revision 22's paragraph metrics at the helper level: what an entry means, how a value is
/// shown, when a style may take one, and that taking one leaves a layer's ranges alone.
#[cfg(test)]
mod metric_tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{Document, Node, TextAlign, TextSpan};

    #[test]
    fn entries_mean_the_design_panel_s_values_and_anything_else_changes_nothing() {
        for (s, want) in [
            ("Auto", Some(None)),
            ("auto", Some(None)),
            ("0", Some(None)),
            ("0%", Some(None)),
            ("140", Some(Some(1.4))),
            ("140%", Some(Some(1.4))),
            (" 12.5 % ", Some(Some(0.125))),
            ("1000", Some(Some(10.0))),
        ] {
            assert_eq!(parse_line_height(s), want, "{s:?}");
        }
        for s in ["", "abc", "inf", "-inf", "NaN", "-5", "1000.5", "1e9", "%"] {
            assert_eq!(parse_line_height(s), None, "{s:?}");
        }
        for (s, want) in [("0", 0.0), ("1.5", 1.5), ("-0.5 px", -0.5), ("-50", -50.0)] {
            assert_eq!(parse_letter_spacing(s), Some(want), "{s:?}");
        }
        for s in ["", "px", "auto", "inf", "NaN", "-50.01", "200.5"] {
            assert_eq!(parse_letter_spacing(s), None, "{s:?}");
        }
        // Shown as stored, to two decimals at most; an unusual value isn't rounded away by
        // being shown (nothing is written unless something else is entered).
        assert_eq!(line_height_text(None), "Auto");
        assert_eq!(line_height_text(Some(1.4)), "140%");
        assert_eq!(line_height_text(Some(1.2345)), "123.45%");
        assert_eq!(letter_spacing_text(-0.0), "0 px");
        assert_eq!(letter_spacing_text(-0.5), "-0.5 px");
    }

    #[test]
    fn a_metric_is_offered_only_for_whole_layers_and_leaves_their_ranges_alone() {
        let text = |s: &str| NodeKind::Text {
            text: s.into(),
            font_size: 20.0,
            auto_width: true,
            align: TextAlign::Left,
        };
        let mut doc = Document::default();
        let whole = doc.alloc_id();
        let mut n = Node::new(whole, text("Æble ✓"), Rect::new(0.0, 0.0, 40.0, 20.0));
        // A colour range: not a size or face, so every character is still 20 Regular.
        n.text_style.spans = vec![TextSpan {
            start: 0,
            end: 2,
            color: Some(Color::rgb(255, 0, 0)),
            ..Default::default()
        }];
        doc.nodes.insert(whole, n);
        doc.root.push(whole);
        let part = doc.alloc_id();
        let mut n = Node::new(part, text("Mixed"), Rect::new(0.0, 50.0, 40.0, 20.0));
        n.text_style.spans = vec![TextSpan {
            start: 0,
            end: 2,
            size: Some(30.0),
            ..Default::default()
        }];
        doc.nodes.insert(part, n);
        doc.root.push(part);
        let mut ed = Editor::new(doc);
        let style = (2000, false, false, FontFamily::Inter);
        let both = style_targets(&ed, style);
        assert_eq!(both.len(), 2);
        assert!(!metrics(&ed, &both).eligible, "part of `part` only");
        let alone: Vec<StyleTarget> = both.iter().filter(|t| t.whole).cloned().collect();
        assert!(metrics(&ed, &alone).eligible);
        let spans = ed.doc.get(whole).unwrap().text_style.spans.clone();
        let r = Restyle {
            from: style,
            now: style,
            targets: alone,
            stamp: 0,
            popup: egui::Id::new("metric"),
        };
        restyle(&mut ed, &r, StyleChange::LineHeight(Some(1.5)));
        restyle(&mut ed, &r, StyleChange::LetterSpacing(-0.5));
        let n = ed.doc.get(whole).unwrap();
        assert_eq!(
            (n.text_style.line_height, n.text_style.letter_spacing),
            (Some(1.5), -0.5)
        );
        assert_eq!(
            n.text_style.spans, spans,
            "its colour range exactly as it was"
        );
        let other = ed.doc.get(part).unwrap();
        assert_eq!(
            (
                other.text_style.line_height,
                other.text_style.letter_spacing
            ),
            (None, 0.0)
        );
        restyle(&mut ed, &r, StyleChange::LineHeight(None));
        assert_eq!(
            ed.doc.get(whole).unwrap().text_style.line_height,
            None,
            "Auto"
        );
    }

    /// Revision 23: a family is part of a style's identity and, like a metric, a whole layer's
    /// value. Setting it keeps the layer's text and ranges, lays it out in the new faces, and is
    /// refused for a style only part of some layer has.
    #[test]
    fn a_family_is_a_whole_layer_value_that_keeps_text_and_ranges() {
        let text = |s: &str| NodeKind::Text {
            text: s.into(),
            font_size: 20.0,
            auto_width: true,
            align: TextAlign::Left,
        };
        let mut doc = Document::default();
        let whole = doc.alloc_id();
        let mut n = Node::new(whole, text("Æøå✓ ok"), Rect::new(0.0, 0.0, 40.0, 20.0));
        n.text_style.spans = vec![TextSpan {
            start: 0,
            end: 2,
            color: Some(Color::rgb(255, 0, 0)),
            ..Default::default()
        }];
        doc.nodes.insert(whole, n);
        doc.root.push(whole);
        let part = doc.alloc_id();
        let mut n = Node::new(part, text("Mixed"), Rect::new(0.0, 50.0, 40.0, 20.0));
        n.text_style.spans = vec![TextSpan {
            start: 0,
            end: 2,
            size: Some(30.0),
            ..Default::default()
        }];
        doc.nodes.insert(part, n);
        doc.root.push(part);
        let mut ed = Editor::new(doc);
        let inter = (2000, false, false, FontFamily::Inter);
        let mono = (2000, false, false, FontFamily::GeistMono);
        // Both layers have 20 Regular Inter (one only partly): not eligible, so refused whole,
        // before anything changes.
        let before = ed.doc.clone();
        let links = inheritors(&ed);
        let family = StyleChange::Family(FontFamily::GeistMono);
        assert!(fresh_restyle(&mut ed, &links, inter, egui::Id::new("f"), family).is_none());
        assert_eq!(ed.doc, before, "nothing changed");
        // The whole layer alone: its family changes, its text, range and box width rule stay.
        let alone: Vec<StyleTarget> = style_targets(&ed, inter)
            .into_iter()
            .filter(|t| t.whole)
            .collect();
        let r = Restyle {
            from: inter,
            now: inter,
            targets: alone,
            stamp: 0,
            popup: egui::Id::new("f"),
        };
        let w0 = ed.doc.get(whole).unwrap().rect.w;
        restyle(&mut ed, &r, family);
        let n = ed.doc.get(whole).unwrap();
        assert_eq!(n.text_style.family, FontFamily::GeistMono);
        assert_eq!(n.kind.text().map(String::as_str), Some("Æøå✓ ok"));
        assert_eq!(
            n.text_style.spans,
            before.get(whole).unwrap().text_style.spans
        );
        assert_ne!(n.rect.w, w0, "laid out again in Geist Mono's advances");
        assert_eq!(
            ed.doc.get(part),
            before.get(part),
            "the other layer untouched"
        );
        // Now its own style: listed apart from Inter's, labelled with its family.
        let styles = text_styles(&ed);
        assert!(
            styles
                .iter()
                .any(|(k, ids)| *k == mono && ids == &vec![whole])
        );
        assert!(
            styles
                .iter()
                .all(|(k, ids)| *k != inter || !ids.contains(&whole))
        );
        assert_eq!(style_label(mono), "20 · Regular · Geist Mono");
        assert_eq!(style_label(inter), "20 · Regular");
        assert_eq!(style_targets(&ed, mono).len(), 1);
        // Inspection names what is in use, and changes nothing.
        let doc = ed.doc.clone();
        let (value, tip) = fonts_status(&families_in_use(&ed));
        assert_eq!(value, "Inter · Geist Mono");
        assert!(
            tip.contains("Inter (1 layer) and Geist Mono (1 layer)"),
            "{tip}"
        );
        assert_eq!(ed.doc, doc);
    }
}

/// The independent R69 review's helper-level probes, kept as regressions: unchanged but for
/// formatting (a named snapshot type, and the failing probe's diagnostic print omitted).
/// `r69_refusal_after_empty_draft_cleanup_is_atomic_too` failed on the reviewed
/// source (its live empty draft was removed and the locked next row moved before the final
/// refusal; the original failure stays the review's evidence); its assertions are the
/// corrected contract, unchanged.
#[cfg(test)]
mod r69_review {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{AutoLayout, Direction, Document, Node, TextAlign};

    fn layer(doc: &mut Document, kind: NodeKind, rect: Rect) -> NodeId {
        let id = doc.alloc_id();
        doc.nodes.insert(id, Node::new(id, kind, rect));
        doc.root.push(id);
        id
    }

    fn text(s: &str) -> NodeKind {
        NodeKind::Text {
            text: s.into(),
            font_size: 24.0,
            auto_width: true,
            align: TextAlign::Left,
        }
    }

    type Snapshot = (
        Document,
        usize,
        String,
        u64,
        u64,
        bool,
        Vec<NodeId>,
        Option<(NodeId, usize, Option<usize>)>,
    );

    fn snapshot(ed: &Editor) -> Snapshot {
        (
            ed.doc.clone(),
            ed.history.undo_len(),
            format!("{:?}", ed.history),
            ed.doc_stamp,
            ed.revision,
            ed.is_dirty(),
            ed.selection.clone(),
            ed.text_edit.as_ref().map(|t| (t.id, t.caret, t.anchor)),
        )
    }

    #[test]
    fn r69_first_dry_run_refusal_keeps_a_pending_text_edit_and_its_caret() {
        let mut doc = Document::default();
        let frame = layer(
            &mut doc,
            NodeKind::Frame { clip: false },
            Rect::new(100.0, 100.0, 300.0, 300.0),
        );
        doc.nodes.get_mut(&frame).unwrap().layout = Some(AutoLayout {
            direction: Direction::Vertical,
            ..Default::default()
        });
        let title = layer(&mut doc, text("Title"), Rect::new(0.0, 0.0, 50.0, 30.0));
        let control = layer(&mut doc, NodeKind::Rect, Rect::new(0.0, 0.0, 80.0, 20.0));
        for id in [title, control] {
            doc.nodes.get_mut(&id).unwrap().parent = Some(frame);
        }
        doc.root.retain(|id| *id == frame);
        doc.nodes.get_mut(&frame).unwrap().children = vec![title, control];
        let draft = layer(&mut doc, text("Draft"), Rect::new(500.0, 100.0, 50.0, 30.0));
        let mut ed = Editor::new(doc);
        ed.selection = vec![title];
        ed.style_text("font size", |sp| sp.size = Some(24.0));
        ed.edit("Lock", |tx| tx.node_mut(control).unwrap().locked = true);
        ed.start_text_edit(draft, None);
        ed.insert_text("ø draft");
        let before = snapshot(&ed);
        let links = inheritors(&ed);
        assert!(
            links.is_empty(),
            "this case requires the new dry run, not the row's inheritance shortcut"
        );
        assert!(
            fresh_restyle(
                &mut ed,
                &links,
                (2400, false, false, FontFamily::Inter),
                egui::Id::new("r69"),
                StyleChange::Size(32.0)
            )
            .is_none()
        );
        assert_eq!(
            snapshot(&ed),
            before,
            "refusal leaves pending recording, text, history and caret alone"
        );
        ed.insert_text("!");
        assert_eq!(
            ed.doc.get(draft).unwrap().kind.text().map(String::as_str),
            Some("ø draft!")
        );
    }

    #[test]
    fn r69_allowed_change_rechecks_targets_after_ending_an_empty_text_draft() {
        let mut doc = Document::default();
        let title = layer(&mut doc, text("Title"), Rect::new(100.0, 100.0, 50.0, 30.0));
        let empty = layer(&mut doc, text("  "), Rect::new(500.0, 100.0, 50.0, 30.0));
        let mut ed = Editor::new(doc);
        ed.start_text_edit(empty, None);
        let stamp = ed.doc_stamp;
        let links = inheritors(&ed);
        let r = fresh_restyle(
            &mut ed,
            &links,
            (2400, false, false, FontFamily::Inter),
            egui::Id::new("r69"),
            StyleChange::Size(32.0),
        )
        .expect("no locks, normal empty-draft finish allowed");
        assert_ne!(
            ed.doc_stamp, stamp,
            "empty text cleanup actually changed the page"
        );
        assert!(ed.text_edit.is_none());
        assert!(ed.doc.get(empty).is_none());
        assert_eq!(
            r.targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![title]
        );
        let after_cleanup = ed.doc.clone();
        restyle(&mut ed, &r, StyleChange::Size(32.0));
        assert_eq!(ed.doc.get(title).unwrap().kind.font_size(), Some(32.0));
        ed.undo();
        assert_eq!(
            ed.doc, after_cleanup,
            "the Theme edit is its own complete undo step after normal draft completion"
        );
    }

    #[test]
    fn r69_refusal_after_empty_draft_cleanup_is_atomic_too() {
        // A wrapping row's taller (24 pt) draft masks the target's 12 -> 18 height change.
        // Once the empty draft is removed, the locked second row would follow that height.
        // Every text box is fitted first by the real Editor; no inconsistent geometry seed.
        let mut doc = Document::default();
        let frame = layer(
            &mut doc,
            NodeKind::Frame { clip: false },
            Rect::new(100.0, 100.0, 200.0, 300.0),
        );
        doc.nodes.get_mut(&frame).unwrap().layout = Some(AutoLayout {
            direction: Direction::Horizontal,
            wrap: true,
            ..Default::default()
        });
        let title = layer(&mut doc, text("Title"), Rect::new(0.0, 0.0, 80.0, 10.0));
        let draft = layer(&mut doc, text("Box"), Rect::new(0.0, 0.0, 60.0, 10.0));
        for id in [title, draft] {
            let n = doc.nodes.get_mut(&id).unwrap();
            n.parent = Some(frame);
            if let NodeKind::Text { auto_width, .. } = &mut n.kind {
                *auto_width = false;
            }
        }
        let control = layer(&mut doc, NodeKind::Rect, Rect::new(0.0, 0.0, 120.0, 20.0));
        doc.nodes.get_mut(&control).unwrap().parent = Some(frame);
        doc.nodes.get_mut(&frame).unwrap().children = vec![title, draft, control];
        doc.root.retain(|id| *id == frame);
        let mut ed = Editor::new(doc);
        ed.selection = vec![draft];
        ed.style_text("font size", |sp| sp.size = Some(24.0));
        ed.selection = vec![title];
        ed.style_text("font size", |sp| sp.size = Some(12.0));
        ed.edit("Lock", |tx| tx.node_mut(control).unwrap().locked = true);
        ed.start_text_edit(draft, None);
        ed.insert_text("  ");
        let before = snapshot(&ed);
        let links = inheritors(&ed);
        // (Styles carry their family since revision 23; Inter is these layers'.)
        let style = (1200, false, false, FontFamily::Inter);
        let proposed = Restyle {
            from: style,
            now: style,
            targets: style_targets(&ed, style),
            stamp: 0,
            popup: egui::Id::new("r69-cleanup"),
        };
        assert!(
            !restyle_reaches_locked(&ed, &proposed, StyleChange::Size(18.0)),
            "the first preview really permits the proposed change"
        );
        let result = fresh_restyle(
            &mut ed,
            &links,
            style,
            egui::Id::new("r69-cleanup"),
            StyleChange::Size(18.0),
        );
        assert!(
            result.is_none(),
            "the final preview refuses the proposed change"
        );
        assert_eq!(
            snapshot(&ed),
            before,
            "a refused first change must also preserve the open draft and locked drawing data \
             when ending the draft would reflow"
        );
    }
}
