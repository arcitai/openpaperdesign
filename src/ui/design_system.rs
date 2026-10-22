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
//! - A text style's "Aa" button changes that style (its size, or its face among Inter's four)
//!   wherever the Paper's text uses it (the same reason: selecting its layers edits their whole
//!   text, not only the characters in that style). It reaches exactly the characters that had
//!   it when the change began, on unlocked text layers: a layer whose every character has it
//!   changes its own size or face, as the Design panel's whole-layer edits do; otherwise a
//!   range style covers exactly those characters (`TextStyle::apply_range`, keeping every
//!   other value a range has). One editor use is one undo step, laid out again by the existing
//!   text layout; instances get overrides. No font family can be chosen (the renderer draws
//!   Inter only), and no named style is created.
//! - Font: the one family the renderer draws (Inter, bundled, in four faces). Read-only.
//! - Named tokens and the project's `DESIGN.md` / `design.md`: not available. The document has
//!   no token store; this view has no admitted project folder (OPD never infers one), and the
//!   guide reading and editing itself isn't built (R19–R21). Both are status rows with their
//!   reason, never a decorative list or a promise.

use super::{inspector, section};
use crate::editor::Editor;
use crate::model::{CharStyle, Color, NodeId, NodeKind, Paint};
use crate::theme;
use egui::{RichText, Ui};
use std::collections::{HashMap, HashSet};

/// A text style a Paper uses: its size in hundredths of a point (so equal sizes group), bold
/// and italic.
pub type StyleKey = (i64, bool, bool);

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
            let key = style_key(c);
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

/// The style key of a character's resolved style.
fn style_key(c: CharStyle) -> StyleKey {
    ((c.size * 100.0).round() as i64, c.bold, c.italic)
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

/// A text style as people read it: "48 · Bold", "13.5 · Italic".
pub fn style_label((size, bold, italic): StyleKey) -> String {
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
    format!("{size} · {face}")
}

/// The Theme view, under the left panel's switch.
pub fn panel(ui: &mut Ui, ed: &mut Editor) {
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
            status(ui);
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
        "The sizes and faces this Paper's text uses. Choose one to select that text; its Aa \
         button changes that style wherever this Paper uses it.",
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
    let mut changed = None;
    let mut open = None;
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
        let (change, is_open, popup) = style_editor(ui, key.with(salt), slot, style, ids.len());
        if let Some(change) = change {
            changed = Some((style, change, popup));
        }
        if is_open {
            open = Some(style);
        }
    }
    if let Some((style, change, popup)) = changed {
        let next = match active.take() {
            Some(a) if a.now == style => {
                // Each later change of a use is tried on a copy too: one that would reach a
                // locked layer isn't applied, and the use ends (its earlier changes stay, one
                // step).
                if restyle_reaches_locked(ed, &a, change) {
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
                fresh_restyle(ed, &links, style, popup, change)
            }
        };
        match next {
            Some(mut r) => {
                restyle(ed, &r, change);
                r.now = match change {
                    StyleChange::Size(v) => ((v * 100.0).round() as i64, r.now.1, r.now.2),
                    StyleChange::Face { bold, italic } => (r.now.0, bold, italic),
                };
                r.stamp = ed.doc_stamp;
                active = Some(r);
            }
            None => {
                // Refused before it applies (see `reaches_locked`, `restyle_reaches_locked`).
                egui::Popup::close_id(ui.ctx(), popup);
                ed.status = INHERITED_STYLE.into();
            }
        }
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

/// One change from a text style's editor: its size, or its face (Inter's four).
#[derive(Clone, Copy)]
enum StyleChange {
    Size(f64),
    Face { bold: bool, italic: bool },
}

/// The faces Theme offers: the bundled Inter's four, as the renderer draws them.
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

/// A new text style use beginning with `change`, or `None` when Theme refuses it before
/// anything changes: a locked instance inherits the style or its text box ([`reaches_locked`]
/// on [`style_groups`]), or applying the change would reach a locked layer some other way
/// ([`restyle_reaches_locked`]). An open text edit is ended only once it is allowed; if that
/// changed the Paper, its targets are found and checked again.
fn fresh_restyle(
    ed: &mut Editor,
    links: &HashMap<NodeId, Vec<NodeId>>,
    style: StyleKey,
    popup: egui::Id,
    change: StyleChange,
) -> Option<Restyle> {
    let allowed = |ed: &Editor, r: &Restyle| {
        !reaches_locked(ed, links, &style_groups(ed, &r.targets))
            && !restyle_reaches_locked(ed, r, change)
    };
    let r = Restyle {
        from: style,
        now: style,
        targets: style_targets(ed, style),
        stamp: 0,
        popup,
    };
    if !allowed(&*ed, &r) {
        return None;
    }
    let stamp = ed.doc_stamp;
    ed.end_text_edit();
    if ed.doc_stamp == stamp {
        return Some(r);
    }
    let r = Restyle {
        targets: style_targets(ed, style),
        ..r
    };
    allowed(&*ed, &r).then_some(r)
}

/// Whether applying `change` to `r`'s targets would change any effectively locked layer: the
/// change applied to a copy of the shown Paper through the very same code (`restyle`: its
/// range edits, `fit_text`, overrides and the commit's fixup, which keeps component
/// instances, groups, auto layout and bound arrows in step), so every derived effect counts,
/// not only the inherited values [`reaches_locked`] foresees for a row (R68). Nothing of the
/// real document or its history is touched.
fn restyle_reaches_locked(ed: &Editor, r: &Restyle, change: StyleChange) -> bool {
    let mut probe = Editor::new(ed.doc.clone());
    restyle(&mut probe, r, change);
    ed.doc
        .nodes
        .keys()
        .any(|id| ed.doc.is_effectively_locked(*id) && probe.doc.get(*id) != ed.doc.get(*id))
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
        let at = |i: usize| style_key(n.text_style.char_style(i, *font_size, Color::INK));
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
/// strike). Text layers are laid out again (`Editor::fit_text`: width kept unless it grows to
/// fit its text); edits inside instances become overrides.
fn restyle(ed: &mut Editor, r: &Restyle, change: StyleChange) {
    let ids: Vec<NodeId> = r.targets.iter().map(|t| t.id).collect();
    let field = match change {
        StyleChange::Size(_) => "font size",
        StyleChange::Face { .. } => "font",
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
                    }
                    StyleChange::Face { bold, italic } => {
                        n.text_style.bold = bold;
                        n.text_style.italic = italic;
                        for sp in &mut n.text_style.spans {
                            sp.bold = None;
                            sp.italic = None;
                        }
                    }
                }
                n.text_style.normalize();
            } else {
                // Only the value that changes: a face from Regular to Bold leaves italic as the
                // characters have it, without writing it into their ranges.
                let (_, was_bold, was_italic) = r.now;
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
/// the selecting target around it), and the small popup it opens: the style's size and Inter's
/// four faces, nothing the renderer can't draw. Returns the change, if any, whether the popup
/// is open, and its id. `id` is the button's own identity, independent of the row's place.
fn style_editor(
    ui: &mut Ui,
    id: egui::Id,
    slot: egui::Rect,
    style: StyleKey,
    count: usize,
) -> (Option<StyleChange>, bool, egui::Id) {
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
    let (_, bold, italic) = style;
    let mut change = None;
    egui::Popup::menu(&resp)
        .id(popup)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
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
            note(ui, "Inter only: the canvas draws its four faces.");
        });
    (change, egui::Popup::is_id_open(ui.ctx(), popup), popup)
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

/// One status row each: the font the canvas draws, and the two things Theme can't do here.
fn status(ui: &mut Ui) {
    header(
        ui,
        "System",
        "What this canvas's design system has, and what it hasn't.",
    );
    status_row(
        ui,
        "Font",
        "Inter · 4 faces",
        "Inter Regular, Bold, Italic and Bold Italic, bundled (SIL OFL 1.1). The canvas draws \
         these faces only.",
    );
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
                "Fonts: the canvas draws the bundled Inter faces only, so another family \
                 can't be chosen.",
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
