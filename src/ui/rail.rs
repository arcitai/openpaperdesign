//! The tool rail (DESIGN.md, revision 2 slice 2): one slim vertical rail at the canvas's left
//! edge with six families that hold all sixteen tools. A family's main button runs its current
//! member; its own chevron (a separate, always visible target) opens a flyout of the family's
//! tools. Choosing a tool, from the flyout or by its shortcut, makes it the family's member.
//! Escape closes an open flyout first (the hosts hold the key back from the canvas then, see
//! [`Rail::captures_input`]); Escape outside it is the editor's own (back to Select), and every
//! family keeps its member.

use crate::editor::shortcuts::{self, Action};
use crate::editor::{Editor, Tool};
use crate::theme::{self, Icon};
use egui::{Align2, Color32, Rect, pos2, vec2};

/// The six families, in rail order, with their tools (the first is the initial member).
pub const FAMILIES: [(&str, &[Tool]); 6] = [
    ("Pointer", &[Tool::Select, Tool::Hand, Tool::Laser]),
    ("Frame", &[Tool::Frame]),
    (
        "Shapes",
        &[
            Tool::Rect,
            Tool::Ellipse,
            Tool::Diamond,
            Tool::Polygon,
            Tool::Star,
        ],
    ),
    ("Lines", &[Tool::Arrow, Tool::Line]),
    ("Text", &[Tool::Text, Tool::Sticky]),
    ("Draw", &[Tool::Pencil, Tool::Pen, Tool::Eraser]),
];

/// Rail geometry, in points (see DESIGN.md: the 52 px of the plan are widened to 64 so the
/// chevron gets a 24 × 32 px target of its own, apart from the 32 × 32 px main button).
pub const ROW_H: f32 = 32.0;
pub const ROW_GAP: f32 = 2.0;
pub const PAD: f32 = 4.0;
pub const MAIN_W: f32 = 32.0;
pub const CHEVRON_W: f32 = 24.0;
pub const RAIL_W: f32 = MAIN_W + CHEVRON_W + 2.0 * PAD;
/// Distance from the canvas's edges.
pub const INSET: f32 = 8.0;
/// Room kept free under the rail for the zoom controls.
pub const BOTTOM_RESERVE: f32 = 48.0;
/// Below this canvas height the rail is anchored to the top and the zoom sits beside it.
pub const SHORT_H: f32 = 270.0;
/// The flyout's preferred width, its narrowest usable width beside the rail, and its row.
pub const FLYOUT_W: f32 = 220.0;
pub const FLYOUT_MIN: f32 = 180.0;
pub const FLYOUT_ROW_H: f32 = 32.0;

/// The rail's height: six rows, their gaps and its padding.
pub fn rail_height() -> f32 {
    FAMILIES.len() as f32 * ROW_H + (FAMILIES.len() - 1) as f32 * ROW_GAP + 2.0 * PAD
}

/// Which family `t` belongs to.
pub fn family_of(t: Tool) -> usize {
    FAMILIES
        .iter()
        .position(|(_, tools)| tools.contains(&t))
        .unwrap_or(0)
}

pub fn tool_icon(t: Tool) -> Icon {
    match t {
        Tool::Select => Icon::Pointer,
        Tool::Hand => Icon::Hand,
        Tool::Frame => Icon::Frame,
        Tool::Rect => Icon::Rect,
        Tool::Ellipse => Icon::Ellipse,
        Tool::Polygon => Icon::Polygon,
        Tool::Diamond => Icon::Diamond,
        Tool::Arrow => Icon::Arrow,
        Tool::Sticky => Icon::Sticky,
        Tool::Laser => Icon::Laser,
        Tool::Star => Icon::Star,
        Tool::Pen => Icon::Pen,
        Tool::Line => Icon::Line,
        Tool::Text => Icon::Text,
        Tool::Pencil => Icon::Pencil,
        Tool::Eraser => Icon::Eraser,
    }
}

/// The tool's name and its actual shortcut (from the shortcut table, not a made-up label).
pub fn tool_tip(t: Tool) -> String {
    let keys = shortcuts::chord_text(Action::Tool(t));
    if keys.is_empty() {
        t.label().to_string()
    } else {
        format!("{}  {keys}", t.label())
    }
}

/// Where the rail and its parts were laid out in the last frame (no behaviour; for hosts'
/// checks and tests, like `PaletteUi::search_id`).
#[derive(Debug, Clone, Default)]
pub struct RailRects {
    pub rail: Option<Rect>,
    /// Per family: the main button and, for families with several tools, the chevron.
    pub rows: Vec<(Rect, Option<Rect>)>,
    pub flyout: Option<Rect>,
    pub flyout_rows: Vec<(Tool, Rect)>,
    /// The rail control that had egui's keyboard focus (reached with Tab): its widget id, and
    /// its tool when it is a flyout row. Enter or Space chooses it, as egui does, so the hosts
    /// keep those keys from the editor while egui's focus is still on it.
    pub focus: Option<(egui::Id, Option<Tool>)>,
}

#[derive(Debug)]
pub struct Rail {
    /// Each family's current member.
    pub members: [Tool; 6],
    /// The family whose flyout is open.
    pub flyout: Option<usize>,
    /// Layout of the last frame.
    pub rects: RailRects,
}

impl Default for Rail {
    fn default() -> Self {
        Rail {
            members: FAMILIES.map(|(_, tools)| tools[0]),
            flyout: None,
            rects: RailRects::default(),
        }
    }
}

impl Rail {
    /// A flyout is open: the next Escape or press belongs to it, not to the canvas (the hosts
    /// hold them back from the editor; the flyout closes on them).
    pub fn captures_input(&self) -> bool {
        self.flyout.is_some()
    }

    pub fn close(&mut self) {
        self.flyout = None;
    }

    /// Whether `key` is the focused rail control's own (see [`RailRects::focus`]): Enter or
    /// Space while egui's keyboard focus (`focused`) is on that control.
    pub fn takes_key(&self, key: egui::Key, focused: Option<egui::Id>) -> bool {
        matches!(key, egui::Key::Enter | egui::Key::Space)
            && self.rects.focus.is_some_and(|(id, _)| focused == Some(id))
    }

    /// Follow the editor's tool (a shortcut, the palette, a menu): it becomes its family's
    /// member.
    pub fn follow(&mut self, t: Tool) {
        self.members[family_of(t)] = t;
    }
}

/// The rail's rectangle for a canvas: at its left edge, vertically centred in the room the
/// zoom reservation leaves, or anchored to the top when the canvas is short.
pub fn rail_rect(canvas: Rect) -> Rect {
    let h = rail_height();
    let x = canvas.left() + INSET;
    let top = canvas.top() + INSET;
    let short = canvas.height() < SHORT_H;
    let y = if short {
        top
    } else {
        let room_bottom = canvas.bottom() - BOTTOM_RESERVE;
        let centred = (canvas.top() + room_bottom - h) / 2.0;
        centred.max(top)
    };
    Rect::from_min_size(pos2(x, y), vec2(RAIL_W, h))
}

/// The flyout of family `f` (with `rows` tools) for its rail row at `row`: inward from the rail
/// (at rail.right + 4) when that leaves the preferred width or at least `FLYOUT_MIN`, otherwise
/// over the rail; never past the canvas's inset, its height clamped (it scrolls then).
pub fn flyout_rect(canvas: Rect, rail: Rect, row: Rect, rows: usize) -> Rect {
    let right = canvas.right() - INSET;
    let beside = rail.right() + 4.0;
    let x = if right - beside >= FLYOUT_MIN {
        beside
    } else {
        canvas.left() + INSET
    };
    let w = FLYOUT_W.min(right - x).max(0.0);
    let want_h = rows as f32 * FLYOUT_ROW_H + 2.0 * PAD;
    let max_h = (canvas.height() - 2.0 * INSET).max(FLYOUT_ROW_H);
    let h = want_h.min(max_h);
    let bottom = canvas.bottom() - INSET;
    let y = row.top().min(bottom - h).max(canvas.top() + INSET);
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

/// Draw the rail (and the open flyout) over `canvas`.
pub fn draw(ctx: &egui::Context, ed: &mut Editor, rail: &mut Rail, canvas: Rect) {
    rail.follow(ed.tool);
    let rr = rail_rect(canvas);
    rail.rects = RailRects {
        rail: Some(rr),
        ..Default::default()
    };
    let current = family_of(ed.tool);
    let mut open_family: Option<usize> = rail.flyout;
    let mut rows = Vec::new();
    let mut focus = None;
    egui::Area::new("tool-rail".into())
        .order(egui::Order::Middle)
        .fixed_pos(rr.min)
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(10.0)
                .inner_margin(PAD)
                .show(ui, |ui| {
                    ui.set_width(RAIL_W - 2.0 * PAD);
                    ui.spacing_mut().item_spacing = vec2(0.0, ROW_GAP);
                    for (fi, (name, tools)) in FAMILIES.iter().enumerate() {
                        let member = rail.members[fi];
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let main =
                                theme::icon_button(ui, tool_icon(member), MAIN_W, fi == current)
                                    .on_hover_text(tool_tip(member));
                            if main.has_focus() {
                                focus = Some((main.id, None));
                            }
                            if main.clicked() {
                                ed.run(Action::Tool(member));
                                open_family = None;
                            }
                            let chevron = if tools.len() > 1 {
                                let (r, resp) = ui.allocate_exact_size(
                                    vec2(CHEVRON_W, ROW_H),
                                    egui::Sense::click(),
                                );
                                let open = open_family == Some(fi);
                                let fill = if open {
                                    theme::selected_fill(ui)
                                } else if resp.hovered() {
                                    p.hover
                                } else {
                                    Color32::TRANSPARENT
                                };
                                ui.painter().rect_filled(r, 6.0, fill);
                                let glyph = Rect::from_center_size(r.center(), vec2(10.0, 10.0));
                                theme::paint_icon(
                                    ui.painter(),
                                    glyph,
                                    Icon::ChevronRight,
                                    if open || resp.hovered() {
                                        p.text
                                    } else {
                                        p.secondary
                                    },
                                );
                                if resp.has_focus() {
                                    focus = Some((resp.id, None));
                                }
                                if resp.on_hover_text(format!("{name} tools")).clicked() {
                                    open_family = if open { None } else { Some(fi) };
                                }
                                Some(r)
                            } else {
                                None
                            };
                            rows.push((main.rect, chevron));
                        });
                    }
                });
        });
    rail.rects.rows = rows;
    rail.rects.focus = focus;
    rail.flyout = open_family;
    let Some(fi) = rail.flyout else { return };
    let (_, tools) = FAMILIES[fi];
    let row = rail.rects.rows[fi].0;
    let fr = flyout_rect(canvas, rr, row, tools.len());
    rail.rects.flyout = Some(fr);
    let mut chosen = None;
    let mut flyout_rows = Vec::new();
    let resp = egui::Area::new("tool-flyout".into())
        .order(egui::Order::Foreground)
        .fixed_pos(fr.min)
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(10.0)
                .inner_margin(PAD)
                .show(ui, |ui| {
                    let inner_w = (fr.width() - 2.0 * PAD).max(0.0);
                    ui.set_width(inner_w);
                    // The rows' viewport is exactly the flyout's room. One Area serves every
                    // family, and egui lays an Area's content out in the size it had last
                    // frame: without the minimum, a family opened after a shorter one (Shapes
                    // after Pointer) would show only that many rows and scroll the rest away.
                    let view_h = (fr.height() - 2.0 * PAD).max(FLYOUT_ROW_H);
                    egui::ScrollArea::vertical()
                        .id_salt(("tool-flyout-rows", fi))
                        .max_height(view_h)
                        .min_scrolled_height(view_h)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for t in tools {
                                let (r, resp) = ui.allocate_exact_size(
                                    vec2(inner_w, FLYOUT_ROW_H),
                                    egui::Sense::click(),
                                );
                                let on = ed.tool == *t;
                                let fill = if on {
                                    theme::selected_fill(ui)
                                } else if resp.hovered() || resp.has_focus() {
                                    p.hover
                                } else {
                                    Color32::TRANSPARENT
                                };
                                ui.painter().rect_filled(r, 6.0, fill);
                                if resp.has_focus() {
                                    ui.painter().rect_stroke(
                                        r.shrink(1.0),
                                        6.0,
                                        egui::Stroke::new(1.5, theme::accent()),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                let icon = Rect::from_center_size(
                                    pos2(r.left() + 16.0, r.center().y),
                                    vec2(18.0, 18.0),
                                );
                                theme::paint_icon(ui.painter(), icon, tool_icon(*t), p.text);
                                ui.painter().text(
                                    pos2(r.left() + 34.0, r.center().y),
                                    Align2::LEFT_CENTER,
                                    t.label(),
                                    egui::FontId::proportional(theme::BODY),
                                    p.text,
                                );
                                let keys = shortcuts::chord_text(Action::Tool(*t));
                                if !keys.is_empty() {
                                    ui.painter().text(
                                        pos2(r.right() - 10.0, r.center().y),
                                        Align2::RIGHT_CENTER,
                                        keys,
                                        theme::medium(theme::SECONDARY),
                                        p.secondary,
                                    );
                                }
                                if resp.clicked() {
                                    chosen = Some(*t);
                                }
                                if resp.has_focus() {
                                    focus = Some((resp.id, Some(*t)));
                                }
                                flyout_rows.push((*t, r));
                            }
                        });
                });
        });
    rail.rects.flyout_rows = flyout_rows;
    rail.rects.focus = focus;
    if let Some(t) = chosen {
        ed.run(Action::Tool(t));
        rail.follow(t);
        rail.flyout = None;
        return;
    }
    // Escape, or a press anywhere outside the flyout and its chevron, closes it.
    let chevron = rail.rects.rows[fi].1;
    let (escape, pressed_at) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.pointer
                .any_pressed()
                .then(|| i.pointer.interact_pos())
                .flatten(),
        )
    });
    let outside = pressed_at.is_some_and(|p| {
        !resp.response.rect.contains(p) && !chevron.is_some_and(|c| c.contains(p))
    });
    if escape || outside {
        rail.flyout = None;
    }
}
