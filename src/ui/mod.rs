//! egui chrome: menu bar, layers panel, design (properties) panel, floating toolbar, and the
//! projects home screen.

pub mod home;
pub mod inspector;
pub mod library;
pub mod pages;
pub mod palette;
pub mod rail;

use crate::editor::Editor;
use crate::editor::shortcuts::{self, Action};
use crate::model::{Color, NodeId, NodeKind};
use crate::theme::{self, Icon};
use egui::{Align2, Color32, RichText, Ui};
use std::collections::HashSet;

#[derive(Default)]
pub struct UiState {
    pub collapsed: HashSet<NodeId>,
    pub renaming: Option<(NodeId, String)>,
    /// The layer rename field's widget id, as of the last frame it was drawn: lets a host
    /// tell keyboard focus in this field from focus in another one.
    pub rename_field: Option<egui::Id>,
    pub show_help: bool,
    pub show_grid_off: bool,
    /// Export settings (inspector Export section).
    pub export: crate::io::export::Spec,
    /// The window has native chrome: a system menu bar (so no egui menu) and traffic lights
    /// inset into the toolbar / sidebar.
    pub native_chrome: bool,
    /// The sidebar sits over an NSVisualEffectView, so paint it transparent.
    pub translucent_sidebar: bool,
    /// When set, the projects home screen is shown instead of the editor.
    pub home: Option<home::HomeView>,
    /// Name of the open project (shown in the toolbar).
    pub project_name: String,
    /// Autosave state for the toolbar subtitle, e.g. "Saved" or "Saving…".
    pub save_state: String,
    pub pages: pages::PagesUi,
    pub library: library::LibraryUi,
    /// Left sidebar shows the library instead of the layers.
    pub show_library: bool,
    pub palette: palette::PaletteUi,
    /// All panels hidden (⌘\\): just the canvas.
    pub hide_ui: bool,
    /// Layer being dragged in the layers panel, and where it would drop.
    pub layer_drag: Option<(NodeId, Option<LayerDrop>)>,
    /// Row rects of the last layers-panel frame (for drop targeting).
    pub layer_rows: Vec<(NodeId, egui::Rect, usize)>,
    /// A host's opt-in width tiers for surfaces too narrow for both side panels beside the
    /// canvas, and the arrangement as last laid out. Off by default (every native window
    /// uses the wide tier).
    pub narrow: NarrowLayout,
    /// The person's side-panel choices: Layers and Design open or closed, and Collapse panels
    /// (DESIGN.md: Layers starts closed; Design starts open on a wide surface only).
    pub panels: Panels,
    /// The tool rail: each family's current member and the open flyout.
    pub rail: rail::Rail,
    /// Where the header's controls and the zoom were laid out in the last frame (no
    /// behaviour; for hosts' checks and tests, like `PaletteUi::search_id`).
    pub chrome: Chrome,
    /// The person's appearance: System (the default, following the OS appearance as
    /// `theme::apply` set it), Light or Dark (Menu ▸ Appearance). A view preference, never part
    /// of the canvas; the chrome and the canvas display follow it (DESIGN.md).
    pub appearance: egui::ThemePreference,
}

/// One side panel, or neither (what a single-panel tier shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NarrowPanel {
    /// Just the canvas, with the rail and zoom.
    #[default]
    Canvas,
    Layers,
    Design,
}

/// How much room the surface has for side panels (DESIGN.md, Layout tiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Both panels may sit beside the canvas, resizable.
    Wide,
    /// At most one panel, docked beside the canvas at its usual width.
    Docked,
    /// At most one panel, as an opaque sheet filling the room under the header.
    Sheet,
}

/// What a frame lays out: the tier and which panels are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrangement {
    pub tier: Tier,
    pub layers: bool,
    pub design: bool,
}

impl Arrangement {
    /// The panel a single-panel tier shows.
    pub fn panel(&self) -> NarrowPanel {
        if self.layers {
            NarrowPanel::Layers
        } else if self.design {
            NarrowPanel::Design
        } else {
            NarrowPanel::Canvas
        }
    }
}

/// The person's side-panel choices (see [`UiState::panels`]). A side never chosen yet follows
/// the fresh default of the current tier: Layers closed; Design open only on a wide surface.
/// The same toggle means the same panel at every width: on a wide surface the two choices are
/// independent; in a single-panel tier opening one closes the other, and closing it returns to
/// the canvas. Collapse panels hides both and keeps the choices, so Restore brings back the
/// same arrangement; opening a panel while collapsed ends the collapse with that panel only.
/// Hide UI (`UiState::hide_ui`) is a separate mode.
#[derive(Debug, Default, Clone)]
pub struct Panels {
    layers: Option<bool>,
    design: Option<bool>,
    /// The side opened last: what a single-panel tier shows when both are chosen.
    last: Option<NarrowPanel>,
    collapsed: bool,
}

impl Panels {
    /// The arrangement these choices want on a surface of tier `tier`.
    pub fn wanted(&self, tier: Tier) -> Arrangement {
        let none = Arrangement {
            tier,
            layers: false,
            design: false,
        };
        if self.collapsed {
            return none;
        }
        let layers = self.layers.unwrap_or(false);
        let design = self.design.unwrap_or(tier == Tier::Wide);
        if tier == Tier::Wide || !(layers && design) {
            return Arrangement {
                tier,
                layers,
                design,
            };
        }
        let layers = self.last != Some(NarrowPanel::Design);
        Arrangement {
            tier,
            layers,
            design: !layers,
        }
    }

    pub fn collapsed(&self) -> bool {
        self.collapsed
    }

    /// Collapse panels, or Restore the arrangement they had.
    pub fn toggle_collapsed(&mut self) {
        self.collapsed = !self.collapsed;
    }

    fn set(&mut self, side: NarrowPanel, on: bool) {
        match side {
            NarrowPanel::Layers => self.layers = Some(on),
            NarrowPanel::Design => self.design = Some(on),
            NarrowPanel::Canvas => {}
        }
    }

    /// The header's toggle for `side` on a surface of tier `tier`.
    pub fn toggle(&mut self, side: NarrowPanel, tier: Tier) {
        let other = match side {
            NarrowPanel::Layers => NarrowPanel::Design,
            NarrowPanel::Design => NarrowPanel::Layers,
            NarrowPanel::Canvas => return,
        };
        let shown = self.wanted(tier);
        let on = if side == NarrowPanel::Layers {
            shown.layers
        } else {
            shown.design
        };
        if self.collapsed {
            // Opening a panel ends the collapse, with that panel only.
            self.collapsed = false;
            self.set(side, true);
            self.set(other, false);
            self.last = Some(side);
            return;
        }
        if on {
            self.set(side, false);
            if tier != Tier::Wide {
                self.set(other, false);
            }
        } else {
            self.set(side, true);
            self.last = Some(side);
            if tier != Tier::Wide {
                self.set(other, false);
            }
        }
    }

    /// Show `side` (Rename shows the layer rows), leaving it open if it already is.
    pub fn open(&mut self, side: NarrowPanel, tier: Tier) {
        let shown = self.wanted(tier);
        let on = match side {
            NarrowPanel::Layers => shown.layers,
            NarrowPanel::Design => shown.design,
            NarrowPanel::Canvas => true,
        };
        if !on {
            self.toggle(side, tier);
        }
    }
}

/// The width tiers (see [`UiState::narrow`]): below `below` logical px of width the surface has
/// room for one side panel at a time, docked beside the canvas from `dock_from` px and a sheet
/// under the header below that. The same panels, commands and canvas are drawn; only where.
///
/// A change of arrangement (a toggle, Collapse panels, or the surface crossing a tier) is
/// applied only after one frame without an egui text field holding the keyboard, so a field
/// that is about to be hidden first sees its focus go and finishes as a click elsewhere
/// finishes it (a numeric draft, a rename), never silently dropped. While a field has the
/// keyboard, the arrangement waits.
#[derive(Debug, Default)]
pub struct NarrowLayout {
    /// The width below which a single-panel tier is used; 0 (the default) never uses one.
    pub below: f32,
    /// The width from which the single panel is docked beside the canvas rather than shown as
    /// a sheet; 0 (the default) always a sheet.
    pub dock_from: f32,
    /// What is laid out (`None` before the first frame).
    shown: Option<Arrangement>,
    laid_out: bool,
    settling: bool,
}

impl NarrowLayout {
    /// Single-panel tiers below `below` logical px of width, as sheets.
    pub fn with_threshold(below: f32) -> NarrowLayout {
        NarrowLayout {
            below,
            ..Default::default()
        }
    }

    /// [`NarrowLayout::with_threshold`], with the panel docked beside the canvas from
    /// `dock_from` px.
    pub fn with_dock(below: f32, dock_from: f32) -> NarrowLayout {
        NarrowLayout {
            below,
            dock_from,
            ..Default::default()
        }
    }

    /// The tier of a surface `width` logical px wide.
    pub fn tier(&self, width: f32) -> Tier {
        if self.below > 0.0 && width < self.below {
            if self.dock_from > 0.0 && width >= self.dock_from {
                Tier::Docked
            } else {
                Tier::Sheet
            }
        } else {
            Tier::Wide
        }
    }

    /// What the last frame laid out.
    pub fn shown(&self) -> Option<Arrangement> {
        self.shown
    }
}

/// Whether an egui text field (a numeric draft, a rename, a search) has the keyboard.
fn text_field_focused(ctx: &egui::Context) -> bool {
    let focused = ctx.memory(|m| m.focused());
    focused.is_some_and(|id| egui::widgets::text_edit::TextEditState::load(ctx, id).is_some())
}

/// Whether going from `a` to `b` only moves the one panel shown between the docked and sheet
/// tiers: it is drawn by the same panel with the same widgets, so nothing is hidden and its
/// fields keep focus (DESIGN.md: docked ↔ sheet is the same panel).
fn same_single_panel(a: Arrangement, b: Arrangement) -> bool {
    a.tier != Tier::Wide && b.tier != Tier::Wide && a.panel() == b.panel()
}

/// The arrangement to lay out this frame (see [`NarrowLayout`]).
fn settle(ctx: &egui::Context, n: &mut NarrowLayout, want: Arrangement) -> Arrangement {
    if !n.laid_out {
        // The first frame lays out what the surface needs at once.
        n.laid_out = true;
        n.shown = Some(want);
    } else if n.shown.is_some_and(|s| same_single_panel(s, want)) {
        // Docked ↔ sheet (or the same arrangement): nothing waits.
        n.settling = false;
        n.shown = Some(want);
    } else if Some(want) == n.shown || text_field_focused(ctx) {
        n.settling = false;
    } else if n.settling {
        n.settling = false;
        n.shown = Some(want);
    } else {
        // One more frame as before: a field that lost the keyboard this frame finishes in it.
        n.settling = true;
        ctx.request_repaint();
    }
    n.shown.unwrap_or(want)
}

/// Where the header's controls and the zoom were laid out (see [`UiState::chrome`]).
#[derive(Debug, Clone, Default)]
pub struct Chrome {
    pub home: Option<egui::Rect>,
    pub menu: Option<egui::Rect>,
    pub layers: Option<egui::Rect>,
    pub design: Option<egui::Rect>,
    /// The file's name and page line, centred in the header.
    pub identity: Option<egui::Rect>,
    /// The zoom group (−, percentage, +) at the canvas's lower left.
    pub zoom: Option<egui::Rect>,
    pub zoom_out: Option<egui::Rect>,
    pub zoom_level: Option<egui::Rect>,
    pub zoom_in: Option<egui::Rect>,
    /// A sheet's way back to the canvas.
    pub sheet_back: Option<egui::Rect>,
}

/// Where a dragged layer lands relative to a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerDrop {
    /// Above the row (in front of it in paint order).
    Above(NodeId),
    Below(NodeId),
    /// Inside a frame or group (on top of its children).
    Into(NodeId),
}

pub fn setup_style(ctx: &egui::Context) {
    theme::apply(ctx);
}

pub(crate) fn c32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}
pub(crate) fn from32(c: Color32) -> Color {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color::rgba(r, g, b, a)
}

/// Section header, as in a macOS sidebar or inspector: 11pt semibold, secondary colour.
pub(crate) fn section(ui: &mut Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .font(theme::semibold(theme::SECONDARY))
            .color(theme::pal(ui).secondary),
    );
}

pub(crate) fn title_text(text: impl Into<String>) -> RichText {
    RichText::new(text).font(theme::semibold(theme::BODY))
}

/// Lay out the chrome and return the remaining canvas rectangle (logical px).
pub fn draw(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) -> egui::Rect {
    // The chosen appearance, applied with egui's own theme preference (the next frame uses it).
    // The default, System, is what `theme::apply` set: unchanged windows lay out as before.
    if ui.ctx().options(|o| o.theme_preference) != st.appearance {
        ui.ctx().set_theme(st.appearance);
        ui.ctx().request_repaint();
    }
    if let Some(hv) = &mut st.home {
        home::draw(ui, hv, st.native_chrome, st.translucent_sidebar);
        return egui::Rect::NOTHING;
    }
    st.chrome = Chrome::default();
    let tier = st.narrow.tier(ui.available_rect_before_wrap().width());
    for ev in std::mem::take(&mut ed.ui_events) {
        use crate::editor::commands::UiEvent;
        match ev {
            UiEvent::TogglePalette => st.palette.toggle(),
            UiEvent::Rename(id) => {
                if let Some(n) = ed.doc.get(id) {
                    st.renaming = Some((id, n.name.clone()));
                    st.show_library = false;
                    st.hide_ui = false;
                    // The rename's field is in the layer rows: show them.
                    st.panels.open(NarrowPanel::Layers, tier);
                }
            }
            UiEvent::ToggleUi => st.hide_ui = !st.hide_ui,
            UiEvent::TogglePanels => st.panels.toggle_collapsed(),
            UiEvent::ToggleHelp => st.show_help = !st.show_help,
            UiEvent::ToggleGrid => st.show_grid_off = !st.show_grid_off,
        }
    }
    if st.hide_ui {
        // No layer or page rows are drawn: an open rename is finished as losing focus does.
        commit_rename(ed, st);
        pages::commit_rename(ed, st);
        st.rail.close();
        let canvas = ui.available_rect_before_wrap();
        palette::draw(ui.ctx(), ed, st, canvas);
        return canvas;
    }
    let want = st.panels.wanted(tier);
    let shown = settle(ui.ctx(), &mut st.narrow, want);
    let p = theme::pal(ui);
    let sidebar_fill = if st.translucent_sidebar {
        Color32::TRANSPARENT
    } else {
        p.sidebar
    };
    // The header's safe area comes first, across the whole surface; the panels start below it.
    egui::Panel::top("toolbar")
        .exact_size(if st.native_chrome {
            theme::TOOLBAR_H
        } else {
            BAR_H
        })
        .frame(
            egui::Frame::NONE
                .fill(p.window)
                .inner_margin(egui::Margin::symmetric(8, 0)),
        )
        .show(ui, |ui| top_bar(ui, ed, st, shown));
    let room = ui.available_rect_before_wrap();
    let layers_frame = egui::Frame::NONE
        .fill(sidebar_fill)
        .inner_margin(egui::Margin::symmetric(10, 8));
    let design_frame = egui::Frame::NONE
        .fill(p.inspector)
        .inner_margin(egui::Margin::symmetric(14, 10));
    match shown.tier {
        Tier::Wide => {
            if shown.layers {
                egui::Panel::left("layers")
                    .resizable(true)
                    .default_size(240.0)
                    .size_range(180.0..=420.0)
                    .frame(layers_frame)
                    .show(ui, |ui| layers(ui, ed, st));
            }
            if shown.design {
                egui::Panel::right("design")
                    .resizable(true)
                    .default_size(260.0)
                    .size_range(220.0..=420.0)
                    .frame(design_frame)
                    .show(ui, |ui| inspector::panel(ui, ed, st));
            }
        }
        Tier::Docked | Tier::Sheet => {
            // Docked: the panel at its usual width, the canvas keeps the rest. A sheet fills
            // the room under the header (opaque; it takes its own input) and says how to get
            // back to the canvas.
            let sheet = shown.tier == Tier::Sheet;
            let width = |usual: f32| if sheet { room.width() } else { usual };
            // The panel's own widgets get the same ids in both tiers whether or not the sheet's
            // back row comes first (egui numbers widgets in order), so a focused field and its
            // draft survive docked ↔ sheet.
            let body = |ui: &Ui| egui::UiBuilder::new().id(ui.id().with("panel-body"));
            match shown.panel() {
                NarrowPanel::Layers => {
                    egui::Panel::left("layers-narrow")
                        .resizable(false)
                        .exact_size(width(240.0))
                        .frame(layers_frame)
                        .show(ui, |ui| {
                            if sheet {
                                sheet_back(ui, st, NarrowPanel::Layers, tier);
                            }
                            ui.scope_builder(body(ui), |ui| layers(ui, ed, st));
                        });
                }
                NarrowPanel::Design => {
                    egui::Panel::right("design-narrow")
                        .resizable(false)
                        .exact_size(width(260.0))
                        .frame(design_frame)
                        .show(ui, |ui| {
                            if sheet {
                                sheet_back(ui, st, NarrowPanel::Design, tier);
                            }
                            ui.scope_builder(body(ui), |ui| inspector::panel(ui, ed, st));
                        });
                }
                NarrowPanel::Canvas => {}
            }
        }
    }
    if !shown.layers && !want.layers {
        // No layer or page rows are drawn, and none are about to be: an open rename is
        // finished as losing focus does (as when the UI is hidden).
        commit_rename(ed, st);
        pages::commit_rename(ed, st);
    }
    // The one canvas rectangle: rendering, clipping and pointer input all use it.
    let canvas = ui.available_rect_before_wrap();
    if canvas.width() > 1.0 && canvas.height() > 1.0 {
        rail::draw(ui.ctx(), ed, &mut st.rail, canvas);
        zoom_controls(ui.ctx(), ed, st, canvas);
        status(ui.ctx(), ed, canvas, st.chrome.zoom);
    } else {
        // A sheet covers the canvas: no rail, and no flyout left open behind it.
        st.rail.close();
        st.rail.rects = Default::default();
    }
    palette::draw(ui.ctx(), ed, st, room);
    if st.show_help {
        help(ui.ctx(), st, room);
    }
    canvas
}

/// A sheet's first row: the way back to the canvas (the header's toggle does the same).
fn sheet_back(ui: &mut Ui, st: &mut UiState, side: NarrowPanel, tier: Tier) {
    let r = ui
        .add(egui::Button::new("← Canvas").frame(false))
        .on_hover_text("Close this panel and return to the canvas");
    st.chrome.sheet_back = Some(r.rect);
    if r.clicked() {
        st.panels.toggle(side, tier);
    }
    ui.add_space(4.0);
}

/// Zoom out, the zoom level and zoom in, at the canvas's lower left (clear of the rail; beside
/// it, at the top, on a short canvas). The level opens Fit, selection and 100%.
fn zoom_controls(ctx: &egui::Context, ed: &mut Editor, st: &mut UiState, canvas: egui::Rect) {
    let rail = rail::rail_rect(canvas);
    let short = canvas.height() < rail::SHORT_H;
    let (pivot, pos) = if short {
        (
            Align2::LEFT_TOP,
            egui::pos2(rail.right() + rail::INSET, canvas.top() + rail::INSET),
        )
    } else {
        (
            Align2::LEFT_BOTTOM,
            egui::pos2(canvas.left() + rail::INSET, canvas.bottom() - rail::INSET),
        )
    };
    let mut parts = (None, None, None);
    let r = egui::Area::new("zoom".into())
        .order(egui::Order::Middle)
        .pivot(pivot)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(8.0)
                .inner_margin(2.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let out =
                            action_button(ui, ed, theme::Icon::Minus, "Zoom out", Action::ZoomOut);
                        let level = ui
                            .menu_button(
                                RichText::new(format!("{:.0}%", ed.camera.zoom * 100.0))
                                    .font(theme::medium(theme::SECONDARY + 1.0))
                                    .color(p.secondary),
                                |ui| {
                                    for (label, a) in [
                                        ("Zoom to Fit", Action::ZoomFit),
                                        ("Zoom to Selection", Action::ZoomSelection),
                                        ("Zoom to 100%", Action::Zoom100),
                                    ] {
                                        menu_item(ui, ed, label, a);
                                    }
                                },
                            )
                            .response
                            .on_hover_text("Zoom options");
                        let inn =
                            action_button(ui, ed, theme::Icon::Plus, "Zoom in", Action::ZoomIn);
                        parts = (Some(out), Some(level.rect), Some(inn));
                    });
                });
        });
    st.chrome.zoom = Some(r.response.rect);
    (st.chrome.zoom_out, st.chrome.zoom_level, st.chrome.zoom_in) = parts;
}

/// The top bar's height without native chrome.
const BAR_H: f32 = 38.0;

/// A text toggle of the top bar (the narrow layout's Layers and Design): a soft accent tint
/// when on, a hover fill otherwise.
fn pill(ui: &mut Ui, label: &str, on: bool) -> egui::Response {
    let p = theme::pal(ui);
    let color = if on { p.text } else { p.secondary };
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        theme::medium(theme::SECONDARY + 1.0),
        color,
    );
    let size = galley.size() + egui::vec2(16.0, 10.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if on {
        theme::selected_fill(ui)
    } else if resp.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        Color32::PLACEHOLDER,
    );
    resp
}

fn menu_item(ui: &mut Ui, ed: &mut Editor, label: &str, action: Action) {
    if ui
        .add(egui::Button::new(label).shortcut_text(shortcuts::chord_text(action)))
        .clicked()
    {
        ed.run(action);
        ui.close();
    }
}

fn menu_entries(ui: &mut Ui, ed: &mut Editor, entries: &[shortcuts::MenuEntry]) {
    for e in entries {
        match e {
            shortcuts::MenuEntry::Item(label, a) => menu_item(ui, ed, label, *a),
            shortcuts::MenuEntry::Separator => {
                ui.separator();
            }
            shortcuts::MenuEntry::Submenu(label, items) => {
                ui.menu_button(*label, |ui| menu_entries(ui, ed, items));
            }
        }
    }
}

/// Menu ▸ Appearance: System (follows the OS appearance), Light or Dark. It changes how the
/// chrome and canvas are shown, never the canvas itself.
fn appearance_menu(ui: &mut Ui, st: &mut UiState) {
    for (pref, label, tip) in [
        (
            egui::ThemePreference::System,
            "System",
            "Follow the system appearance",
        ),
        (egui::ThemePreference::Light, "Light", "Always light"),
        (egui::ThemePreference::Dark, "Dark", "Always dark"),
    ] {
        if ui
            .radio(st.appearance == pref, label)
            .on_hover_text(tip)
            .clicked()
        {
            st.appearance = pref;
            ui.close();
        }
    }
}

/// An icon button with its action's name and shortcut as its tooltip; returns where it is.
fn action_button(
    ui: &mut Ui,
    ed: &mut Editor,
    icon: theme::Icon,
    name: &str,
    action: Action,
) -> egui::Rect {
    let keys = shortcuts::chord_text(action);
    let tip = if keys.is_empty() {
        name.to_string()
    } else {
        format!("{name}  {keys}")
    };
    let r = theme::quiet_icon_button(ui, icon, 26.0, false).on_hover_text(tip);
    if r.clicked() {
        ed.run(action);
    }
    r.rect
}

/// The header (DESIGN.md: the top safe area, laid out before the panels and across the whole
/// surface, at every width). Left: All projects (the way home), one compact menu holding every
/// upstream menu (platforms without a native menu bar) and the Layers toggle. Centre: the
/// file's real name over its page and save state, centred on the surface and truncated rather
/// than overlapped. Right: the Design toggle. Undo, redo, zoom and help stay in the menu, the
/// palette and their shortcuts (zoom also at the canvas's lower left). Its controls are quiet
/// and only an open toggle is tinted, so the header recedes around the canvas.
fn top_bar(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, shown: Arrangement) {
    let p = theme::pal(ui);
    let bar = ui.max_rect();
    let tier = shown.tier;
    // Left group (after the traffic lights where the window has native chrome).
    let left_start = bar.left()
        + if st.native_chrome {
            theme::TRAFFIC_LIGHTS_W
        } else {
            0.0
        };
    let left = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    egui::pos2(left_start, bar.top()),
                    bar.max,
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let home = theme::quiet_icon_button(ui, theme::Icon::Grid, 26.0, false)
                    .on_hover_text("All projects");
                st.chrome.home = Some(home.rect);
                if home.clicked() {
                    ed.requests.push(crate::editor::AppRequest::Home);
                }
                if !st.native_chrome {
                    let menu = ui
                        .menu_button(
                            RichText::new("···")
                                .font(theme::semibold(theme::BODY))
                                .color(p.secondary),
                            |ui| {
                                for (title, entries) in shortcuts::menus() {
                                    ui.menu_button(title, |ui| menu_entries(ui, ed, &entries));
                                }
                                ui.separator();
                                ui.menu_button("Appearance", |ui| appearance_menu(ui, st));
                            },
                        )
                        .response
                        .on_hover_text("Menu");
                    st.chrome.menu = Some(menu.rect);
                }
                let layers = theme::quiet_icon_button(ui, theme::Icon::Sidebar, 26.0, shown.layers)
                    .on_hover_text("Layers and pages");
                st.chrome.layers = Some(layers.rect);
                if layers.clicked() {
                    st.panels.toggle(NarrowPanel::Layers, tier);
                }
                ui.min_rect().right()
            },
        )
        .inner;
    // Right group.
    let right = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(bar)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let design = pill(ui, "Design", shown.design).on_hover_text("Design inspector");
                st.chrome.design = Some(design.rect);
                if design.clicked() {
                    st.panels.toggle(NarrowPanel::Design, tier);
                }
                ui.min_rect().left()
            },
        )
        .inner;
    // Centre: on the surface's centre while that leaves room on both sides, else in the room
    // between the two groups.
    let gap = 8.0;
    let (lo, hi) = (left + gap, right - gap);
    let half = (bar.center().x - lo).min(hi - bar.center().x);
    let (x0, x1) = if half >= 60.0 {
        (bar.center().x - half, bar.center().x + half)
    } else {
        (lo, hi.max(lo))
    };
    let id_rect = egui::Rect::from_min_max(egui::pos2(x0, bar.top()), egui::pos2(x1, bar.bottom()));
    st.chrome.identity = Some(id_rect);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(id_rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| file_identity(ui, ed, st),
    );
    // 1px separator under the header.
    ui.painter().hline(
        bar.x_range(),
        bar.bottom() - 0.5,
        egui::Stroke::new(1.0, p.separator),
    );
}

/// The file's name (upstream's project title) over its page name and save state, each
/// truncated to the room it has.
fn file_identity(ui: &mut Ui, ed: &Editor, st: &UiState) {
    let p = theme::pal(ui);
    let title = if st.project_name.is_empty() {
        ed.doc.name.clone()
    } else {
        st.project_name.clone()
    };
    let state = if !st.save_state.is_empty() {
        st.save_state.clone()
    } else if ed.is_dirty() {
        "Edited".into()
    } else {
        String::new()
    };
    let sub = if state.is_empty() {
        ed.page_name().to_string()
    } else {
        format!("{} — {state}", ed.page_name())
    };
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(if st.native_chrome { 6.0 } else { 2.0 });
    ui.add(egui::Label::new(title_text(title)).truncate());
    ui.add(
        egui::Label::new(RichText::new(sub).size(theme::SECONDARY).color(p.secondary)).truncate(),
    );
}

fn layers(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    pages::panel(ui, ed, st);
    ui.add_space(4.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.x_range(),
        r.top(),
        egui::Stroke::new(1.0, theme::pal(ui).separator),
    );
    ui.add_space(6.0);
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        for (lib, label) in [(false, "Layers"), (true, "Library")] {
            let on = st.show_library == lib;
            let r = ui.add(
                egui::Label::new(
                    RichText::new(label)
                        .font(theme::semibold(theme::SECONDARY))
                        .color(if on { p.text } else { p.secondary }),
                )
                .sense(egui::Sense::click()),
            );
            if r.clicked() {
                st.show_library = lib;
            }
        }
        // Arrange is contextual: shown only while something is selected (it stays in the menu,
        // the command palette and its shortcuts), so an empty or unselected layers panel is quiet.
        if st.show_library || ed.selection.is_empty() {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if theme::icon_button(ui, Icon::ChevronDown, 20.0, false)
                .on_hover_text("Send backward")
                .clicked()
            {
                ed.run(Action::Backward);
            }
            if theme::icon_button(ui, Icon::ChevronUp, 20.0, false)
                .on_hover_text("Bring forward")
                .clicked()
            {
                ed.run(Action::Forward);
            }
        });
    });
    ui.add_space(2.0);
    if st.show_library {
        // The layer rows (and an open rename field) aren't drawn: finish the rename now, as
        // losing focus does, while its node is still the shown page's.
        commit_rename(ed, st);
        library::panel(ui, ed, st);
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let roots = ed.doc.root.clone();
            if roots.is_empty() {
                ui.label(
                    RichText::new(
                        "Nothing here yet. Pick a tool on the left\nand draw on the canvas.",
                    )
                    .size(theme::SECONDARY)
                    .color(theme::pal(ui).secondary),
                );
            }
            st.layer_rows.clear();
            for id in roots.iter().rev() {
                layer_row(ui, ed, st, *id, 0);
            }
            layer_drag(ui, ed, st);
        });
    // A rename whose row wasn't drawn (inside a collapsed group, or of a node that is gone)
    // has no field to finish it: finish it now, on this page.
    if st
        .renaming
        .as_ref()
        .is_some_and(|(id, _)| !st.layer_rows.iter().any(|(r, _, _)| r == id))
    {
        commit_rename(ed, st);
    }
}

/// Drag-and-drop reordering in the layers tree: above / below a row, or into a frame/group.
fn layer_drag(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let Some((dragged, _)) = st.layer_drag else {
        return;
    };
    let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
        return;
    };
    let target = st
        .layer_rows
        .iter()
        .find(|(_, r, _)| r.y_range().contains(pos.y))
        .copied();
    let drop = target.and_then(|(id, r, _)| {
        if id == dragged || ed.doc.is_ancestor(dragged, id) {
            return None;
        }
        let n = ed.doc.get(id)?;
        let t = (pos.y - r.top()) / r.height();
        Some(if n.is_container() && (0.3..0.7).contains(&t) {
            LayerDrop::Into(id)
        } else if t < 0.5 {
            LayerDrop::Above(id)
        } else {
            LayerDrop::Below(id)
        })
    });
    st.layer_drag = Some((dragged, drop));
    // Indicator.
    if let (Some(d), Some((_, r, depth))) = (drop, target) {
        let x0 = r.left() + 4.0 + depth as f32 * 14.0 + 16.0;
        let stroke = egui::Stroke::new(2.0, theme::accent());
        match d {
            LayerDrop::Above(_) => {
                ui.painter().hline(x0..=r.right(), r.top(), stroke);
            }
            LayerDrop::Below(_) => {
                ui.painter().hline(x0..=r.right(), r.bottom(), stroke);
            }
            LayerDrop::Into(_) => {
                ui.painter()
                    .rect_stroke(r, 6.0, stroke, egui::StrokeKind::Inside);
            }
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        st.layer_drag = None;
        if let Some(d) = drop {
            drop_layer(ed, dragged, d);
        }
    }
}

/// Move a layer in the tree (keeping its position on the canvas).
pub fn drop_layer(ed: &mut Editor, id: NodeId, d: LayerDrop) {
    let (parent, index) = match d {
        LayerDrop::Into(c) => (Some(c), None),
        LayerDrop::Above(t) | LayerDrop::Below(t) => {
            let parent = ed.doc.get(t).and_then(|n| n.parent);
            let list: Vec<NodeId> = ed
                .doc
                .children(parent)
                .iter()
                .copied()
                .filter(|x| *x != id)
                .collect();
            let i = list.iter().position(|x| *x == t).unwrap_or(0);
            (
                parent,
                Some(if matches!(d, LayerDrop::Above(_)) {
                    i + 1
                } else {
                    i
                }),
            )
        }
    };
    if parent.is_some_and(|p| p == id || ed.doc.is_ancestor(id, p)) {
        return;
    }
    ed.edit("Move layer", |tx| {
        let same = tx.doc().get(id).and_then(|n| n.parent) == parent;
        if same {
            if let Some(i) = index {
                tx.move_to_index(id, i);
            } else {
                let len = tx.doc().children(parent).len();
                tx.move_to_index(id, len);
            }
        } else {
            tx.reparent(id, parent, index);
        }
    });
    ed.selection = vec![id];
}

fn node_icon(n: &crate::model::Node) -> Icon {
    if n.component {
        return Icon::Component;
    }
    if n.instance.is_some() && n.parent.is_none_or(|_| n.is_frame()) && n.is_frame() {
        return Icon::Instance;
    }
    if n.style
        .fills
        .iter()
        .any(|p| matches!(p, crate::model::Paint::Image { .. }))
        && matches!(n.kind, NodeKind::Rect)
    {
        return Icon::Image;
    }
    kind_icon(&n.kind)
}

fn kind_icon(k: &NodeKind) -> Icon {
    match k {
        NodeKind::Group => Icon::Group,
        NodeKind::Connector { .. } => Icon::Arrow,
        NodeKind::Sticky { .. } => Icon::Sticky,
        NodeKind::Path { .. } => Icon::Pen,
        NodeKind::Polygon { .. } => Icon::Polygon,
        NodeKind::Star { .. } => Icon::Star,
        NodeKind::Frame { .. } => Icon::Frame,
        NodeKind::Rect => Icon::Rect,
        NodeKind::Ellipse => Icon::Ellipse,
        NodeKind::Line { .. } => Icon::Line,
        NodeKind::Text { .. } => Icon::Text,
        NodeKind::Stroke { .. } => Icon::Ink,
    }
}

/// Finish the inline layer rename as losing focus does: its trimmed name, if not empty and
/// changed, as one undoable edit of the shown page's node. The rename names that node by its
/// id in the shown document, so it is finished before another page is shown (`pages::run`).
fn commit_rename(ed: &mut Editor, st: &mut UiState) {
    let Some((id, name)) = st.renaming.take() else {
        return;
    };
    let new = name.trim().to_string();
    if !new.is_empty() && ed.doc.get(id).is_some_and(|n| n.name != new) {
        ed.edit("Rename", |tx| tx.node_mut(id).unwrap().name = new);
    }
}

/// One sidebar row, drawn like an NSOutlineView row: 24pt tall, 6pt radius selection as an
/// accent tint, subtle hover, disclosure chevron, symbol, name, and eye/lock on hover.
fn layer_row(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, id: NodeId, depth: usize) {
    let Some(n) = ed.doc.get(id).cloned() else {
        return;
    };
    let p = theme::pal(ui);
    let selected = ed.selection.contains(&id);
    let width = ui.available_width();
    let (row, resp) =
        ui.allocate_exact_size(egui::vec2(width, 24.0), egui::Sense::click_and_drag());
    st.layer_rows.push((id, row, depth));
    if resp.drag_started() && !n.locked {
        st.layer_drag = Some((id, None));
    }
    let hovered = ui.rect_contains_pointer(row);
    // Selection is a soft accent tint (the name keeps its text colour), as in the pages list
    // and the top bar's toggles.
    if selected {
        ui.painter().rect_filled(row, 6.0, theme::selected_fill(ui));
    } else if hovered {
        ui.painter().rect_filled(row, 6.0, p.hover);
    }
    let fg = if n.visible { p.text } else { p.secondary };
    let fg2 = p.secondary;
    let mut x = row.left() + 4.0 + depth as f32 * 14.0;
    let cy = row.center().y;
    let sq = |x: f32, s: f32| {
        egui::Rect::from_center_size(egui::pos2(x + s / 2.0, cy), egui::vec2(s, s))
    };

    // Disclosure chevron.
    if !n.children.is_empty() {
        let open = !st.collapsed.contains(&id);
        let r = sq(x, 14.0);
        let cr = ui.interact(
            r.expand(2.0),
            ui.id().with(("disclose", id)),
            egui::Sense::click(),
        );
        theme::paint_icon(
            ui.painter(),
            r.shrink(2.0),
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            fg2,
        );
        if cr.clicked() {
            if open {
                st.collapsed.insert(id);
            } else {
                st.collapsed.remove(&id);
            }
        }
    }
    x += 16.0;
    theme::paint_icon(ui.painter(), sq(x, 14.0), node_icon(&n), theme::accent());
    x += 20.0;

    // Trailing eye / lock buttons: always shown when hidden or locked, otherwise on hover.
    let mut right = row.right() - 4.0;
    let mut trailing = |ui: &mut Ui, icon: Icon, tip: &str, show: bool| -> bool {
        let r = egui::Rect::from_center_size(egui::pos2(right - 9.0, cy), egui::vec2(18.0, 18.0));
        right -= 20.0;
        if !show {
            return false;
        }
        let b = ui.interact(r, ui.id().with((tip, id)), egui::Sense::click());
        if b.hovered() {
            ui.painter().rect_filled(r, 5.0, p.hover);
        }
        theme::paint_icon(ui.painter(), r.shrink(3.0), icon, fg2);
        b.on_hover_text(tip).clicked()
    };
    let eye = if n.visible { Icon::Eye } else { Icon::EyeSlash };
    if trailing(
        ui,
        eye,
        if n.visible { "Hide" } else { "Show" },
        hovered || !n.visible,
    ) {
        ed.edit(if n.visible { "Hide" } else { "Show" }, |tx| {
            tx.node_mut(id).unwrap().visible = !n.visible
        });
    }
    let lock = if n.locked { Icon::Lock } else { Icon::LockOpen };
    if trailing(
        ui,
        lock,
        if n.locked { "Unlock" } else { "Lock" },
        hovered || n.locked,
    ) {
        ed.edit(if n.locked { "Unlock" } else { "Lock" }, |tx| {
            tx.node_mut(id).unwrap().locked = !n.locked
        });
    }

    // Name, or an inline rename field.
    let name_rect =
        egui::Rect::from_min_max(egui::pos2(x, row.top()), egui::pos2(right, row.bottom()));
    if let Some((rid, name)) = &mut st.renaming
        && *rid == id
    {
        let r = ui.put(
            name_rect.shrink2(egui::vec2(0.0, 2.0)),
            egui::TextEdit::singleline(name),
        );
        st.rename_field = Some(r.id);
        // Check the loss first: `request_focus` takes focus back at once, so calling it before
        // `lost_focus` would hide Enter or a click elsewhere and the rename would never commit.
        if r.lost_focus() {
            commit_rename(ed, st);
        } else if !r.has_focus() {
            r.request_focus();
        }
    } else {
        let font = if n.is_frame() {
            theme::medium(theme::BODY)
        } else {
            egui::FontId::proportional(theme::BODY)
        };
        let galley = ui.painter().layout(n.name.clone(), font, fg, f32::INFINITY);
        let clip = ui
            .painter()
            .with_clip_rect(name_rect.intersect(ui.clip_rect()));
        clip.galley(egui::pos2(x, cy - galley.size().y / 2.0), galley, fg);
    }

    if resp.double_clicked() {
        st.renaming = Some((id, n.name.clone()));
    } else if resp.clicked() {
        ed.end_text_edit();
        if ui.input(|i| i.modifiers.shift || i.modifiers.command) {
            if selected {
                ed.selection.retain(|s| *s != id);
            } else {
                ed.selection.push(id);
            }
        } else {
            ed.selection = vec![id];
        }
    }
    if hovered {
        ed.hover = Some(id);
    }
    if !st.collapsed.contains(&id) {
        for c in n.children.iter().rev() {
            layer_row(ui, ed, st, *c, depth + 1);
        }
    }
}

/// A transient status line, right of the zoom controls (or at the canvas's lower left when
/// they are at the top), so it never covers them or the rail.
fn status(ctx: &egui::Context, ed: &Editor, canvas: egui::Rect, zoom: Option<egui::Rect>) {
    if ed.status.is_empty() {
        return;
    }
    let left = zoom
        .filter(|z| z.bottom() > canvas.center().y)
        .map_or(canvas.left() + 12.0, |z| z.right() + 12.0);
    egui::Area::new("status".into())
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(egui::pos2(left, canvas.bottom() - 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_max_width((canvas.right() - left - 12.0).max(40.0));
            ui.label(RichText::new(&ed.status).weak().small());
        });
}

fn help(ctx: &egui::Context, st: &mut UiState, canvas: egui::Rect) {
    let width = 820.0f32.min(canvas.width() - 40.0);
    egui::Area::new("help".into())
        .order(egui::Order::Foreground)
        .fade_in(false)
        .pivot(Align2::CENTER_CENTER)
        .fixed_pos(canvas.center())
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(12.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Keyboard shortcuts").font(theme::semibold(15.0)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Done").on_hover_text("Close (Esc)").clicked() {
                                st.show_help = false;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let sections = shortcuts::help_sections();
                    egui::ScrollArea::vertical()
                        .max_height((canvas.height() - 160.0).max(200.0))
                        .show(ui, |ui| {
                            ui.columns(3, |cols| {
                                // Balance columns by row count.
                                let total: usize = sections.iter().map(|(_, r)| r.len() + 2).sum();
                                let mut acc = 0;
                                for (i, (title, rows)) in sections.iter().enumerate() {
                                    let col = (acc * 3 / total.max(1)).min(2);
                                    acc += rows.len() + 2;
                                    let ui = &mut cols[col];
                                    ui.label(title_text(*title));
                                    egui::Grid::new(("keys", i)).num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
                                        for (k, d) in rows {
                                            ui.label(
                                                RichText::new(k)
                                                    .font(theme::medium(11.5))
                                                    .color(p.secondary),
                                            );
                                            ui.label(RichText::new(*d).size(12.0));
                                            ui.end_row();
                                        }
                                    });
                                    ui.add_space(10.0);
                                }
                            });
                        });
                    ui.label(
                        RichText::new("Space-drag pans · wheel pans · ⌘/Ctrl+wheel or pinch zooms · ⌥-drag duplicates · hold ⌥ to measure")
                            .size(theme::SECONDARY)
                            .color(p.secondary),
                    );
                });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.show_help = false;
    }
}
