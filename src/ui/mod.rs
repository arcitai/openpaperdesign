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
    /// The canvas's background pattern (DESIGN.md: None by default; Dots or Lines from the
    /// canvas's menu, View and the palette). A view preference: never the document, its undo,
    /// its saves, snapping or an export. Set it with [`UiState::set_background`].
    pub background: crate::render::scene::Background,
    /// The last pattern other than None, which the grid shortcut turns back on (kept by
    /// `set_background`; public only so hosts can build a `UiState` with `..Default::default()`).
    pub background_last: Option<crate::render::scene::Background>,
    /// The canvas's own menu (a secondary click on blank canvas), where it opened.
    pub canvas_menu: Option<egui::Pos2>,
    /// About & notices (the menu's About): licences and notices, as the browser page shows them.
    pub show_about: bool,
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
    /// A host that saves the file's name (the web editor, through its canvas save) opts in:
    /// the centred title is then an inline rename field. Off by default: a native window has
    /// no such save, so its title stays a label rather than suggesting a rename it can't keep.
    pub file_rename: bool,
    /// The title's unfinished rename: Enter or a click elsewhere finishes it into
    /// `project_name`, Escape drops it. Not the file's name until then.
    pub file_title_draft: Option<String>,
    /// The title field's widget id, as of the last frame it was drawn (like
    /// `UiState::rename_field`): lets a host judge exactly that field by its draft.
    pub file_title_field: Option<egui::Id>,
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

impl UiState {
    /// Show `b` behind the canvas (a view preference; nothing in the document changes).
    pub fn set_background(&mut self, b: crate::render::scene::Background) {
        use crate::render::scene::Background;
        if b != Background::None {
            self.background_last = Some(b);
        }
        self.background = b;
    }

    /// The grid shortcut and menu item: None against the last other pattern (Dots at first).
    pub fn toggle_background(&mut self) {
        use crate::render::scene::Background;
        let next = if self.background == Background::None {
            self.background_last.unwrap_or(Background::Dots)
        } else {
            Background::None
        };
        self.set_background(next);
    }

    /// A press or Escape belongs to an open tool flyout or to the canvas's own menu (it closes
    /// it), never to the canvas: the hosts hold it back from the editor then.
    pub fn captures_input(&self) -> bool {
        self.rail.captures_input() || self.canvas_menu.is_some()
    }

    /// Whether egui's top layer `l` at a point (`Context::layer_id_at`) is floating UI that
    /// takes the pointer there rather than the canvas: any area above the background (the rail,
    /// zoom, menus, popups, tooltips), except the canvas's own menu once it is closed. egui
    /// counts an area visible for one frame after its last one, so the just-closed menu would
    /// otherwise still take a press at its place until the next frame is laid out.
    pub fn takes_pointer(&self, l: egui::LayerId) -> bool {
        let closed_menu = self.canvas_menu.is_none() && l == canvas_menu_layer();
        l.order != egui::Order::Background && !closed_menu
    }
}

/// Whether a pointer at `p` is over floating UI rather than the canvas
/// ([`UiState::takes_pointer`] for egui's top layer there). Both hosts check it before giving
/// the editor pointer input, and the canvas's menu before it opens.
pub fn over_floating_ui(ctx: &egui::Context, st: &UiState, p: egui::Pos2) -> bool {
    ctx.layer_id_at(p).is_some_and(|l| st.takes_pointer(l))
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
    /// Where each resizable panel's own resize handle takes a press (see [`panel_edge`]).
    pub panel_edges: Vec<egui::Rect>,
}

impl Chrome {
    /// Whether a press at `p` is a panel resize handle's (egui's), not the canvas's: the hosts
    /// keep it from the editor, so resizing a panel never also edits what lies behind its edge.
    pub fn on_panel_edge(&self, p: egui::Pos2) -> bool {
        self.panel_edges.iter().any(|r| r.contains(p))
    }
}

/// Where the resizable panel `id`'s resize handle takes a press, as laid out this frame: egui's
/// own handle widget (registered by egui 0.36 as `<panel id>.with("__resize")`, across the edge
/// and into the canvas) widened by egui's interact radius, within which egui's hit test gives a
/// press to the closest widget, this handle, though the pointer isn't on it.
fn panel_edge(ui: &Ui, id: &str) -> Option<egui::Rect> {
    let handle = ui.ctx().read_response(egui::Id::new(id).with("__resize"))?;
    let radius = ui.style().interaction.interact_radius;
    Some(handle.interact_rect.expand(radius))
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
            UiEvent::ToggleGrid => st.toggle_background(),
        }
    }
    if st.hide_ui {
        // No layer or page rows are drawn: an open rename is finished as losing focus does.
        commit_rename(ed, st);
        pages::commit_rename(ed, st);
        // The header isn't drawn either: the title's draft is finished as losing focus does.
        if let Some(n) = st.file_title_draft.take().as_deref().and_then(file_name) {
            st.project_name = n;
        }
        st.rail.close();
        st.canvas_menu = None;
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
                st.chrome.panel_edges.extend(panel_edge(ui, "layers"));
            }
            if shown.design {
                egui::Panel::right("design")
                    .resizable(true)
                    .default_size(260.0)
                    .size_range(220.0..=420.0)
                    .frame(design_frame)
                    .show(ui, |ui| inspector::panel(ui, ed, st));
                st.chrome.panel_edges.extend(panel_edge(ui, "design"));
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
        // Before the rail: a flyout open in the last frame keeps the press its own.
        let opened = open_canvas_menu(ui.ctx(), ed, st, canvas);
        rail::draw(ui.ctx(), ed, &mut st.rail, canvas);
        zoom_controls(ui.ctx(), ed, st, canvas);
        status(ui.ctx(), ed, canvas, [st.chrome.zoom, st.rail.rects.rail]);
        canvas_menu(ui.ctx(), ed, st, canvas, opened);
    } else {
        // A sheet covers the canvas: no rail, and no flyout or canvas menu left open behind it.
        st.rail.close();
        st.rail.rects = Default::default();
        st.canvas_menu = None;
    }
    palette::draw(ui.ctx(), ed, st, room);
    if st.show_help {
        help(ui.ctx(), st, room);
    }
    if st.show_about {
        about(ui.ctx(), st, room);
    }
    canvas
}

/// The canvas's own menu opens on a secondary click (pressed and released without a drag)
/// on blank canvas: inside the canvas rectangle, under no egui area (panel, rail, zoom, popup)
/// and over no node, while no flyout or other canvas menu is open. Returns whether it opened
/// in this frame. The editor ignores the secondary button, so nothing is drawn or selected;
/// Hand, Space and middle-button panning are untouched.
fn open_canvas_menu(
    ctx: &egui::Context,
    ed: &Editor,
    st: &mut UiState,
    canvas: egui::Rect,
) -> bool {
    if st.rail.flyout.is_some() || st.canvas_menu.is_some() {
        return false;
    }
    let clicked = ctx.input(|i| {
        i.pointer
            .button_clicked(egui::PointerButton::Secondary)
            .then(|| i.pointer.interact_pos())
            .flatten()
    });
    let Some(at) = clicked else {
        return false;
    };
    // The menu just closed (it is closed here) is no area over the canvas, though egui still
    // counts it visible for a frame: a click where it was opens it again.
    let over_area = over_floating_ui(ctx, st, at);
    if !canvas.contains(at) || over_area || st.chrome.on_panel_edge(at) {
        return false;
    }
    let screen = crate::geom::v2((at.x - canvas.min.x) as f64, (at.y - canvas.min.y) as f64);
    let world = ed.screen_to_world(screen);
    if crate::editor::hit::hit_test(&ed.doc, world, ed.camera.zoom).is_some() {
        return false;
    }
    st.canvas_menu = Some(at);
    true
}

/// The canvas menu's egui area.
const CANVAS_MENU: &str = "canvas-menu";

/// The canvas menu's egui layer.
fn canvas_menu_layer() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Foreground, CANVAS_MENU.into())
}

/// The canvas's own menu (DESIGN.md): Background (None, Dots, Lines), Zoom to Fit and
/// Collapse or Restore Panels. A choice, Escape or a press anywhere else closes it; while it is
/// open, the hosts keep presses and Escape from the editor (`UiState::captures_input`).
fn canvas_menu(
    ctx: &egui::Context,
    ed: &mut Editor,
    st: &mut UiState,
    canvas: egui::Rect,
    opened: bool,
) {
    use crate::render::scene::Background;
    let Some(at) = st.canvas_menu else {
        return;
    };
    if !canvas.contains(at) {
        st.canvas_menu = None;
        return;
    }
    let mut close = false;
    let shown = egui::Area::new(CANVAS_MENU.into())
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(8.0)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.set_min_width(180.0);
                    section(ui, "Background");
                    for b in Background::ALL {
                        if ui.radio(st.background == b, b.label()).clicked() {
                            st.set_background(b);
                            close = true;
                        }
                    }
                    ui.separator();
                    let fit = egui::Button::new("Zoom to Fit")
                        .shortcut_text(shortcuts::chord_text(Action::ZoomFit));
                    if ui.add(fit).clicked() {
                        ed.run(Action::ZoomFit);
                        close = true;
                    }
                    let panels = if st.panels.collapsed() {
                        "Restore Panels"
                    } else {
                        "Collapse Panels"
                    };
                    if ui.button(panels).clicked() {
                        st.panels.toggle_collapsed();
                        close = true;
                    }
                });
        });
    let (escape, pressed_at) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.pointer
                .any_pressed()
                .then(|| i.pointer.interact_pos())
                .flatten(),
        )
    });
    // The click that opened it is no press elsewhere.
    let outside = !opened && pressed_at.is_some_and(|p| !shown.response.rect.contains(p));
    if close || escape || outside {
        st.canvas_menu = None;
    }
}

/// Menu ▸ Background: the same three patterns as the canvas's own menu.
fn background_menu(ui: &mut Ui, st: &mut UiState) {
    use crate::render::scene::Background;
    for b in Background::ALL {
        if ui.radio(st.background == b, b.label()).clicked() {
            st.set_background(b);
            ui.close();
        }
    }
}

/// About & notices: the licences and notices every distribution carries (the browser page's
/// About says the same).
pub const NOTICES: &str = "OpenPaperDesign is a fork of paper-rs (MIT OR Apache-2.0). \
    Inter font: SIL Open Font License 1.1. Not affiliated with or endorsed by Paper. The full \
    licence texts and third-party notices ship with every distribution (LICENSE-MIT, \
    LICENSE-APACHE, THIRD_PARTY.md, OFL.txt).";

fn about(ctx: &egui::Context, st: &mut UiState, room: egui::Rect) {
    let width = 420.0f32.min(room.width() - 40.0).max(160.0);
    egui::Area::new("about".into())
        .order(egui::Order::Foreground)
        .fade_in(false)
        .pivot(Align2::CENTER_CENTER)
        .fixed_pos(room.center())
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(12.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("About & notices").font(theme::semibold(15.0)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Done").on_hover_text("Close (Esc)").clicked() {
                                st.show_about = false;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(NOTICES)
                            .size(theme::SECONDARY + 1.0)
                            .color(p.secondary),
                    );
                });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.show_about = false;
    }
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

/// Until the zoom group was measured: its width besides the level's text, whole and level only
/// (generous guesses; a new area's first frame is egui's invisible sizing pass anyway).
const ZOOM_FULL_GUESS: f32 = 90.0;
const ZOOM_LEVEL_GUESS: f32 = 30.0;

/// Zoom out, the zoom level and zoom in, at the canvas's lower left (clear of the rail; beside
/// it, at the top, on a short canvas). The level opens Fit, selection and 100%. The group stays
/// inside the canvas: its room (from where it starts to the canvas's inset) is judged from its
/// measured width with the level's current text, so neither widened panels nor a longer
/// percentage push it into a panel. Without room for the group only the level is shown, its
/// menu holding Zoom In and Zoom Out too; without room even for that (a canvas narrower than
/// the rail and the level), there is no zoom control and the menu and shortcuts still zoom.
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
    let level_text = format!("{:.0}%", ed.camera.zoom * 100.0);
    let font = theme::medium(theme::SECONDARY + 1.0);
    let text_w = ctx
        .layer_painter(egui::LayerId::background())
        .layout_no_wrap(level_text.clone(), font.clone(), Color32::PLACEHOLDER)
        .size()
        .x;
    // The group's width besides the level's text, as last measured (egui's own memory).
    let (full_id, level_id) = (egui::Id::new("zoom-full"), egui::Id::new("zoom-level"));
    let measured = |id, guess: f32| ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or(guess);
    let room = canvas.right() - rail::INSET - pos.x;
    let compact = measured(full_id, ZOOM_FULL_GUESS) + text_w > room;
    if compact && measured(level_id, ZOOM_LEVEL_GUESS) + text_w > room {
        return;
    }
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
                        let out = (!compact).then(|| {
                            action_button(ui, ed, theme::Icon::Minus, "Zoom out", Action::ZoomOut)
                        });
                        let level = ui
                            .menu_button(
                                RichText::new(level_text).font(font).color(p.secondary),
                                |ui| {
                                    if compact {
                                        menu_item(ui, ed, "Zoom In", Action::ZoomIn);
                                        menu_item(ui, ed, "Zoom Out", Action::ZoomOut);
                                        ui.separator();
                                    }
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
                        let inn = (!compact).then(|| {
                            action_button(ui, ed, theme::Icon::Plus, "Zoom in", Action::ZoomIn)
                        });
                        parts = (out, Some(level.rect), inn);
                    });
                });
        });
    let rest = r.response.rect.width() - text_w;
    ctx.data_mut(|d| d.insert_temp(if compact { level_id } else { full_id }, rest));
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
                                ui.menu_button("Background", |ui| background_menu(ui, st));
                                ui.separator();
                                if ui.button("About & notices").clicked() {
                                    st.show_about = true;
                                    ui.close();
                                }
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
        |ui| file_identity(ui, ed, st, tier),
    );
    // 1px separator under the header.
    ui.painter().hline(
        bar.x_range(),
        bar.bottom() - 0.5,
        egui::Stroke::new(1.0, p.separator),
    );
}

/// The longest file name, in characters (the app library's rename and canvas-save rule).
pub const MAX_FILE_NAME: usize = 60;

/// A finished title draft as a file name: trimmed, 1 to `MAX_FILE_NAME` characters, without
/// control characters; `None` when it isn't one (the old name then stays).
pub fn file_name(draft: &str) -> Option<String> {
    let n = draft.trim();
    let ok =
        !n.is_empty() && n.chars().count() <= MAX_FILE_NAME && !n.chars().any(char::is_control);
    ok.then(|| n.to_string())
}

/// The title's inline rename field. Enter or a click elsewhere (losing focus) finishes it:
/// a valid changed name becomes the file's name in this view (the host saves it with the
/// canvas); Escape, an empty or invalid name drop it, and the name stays as it was.
fn file_title_field(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let Some(draft) = st.file_title_draft.as_mut() else {
        return;
    };
    let width = ui.available_width().clamp(80.0, 360.0);
    let r = ui.add(
        egui::TextEdit::singleline(draft)
            .char_limit(MAX_FILE_NAME)
            .desired_width(width)
            .horizontal_align(egui::Align::Center)
            .font(theme::semibold(theme::BODY)),
    );
    st.file_title_field = Some(r.id);
    // As the layer and page renames: the loss is checked before focus is asked for again.
    if r.lost_focus() {
        let text = st.file_title_draft.take().unwrap_or_default();
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return;
        }
        match file_name(&text) {
            Some(n) if n != st.project_name => st.project_name = n,
            Some(_) => {}
            None => {
                ed.status = format!(
                    "A file name has 1 to {MAX_FILE_NAME} characters, without control \
                     characters. The name wasn't changed."
                )
            }
        }
    } else if !r.has_focus() {
        r.request_focus();
    }
}

/// The page line's menu: the shown page's own actions from the pages panel, run the same way
/// (open fields are finished first). Rename opens that page's rename in the pages list.
fn page_actions(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, tier: Tier) {
    let active = ed.active_page;
    if ui.button("Rename page…").clicked() {
        st.pages.collapsed = false;
        st.panels.open(NarrowPanel::Layers, tier);
        st.pages.renaming = Some((active, ed.page_name().to_string()));
        ui.close();
    }
    if ui.button("Duplicate page").clicked() {
        pages::run_action(ui.ctx(), ed, st, pages::PageAction::Duplicate(active));
        ui.close();
    }
    if ui.button("Add page").clicked() {
        pages::run_action(ui.ctx(), ed, st, pages::PageAction::Add);
        ui.close();
    }
    if ui
        .add_enabled(ed.pages.len() > 1, egui::Button::new("Delete page"))
        .clicked()
    {
        pages::run_action(ui.ctx(), ed, st, pages::PageAction::Delete(active));
        ui.close();
    }
}

/// The file's name (upstream's project title) over its page name and save state, each
/// truncated to the room it has. Where the host saves the name (`UiState::file_rename`), a click
/// on it starts its inline rename; the page line opens the shown page's actions.
fn file_identity(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, tier: Tier) {
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
    if st.file_title_draft.is_some() {
        file_title_field(ui, ed, st);
    } else if st.file_rename {
        let r = ui
            .add(
                egui::Label::new(title_text(title.clone()))
                    .truncate()
                    .sense(egui::Sense::click()),
            )
            .on_hover_text("Rename file");
        if r.clicked() {
            st.file_title_draft = Some(title);
        }
    } else {
        ui.add(egui::Label::new(title_text(title)).truncate());
    }
    let page = ui
        .add(
            egui::Label::new(RichText::new(sub).size(theme::SECONDARY).color(p.secondary))
                .truncate()
                .sense(egui::Sense::click()),
        )
        .on_hover_text("Page actions");
    egui::Popup::menu(&page).show(|ui| page_actions(ui, ed, st, tier));
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

/// A status line (feedback, refusals and errors, shown until the editor replaces it) at the
/// canvas's lower edge, right of each control laid out this frame that reaches the canvas's
/// lower half, where the line sits and wraps upward: the zoom at the lower left, and on a short
/// canvas the rail, which then runs down the whole left side. It never covers them, at any
/// height, and has the rest of the canvas's width.
fn status(ctx: &egui::Context, ed: &Editor, canvas: egui::Rect, near: [Option<egui::Rect>; 2]) {
    if ed.status.is_empty() {
        return;
    }
    let left = near
        .iter()
        .flatten()
        .filter(|r| r.bottom() > canvas.center().y)
        .map(|r| r.right() + 12.0)
        .fold(canvas.left() + 12.0, f32::max);
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
