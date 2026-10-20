//! The Design inspector: macOS-style grouped sections for the selection's geometry, layout,
//! constraints, text, fills, stroke and layer properties. Edits are coalesced per field so a
//! drag on a number is one undo step, and edits inside instances become overrides.

use super::{UiState, c32, from32, title_text};
use crate::editor::vector_ops::BoolOp;
use crate::editor::{AppRequest, Editor};
use crate::geom::v2;
use crate::model::{
    Align, AutoLayout, BlendMode, Cap, Color, Constraint, Direction, ImageMode, Join, Node, NodeId,
    NodeKind, Paint, Sizing, Stop, StrokeAlign, TextAlign,
};
use crate::theme;
use egui::{RichText, Ui};

const LABEL_W: f32 = 64.0;

/// Apply a property edit as a coalesced transaction keyed by field + selection.
pub fn apply(ed: &mut Editor, field: &str, ids: &[NodeId], f: impl Fn(&mut Node)) {
    let key = format!("{field}:{ids:?}");
    ed.begin_coalescing(&format!("Change {field}"), Some(&key));
    {
        let mut tx = ed.tx();
        for id in ids {
            if let Some(n) = tx.node_mut(*id) {
                f(n);
            }
        }
    }
    ed.mark_overrides(ids, field);
    for id in ids {
        ed.fit_text(*id);
    }
    ed.commit();
}

fn seal_on_release(ed: &mut Editor, r: &egui::Response) {
    if r.drag_stopped() || r.lost_focus() {
        ed.history.seal();
    }
}

/// A row: secondary label on the left, controls on the right.
fn row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(LABEL_W, 20.0), egui::Sense::hover());
        ui.painter().text(
            r.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(theme::SECONDARY + 1.0),
            theme::pal(ui).secondary,
        );
        add(ui)
    })
    .inner
}

/// A compact number field with a leading letter (like Figma's "X 120").
fn num_field(
    ui: &mut Ui,
    ed: &mut Editor,
    prefix: &str,
    value: f64,
    speed: f64,
    range: std::ops::RangeInclusive<f64>,
    width: f32,
) -> Option<f64> {
    let mut v = value;
    let r = ui.add_sized(
        egui::vec2(width, 20.0),
        egui::DragValue::new(&mut v)
            .speed(speed)
            .range(range)
            .max_decimals(2)
            .prefix(prefix),
    );
    seal_on_release(ed, &r);
    (r.changed() && v != value).then_some(v)
}

fn color_button(ui: &mut Ui, ed: &mut Editor, c: Color) -> Option<Color> {
    let mut c2 = c32(c);
    let r = ui.color_edit_button_srgba(&mut c2);
    seal_on_release(ed, &r);
    (r.changed() && from32(c2) != c).then(|| from32(c2))
}

fn combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    current: T,
    options: &[(T, &str)],
    width: f32,
) -> Option<T> {
    let mut v = current;
    let label = options
        .iter()
        .find(|(o, _)| *o == current)
        .map_or("—", |(_, l)| *l);
    egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .width(width)
        .show_ui(ui, |ui| {
            for (o, l) in options {
                ui.selectable_value(&mut v, *o, *l);
            }
        });
    (v != current).then_some(v)
}

/// The value every selected layer agrees on, or `None` when they differ (Mixed). Uniform values
/// are shown as values; differing ones are never shown as the first layer's.
pub fn uniform<T: PartialEq>(nodes: &[Node], f: impl Fn(&Node) -> T) -> Option<T> {
    let (first, rest) = nodes.split_first()?;
    let v = f(first);
    rest.iter().all(|n| f(n) == v).then_some(v)
}

/// Every distinct style the characters of a text layer have, as upstream's
/// `TextStyle::char_style` resolves them for the layout: one per run between its ranges' edges,
/// or the layer's own style when it has no text.
fn char_styles(n: &Node) -> Vec<crate::model::CharStyle> {
    let len = n.kind.text().map_or(0, |t| t.chars().count());
    let size = n.kind.font_size().unwrap_or(16.0);
    let color = n
        .style
        .fills
        .last()
        .map_or(Color::INK, |p| p.preview_color());
    let mut starts: Vec<usize> = std::iter::once(0)
        .chain(n.text_style.spans.iter().flat_map(|s| [s.start, s.end]))
        .filter(|i| *i == 0 || *i < len)
        .collect();
    starts.sort_unstable();
    starts.dedup();
    starts
        .into_iter()
        .map(|i| n.text_style.char_style(i, size, color))
        .collect()
}

/// The value every character of every selected text layer has ([`char_styles`]), or `None`
/// when any differ (Mixed).
fn shared_char<T: PartialEq>(
    styles: &[Vec<crate::model::CharStyle>],
    f: impl Fn(&crate::model::CharStyle) -> T,
) -> Option<T> {
    let mut all = styles.iter().flatten();
    let v = f(all.next()?);
    all.all(|s| f(s) == v).then_some(v)
}

/// The placeholder of a field whose values differ across the selection: it says Mixed and
/// edits nothing (a number field can't show Mixed).
const MIXED: &str = "Mixed";

fn mixed(ui: &mut Ui, prefix: &str, width: f32) {
    let r = ui.add_sized(
        egui::vec2(width, 20.0),
        egui::Label::new(
            RichText::new(format!("{prefix}{MIXED}"))
                .size(theme::SECONDARY + 1.0)
                .color(theme::pal(ui).secondary),
        ),
    );
    let _ = r.on_hover_text("The selected layers differ here");
}

/// A [`combo`] that can show Mixed: choosing an option sets it on every selected layer.
fn combo_mixed<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    current: Option<T>,
    options: &[(T, &str)],
    width: f32,
) -> Option<T> {
    let label = current.map_or(MIXED, |c| {
        options
            .iter()
            .find(|(o, _)| *o == c)
            .map_or("—", |(_, l)| *l)
    });
    let mut chosen = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .width(width)
        .show_ui(ui, |ui| {
            for (o, l) in options {
                if ui.selectable_label(current == Some(*o), *l).clicked() && current != Some(*o) {
                    chosen = Some(*o);
                }
            }
        });
    chosen
}

/// A quiet one-line explanation under a section.
fn hint(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::SECONDARY)
            .color(theme::pal(ui).secondary),
    );
}

fn separator(ui: &mut Ui) {
    ui.add_space(6.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.x_range(),
        r.top(),
        egui::Stroke::new(1.0, theme::pal(ui).separator),
    );
    ui.add_space(8.0);
}

fn icon_btn(ui: &mut Ui, icon: theme::Icon, on: bool, tip: &str) -> bool {
    theme::icon_button(ui, icon, 24.0, on)
        .on_hover_text(tip)
        .clicked()
}

fn icon_toggle(ui: &mut Ui, label: &str, on: bool, tip: &str) -> bool {
    ui.add(
        egui::Button::selectable(on, RichText::new(label).font(theme::semibold(12.0)))
            .min_size(egui::vec2(24.0, 20.0)),
    )
    .on_hover_text(tip)
    .clicked()
}

/// The Design inspector (DESIGN.md, revision 2 slice 3 and revision 18's states): quiet and
/// contextual, under the panel's own top row (its title and toggle). With nothing selected (or
/// after a click on empty canvas) it shows, for a creation tool, that tool's defaults first
/// (labelled "For new …", which never edit a selection), then the shown Paper and the view's
/// grid. With a selection it shows the sections every selected layer has; values they share
/// are shown, values that differ show Mixed (or the widget is left out, with a word on why),
/// and editing one sets it on all of them in one coalesced change. Sketch and Export are
/// folded sections.
pub fn panel(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            let ids = ed.top_selection();
            if ids.is_empty() {
                nothing_selected(ui, ed, st);
                separator(ui);
                export_section(ui, ed, st);
                return;
            }
            let nodes: Vec<Node> = ids
                .iter()
                .filter_map(|id| ed.doc.get(*id).cloned())
                .collect();
            let first = nodes[0].clone();
            let all = |f: &dyn Fn(&Node) -> bool| nodes.iter().all(f);
            header(ui, ed, &ids, &first);
            if ids.len() > 1 {
                let kinds_differ = uniform(&nodes, |n| n.kind.type_name()).is_none();
                hint(
                    ui,
                    if kinds_differ {
                        "Only what every selected layer has is shown. Mixed means the values \
                         differ; a change applies to all of them."
                    } else {
                        "Mixed means the values differ; a change applies to all of them."
                    },
                );
            }
            separator(ui);
            align_row(ui, ed, &ids);
            separator(ui);
            geometry(ui, ed, &ids, &nodes);
            if ids.len() == 1 {
                child_layout(ui, ed, &ids, &first);
                if first.is_frame() {
                    auto_layout(ui, ed, &ids, &first);
                }
            }
            if all(&|n| matches!(n.kind, NodeKind::Text { .. })) {
                separator(ui);
                text_section(ui, ed, &ids, &nodes);
            }
            if all(&|n| {
                crate::render::draw::is_area(n) && !matches!(n.kind, NodeKind::Stroke { .. })
            }) {
                separator(ui);
                fills(ui, ed, &ids, &nodes);
            }
            if all(&|n| {
                !matches!(
                    n.kind,
                    NodeKind::Text { .. } | NodeKind::Stroke { .. } | NodeKind::Group
                )
            }) {
                separator(ui);
                stroke(ui, ed, &ids, &nodes);
            }
            if all(&|n| matches!(n.kind, NodeKind::Connector { .. })) {
                separator(ui);
                arrow_section(ui, ed, &ids, &nodes);
            }
            if all(&|n| matches!(n.kind, NodeKind::Sticky { .. })) {
                separator(ui);
                sticky_section(ui, ed, &ids, &nodes);
            }
            separator(ui);
            layer(ui, ed, &ids, &nodes);
            if all(&sketchable) {
                separator(ui);
                sketch_section(ui, ed, &ids, &nodes);
            }
            separator(ui);
            export_section(ui, ed, st);
        });
}

/// Layers the optional hand-drawn style applies to.
fn sketchable(n: &Node) -> bool {
    !matches!(
        n.kind,
        NodeKind::Text { .. } | NodeKind::Stroke { .. } | NodeKind::Group | NodeKind::Sticky { .. }
    )
}

/// What the Sketch section says about itself.
const SKETCH_HELP: &str = "Hand-drawn style draws shapes with sketchy, uneven lines, like a \
    whiteboard. Shapes are clean geometry unless you turn it on.";

/// FigJam/Excalidraw-style sticky colours.
pub const STICKY_COLORS: [Color; 6] = [
    Color::rgb(0xff, 0xdf, 0x6b),
    Color::rgb(0xff, 0xb8, 0xa8),
    Color::rgb(0xb5, 0xe6, 0xa2),
    Color::rgb(0xa8, 0xd8, 0xff),
    Color::rgb(0xd7, 0xc2, 0xff),
    Color::rgb(0xe8, 0xe8, 0xec),
];

fn swatches(ui: &mut Ui, current: Option<Color>) -> Option<Color> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for c in STICKY_COLORS {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
            ui.painter().rect_filled(r, 5.0, c32(c));
            if Some(c) == current {
                ui.painter().rect_stroke(
                    r,
                    5.0,
                    egui::Stroke::new(2.0, theme::accent()),
                    egui::StrokeKind::Outside,
                );
            }
            if resp.clicked() {
                out = Some(c);
            }
        }
    });
    out
}

fn sticky_section(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], nodes: &[Node]) {
    ui.label(title_text("Sticky note"));
    // No swatch is marked when the notes' colours differ.
    let cur = uniform(nodes, |n| n.style.fill().unwrap_or(STICKY_COLORS[0]));
    if let Some(c) = swatches(ui, cur) {
        apply(ed, "fill", ids, |n| n.style.set_fill(Some(c)));
        ed.sticky_color = c;
    }
    let size = uniform(nodes, |n| match n.kind {
        NodeKind::Sticky { font_size, .. } => Some(font_size),
        _ => None,
    })
    .flatten();
    row(ui, "Font size", |ui| match size {
        Some(font_size) => {
            if let Some(v) = num_field(ui, ed, "", font_size, 0.5, 8.0..=200.0, 70.0) {
                apply(ed, "font size", ids, |n| {
                    if let NodeKind::Sticky { font_size, .. } = &mut n.kind {
                        *font_size = v;
                    }
                });
            }
        }
        None => mixed(ui, "", 70.0),
    });
}

fn arrow_section(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], nodes: &[Node]) {
    use crate::model::{Head, Route};
    let connector = |n: &Node| match &n.kind {
        NodeKind::Connector {
            route,
            start_head,
            end_head,
            ..
        } => Some((*route, *start_head, *end_head)),
        _ => None,
    };
    let route = uniform(nodes, |n| connector(n).map(|c| c.0)).flatten();
    let start_head = uniform(nodes, |n| connector(n).map(|c| c.1)).flatten();
    let end_head = uniform(nodes, |n| connector(n).map(|c| c.2)).flatten();
    ui.label(title_text("Arrow"));
    let routes = [
        (Route::Straight, "Straight"),
        (Route::Curved, "Curved"),
        (Route::Elbow, "Elbow"),
    ];
    row(ui, "Type", |ui| {
        if let Some(v) = combo_mixed(ui, "route", route, &routes, 96.0) {
            apply(ed, "route", ids, |n| {
                if let NodeKind::Connector { route, points, .. } = &mut n.kind {
                    *route = v;
                    if v != Route::Elbow && points.len() > 2 && v == Route::Straight {
                        let (a, b) = (points[0], *points.last().unwrap());
                        *points = vec![a, b];
                    }
                }
            });
            ed.arrow_route = v;
            // Re-route through the binding logic.
            ed.begin("Change route");
            {
                let mut tx = ed.tx();
                for id in ids {
                    crate::editor::whiteboard::update_connector(&mut tx, *id);
                }
            }
            ed.commit();
        }
    });
    let heads: Vec<(Head, &str)> = Head::ALL.iter().map(|h| (*h, h.label())).collect();
    row(ui, "Heads", |ui| {
        if let Some(v) = combo_mixed(ui, "h-start", start_head, &heads, 72.0) {
            apply(ed, "heads", ids, |n| {
                if let NodeKind::Connector { start_head, .. } = &mut n.kind {
                    *start_head = v;
                }
            });
        }
        if let Some(v) = combo_mixed(ui, "h-end", end_head, &heads, 72.0) {
            apply(ed, "heads", ids, |n| {
                if let NodeKind::Connector { end_head, .. } = &mut n.kind {
                    *end_head = v;
                }
            });
        }
    });
    if let [only] = nodes
        && let NodeKind::Connector { start, end, .. } = &only.kind
    {
        let bound = |b: &Option<crate::model::Binding>| {
            b.and_then(|b| ed.doc.get(b.node).map(|n| n.name.clone()))
                .unwrap_or_else(|| "—".into())
        };
        hint(ui, &format!("Connects {} → {}", bound(start), bound(end)));
    }
}

/// The optional hand-drawn style, folded by default and explained (clean geometry unless it is
/// turned on; drawings that use it keep it).
fn sketch_section(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], nodes: &[Node]) {
    use crate::model::rough::{FillStyle, Rough};
    let on = uniform(nodes, |n| n.style.rough.is_some());
    let title = match on {
        Some(true) => "Sketch · on",
        Some(false) => "Sketch",
        None => "Sketch · mixed",
    };
    egui::CollapsingHeader::new(title_text(title))
        .id_salt("sketch-section")
        .default_open(false)
        .show(ui, |ui| {
            hint(ui, SKETCH_HELP);
            match on {
                Some(on) => {
                    let mut v = on;
                    if ui
                        .checkbox(&mut v, "Hand-drawn style")
                        .on_hover_text("Sketchy, uneven lines (⌘⌥H)")
                        .changed()
                    {
                        ed.toggle_sketch();
                    }
                }
                None => {
                    hint(ui, "Some selected layers are hand-drawn, some are clean.");
                    if ui.small_button("Make all clean").clicked() {
                        apply(ed, "rough", ids, |n| n.style.rough = None);
                    }
                }
            }
            if on != Some(true) {
                return;
            }
            let levels = [(0u8, "Architect"), (1, "Artist"), (2, "Cartoonist")];
            let cur = uniform(nodes, |n| {
                n.style
                    .rough
                    .map(|r| r.roughness.round().clamp(0.0, 2.0) as u8)
            })
            .flatten();
            row(ui, "Sloppiness", |ui| {
                if let Some(v) = combo_mixed(ui, "rough", cur, &levels, 96.0) {
                    apply(ed, "rough", ids, |n| {
                        if let Some(r) = &mut n.style.rough {
                            r.roughness = v as f64;
                        }
                    });
                }
            });
            let fills: Vec<(FillStyle, &str)> =
                FillStyle::ALL.iter().map(|f| (*f, f.label())).collect();
            let style = uniform(nodes, |n| n.style.rough.map(|r| r.fill_style)).flatten();
            row(ui, "Fill style", |ui| {
                if let Some(v) = combo_mixed(ui, "fillstyle", style, &fills, 96.0) {
                    apply(ed, "rough", ids, |n| {
                        if let Some(r) = &mut n.style.rough {
                            r.fill_style = v;
                        }
                    });
                }
            });
            if ui
                .small_button("New wobble")
                .on_hover_text("Re-roll the random seed")
                .clicked()
            {
                apply(ed, "rough", ids, |n| {
                    if let Some(r) = &mut n.style.rough {
                        *r = Rough {
                            seed: r.seed.wrapping_mul(1103515245).wrapping_add(12345) | 1,
                            ..*r
                        };
                    }
                });
            }
        });
}

/// Export settings and button, like Figma's Export section; folded until wanted (the menu and
/// palette keep every export too).
fn export_section(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    egui::CollapsingHeader::new(title_text("Export"))
        .id_salt("export-section")
        .default_open(false)
        .show(ui, |ui| export_body(ui, ed, st));
}

fn export_body(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    use crate::io::export::{Format, Scope};
    let spec = &mut st.export;
    ui.horizontal(|ui| {
        let fmts: Vec<(Format, &str)> = Format::ALL
            .iter()
            .map(|f| {
                (
                    *f,
                    match f {
                        Format::Png => "PNG",
                        Format::Svg => "SVG",
                        Format::Pdf => "PDF",
                    },
                )
            })
            .collect();
        if let Some(v) = combo(ui, "exp-format", spec.format, &fmts, 64.0) {
            spec.format = v;
        }
        if spec.format == Format::Png {
            let scales = [(1u8, "1x"), (2, "2x"), (3, "3x"), (4, "4x")];
            if let Some(v) = combo(ui, "exp-scale", spec.scale as u8, &scales, 48.0) {
                spec.scale = v as f64;
            }
        }
    });
    let scopes: Vec<(Scope, &str)> = Scope::ALL.iter().map(|s| (*s, s.label())).collect();
    row(ui, "Export", |ui| {
        if let Some(v) = combo(ui, "exp-scope", spec.scope, &scopes, 110.0) {
            spec.scope = v;
        }
    });
    if spec.format != Format::Svg || spec.transparent {
        ui.checkbox(&mut spec.transparent, "Transparent background");
    }
    let what = match (spec.scope, ed.top_selection().len()) {
        (Scope::Selection, 0) => format!("Paper “{}”", ed.page_name()),
        (Scope::Selection, 1) => ed
            .doc
            .get(ed.top_selection()[0])
            .map_or("selection".into(), |n| format!("“{}”", n.name)),
        (Scope::Selection, n) => format!("{n} layers"),
        (Scope::Frames, _) => "each frame".into(),
        (Scope::Page, _) => format!("Paper “{}”", ed.page_name()),
        (Scope::AllPages, _) => format!("{} Papers", ed.pages.len()),
    };
    if ui
        .add(
            egui::Button::new(format!("Export {what}…"))
                .min_size(egui::vec2(ui.available_width(), 24.0)),
        )
        .clicked()
    {
        ed.requests.push(AppRequest::Export(*spec));
    }
    ui.horizontal(|ui| {
        if ui.small_button("Copy as PNG").clicked() {
            ed.run(crate::editor::shortcuts::Action::CopyAsPng);
        }
        if ui.small_button("Copy as SVG").clicked() {
            ed.run(crate::editor::shortcuts::Action::CopyAsSvg);
        }
    });
}

/// Nothing selected: for a creation tool, its defaults for what it creates next ("For new …",
/// which never edit existing layers); then, with every tool, the shown Paper's context
/// ([`paper_context`]).
fn nothing_selected(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    use crate::editor::Tool;
    match ed.tool {
        Tool::Select | Tool::Hand | Tool::Laser => {}
        Tool::Arrow => {
            ui.label(title_text("For new arrows"));
            let routes = [
                (crate::model::Route::Straight, "Straight"),
                (crate::model::Route::Curved, "Curved"),
                (crate::model::Route::Elbow, "Elbow"),
            ];
            row(ui, "Route", |ui| {
                if let Some(v) = combo(ui, "new-route", ed.arrow_route, &routes, 96.0) {
                    ed.arrow_route = v;
                }
            });
            hint(ui, "Drag from one shape to another to connect them.");
            separator(ui);
        }
        Tool::Sticky => {
            ui.label(title_text("For new sticky notes"));
            row(ui, "Color", |ui| {
                if let Some(c) = swatches(ui, Some(ed.sticky_color)) {
                    ed.sticky_color = c;
                }
            });
            separator(ui);
        }
        Tool::Pencil => {
            ui.label(title_text("For new pencil strokes"));
            row(ui, "Width", |ui| {
                ui.add(egui::Slider::new(&mut ed.ink_width, 0.5..=24.0))
            });
            row(ui, "Smoothing", |ui| {
                ui.add(egui::Slider::new(&mut ed.ink_smoothing, 0.0..=1.0))
            });
            row(ui, "Color", |ui| {
                let mut c = c32(ed.ink_color);
                if ui.color_edit_button_srgba(&mut c).changed() {
                    ed.ink_color = from32(c);
                }
            });
            separator(ui);
        }
        Tool::Rect | Tool::Ellipse | Tool::Diamond | Tool::Polygon | Tool::Star | Tool::Line => {
            ui.label(title_text("For new shapes"));
            egui::CollapsingHeader::new(title_text(if ed.sketch.is_some() {
                "Sketch · on"
            } else {
                "Sketch"
            }))
            .id_salt("new-sketch-section")
            .default_open(false)
            .show(ui, |ui| {
                hint(ui, SKETCH_HELP);
                let mut sketch = ed.sketch.is_some();
                if ui.checkbox(&mut sketch, "Hand-drawn style").changed() {
                    ed.toggle_sketch();
                }
            });
            separator(ui);
        }
        Tool::Frame | Tool::Text | Tool::Pen | Tool::Eraser => {
            let what = match ed.tool {
                Tool::Frame => "Drag on the canvas to draw a frame.",
                Tool::Text => "Click on the canvas to type text.",
                Tool::Pen => "Click to place points; Enter finishes the path.",
                _ => "Drag across pencil strokes to erase them.",
            };
            hint(ui, what);
            separator(ui);
        }
    }
    paper_context(ui, ed, st);
}

/// The shown Paper (a page of the Canvas, DESIGN.md) and the view, with nothing selected. Paper:
/// its name and what it holds, which are the document's (a Paper has no fill or size of its own
/// to show); Rename Paper… opens its one rename field, in the Papers list, so a name is changed
/// in one place, once, on its own Paper. View: the canvas's grid, a view preference that is
/// never the Paper's, its undo, its saves or an export.
fn paper_context(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    use crate::render::scene::Background;
    ui.label(title_text("Paper"));
    let name = ed.page_name().to_string();
    let secondary = |t: String| RichText::new(t).size(theme::SECONDARY + 1.0);
    row(ui, "Name", |ui| {
        let label = egui::Label::new(secondary(name.clone())).truncate();
        ui.add(label).on_hover_text(name.clone());
    });
    let count = format!("{} layers", ed.doc.nodes.len());
    row(ui, "Content", |ui| {
        ui.label(secondary(count));
    });
    let place = format!("{} of {}", ed.active_page + 1, ed.pages.len());
    row(ui, "Paper", |ui| {
        ui.label(secondary(place));
    });
    if ui
        .small_button("Rename Paper…")
        .on_hover_text("Rename this Paper in the Papers list")
        .clicked()
    {
        super::start_paper_rename(ed, st);
    }
    separator(ui);
    ui.label(title_text("View"));
    let grids: Vec<(Background, &str)> = Background::ALL.iter().map(|b| (*b, b.label())).collect();
    row(ui, "Grid", |ui| {
        if let Some(b) = combo(ui, "view-grid", st.background, &grids, 96.0) {
            st.set_background(b);
        }
    });
    hint(
        ui,
        "Your view only: not part of the Paper, its undo or its exports.",
    );
}

fn header(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    if ids.len() > 1 {
        ui.label(title_text(format!("{} layers", ids.len())));
        ui.horizontal_wrapped(|ui| {
            for op in [
                BoolOp::Union,
                BoolOp::Subtract,
                BoolOp::Intersect,
                BoolOp::Exclude,
            ] {
                if ui.small_button(op.label()).clicked() {
                    ed.boolean(op);
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.small_button("Group").clicked() {
                ed.group_selection();
            }
            if ui.small_button("Frame").clicked() {
                ed.run(crate::editor::shortcuts::Action::WrapInFrame);
            }
            if ui.small_button("Component").clicked() {
                ed.create_component();
            }
        });
        return;
    }
    let kind = if first.component {
        "Component"
    } else if ed.is_instance_root(first.id) {
        "Instance"
    } else {
        first.kind.type_name()
    };
    ui.label(title_text(format!("{kind} · {}", first.name)));
    ui.horizontal_wrapped(|ui| {
        if first.component {
            if ui.small_button("Create instance").clicked() {
                ed.instance_of_selection();
            }
        } else if ed.is_instance_root(first.id) {
            if ui.small_button("Go to main").clicked() {
                ed.go_to_main();
            }
            if ui.small_button("Detach").clicked() {
                ed.detach_instance();
            }
        } else if (first.is_frame() || first.is_group())
            && ui.small_button("Create component").clicked()
        {
            ed.create_component();
        }
        if first.instance.is_some()
            && first
                .instance
                .as_ref()
                .is_some_and(|l| !l.overrides.is_empty())
            && ui.small_button("Reset overrides").clicked()
        {
            ed.reset_overrides();
        }
        if first.is_container()
            && !first.children.is_empty()
            && ui.small_button("Ungroup").clicked()
        {
            ed.ungroup_selection();
        }
        if !first.is_container()
            && !matches!(first.kind, NodeKind::Path { .. })
            && ui.small_button("Flatten").clicked()
        {
            ed.flatten_selection();
        }
        if matches!(first.kind, NodeKind::Path { .. }) && ui.small_button("Edit points").clicked() {
            ed.start_path_edit(first.id);
        }
    });
    if let Some(l) = &first.instance
        && !l.overrides.is_empty()
    {
        ui.label(
            RichText::new(format!("Overrides: {}", l.overrides.join(", ")))
                .size(theme::SECONDARY)
                .color(theme::pal(ui).secondary),
        );
    }
}

fn align_row(ui: &mut Ui, ed: &mut Editor, _ids: &[NodeId]) {
    use theme::Icon as I;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (icon, axis, to, tip) in [
            (I::AlignLeft, 0, 0.0, "Align left (⌥A)"),
            (I::AlignHCenter, 0, 0.5, "Align horizontal centers (⌥H)"),
            (I::AlignRight, 0, 1.0, "Align right (⌥D)"),
            (I::AlignTop, 1, 0.0, "Align top (⌥W)"),
            (I::AlignVCenter, 1, 0.5, "Align vertical centers (⌥V)"),
            (I::AlignBottom, 1, 1.0, "Align bottom (⌥S)"),
        ] {
            if icon_btn(ui, icon, false, tip) {
                ed.align(axis, to);
            }
        }
        ui.add_space(8.0);
        if icon_btn(
            ui,
            I::DistributeH,
            false,
            "Distribute horizontal spacing (⌥⇧H)",
        ) {
            ed.distribute(0);
        }
        if icon_btn(
            ui,
            I::DistributeV,
            false,
            "Distribute vertical spacing (⌥⇧V)",
        ) {
            ed.distribute(1);
        }
    });
}

/// Position, size and shape parameters. For several layers each value is shown when they all
/// share it (and an edit sets it on all of them) and Mixed when they differ.
fn geometry(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], nodes: &[Node]) {
    let first = &nodes[0];
    ui.horizontal(|ui| {
        match uniform(nodes, |n| n.rect.x) {
            Some(x) => {
                if let Some(v) = num_field(ui, ed, "X  ", x, 1.0, f64::MIN..=f64::MAX, 92.0) {
                    apply(ed, "x", ids, |n| n.rect.x = v);
                }
            }
            None => mixed(ui, "X  ", 92.0),
        }
        match uniform(nodes, |n| n.rect.y) {
            Some(y) => {
                if let Some(v) = num_field(ui, ed, "Y  ", y, 1.0, f64::MIN..=f64::MAX, 92.0) {
                    apply(ed, "y", ids, |n| n.rect.y = v);
                }
            }
            None => mixed(ui, "Y  ", 92.0),
        }
    });
    ui.horizontal(|ui| {
        match uniform(nodes, |n| n.rect.w) {
            Some(w) => {
                if let Some(v) = num_field(ui, ed, "W  ", w, 1.0, 0.0..=1e6, 92.0) {
                    resize(ed, ids, Some(v), None);
                }
            }
            None => mixed(ui, "W  ", 92.0),
        }
        // A text layer's height follows its text.
        let text = nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Text { .. }));
        ui.add_enabled_ui(!text, |ui| match uniform(nodes, |n| n.rect.h) {
            Some(h) => {
                if let Some(v) = num_field(ui, ed, "H  ", h, 1.0, 0.0..=1e6, 92.0) {
                    resize(ed, ids, None, Some(v));
                }
            }
            None => mixed(ui, "H  ", 92.0),
        });
    });
    // Shape parameters, when every selected layer is that shape.
    if nodes
        .iter()
        .all(|n| matches!(n.kind, NodeKind::Polygon { .. }))
    {
        let sides = uniform(nodes, |n| match n.kind {
            NodeKind::Polygon { sides } => sides,
            _ => 0,
        });
        row(ui, "Sides", |ui| match sides {
            Some(sides) => {
                if let Some(v) = num_field(ui, ed, "", sides as f64, 0.1, 3.0..=60.0, 60.0) {
                    apply(ed, "sides", ids, |n| {
                        if let NodeKind::Polygon { sides } = &mut n.kind {
                            *sides = v.round() as u32;
                        }
                    });
                }
            }
            None => mixed(ui, "", 60.0),
        });
    }
    if nodes
        .iter()
        .all(|n| matches!(n.kind, NodeKind::Star { .. }))
    {
        let points = uniform(nodes, |n| match n.kind {
            NodeKind::Star { points, .. } => points,
            _ => 0,
        });
        let inner = uniform(nodes, |n| match n.kind {
            NodeKind::Star { inner, .. } => inner,
            _ => 0.0,
        });
        ui.horizontal(|ui| {
            match points {
                Some(points) => {
                    if let Some(v) =
                        num_field(ui, ed, "Points ", points as f64, 0.1, 3.0..=60.0, 92.0)
                    {
                        apply(ed, "points", ids, |n| {
                            if let NodeKind::Star { points, .. } = &mut n.kind {
                                *points = v.round() as u32;
                            }
                        });
                    }
                }
                None => mixed(ui, "Points ", 92.0),
            }
            match inner {
                Some(inner) => {
                    if let Some(v) =
                        num_field(ui, ed, "Ratio ", inner * 100.0, 0.5, 1.0..=100.0, 92.0)
                    {
                        apply(ed, "ratio", ids, |n| {
                            if let NodeKind::Star { inner, .. } = &mut n.kind {
                                *inner = v / 100.0;
                            }
                        });
                    }
                }
                None => mixed(ui, "Ratio ", 92.0),
            }
        });
    }
    let rounded = |n: &Node| {
        matches!(
            n.kind,
            NodeKind::Rect
                | NodeKind::Frame { .. }
                | NodeKind::Polygon { .. }
                | NodeKind::Star { .. }
        )
    };
    if nodes.iter().all(rounded) {
        let per_corner = uniform(nodes, |n| n.style.radii.is_some());
        ui.horizontal(|ui| {
            match per_corner {
                Some(false) => match uniform(nodes, |n| n.style.corner_radius) {
                    Some(radius) => {
                        if let Some(v) = num_field(ui, ed, "Radius  ", radius, 0.5, 0.0..=1e4, 92.0)
                        {
                            apply(ed, "radius", ids, |n| n.style.corner_radius = v);
                        }
                    }
                    None => mixed(ui, "Radius  ", 92.0),
                },
                // Independent corners (on some or all of the layers): the corners every layer
                // has, compared corner by corner; Mixed only when they really differ.
                _ => match uniform(nodes, |n| n.style.corner_radii()) {
                    Some(rr) => {
                        for (i, p) in ["TL", "TR", "BR", "BL"].iter().enumerate() {
                            if let Some(v) =
                                num_field(ui, ed, &format!("{p} "), rr[i], 0.5, 0.0..=1e4, 46.0)
                            {
                                apply(ed, "radius", ids, |n| {
                                    let mut r = n.style.corner_radii();
                                    r[i] = v;
                                    n.style.radii = Some(r);
                                });
                            }
                        }
                    }
                    None => mixed(ui, "Radius  ", 92.0),
                },
            }
            if ids.len() == 1
                && matches!(first.kind, NodeKind::Rect | NodeKind::Frame { .. })
                && icon_btn(
                    ui,
                    theme::Icon::Corners,
                    per_corner == Some(true),
                    "Independent corners",
                )
            {
                apply(ed, "radius", ids, |n| {
                    n.style.radii = if n.style.radii.is_some() {
                        None
                    } else {
                        Some([n.style.corner_radius; 4])
                    }
                });
            }
        });
    }
    if nodes
        .iter()
        .all(|n| matches!(n.kind, NodeKind::Frame { .. }))
    {
        match uniform(nodes, |n| matches!(n.kind, NodeKind::Frame { clip: true })) {
            Some(clip) => {
                let mut c = clip;
                if ui.checkbox(&mut c, "Clip content").changed() {
                    apply(ed, "clip", ids, |n| n.kind = NodeKind::Frame { clip: c });
                }
            }
            None => hint(ui, "Clip content: Mixed"),
        }
    }
}

/// Resize through the layout engine so constraints and auto layout follow: the width and/or
/// height of every layer in `ids`, as one coalesced change.
fn resize(ed: &mut Editor, ids: &[NodeId], w: Option<f64>, h: Option<f64>) {
    let key = format!("size:{ids:?}");
    ed.begin_coalescing("Resize", Some(&key));
    for id in ids.iter().copied() {
        let mut tx = ed.tx();
        let tx = &mut tx;
        let Some(old) = tx.doc().get(id).map(|n| n.rect) else {
            continue;
        };
        let new = crate::geom::Rect::new(
            old.x,
            old.y,
            w.unwrap_or(old.w).max(0.0),
            h.unwrap_or(old.h).max(0.0),
        );
        let n = tx.node_mut(id).unwrap();
        n.set_rect_scaling(new);
        if (new.w - old.w).abs() > 1e-9 {
            n.layout_child.sizing[0] = Sizing::Fixed;
        }
        if (new.h - old.h).abs() > 1e-9 {
            n.layout_child.sizing[1] = Sizing::Fixed;
        }
        crate::layout::resize_children(tx, id, old, new);
    }
    ed.mark_overrides(ids, "size");
    for id in ids {
        ed.fit_text(*id);
    }
    ed.commit();
}

fn child_layout(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    let parent = first.parent.and_then(|p| ed.doc.get(p)).cloned();
    let in_auto = parent.as_ref().is_some_and(|p| p.layout.is_some());
    let sizing_opts_for = |axis: usize| -> Vec<(Sizing, &'static str)> {
        let mut v = vec![(Sizing::Fixed, "Fixed")];
        if first.layout.is_some() || matches!(first.kind, NodeKind::Text { .. }) {
            v.push((Sizing::Hug, "Hug"));
        }
        if in_auto && !first.layout_child.absolute {
            v.push((Sizing::Fill, "Fill"));
        }
        let _ = axis;
        v
    };
    let show_sizing = first.layout.is_some() || (in_auto && !first.layout_child.absolute);
    if show_sizing {
        ui.horizontal(|ui| {
            for (axis, label) in [(0usize, "W"), (1, "H")] {
                ui.label(RichText::new(label).color(theme::pal(ui).secondary));
                let opts = sizing_opts_for(axis);
                if let Some(v) = combo(
                    ui,
                    &format!("sizing{axis}"),
                    first.layout_child.sizing[axis],
                    &opts,
                    66.0,
                ) {
                    apply(ed, "layout", ids, |n| {
                        n.layout_child.sizing[axis] = v;
                        if let NodeKind::Text { auto_width, .. } = &mut n.kind
                            && axis == 0
                        {
                            *auto_width = v == Sizing::Hug;
                        }
                    });
                }
            }
        });
    }
    if in_auto {
        let mut abs = first.layout_child.absolute;
        if ui.checkbox(&mut abs, "Ignore auto layout").changed() {
            apply(ed, "layout", ids, |n| n.layout_child.absolute = abs);
        }
    }
    let constrained =
        parent.as_ref().is_some_and(|p| p.is_frame()) && (!in_auto || first.layout_child.absolute);
    if constrained {
        let opts_h = [
            (Constraint::Min, "Left"),
            (Constraint::Max, "Right"),
            (Constraint::Stretch, "Left & right"),
            (Constraint::Center, "Center"),
            (Constraint::Scale, "Scale"),
        ];
        let opts_v = [
            (Constraint::Min, "Top"),
            (Constraint::Max, "Bottom"),
            (Constraint::Stretch, "Top & bottom"),
            (Constraint::Center, "Center"),
            (Constraint::Scale, "Scale"),
        ];
        row(ui, "Constraints", |ui| {
            if let Some(v) = combo(ui, "con-h", first.layout_child.constraints.h, &opts_h, 74.0) {
                apply(ed, "layout", ids, |n| n.layout_child.constraints.h = v);
            }
            if let Some(v) = combo(ui, "con-v", first.layout_child.constraints.v, &opts_v, 74.0) {
                apply(ed, "layout", ids, |n| n.layout_child.constraints.v = v);
            }
        });
    }
}

fn auto_layout(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    separator(ui);
    ui.horizontal(|ui| {
        ui.label(title_text("Auto layout"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if first.layout.is_some() {
                if ui
                    .small_button("−")
                    .on_hover_text("Remove auto layout (⌥⇧A)")
                    .clicked()
                {
                    ed.remove_auto_layout();
                }
            } else if ui
                .small_button("+")
                .on_hover_text("Add auto layout (⇧A)")
                .clicked()
            {
                ed.add_auto_layout();
            }
        });
    });
    let Some(l) = first.layout else { return };
    let set = |ed: &mut Editor, f: &dyn Fn(&mut AutoLayout)| {
        apply(ed, "layout", ids, |n| {
            if let Some(l) = &mut n.layout {
                f(l);
            }
        });
    };
    ui.horizontal(|ui| {
        if icon_btn(
            ui,
            theme::Icon::ArrowRight,
            l.direction == Direction::Horizontal,
            "Horizontal",
        ) {
            set(ed, &|l| l.direction = Direction::Horizontal);
        }
        if icon_btn(
            ui,
            theme::Icon::ArrowDown,
            l.direction == Direction::Vertical,
            "Vertical",
        ) {
            set(ed, &|l| l.direction = Direction::Vertical);
        }
        if icon_btn(ui, theme::Icon::Wrap, l.wrap, "Wrap") {
            set(ed, &|l| l.wrap = !l.wrap);
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(!l.gap_auto, |ui| {
            if let Some(v) = num_field(ui, ed, "Gap ", l.gap, 0.5, -1e4..=1e4, 70.0) {
                set(ed, &|l| l.gap = v);
            }
        });
        if icon_btn(ui, theme::Icon::SpaceBetween, l.gap_auto, "Space between") {
            set(ed, &|l| l.gap_auto = !l.gap_auto);
        }
    });
    ui.horizontal(|ui| {
        for (i, p) in ["T ", "R ", "B ", "L "].iter().enumerate() {
            if let Some(v) = num_field(ui, ed, p, l.padding[i], 0.5, 0.0..=1e4, 44.0) {
                set(ed, &|l| l.padding[i] = v);
            }
        }
    });
    let opts = [
        (Align::Start, "Start"),
        (Align::Center, "Center"),
        (Align::End, "End"),
    ];
    row(ui, "Align", |ui| {
        if let Some(v) = combo(ui, "al-main", l.align_main, &opts, 64.0) {
            set(ed, &|l| l.align_main = v);
        }
        if let Some(v) = combo(ui, "al-cross", l.align_cross, &opts, 64.0) {
            set(ed, &|l| l.align_cross = v);
        }
    });
}

/// Typography. One text layer shows the style at the caret (or of the whole layer); several
/// show each value they share and Mixed for the rest, and the toggles that can't show Mixed
/// (bold, italic) are left out while they differ. Underline and strikethrough live in text
/// ranges, so they are offered for one layer at a time.
fn text_section(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], nodes: &[Node]) {
    let first = &nodes[0];
    let one = nodes.len() == 1;
    let editing_range = ed.text_selection().is_some();
    ui.horizontal(|ui| {
        ui.label(title_text("Text"));
        if editing_range {
            ui.label(
                RichText::new("selection")
                    .size(theme::SECONDARY)
                    .color(theme::pal(ui).secondary),
            );
        }
    });
    let cs = ed.current_char_style();
    // (size, colour, bold, italic) as one layer shows them, or as every character of the
    // selected layers has them, their text ranges included (the same styles the layout draws):
    // equal base sizes with a range of another size are Mixed, never a shared size.
    let (size, color, bold, italic) = if one {
        let font_size = first.kind.font_size().unwrap_or(16.0);
        (
            Some(cs.map_or(font_size, |c| c.size)),
            Some(cs.map(|c| c.color).unwrap_or(Color::INK)),
            Some(cs.is_some_and(|c| c.bold)),
            Some(cs.is_some_and(|c| c.italic)),
        )
    } else {
        let styles: Vec<_> = nodes.iter().map(char_styles).collect();
        (
            shared_char(&styles, |c| c.size),
            shared_char(&styles, |c| c.color),
            shared_char(&styles, |c| c.bold),
            shared_char(&styles, |c| c.italic),
        )
    };
    ui.horizontal(|ui| {
        // Typing a size or dragging it (or the colour) is one undo step per interaction.
        match size {
            Some(size) => {
                if let Some(v) = num_field(ui, ed, "Size ", size, 0.5, 1.0..=1000.0, 80.0) {
                    ed.style_text_coalesced("font size", |s| s.size = Some(v));
                }
            }
            None => mixed(ui, "Size ", 80.0),
        }
        match color {
            Some(c) => {
                if let Some(nc) = color_button(ui, ed, c) {
                    ed.style_text_coalesced("color", |s| s.color = Some(nc));
                }
            }
            None => mixed(ui, "", 44.0),
        }
        ui.add_space(4.0);
        if let Some(b) = bold
            && icon_toggle(ui, "B", b, "Bold (⌘B)")
        {
            ed.style_text("bold", |s| s.bold = Some(!b));
        }
        if let Some(i) = italic
            && icon_toggle(ui, "I", i, "Italic (⌘I)")
        {
            ed.style_text("italic", |s| s.italic = Some(!i));
        }
        if one {
            if icon_toggle(ui, "U", cs.is_some_and(|c| c.underline), "Underline (⌘U)") {
                ed.toggle_underline();
            }
            if icon_toggle(ui, "S", cs.is_some_and(|c| c.strike), "Strikethrough (⌘⇧X)") {
                ed.toggle_strike();
            }
        }
    });
    if !one && (bold.is_none() || italic.is_none()) {
        hint(
            ui,
            "Bold or italic differ across the selection: set them per layer.",
        );
    }
    if !one && (size.is_none() || color.is_none()) {
        hint(
            ui,
            "Size or colour differs within or across these layers: set it per layer or range.",
        );
    }
    let align = uniform(nodes, |n| match n.kind {
        NodeKind::Text { align, .. } => align,
        _ => TextAlign::Left,
    });
    ui.horizontal(|ui| {
        // No alignment is marked when the layers differ; choosing one sets it on all.
        for (a, ic) in [
            (TextAlign::Left, theme::Icon::AlignLeft),
            (TextAlign::Center, theme::Icon::AlignHCenter),
            (TextAlign::Right, theme::Icon::AlignRight),
        ] {
            if icon_btn(ui, ic, align == Some(a), "Text alignment") {
                apply(ed, "align", ids, |n| {
                    if let NodeKind::Text { align, .. } = &mut n.kind {
                        *align = a;
                    }
                });
            }
        }
        ui.add_space(6.0);
        match uniform(nodes, |n| n.text_style.line_height.unwrap_or(0.0)) {
            Some(lh) => {
                if let Some(v) = num_field(ui, ed, "LH ", lh * 100.0, 1.0, 0.0..=1000.0, 64.0) {
                    apply(ed, "line height", ids, |n| {
                        n.text_style.line_height = (v > 0.0).then_some(v / 100.0)
                    });
                }
            }
            None => mixed(ui, "LH ", 64.0),
        }
        match uniform(nodes, |n| n.text_style.letter_spacing) {
            Some(ls) => {
                if let Some(v) = num_field(ui, ed, "LS ", ls, 0.1, -50.0..=200.0, 60.0) {
                    apply(ed, "letter spacing", ids, |n| {
                        n.text_style.letter_spacing = v
                    });
                }
            }
            None => mixed(ui, "LS ", 60.0),
        }
    });
    if ids.len() == 1 && ed.text_edit.is_none() {
        let NodeKind::Text { text, .. } = &first.kind else {
            return;
        };
        let mut t = text.clone();
        let r = ui.add(
            egui::TextEdit::multiline(&mut t)
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if r.changed() {
            apply(ed, "text", ids, |n| {
                if let NodeKind::Text { text, .. } = &mut n.kind {
                    *text = t.clone();
                }
                n.text_style.spans.retain(|s| s.end <= t.chars().count());
            });
        }
        seal_on_release(ed, &r);
    }
}

/// The default solid fill a new fill layer starts with.
const NEW_FILL: Color = Color::rgb(0xd9, 0xd9, 0xd9);

fn fills(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], nodes: &[Node]) {
    let Some(fills) = uniform(nodes, |n| n.style.fills.clone()) else {
        // Different fills can't be shown as one list: say so, and offer one fill for all.
        ui.horizontal(|ui| {
            ui.label(title_text("Fill"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("Use one fill")
                    .on_hover_text("Replace every selected layer's fills with one solid fill")
                    .clicked()
                {
                    apply(ed, "fills", ids, |n| {
                        n.style.fills = vec![Paint::solid(NEW_FILL)]
                    });
                }
            });
        });
        hint(ui, "Mixed: the selected layers have different fills.");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(title_text("Fill"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+").on_hover_text("Add fill").clicked() {
                let c = fills.last().map_or(NEW_FILL, |p| p.preview_color());
                apply(ed, "fills", ids, |n| n.style.fills.push(Paint::solid(c)));
            }
        });
    });
    // Top fill first, as in Figma.
    for i in (0..fills.len()).rev() {
        let p = fills[i].clone();
        ui.push_id(i, |ui| fill_row(ui, ed, ids, i, &p));
    }
}

fn fill_row(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], i: usize, p: &Paint) {
    let set = |ed: &mut Editor, np: Paint| {
        apply(ed, "fills", ids, |n| {
            if let Some(slot) = n.style.fills.get_mut(i) {
                *slot = np.clone();
            }
        });
    };
    ui.horizontal(|ui| {
        let kinds = [("Solid", 0u8), ("Linear", 1), ("Radial", 2), ("Image", 3)];
        let cur = match p {
            Paint::Solid { .. } => 0,
            Paint::Linear { .. } => 1,
            Paint::Radial { .. } => 2,
            Paint::Image { .. } => 3,
        };
        let opts: Vec<(u8, &str)> = kinds.iter().map(|(l, k)| (*k, *l)).collect();
        if let Some(k) = combo(ui, "kind", cur, &opts, 62.0) {
            let c = p.preview_color();
            let np = match k {
                0 => Paint::solid(c),
                1 => Paint::linear(c, Color::rgba(c.r, c.g, c.b, 0)),
                2 => Paint::radial(c, Color::rgba(c.r, c.g, c.b, 0)),
                _ => {
                    ed.requests.push(AppRequest::ImageFill(ids.to_vec(), i));
                    return;
                }
            };
            set(ed, np);
            return;
        }
        match p {
            Paint::Solid { color } => {
                if let Some(c) = color_button(ui, ed, *color) {
                    set(ed, Paint::solid(c));
                }
                ui.label(RichText::new(color.to_hex()).monospace().weak());
            }
            Paint::Image {
                mode,
                opacity,
                asset,
            } => {
                let modes = [
                    (ImageMode::Fill, "Fill"),
                    (ImageMode::Fit, "Fit"),
                    (ImageMode::Stretch, "Stretch"),
                    (ImageMode::Tile, "Tile"),
                ];
                if let Some(m) = combo(ui, "mode", *mode, &modes, 62.0) {
                    set(
                        ed,
                        Paint::Image {
                            asset: asset.clone(),
                            mode: m,
                            opacity: *opacity,
                        },
                    );
                }
                if ui.small_button("Replace…").clicked() {
                    ed.requests.push(AppRequest::ImageFill(ids.to_vec(), i));
                }
            }
            _ => {}
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("−").on_hover_text("Remove fill").clicked() {
                apply(ed, "fills", ids, |n| {
                    if i < n.style.fills.len() {
                        n.style.fills.remove(i);
                    }
                });
            }
        });
    });
    // Gradient stops and geometry.
    if let Paint::Linear { stops, from, to }
    | Paint::Radial {
        stops,
        center: from,
        radius: to,
    } = p
    {
        let linear = matches!(p, Paint::Linear { .. });
        let rebuild = |stops: Vec<Stop>, a: crate::geom::V2, b: crate::geom::V2| {
            if linear {
                Paint::Linear {
                    stops,
                    from: a,
                    to: b,
                }
            } else {
                Paint::Radial {
                    stops,
                    center: a,
                    radius: b,
                }
            }
        };
        for (si, s) in stops.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(LABEL_W * 0.5);
                if let Some(c) = color_button(ui, ed, s.color) {
                    let mut ns = stops.clone();
                    ns[si].color = c;
                    set(ed, rebuild(ns, *from, *to));
                }
                if let Some(v) = num_field(ui, ed, "", s.pos * 100.0, 0.5, 0.0..=100.0, 56.0) {
                    let mut ns = stops.clone();
                    ns[si].pos = v / 100.0;
                    set(ed, rebuild(ns, *from, *to));
                }
                ui.label(RichText::new("%").weak());
                if stops.len() > 2 && ui.small_button("−").clicked() {
                    let mut ns = stops.clone();
                    ns.remove(si);
                    set(ed, rebuild(ns, *from, *to));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W * 0.5);
            if ui.small_button("+ stop").clicked() {
                let mut ns = stops.clone();
                let c = ns.last().map_or(Color::BLACK, |s| s.color);
                ns.push(Stop { pos: 0.5, color: c });
                ns.sort_by(|a, b| a.pos.total_cmp(&b.pos));
                set(ed, rebuild(ns, *from, *to));
            }
            if linear {
                let d = *to - *from;
                let ang = d.y.atan2(d.x).to_degrees();
                if let Some(v) = num_field(ui, ed, "Angle ", ang, 1.0, -360.0..=360.0, 70.0) {
                    let a = v.to_radians();
                    let half = v2(a.cos(), a.sin()) * 0.5;
                    let c = v2(0.5, 0.5);
                    set(ed, rebuild(stops.clone(), c - half, c + half));
                }
            } else if let Some(v) = num_field(ui, ed, "R ", to.x * 100.0, 0.5, 1.0..=400.0, 70.0) {
                set(ed, rebuild(stops.clone(), *from, v2(v / 100.0, v / 100.0)));
            }
        });
    }
}

fn stroke(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], nodes: &[Node]) {
    let stroke_of = |n: &Node| {
        let s = &n.style;
        (
            s.stroke,
            s.stroke_width,
            s.stroke_align,
            s.dash.clone(),
            s.cap,
            s.join,
        )
    };
    if uniform(nodes, stroke_of).is_none() {
        // Different strokes: one control can't show them; offer one stroke for all.
        ui.horizontal(|ui| {
            ui.label(title_text("Stroke"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("Use one stroke")
                    .on_hover_text("Give every selected layer the same plain 1 px stroke")
                    .clicked()
                {
                    apply(ed, "stroke", ids, |n| {
                        n.style.stroke = Some(Color::INK);
                        n.style.stroke_width = 1.0;
                        n.style.stroke_align = StrokeAlign::Center;
                        n.style.dash = Vec::new();
                        n.style.cap = Cap::Butt;
                        n.style.join = Join::Miter;
                    });
                }
            });
        });
        hint(ui, "Mixed: the selected layers have different strokes.");
        return;
    }
    let s = &nodes[0].style;
    ui.horizontal(|ui| {
        ui.label(title_text("Stroke"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if s.stroke.is_some() {
                if ui.small_button("−").clicked() {
                    apply(ed, "stroke", ids, |n| n.style.stroke = None);
                }
            } else if ui.small_button("+").clicked() {
                apply(ed, "stroke", ids, |n| n.style.stroke = Some(Color::INK));
            }
        });
    });
    let Some(c) = s.stroke else { return };
    ui.horizontal(|ui| {
        if let Some(nc) = color_button(ui, ed, c) {
            apply(ed, "stroke", ids, |n| n.style.stroke = Some(nc));
        }
        if let Some(v) = num_field(ui, ed, "W ", s.stroke_width, 0.1, 0.0..=500.0, 64.0) {
            apply(ed, "stroke width", ids, |n| n.style.stroke_width = v);
        }
        let al = [
            (StrokeAlign::Inside, "Inside"),
            (StrokeAlign::Center, "Center"),
            (StrokeAlign::Outside, "Outside"),
        ];
        if let Some(v) = combo(ui, "salign", s.stroke_align, &al, 64.0) {
            apply(ed, "stroke align", ids, |n| n.style.stroke_align = v);
        }
    });
    ui.horizontal(|ui| {
        let dash_kind = match s.dash.as_slice() {
            [] => 0u8,
            [a, _] if *a <= 2.0 => 2,
            _ => 1,
        };
        let dashes = [(0u8, "Solid"), (1, "Dashed"), (2, "Dotted")];
        if let Some(k) = combo(ui, "dash", dash_kind, &dashes, 64.0) {
            let w = s.stroke_width.max(1.0);
            apply(ed, "dash", ids, |n| {
                n.style.dash = match k {
                    0 => vec![],
                    1 => vec![w * 4.0, w * 3.0],
                    _ => vec![w.min(2.0), w * 2.5],
                };
                if k == 2 {
                    n.style.cap = Cap::Round;
                }
            });
        }
        let caps = [
            (Cap::Butt, "Butt"),
            (Cap::Round, "Round"),
            (Cap::Square, "Square"),
        ];
        if let Some(v) = combo(ui, "cap", s.cap, &caps, 60.0) {
            apply(ed, "cap", ids, |n| n.style.cap = v);
        }
        let joins = [
            (Join::Miter, "Miter"),
            (Join::Round, "Round"),
            (Join::Bevel, "Bevel"),
        ];
        if let Some(v) = combo(ui, "join", s.join, &joins, 60.0) {
            apply(ed, "join", ids, |n| n.style.join = v);
        }
    });
}

fn layer(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], nodes: &[Node]) {
    ui.label(title_text("Layer"));
    ui.horizontal(|ui| {
        let opts: Vec<(BlendMode, &str)> = BlendMode::ALL.iter().map(|b| (*b, b.label())).collect();
        let blend = uniform(nodes, |n| n.style.blend);
        if let Some(v) = combo_mixed(ui, "blend", blend, &opts, 96.0) {
            apply(ed, "blend", ids, |n| n.style.blend = v);
        }
        match uniform(nodes, |n| n.style.opacity) {
            Some(opacity) => {
                let mut op = opacity * 100.0;
                let r = ui.add_sized(
                    egui::vec2(70.0, 20.0),
                    egui::DragValue::new(&mut op)
                        .range(0.0..=100.0)
                        .suffix("%")
                        .speed(0.5),
                );
                if r.changed() {
                    apply(ed, "opacity", ids, |n| n.style.opacity = op / 100.0);
                }
                seal_on_release(ed, &r);
            }
            None => mixed(ui, "", 70.0),
        }
    });
}
