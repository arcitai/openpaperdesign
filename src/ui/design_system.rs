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
//!   undoable edits. Nothing here writes the document itself.
//! - Font: the one family the renderer draws (Inter, bundled, in four faces). Read-only.
//! - Named tokens and the project's `DESIGN.md` / `design.md`: not available. The document has
//!   no token store; this view has no admitted project folder (OPD never infers one), and the
//!   guide reading and editing itself isn't built (R19–R21). Both are status rows with their
//!   reason, never a decorative list or a promise.

use super::{inspector, section};
use crate::editor::Editor;
use crate::model::{Color, NodeId, NodeKind, Paint};
use crate::theme;
use egui::{RichText, Ui};

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
            let key = ((c.size * 100.0).round() as i64, c.bold, c.italic);
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
        "The sizes and faces this Paper's text uses. Choose one to select that text and edit \
         it in the Design panel.",
    );
    let styles = text_styles(ed);
    if styles.is_empty() {
        note(ui, "No text on this Paper.");
        return;
    }
    for (key, ids) in styles {
        let label = style_label(key);
        if value_row(ui, None, &label, ids.len()).clicked() {
            select(ed, ids);
        }
    }
}

fn used_colours(ui: &mut Ui, ed: &mut Editor) {
    header(
        ui,
        "Colours",
        "The solid fill and stroke colours this Paper uses. Choose one to select its layers \
         and edit Fill or Stroke in the Design panel.",
    );
    let colours = colours(ed);
    if colours.is_empty() {
        note(ui, "No solid colours on this Paper.");
        return;
    }
    let more = colours.len().saturating_sub(MAX_COLOURS);
    for (c, ids) in colours.into_iter().take(MAX_COLOURS) {
        if value_row(ui, Some(c), &c.to_hex(), ids.len()).clicked() {
            select(ed, ids);
        }
    }
    if more > 0 {
        note(ui, &format!("And {more} more."));
    }
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
                 styles and colours are listed above.",
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

/// One chosen value: an optional colour swatch, its label and how many layers use it. The
/// whole row is the target, with the hover fill and the focus ring of the other rows.
fn value_row(ui: &mut Ui, swatch: Option<Color>, label: &str, count: usize) -> egui::Response {
    let p = theme::pal(ui);
    let size = egui::vec2(ui.available_width(), theme::FIELD_H);
    let (row, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(row, theme::RADIUS, p.hover);
    }
    let mut x = row.left() + 6.0;
    if let Some(c) = swatch {
        let r = egui::Rect::from_center_size(
            egui::pos2(x + 7.0, row.center().y),
            egui::vec2(14.0, 14.0),
        );
        let edge = egui::Stroke::new(1.0, p.control_border);
        ui.painter()
            .rect(r, 3.0, super::c32(c), edge, egui::StrokeKind::Inside);
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
    resp.on_hover_text(tip)
}

/// Select `ids` (as the palette's Select layer does): the Design panel then shows, and edits,
/// what they share. A selection only: nothing in the document changes.
fn select(ed: &mut Editor, ids: Vec<NodeId>) {
    ed.end_text_edit();
    ed.selection = ids;
}
