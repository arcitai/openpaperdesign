//! egui chrome: the full-height left panel (All Projects and its menu, the Canvas's title, the
//! Design/Theme switch over the Papers, layers and library or the Theme), the full-height Design
//! panel, the floating tool rail and the bars that float over the canvas's top corners while a
//! panel is closed, and the projects home screen.

pub mod design_system;
pub mod field_drafts;
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
    /// The canvas's background pattern (DESIGN.md: None by default; Dots or Grid from the
    /// canvas's menu, the menu's Background, the inspector's View, View and the palette). A view
    /// preference: never the document, its undo, its saves, snapping or an export. Set it with
    /// [`UiState::set_background`].
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
    /// Name of the open project: the Canvas's name in its identity (the left panel's top, or
    /// the compact identity over the canvas while that panel is closed).
    pub project_name: String,
    /// A host that saves the file's name (the web editor, through its canvas save) opts in:
    /// the Canvas's name then renames inline (Rename Canvas). Off by default: a native window
    /// has no such save, so its name stays a label rather than suggesting a rename it can't
    /// keep.
    pub file_rename: bool,
    /// The Canvas name's unfinished rename: Enter or a click elsewhere finishes it into
    /// `project_name`, Escape drops it. Not the file's name until then.
    pub file_title_draft: Option<String>,
    /// The Canvas name field's widget id, as of the last frame it was drawn (like
    /// `UiState::rename_field`): lets a host judge exactly that field by its draft.
    pub file_title_field: Option<egui::Id>,
    /// Autosave state for the identity's Paper line, e.g. "Saved" or "Saving…".
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
    /// Where the panels' top-row controls, the identity and the zoom were laid out in the last
    /// frame (no behaviour; for hosts' checks and tests, like `PaletteUi::search_id`).
    pub chrome: Chrome,
    /// The person's appearance: System (the default, following the OS appearance as
    /// `theme::apply` set it), Light or Dark (Menu ▸ Appearance). A view preference, never part
    /// of the canvas; the chrome and the canvas display follow it (DESIGN.md).
    pub appearance: egui::ThemePreference,
    /// The left panel's switch (DESIGN.md, revision 20): Design (Papers, Layers, Library) or
    /// Theme (the design system: fonts, the Paper's text styles and colours, tokens and the
    /// project's design source). Not the appearance.
    pub left_tab: LeftTab,
    /// The width of the Canvas title's slot in the open left panel's top row, as last laid out
    /// (0 until then): the floating bar gives its title the same slot, so its controls keep
    /// their places when the panel opens or closes.
    pub left_name_w: f32,
    /// The selection as the last frame saw it: a different one is a new selection, which may
    /// reveal the Design panel (`Panels::arranged`).
    pub seen_selection: Vec<NodeId>,
    /// When (egui's clock, seconds) the selection last changed: a reveal that would cover the
    /// canvas waits [`COVER_WAIT`] from then (`Panels::hold_cover`).
    pub selected_at: f64,
}

/// How long (seconds) after a selection a reveal that covers the canvas (a sheet) waits: the
/// editor's double-click window, so a double-click's second press (which edits a text or a
/// path) still reaches the canvas.
pub const COVER_WAIT: f64 = 0.4;

/// The left panel's two views (see [`UiState::left_tab`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeftTab {
    #[default]
    Design,
    Theme,
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

/// Whether an egui popup is open (run147): a menu or one of its submenus, a context menu, a
/// choice's list. While one is, the keyboard and a press on the canvas are its, as they are an
/// open flyout's or the canvas menu's (`UiState::captures_input`): egui moves the focus, chooses
/// and closes it (Escape, a press elsewhere), and both hosts give neither the key nor the
/// press to the editor, so nothing in the drawing moves, is deleted, typed into or panned.
/// egui's own record (`Popup::is_any_open`), so it is right for every popup the UI shows.
pub fn popup_open(ctx: &egui::Context) -> bool {
    egui::Popup::is_any_open(ctx)
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
    /// At most one panel, as an opaque sheet filling the surface.
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
///
/// The Design panel is also revealed for a selection (revision 20, [`Panels::arranged`]), at
/// every width: where the person closed it, selecting or creating something shows it, and
/// clearing the selection hides it again. That is never one of the choices: the closed choice
/// (and the left panel's) stays as it was, and an open (pinned) Design panel simply stays open.
/// A toggle always wins over a reveal: it ends one that is shown, or one not shown yet.
#[derive(Debug, Default, Clone)]
pub struct Panels {
    layers: Option<bool>,
    design: Option<bool>,
    /// The side opened last: what a single-panel tier shows when both are chosen.
    last: Option<NarrowPanel>,
    collapsed: bool,
    /// A reveal is wanted for the current selection (it shows once nothing holds it).
    revealed: bool,
    /// The reveal shows the Design panel, as last laid out (`Panels::settled`).
    showing: bool,
    /// The person closed a revealed Design panel: it stays hidden until the selection clears.
    dismissed: bool,
    /// The reveal stands in for the open left panel (one panel fits), as last arranged.
    swapped: bool,
    /// The docked surface fits the left panel, the Design panel and a usable canvas side by
    /// side (`BOTH_DOCKED`), as last arranged.
    both_fit: bool,
    /// The canvas is still the selection's (see [`Panels::hold_cover`]): a reveal that would
    /// cover it waits.
    cover_held: bool,
}

/// From this surface width a docked reveal shows the Design panel beside the open left panel
/// (240 + 260 px panels and at least 300 px of canvas); narrower, it stands in for it.
pub const BOTH_DOCKED: f32 = 800.0;

impl Panels {
    /// The arrangement to lay out for a surface `width` px wide of tier `tier`:
    /// [`Panels::wanted`], with the Design panel revealed for a selection where the person
    /// closed it. A new selection (`changed` to a non-empty `selected`) starts a reveal unless
    /// it was dismissed; clearing the selection ends it. Where it goes, by tier:
    /// - wide: beside whatever is open;
    /// - docked: beside the canvas, or beside the open left panel when the surface fits both
    ///   (`BOTH_DOCKED`), else in the left panel's place, which comes back when it ends;
    /// - sheet: in the open left sheet's place (it comes back the same way), or over the
    ///   canvas, once the canvas is done with the selection ([`Panels::hold_cover`]); its way
    ///   back returns to the canvas, the selection kept.
    ///
    /// Collapse Panels never reveals. Nothing here changes the choices.
    pub fn arranged(
        &mut self,
        tier: Tier,
        selected: bool,
        changed: bool,
        width: f32,
    ) -> Arrangement {
        let base = self.wanted(tier);
        self.selection(selected, changed);
        self.swapped = false;
        self.showing = false;
        self.both_fit = tier == Tier::Docked && width >= BOTH_DOCKED;
        let covers = tier == Tier::Sheet && !base.layers;
        if !self.revealed || self.collapsed || base.design || (covers && self.cover_held) {
            return base;
        }
        let mut a = base;
        a.design = true;
        self.showing = true;
        if base.layers && tier != Tier::Wide && !self.both_fit {
            a.layers = false;
            self.swapped = true;
        }
        a
    }

    /// A selection (non-empty `selected`, new if `changed`) starts a reveal unless it was
    /// dismissed; none ends it. `draw` notes it before the frame's commands run, so an explicit
    /// request in the same frame (Rename showing the layer rows) ends the new reveal rather
    /// than being overridden by it; [`Panels::arranged`] notes it again (the same answer).
    pub fn selection(&mut self, selected: bool, changed: bool) {
        if !selected {
            self.revealed = false;
            self.dismissed = false;
        } else if changed && !self.dismissed {
            self.revealed = true;
        }
    }

    /// Whether the Design panel is shown only for the selection (see [`Panels::arranged`]).
    pub fn revealed(&self) -> bool {
        self.showing
    }

    /// Whether the canvas is still the selection's (a double-click's second press may follow;
    /// a text or path draft is open on it): a reveal that would cover the canvas waits, and
    /// shows once it isn't. Set before [`Panels::arranged`] each frame.
    pub fn hold_cover(&mut self, held: bool) {
        self.cover_held = held;
    }

    /// What the frame actually laid out (`shown`, which may still wait for a field or a
    /// gesture): the toggles then act on the reveal the person sees, not one not shown yet.
    pub fn settled(&mut self, shown: Arrangement) {
        self.showing &= shown.design;
        self.swapped &= self.showing && !shown.layers;
    }

    /// Whether the reveal stands in for the open left panel (which comes back when it ends).
    pub fn swapped(&self) -> bool {
        self.swapped
    }

    /// End the reveal until the selection clears.
    fn dismiss(&mut self) {
        self.revealed = false;
        self.showing = false;
        self.dismissed = true;
        self.swapped = false;
    }

    /// The person's own Design choice: open, closed, or never chosen (the tier's default).
    pub fn design_choice(&self) -> Option<bool> {
        self.design
    }

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

    /// The toggle for `side` (in its panel's top row, or floating over the canvas while that
    /// panel is closed) on a surface of tier `tier`.
    pub fn toggle(&mut self, side: NarrowPanel, tier: Tier) {
        if self.showing && side == NarrowPanel::Design {
            // Closing a Design panel shown for the selection: hidden until the selection
            // clears, and the person's closed choice stays as it was (an open left panel the
            // reveal stood in for comes back).
            self.dismiss();
            return;
        }
        if self.revealed && !self.showing {
            // A reveal not on screen (it waits for a gesture, a field or the canvas, or an open
            // Design panel the person chose covers it): the person's own toggle wins, so it
            // ends here rather than appear after it (or keep a closed Design panel open).
            self.dismiss();
        }
        if self.revealed && side == NarrowPanel::Layers && tier != Tier::Wide {
            if self.swapped {
                // The left panel the reveal stood in for: it comes back as it was.
                self.dismiss();
                return;
            }
            if !self.wanted(tier).layers && !self.both_fit {
                // Opening the left panel where only one fits: the reveal yields to it.
                self.dismiss();
            }
        }
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
            NarrowPanel::Layers => shown.layers && !self.swapped,
            NarrowPanel::Design => shown.design || self.showing,
            NarrowPanel::Canvas => true,
        };
        if !on {
            self.toggle(side, tier);
        }
    }
}

/// The width tiers (see [`UiState::narrow`]): below `below` logical px of width the surface has
/// room for one side panel at a time, docked beside the canvas from `dock_from` px and a sheet
/// filling the surface below that. The same panels, commands and canvas are drawn; only where.
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

/// The arrangement to lay out this frame (see [`NarrowLayout`]). While `hold` (an editor pointer
/// gesture owns the canvas: a drag, a creation, a marquee) nothing changes either, so the
/// canvas never moves or resizes under the pointer; the change follows its release.
fn settle(ctx: &egui::Context, n: &mut NarrowLayout, want: Arrangement, hold: bool) -> Arrangement {
    if !n.laid_out {
        // The first frame lays out what the surface needs at once.
        n.laid_out = true;
        n.shown = Some(want);
    } else if hold && n.shown != Some(want) {
        n.settling = false;
        ctx.request_repaint();
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

/// Where the chrome's controls and the zoom were laid out (see [`UiState::chrome`]).
#[derive(Debug, Clone, Default)]
pub struct Chrome {
    /// All Projects: the square control whose menu starts with Back to All Projects.
    pub home: Option<egui::Rect>,
    /// The menu's control: since revision 20 the same All Projects control as `home` (there
    /// is no separate `···` button).
    pub menu: Option<egui::Rect>,
    /// The left panel's toggle ("Papers and layers"), at its bar's right end.
    pub layers: Option<egui::Rect>,
    /// The right panel's toggle ("Design"), at its bar's right end.
    pub design: Option<egui::Rect>,
    /// The Canvas's title: in the left panel's top row, or in the bar floating over the canvas
    /// while that panel is closed (the same slot).
    pub identity: Option<egui::Rect>,
    /// The canvas rectangle (`draw`'s result).
    pub canvas: Option<egui::Rect>,
    /// The whole surface the chrome lays out in: menus fit their height to it (they scroll).
    pub surface: Option<egui::Rect>,
    /// While the left panel is closed: its top row floating at the canvas's top left (All
    /// Projects, the Canvas's title and the toggle).
    pub left_float: Option<egui::Rect>,
    /// While the right panel is closed: its top row floating at the canvas's top right
    /// (Export, the zoom and the toggle).
    pub right_float: Option<egui::Rect>,
    /// The layer row that shows the canvas's hover (see `canvas_hover_row`): highlighted only,
    /// never selected, scrolled to or expanded.
    pub hover_row: Option<NodeId>,
    /// The Export control (its menu: PNG, SVG, PDF, Copy as PNG or SVG) in the right bar.
    pub export: Option<egui::Rect>,
    /// The zoom group (−, percentage, +) in the right bar, left of the Design toggle.
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

/// A panel's list header (Papers, Layers, the right panel's Design): 11pt Inter Medium in the
/// secondary colour, quieter than the rows it heads.
pub(crate) fn section(ui: &mut Ui, title: &str) {
    ui.label(section_text(title, theme::pal(ui).secondary));
}

/// [`section`]'s text in `color` (a tab that is on uses the text colour).
pub(crate) fn section_text(title: &str, color: Color32) -> RichText {
    RichText::new(title)
        .font(theme::medium(theme::SECONDARY))
        .color(color)
}

/// An inspector section's title (Fill, Stroke, Paper, View…): the body size in Inter Medium,
/// in the text colour, as Paper's own property groups are titled.
pub(crate) fn title_text(text: impl Into<String>) -> RichText {
    RichText::new(text).font(theme::medium(theme::BODY))
}

/// The Canvas's name: the panel's one title, 13pt Inter SemiBold.
fn name_font() -> egui::FontId {
    theme::semibold(theme::TITLE)
}

/// Lay out the chrome and return the remaining canvas rectangle (logical px).
pub fn draw(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) -> egui::Rect {
    // The chosen appearance, applied with egui's own theme preference (the next frame uses it).
    // The default, System, is what `theme::apply` set: unchanged windows lay out as before.
    if ui.ctx().options(|o| o.theme_preference) != st.appearance {
        ui.ctx().set_theme(st.appearance);
        ui.ctx().request_repaint();
    }
    // A line height or letter spacing typed in Theme and not yet entered, whose field wasn't
    // drawn last frame (Design chosen, panels collapsed, UI hidden): finished as losing focus
    // finishes it, as the renames are (R171 panel finding).
    design_system::begin_frame(ui.ctx(), ed);
    field_drafts::begin_frame(ui.ctx());
    if let Some(hv) = &mut st.home {
        home::draw(ui, hv, st.native_chrome, st.translucent_sidebar);
        return egui::Rect::NOTHING;
    }
    // The canvas the last frame laid out: where a canvas hover can be (`canvas_hover_row`).
    let last_canvas = st.chrome.canvas;
    st.chrome = Chrome::default();
    st.chrome.surface = Some(ui.available_rect_before_wrap());
    let tier = st.narrow.tier(ui.available_rect_before_wrap().width());
    // A new selection is noted before the frame's commands, so an explicit request among them
    // (Rename shows the layer rows) wins over the Design reveal the selection starts.
    let changed = ed.selection != st.seen_selection;
    let now = ui.input(|i| i.time);
    if changed {
        st.seen_selection = ed.selection.clone();
        st.selected_at = now;
    }
    st.panels.selection(!ed.selection.is_empty(), changed);
    for ev in std::mem::take(&mut ed.ui_events) {
        use crate::editor::commands::UiEvent;
        match ev {
            UiEvent::TogglePalette => st.palette.toggle(),
            UiEvent::Rename(id) => {
                if let Some(n) = ed.doc.get(id) {
                    st.renaming = Some((id, n.name.clone()));
                    st.show_library = false;
                    st.left_tab = LeftTab::Design;
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
        // The identity isn't drawn either: the name's draft is finished as losing focus does.
        finish_title_draft(st);
        st.rail.close();
        st.canvas_menu = None;
        let canvas = ui.available_rect_before_wrap();
        palette::draw(ui.ctx(), ed, st, canvas);
        return canvas;
    }
    // The person's choices, with the Design panel revealed for a new selection where they
    // closed it (revision 20); settled like any other arrangement change, so it waits for a
    // field that has the keyboard and for a pointer gesture on the canvas. One that would cover
    // the canvas (a sheet) also waits while the canvas is still the selection's.
    let wait = COVER_WAIT - (now - st.selected_at);
    let drafting = ed.text_edit.is_some() || ed.pen.is_some() || ed.path_edit.is_some();
    st.panels.hold_cover(wait > 0.0 || drafting);
    if tier == Tier::Sheet && wait > 0.0 && !ed.selection.is_empty() {
        let delay = std::time::Duration::from_secs_f64(wait);
        ui.ctx().request_repaint_after(delay);
    }
    let width = ui.available_rect_before_wrap().width();
    let want = st
        .panels
        .arranged(tier, !ed.selection.is_empty(), changed, width);
    let shown = settle(ui.ctx(), &mut st.narrow, want, ed.is_busy());
    st.panels.settled(shown);
    let p = theme::pal(ui);
    let sidebar_fill = if st.translucent_sidebar {
        Color32::TRANSPARENT
    } else {
        p.sidebar
    };
    // No header across the surface (DESIGN.md, revision 18): both panels run its full height,
    // each with its own top row in the safe area; a closed panel's controls float over the
    // canvas instead.
    let room = ui.available_rect_before_wrap();
    st.chrome.hover_row = canvas_hover_row(ui.ctx(), ed, st, last_canvas);
    let layers_frame = egui::Frame::NONE
        .fill(sidebar_fill)
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 0,
            bottom: 8,
        });
    let design_frame = egui::Frame::NONE
        .fill(p.inspector)
        .inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 0,
            bottom: 10,
        });
    match shown.tier {
        Tier::Wide => {
            if shown.layers {
                egui::Panel::left("layers")
                    .resizable(true)
                    .default_size(240.0)
                    .size_range(180.0..=420.0)
                    .frame(layers_frame)
                    .show(ui, |ui| left_panel(ui, ed, st, shown, false));
                st.chrome.panel_edges.extend(panel_edge(ui, "layers"));
            }
            if shown.design {
                egui::Panel::right("design")
                    .resizable(true)
                    .default_size(260.0)
                    .size_range(220.0..=420.0)
                    .frame(design_frame)
                    .show(ui, |ui| right_panel(ui, ed, st, shown, false));
                st.chrome.panel_edges.extend(panel_edge(ui, "design"));
            }
        }
        Tier::Docked | Tier::Sheet => {
            // Docked: the panel at its usual width, the canvas keeps the rest. A sheet fills
            // the surface (opaque; it takes its own input), carries both toggles and the way
            // home, and says how to get back to the canvas.
            let sheet = shown.tier == Tier::Sheet;
            let width = |usual: f32| if sheet { room.width() } else { usual };
            // The panel's own widgets get the same ids in both tiers whether or not the sheet's
            // back row comes first (egui numbers widgets in order), so a focused field and its
            // draft survive docked ↔ sheet.
            let body = |ui: &Ui| egui::UiBuilder::new().id(ui.id().with("panel-body"));
            if !sheet && shown.layers && shown.design {
                // A docked reveal where both fit (`BOTH_DOCKED`): the open left panel and the
                // Design panel, each at its usual width, with the same ids as alone (so the
                // left panel's fields keep focus when the Design panel joins it).
                egui::Panel::left("layers-narrow")
                    .resizable(false)
                    .exact_size(240.0)
                    .frame(layers_frame)
                    .show(ui, |ui| {
                        let add = |ui: &mut Ui| left_panel(ui, ed, st, shown, false);
                        ui.scope_builder(body(ui), add);
                    });
                egui::Panel::right("design-narrow")
                    .resizable(false)
                    .exact_size(260.0)
                    .frame(design_frame)
                    .show(ui, |ui| {
                        let add = |ui: &mut Ui| right_panel(ui, ed, st, shown, false);
                        ui.scope_builder(body(ui), add);
                    });
            } else {
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
                                let add = |ui: &mut Ui| left_panel(ui, ed, st, shown, sheet);
                                ui.scope_builder(body(ui), add);
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
                                let add = |ui: &mut Ui| right_panel(ui, ed, st, shown, sheet);
                                ui.scope_builder(body(ui), add);
                            });
                    }
                    NarrowPanel::Canvas => {}
                }
            }
        }
    }
    // Whether the rows are about to be shown is asked again here, after the panels' widgets ran:
    // one of them (a Rename Paper… in a menu) may just have opened them for a rename it asked
    // for, which must wait for its row rather than be finished at once (R57). Rows a reveal's
    // stand-in hides (`Panels::swapped`) aren't about to be shown.
    let rows_coming = st.panels.wanted(tier).layers && !st.panels.swapped();
    if !shown.layers && !rows_coming {
        // No layer or page rows are drawn, and none are about to be: an open rename is
        // finished as losing focus does (as when the UI is hidden).
        commit_rename(ed, st);
        pages::commit_rename(ed, st);
    }
    // The one canvas rectangle: rendering, clipping and pointer input all use it. The floating
    // controls lie over it and never change it.
    let canvas = ui.available_rect_before_wrap();
    st.chrome.canvas = Some(canvas);
    if canvas.width() > 1.0 && canvas.height() > 1.0 {
        // Before the rail: a flyout open in the last frame keeps the press its own.
        let opened = open_canvas_menu(ui.ctx(), ed, st, canvas);
        // A closed panel's top row, over the canvas's top corners: the right one first (Export,
        // the zoom and its toggle), whose left edge bounds the left one's room; the rail keeps
        // to the room under the left one.
        let right_edge = if shown.design {
            canvas.right() - rail::INSET
        } else {
            float_right(ui.ctx(), ed, st, canvas, shown).left() - 6.0
        };
        let mut tools = canvas;
        if !shown.layers {
            let r = float_left(ui.ctx(), ed, st, canvas, shown, right_edge);
            tools.min.y = r.bottom().clamp(canvas.top(), canvas.bottom());
        }
        rail::draw(ui.ctx(), ed, &mut st.rail, tools);
        status(ui.ctx(), ed, canvas, [st.chrome.zoom, st.rail.rects.rail]);
        canvas_menu(ui.ctx(), ed, st, canvas, opened);
    } else {
        // A sheet covers the canvas: no rail, and no flyout or canvas menu left open behind it.
        st.rail.close();
        st.rail.rects = Default::default();
        st.canvas_menu = None;
    }
    if st.chrome.identity.is_none() {
        // No identity is drawn (the Design sheet covers the surface): the name's draft is
        // finished as losing focus does, as when the UI is hidden.
        finish_title_draft(st);
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

/// The canvas's own menu (DESIGN.md): Background (None, Dots, Grid), Zoom to Fit and
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
                .corner_radius(10.0)
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

/// The All Projects menu's Background: the same three patterns as the canvas's own menu.
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

/// A sheet's first row: the way back (its panel's toggle does the same). Usually to the canvas;
/// on a Design sheet revealed in place of the left sheet (`Panels::swapped`), back to that
/// sheet (Layers or Theme), as it was. From a Design sheet revealed over the canvas, back to
/// the canvas with the selection kept.
fn sheet_back(ui: &mut Ui, st: &mut UiState, side: NarrowPanel, tier: Tier) {
    // The panel's frame has no top margin (its top row is the safe area); this row has its own.
    ui.add_space(6.0);
    let revealed = side == NarrowPanel::Design && st.panels.revealed();
    let (label, tip) = if revealed && st.panels.swapped() {
        let back = match st.left_tab {
            LeftTab::Design => "← Layers",
            LeftTab::Theme => "← Theme",
        };
        (
            back,
            "Close the Design panel and return to the panel you had open",
        )
    } else if revealed {
        (
            "← Canvas",
            "Close the Design panel and return to the canvas; the selection stays",
        )
    } else {
        ("← Canvas", "Close this panel and return to the canvas")
    };
    let r = ui
        .add(egui::Button::new(label).frame(false))
        .on_hover_text(tip);
    st.chrome.sheet_back = Some(r.rect);
    if r.clicked() {
        st.panels.toggle(side, tier);
    }
    ui.add_space(4.0);
}

/// The zoom level's horizontal padding around its percentage.
const ZOOM_LEVEL_PAD: f32 = 8.0;
/// Below this canvas width the floating right bar shows the zoom level only (its menu holding
/// Zoom In and Zoom Out too), so the left bar keeps room for the Canvas's title.
const ZOOM_COMPACT_BELOW: f32 = 480.0;

/// The zoom level's text and its painted width.
fn zoom_text(ui: &Ui, ed: &Editor) -> (String, f32) {
    let text = format!("{:.0}%", ed.camera.zoom * 100.0);
    let font = theme::medium(theme::SECONDARY + 1.0);
    let w = ui
        .painter()
        .layout_no_wrap(text.clone(), font, Color32::PLACEHOLDER)
        .size()
        .x;
    (text, w)
}

/// The zoom group's width for a percentage `text_w` wide: −, the level and +, or the level
/// alone (`compact`).
fn zoom_width(text_w: f32, compact: bool) -> f32 {
    let level = text_w + 2.0 * ZOOM_LEVEL_PAD;
    if compact {
        level
    } else {
        level + 2.0 * CONTROL
    }
}

/// Zoom out, the zoom level and zoom in (revision 20): in the right bar, left of the Design
/// toggle, whether that bar is the open panel's top row or floats at the canvas's top right, so
/// the zoom keeps its place when the panel opens or closes. The level opens Fit, selection and
/// 100%; `compact` shows the level only, its menu holding Zoom In and Zoom Out too.
fn zoom_group(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, compact: bool) {
    let (text, text_w) = zoom_text(ui, ed);
    let size = egui::vec2(zoom_width(text_w, compact), CONTROL);
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let mut parts = (None, None, None);
    let r = ui.allocate_ui_with_layout(size, layout, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let out =
            (!compact).then(|| action_button(ui, ed, Icon::Minus, "Zoom out", Action::ZoomOut));
        let level_size = egui::vec2(text_w + 2.0 * ZOOM_LEVEL_PAD, CONTROL);
        let level = text_button(ui, &text, level_size).on_hover_text("Zoom options");
        egui::Popup::menu(&level).show(|ui| {
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
        });
        let inn = (!compact).then(|| action_button(ui, ed, Icon::Plus, "Zoom in", Action::ZoomIn));
        parts = (out, Some(level.rect), inn);
    });
    st.chrome.zoom = Some(r.response.rect);
    (st.chrome.zoom_out, st.chrome.zoom_level, st.chrome.zoom_in) = parts;
}

/// A quiet text control `size` big (the zoom level): its text in the secondary colour, in the
/// text colour with the hover fill while hovered or focused, the pressed fill while pressed,
/// and the focus ring of the other chrome controls.
fn text_button(ui: &mut Ui, text: &str, size: egui::Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = theme::pal(ui);
        let lit = resp.hovered() || resp.has_focus();
        let fill = if resp.is_pointer_button_down_on() {
            p.pressed
        } else if lit {
            p.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, theme::RADIUS, fill);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            theme::medium(theme::SECONDARY + 1.0),
            if lit { p.text } else { p.secondary },
        );
        theme::focus_ring(ui, rect, &resp, false);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    resp
}

/// Export (revision 20, in the right bar): the existing exports and copies as one menu. It is
/// labelled Export because that is what it does; there is no sharing service behind it.
fn export_button(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    use crate::io::export::Format;
    let r = theme::quiet_icon_button(ui, Icon::Export, CONTROL, false).on_hover_text("Export");
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Export"));
    st.chrome.export = Some(r.rect);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(180.0);
        for (label, format) in [
            ("Export PNG…", Format::Png),
            ("Export SVG…", Format::Svg),
            ("Export PDF…", Format::Pdf),
        ] {
            if ui.button(label).clicked() {
                ed.requests
                    .push(crate::editor::AppRequest::ExportFormat(format));
                ui.close();
            }
        }
        ui.separator();
        menu_item(ui, ed, "Copy as PNG", Action::CopyAsPng);
        menu_item(ui, ed, "Copy as SVG", Action::CopyAsSvg);
    });
}

/// The right bar's tools after its toggle, right to left: the zoom group (compact where the
/// bar has no room for all of it), then Export.
fn right_tools(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let (_, text_w) = zoom_text(ui, ed);
    let spacing = ui.spacing().item_spacing.x;
    let compact = ui.available_width() < zoom_width(text_w, false) + 2.0 * spacing + CONTROL;
    zoom_group(ui, ed, st, compact);
    export_button(ui, ed, st);
}

/// The panels' top row height without native chrome (with it, `theme::TOOLBAR_H`, the
/// traffic lights' bar).
const BAR_H: f32 = 38.0;
/// The chrome's icon controls (All Projects, Export, the zoom's −/+, the panel toggles), and the
/// Canvas title's slot height.
const CONTROL: f32 = 26.0;
/// The left panel's content inset (its frame's margin), where its top row's first control is.
const LEFT_INSET: f32 = 10.0;
/// The right panel's content inset, where its toggle ends.
const RIGHT_INSET: f32 = 14.0;
/// A floating top row's padding (its frame's stroke adds one more point).
const FLOAT_PAD: f32 = 4.0;
/// The gap between a top row's controls.
const ROW_GAP: f32 = 2.0;
/// The Canvas title's slot in a 240 px left panel's top row (its 220 px content less All
/// Projects, the toggle and their gaps), until the panel has been laid out once.
const NAME_W_DEFAULT: f32 = 240.0 - 2.0 * LEFT_INSET - 2.0 * CONTROL - 2.0 * ROW_GAP;
/// Below this the floating bar leaves the Canvas's title out (its controls stay).
const NAME_W_MIN: f32 = 60.0;
/// The title's text starts this far into its slot, clear of All Projects.
const NAME_PAD: f32 = 6.0;

/// The height of a panel's top row (the surface's safe area).
fn top_h(st: &UiState) -> f32 {
    if st.native_chrome {
        theme::TOOLBAR_H
    } else {
        BAR_H
    }
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

fn menu_entries(ui: &mut Ui, ed: &mut Editor, entries: &[shortcuts::MenuEntry], max_h: f32) {
    for e in entries {
        match e {
            shortcuts::MenuEntry::Item(label, a) => menu_item(ui, ed, label, *a),
            shortcuts::MenuEntry::Separator => {
                ui.separator();
            }
            shortcuts::MenuEntry::Submenu(label, items) => {
                ui.menu_button(*label, |ui| {
                    scrolled(ui, max_h, |ui| menu_entries(ui, ed, items, max_h))
                });
            }
        }
    }
}

/// A menu's items in a vertical scroll area at most `max_h` high (revision 20): on a short
/// surface (320 × 240) a long menu keeps every item reachable by scrolling it, rather than
/// running past the surface's bottom. Short menus are unchanged (the area shrinks to them).
/// `max_h` is also its least height before it shrinks to its items: a popup offers its
/// contents less room than the surface has (run145: a 1280 × 800 menu stopped at about 430 px,
/// its Appearance and About out of view), so a menu that fits the surface is shown whole and
/// only a taller one scrolls.
fn scrolled(ui: &mut Ui, max_h: f32, add: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical()
        .max_height(max_h)
        .min_scrolled_height(max_h)
        .show(ui, add);
}

/// The height a menu may take on this surface (less a margin), for [`scrolled`]; `below` is
/// where it opens (its top), when known.
fn menu_height(st: &UiState, below: Option<f32>) -> f32 {
    let Some(surface) = st.chrome.surface else {
        return 600.0;
    };
    let top = below.unwrap_or(surface.top());
    (surface.bottom() - top - 24.0).max(120.0)
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

/// The left panel (DESIGN.md, revision 20), from the surface's top: its top row (All Projects
/// with its menu, the Canvas's title, and its own toggle at the right end; on a sheet, which
/// covers the canvas, the right panel's toggle too), then the Design/Theme switch over either
/// the Papers, Layers and Library (Design) or the design system (Theme). Its controls are
/// quiet; an open toggle stays pushed in.
fn left_panel(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, shown: Arrangement, sheet: bool) {
    top_row(ui, top_h(st), |ui| {
        if st.native_chrome && !sheet {
            // The traffic lights sit at the window's top left, in this row.
            ui.add_space(theme::TRAFFIC_LIGHTS_W - LEFT_INSET);
        }
        home_menu(ui, ed, st);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if sheet {
                design_toggle(ui, st, shown);
            }
            layers_toggle(ui, st, shown);
            // The title takes the rest of the row: one slot, the same one the floating bar
            // gives it, so it never moves when the panel opens or closes.
            let w = ui.available_width();
            if w >= NAME_W_MIN {
                let size = egui::vec2(w, CONTROL);
                let (slot, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                if !sheet {
                    st.left_name_w = w;
                }
                canvas_name(ui, ed, st, slot);
                st.chrome.identity = Some(slot);
            }
        });
    });
    ui.add_space(2.0);
    left_switch(ui, st);
    ui.add_space(8.0);
    match st.left_tab {
        LeftTab::Design => layers(ui, ed, st),
        LeftTab::Theme => {
            // No Paper or layer rows are drawn: an open rename is finished as losing focus
            // finishes it, on its own Paper (as the Library tab and Hide UI do).
            commit_rename(ed, st);
            pages::commit_rename(ed, st);
            design_system::panel(ui, ed);
        }
    }
}

/// The left panel's Design/Theme switch: one quiet segmented control, the chosen half raised
/// as a control on the field's surface. Design shows the Papers, Layers and Library; Theme the
/// design system. Neither is the appearance (System, Light, Dark), which stays in the menu.
fn left_switch(ui: &mut Ui, st: &mut UiState) {
    let p = theme::pal(ui);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, theme::RADIUS + 1.0, p.field);
    let half = (rect.width() - 4.0) / 2.0;
    for (i, (tab, label)) in [(LeftTab::Design, "Design"), (LeftTab::Theme, "Theme")]
        .into_iter()
        .enumerate()
    {
        let min = rect.min + egui::vec2(2.0 + i as f32 * half, 2.0);
        let r = egui::Rect::from_min_size(min, egui::vec2(half, rect.height() - 4.0));
        let resp = ui.interact(r, ui.id().with(("left-tab", i)), egui::Sense::click());
        let on = st.left_tab == tab;
        if on {
            let edge = egui::Stroke::new(1.0, p.control_border);
            ui.painter()
                .rect(r, theme::RADIUS, p.control, edge, egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, theme::RADIUS, p.hover);
        }
        let color = if on || resp.hovered() {
            p.text
        } else {
            p.secondary
        };
        let font = theme::medium(theme::BODY);
        ui.painter()
            .text(r.center(), Align2::CENTER_CENTER, label, font, color);
        theme::focus_ring(ui, r, &resp, false);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, label));
        if resp.clicked() {
            st.left_tab = tab;
        }
    }
}

/// The right panel, from the surface's top: its top row (Export and the zoom, its own toggle at
/// the right end; on a sheet, which covers the canvas, All Projects and the left panel's
/// toggle instead of the canvas's tools) over the inspector.
fn right_panel(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, shown: Arrangement, sheet: bool) {
    top_row(ui, top_h(st), |ui| {
        if sheet {
            home_menu(ui, ed, st);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            design_toggle(ui, st, shown);
            if sheet {
                layers_toggle(ui, st, shown);
            } else {
                right_tools(ui, ed, st);
            }
        });
    });
    inspector::panel(ui, ed, st);
}

/// A panel's top row: the surface's safe area, `h` high, laid out left to right.
fn top_row(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui)) {
    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), h));
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.set_min_size(rect.size());
        ui.spacing_mut().item_spacing.x = 2.0;
        add(ui);
    });
}

/// All Projects (revision 20): the square control at the left bar's start, wherever that bar is
/// (the left panel's top row, or floating over the canvas while that panel is closed). It opens
/// one menu whose first item, Back to All Projects, is the same guarded way home as before
/// (`AppRequest::Home`: the host saves and drains first, and refuses while work is unsaved).
fn home_menu(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let r = theme::quiet_icon_button(ui, Icon::Grid, CONTROL, false)
        .on_hover_text("All Projects and menu");
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "All Projects"));
    st.chrome.home = Some(r.rect);
    st.chrome.menu = Some(r.rect);
    // The menu opens under the control and scrolls within the surface's remaining height, so
    // its lower items (the upstream menus, Appearance, Background, About) stay reachable on a
    // short surface.
    let max_h = menu_height(st, Some(r.rect.bottom()));
    // Opened from the keyboard (Enter or Space on the focused control), the menu takes the focus
    // at its first item, so the arrow keys move through it, Enter or Space chooses and Escape
    // closes it, as egui's own focus does (run147).
    let by_keyboard = r.clicked() && !r.clicked_by(egui::PointerButton::Primary);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(220.0);
        scrolled(ui, max_h, |ui| all_projects_menu(ui, ed, st, by_keyboard));
    });
}

/// The All Projects menu: Back to All Projects first, then the Canvas's and the shown Paper's
/// own actions, Go to Paper, and (where the window has no native menu bar) every upstream menu,
/// Appearance, Background and About & notices. It replaces revision 18's separate `···`.
/// `focus_first`: it was just opened from the keyboard, so its first item takes the focus.
fn all_projects_menu(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, focus_first: bool) {
    // A submenu may open anywhere on the surface: it scrolls within the whole height.
    let sub_h = menu_height(st, None);
    let back = ui.button("Back to All Projects");
    if focus_first {
        back.request_focus();
    }
    if back.clicked() {
        ed.requests.push(crate::editor::AppRequest::Home);
        ui.close();
    }
    ui.separator();
    identity_menu(ui, ed, st);
    ui.menu_button("Go to Paper", |ui| {
        scrolled(ui, sub_h, |ui| go_to_paper(ui, ed, st))
    });
    if st.native_chrome {
        return;
    }
    ui.separator();
    for (title, entries) in shortcuts::menus() {
        ui.menu_button(title, |ui| {
            scrolled(ui, sub_h, |ui| menu_entries(ui, ed, &entries, sub_h))
        });
    }
    ui.separator();
    ui.menu_button("Appearance", |ui| appearance_menu(ui, st));
    ui.menu_button("Background", |ui| background_menu(ui, st));
    ui.separator();
    if ui.button("About & notices").clicked() {
        st.show_about = true;
        ui.close();
    }
}

/// The left panel's toggle, at its bar's right end (the open panel's top row, or the floating
/// bar while it is closed).
fn layers_toggle(ui: &mut Ui, st: &mut UiState, shown: Arrangement) {
    let layers = sidebar_toggle(ui, Icon::Sidebar, shown.layers, "Papers and layers");
    st.chrome.layers = Some(layers.rect);
    if layers.clicked() {
        st.panels.toggle(NarrowPanel::Layers, shown.tier);
    }
}

/// The right panel's toggle, at its bar's right end (its top row, or the floating bar at the
/// canvas's top right). It is named Design, as the panel is.
fn design_toggle(ui: &mut Ui, st: &mut UiState, shown: Arrangement) {
    let design = sidebar_toggle(ui, Icon::SidebarRight, shown.design, "Design");
    st.chrome.design = Some(design.rect);
    if design.clicked() {
        st.panels.toggle(NarrowPanel::Design, shown.tier);
    }
}

/// A side panel's toggle. The left and right ones are one mirrored pair: the sidebar glyph and
/// its mirror image at the same size, in the same quiet colours, with the same tooltip style,
/// focus ring and pressed look (an open panel's toggle stays pushed in, neutral grey rather
/// than accent), and its name and on state for assistive technology.
fn sidebar_toggle(ui: &mut Ui, icon: Icon, on: bool, tip: &str) -> egui::Response {
    let r = theme::quiet_icon_button(ui, icon, CONTROL, on).on_hover_text(tip);
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, tip));
    r
}

/// While the left panel is closed, its top row floats over the canvas's top-left corner (there
/// is no header across the surface): All Projects, the Canvas's title and the toggle, each where
/// it is in the open panel's row (the title gets the slot it has there), narrower only where
/// the right bar, ending at `right_edge`, needs the room; below `NAME_W_MIN` the title is left
/// out. Returns where it lies (the rail keeps below it).
fn float_left(
    ctx: &egui::Context,
    ed: &mut Editor,
    st: &mut UiState,
    canvas: egui::Rect,
    shown: Arrangement,
    right_edge: f32,
) -> egui::Rect {
    let traffic = st.native_chrome && canvas.left() < theme::TRAFFIC_LIGHTS_W;
    let inset = if traffic {
        theme::TRAFFIC_LIGHTS_W
    } else {
        LEFT_INSET
    };
    // The frame's padding and stroke lie around the controls.
    let edge = FLOAT_PAD + 1.0;
    let top = canvas.top() + (top_h(st) - CONTROL) / 2.0 - edge;
    let x0 = canvas.left() + inset;
    let pos = egui::pos2(x0 - edge, top);
    let open_w = if st.left_name_w > 0.0 {
        st.left_name_w
    } else if st.native_chrome {
        NAME_W_DEFAULT - (theme::TRAFFIC_LIGHTS_W - LEFT_INSET)
    } else {
        NAME_W_DEFAULT
    };
    let room = right_edge - edge - x0 - 2.0 * CONTROL - 2.0 * ROW_GAP;
    let name_w = open_w.min(room);
    let r = egui::Area::new("chrome-left".into())
        .order(egui::Order::Middle)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(10.0)
                .inner_margin(FLOAT_PAD)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = ROW_GAP;
                        home_menu(ui, ed, st);
                        if name_w >= NAME_W_MIN {
                            let size = egui::vec2(name_w, CONTROL);
                            let (slot, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                            canvas_name(ui, ed, st, slot);
                            st.chrome.identity = Some(slot);
                        }
                        layers_toggle(ui, st, shown);
                    });
                });
        });
    st.chrome.left_float = Some(r.response.rect);
    r.response.rect
}

/// While the right panel is closed, its top row floats at the canvas's top right: Export, the
/// zoom and the toggle, each where it is in the open panel's row (the toggle at the end). On a
/// narrow canvas the zoom shows its level only. Returns where it lies.
fn float_right(
    ctx: &egui::Context,
    ed: &mut Editor,
    st: &mut UiState,
    canvas: egui::Rect,
    shown: Arrangement,
) -> egui::Rect {
    let edge = FLOAT_PAD + 1.0;
    let top = canvas.top() + (top_h(st) - CONTROL) / 2.0 - edge;
    let compact = canvas.width() < ZOOM_COMPACT_BELOW;
    let r = egui::Area::new("chrome-right".into())
        .order(egui::Order::Middle)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(egui::pos2(canvas.right() - RIGHT_INSET + edge, top))
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(10.0)
                .inner_margin(FLOAT_PAD)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = ROW_GAP;
                        export_button(ui, ed, st);
                        zoom_group(ui, ed, st, compact);
                        design_toggle(ui, st, shown);
                    });
                });
        });
    st.chrome.right_float = Some(r.response.rect);
    r.response.rect
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

/// Whether another open name field than `mine` (the Canvas name's, a Paper's or a layer's
/// rename) has the keyboard. A name field asks for it only while none does: names taken up
/// together from another view (run 196) would otherwise take it from each other each frame, and
/// the one that lost it would be finished as a click elsewhere finishes it, unasked.
fn other_name_field_focused(ctx: &egui::Context, st: &UiState, mine: egui::Id) -> bool {
    let Some(f) = ctx.memory(|m| m.focused()).filter(|f| *f != mine) else {
        return false;
    };
    let open = [
        st.file_title_draft.is_some().then_some(st.file_title_field),
        st.pages.renaming.is_some().then_some(st.pages.rename_field),
        st.renaming.is_some().then_some(st.rename_field),
    ];
    open.into_iter().flatten().flatten().any(|id| id == f)
}

/// Finish the Canvas name's draft as losing focus does, where its field isn't drawn.
fn finish_title_draft(st: &mut UiState) {
    if let Some(n) = st.file_title_draft.take().as_deref().and_then(file_name) {
        st.project_name = n;
    }
}

/// The Canvas's name: the file's real name (upstream's project title), else the document's.
fn canvas_title(ed: &Editor, st: &UiState) -> String {
    if st.project_name.is_empty() {
        ed.doc.name.clone()
    } else {
        st.project_name.clone()
    }
}

/// Rename Canvas… (the title's own menu, the All Projects menu): the name's field opens in the
/// title's own slot, wherever the title is. Only where the host saves the name.
fn start_canvas_rename(ed: &Editor, st: &mut UiState) {
    if st.file_rename && st.file_title_draft.is_none() {
        st.file_title_draft = Some(canvas_title(ed, st));
    }
}

/// Rename Paper… (the title's own menu, the All Projects menu; a row's own menu and a
/// double-click in the Papers list open it in place): the shown Paper's one rename field, in its
/// row of the Papers list, which is shown for it (the left panel opened on its Design view).
/// Every entry point opens that same field, so the name is saved one way and once, and never
/// on another Paper.
fn start_paper_rename(ed: &Editor, st: &mut UiState) {
    let tier = st.narrow.shown().map_or(Tier::Wide, |a| a.tier);
    st.pages.collapsed = false;
    st.left_tab = LeftTab::Design;
    st.panels.open(NarrowPanel::Layers, tier);
    st.pages.renaming = Some((ed.active_page, ed.page_name().to_string()));
}

/// Lay `add` out in the fixed slot `rect` (already allocated by the caller), left to right and
/// vertically centred, in a child ui (`salt` names it) that allocates nothing in `ui`. Only the
/// slot takes room, so whatever it holds (a label, or a rename field, which egui makes at least
/// its interact height) never moves or resizes what follows it.
fn in_slot<R>(ui: &mut Ui, rect: egui::Rect, salt: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let slot = egui::UiBuilder::new()
        .id_salt(salt)
        .max_rect(rect)
        .layout(layout);
    let mut child = ui.new_child(slot);
    add(&mut child)
}

/// The save state the title shows after the name, while there is one worth saying: the host's
/// own (e.g. "Saving…"), or "Edited" for unsaved changes where the host says nothing. Nothing
/// once saved: the healthy state stays quiet.
fn save_hint(ed: &Editor, st: &UiState) -> Option<String> {
    if !st.save_state.is_empty() {
        (st.save_state != "Saved").then(|| st.save_state.clone())
    } else {
        ed.is_dirty().then(|| "Edited".to_string())
    }
}

/// The Canvas's title in its slot (revision 20: the one title, without a Paper line under it):
/// the name, then a quiet save state while there is one, or while it is renamed its field in
/// the same place. Where the host saves the name (`UiState::file_rename`), a click (or Enter,
/// once Tab has reached it) starts Rename Canvas; a secondary click offers both renames and the
/// shown Paper's actions.
fn canvas_name(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, rect: egui::Rect) {
    let rect = egui::Rect::from_min_max(rect.min + egui::vec2(NAME_PAD, 0.0), rect.max);
    if st.file_title_draft.is_some() {
        file_title_field(ui, ed, st, rect);
        return;
    }
    let title = canvas_title(ed, st);
    let p = theme::pal(ui);
    let hint = save_hint(ed, st).map(|h| {
        let font = egui::FontId::proportional(theme::SECONDARY);
        let w = ui
            .painter()
            .layout_no_wrap(h.clone(), font, Color32::PLACEHOLDER)
            .size()
            .x;
        (h, w)
    });
    let hint_w = hint.as_ref().map_or(0.0, |(_, w)| w + 6.0);
    let r = in_slot(ui, rect, "canvas-name", |ui| {
        let label = egui::Label::new(RichText::new(title.clone()).font(name_font())).truncate();
        let r = ui
            .scope(|ui| {
                ui.set_max_width((rect.width() - hint_w).max(0.0));
                ui.add(label.sense(egui::Sense::click()))
            })
            .inner;
        if let Some((h, _)) = &hint {
            ui.label(RichText::new(h).size(theme::SECONDARY).color(p.secondary));
        }
        r
    });
    let r = r.on_hover_text(if st.file_rename {
        "Rename Canvas"
    } else {
        title.as_str()
    });
    if st.file_rename && r.clicked() {
        st.file_title_draft = Some(title.clone());
    }
    r.context_menu(|ui| identity_menu(ui, ed, st));
}

/// Go to Paper (the All Projects menu): the Canvas's Papers, the shown one marked; choosing
/// one shows it, as its row in the Papers list does (open fields are finished first).
fn go_to_paper(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let active = ed.active_page;
    let mut chosen = None;
    for (i, page) in ed.pages.iter().enumerate() {
        if ui
            .add(egui::Button::selectable(i == active, page.name.as_str()))
            .clicked()
        {
            chosen = Some(i);
        }
    }
    if let Some(i) = chosen {
        if i != active {
            pages::run_action(ui.ctx(), ed, st, pages::PageAction::Switch(i));
        }
        ui.close();
    }
}

/// Rename Canvas… (where the host saves the name) and Rename Paper…, then the shown Paper's
/// own actions: what the title's own menu and the All Projects menu offer.
fn identity_menu(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    if st.file_rename && ui.button("Rename Canvas…").clicked() {
        start_canvas_rename(ed, st);
        ui.close();
    }
    if ui.button("Rename Paper…").clicked() {
        start_paper_rename(ed, st);
        ui.close();
    }
    ui.separator();
    paper_actions(ui, ed, st);
}

/// The Canvas name's inline field, in the name's own slot `rect`. Enter or a click elsewhere
/// (losing focus) finishes it: a valid changed name becomes the file's name in this view (the
/// host saves it with the canvas, one change); Escape, an empty or invalid name drop it, and
/// the name stays as it was.
fn file_title_field(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, rect: egui::Rect) {
    let Some(draft) = st.file_title_draft.as_mut() else {
        return;
    };
    // The whole slot (CONTROL high, at least egui's 24 px interact height a single-line field
    // takes, open or floating alike), in a child that allocates nothing outside it.
    let field = egui::TextEdit::singleline(draft)
        .char_limit(MAX_FILE_NAME)
        .desired_width(rect.width())
        .font(name_font());
    let r = in_slot(ui, rect, "canvas-name", |ui| ui.add(field));
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
                    "A Canvas name has 1 to {MAX_FILE_NAME} characters, without control \
                     characters. The name wasn't changed."
                )
            }
        }
    } else if !r.has_focus() && !other_name_field_focused(ui.ctx(), st, r.id) {
        r.request_focus();
    }
}

/// The shown Paper's own actions from the Papers list, run the same way (open fields are
/// finished first).
fn paper_actions(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let active = ed.active_page;
    if ui.button("Duplicate Paper").clicked() {
        pages::run_action(ui.ctx(), ed, st, pages::PageAction::Duplicate(active));
        ui.close();
    }
    if ui.button("Add Paper").clicked() {
        pages::run_action(ui.ctx(), ed, st, pages::PageAction::Add);
        ui.close();
    }
    if ui
        .add_enabled(ed.pages.len() > 1, egui::Button::new("Delete Paper"))
        .clicked()
    {
        pages::run_action(ui.ctx(), ed, st, pages::PageAction::Delete(active));
        ui.close();
    }
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
            let color = if on { p.text } else { p.secondary };
            let tab = egui::Label::new(section_text(label, color));
            let r = ui.add(tab.sense(egui::Sense::click()));
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
    // The row is being carried (revision 20): the grabbing hand says so while it moves.
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
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
                    .rect_stroke(r, theme::RADIUS, stroke, egui::StrokeKind::Inside);
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

/// Unfold, in the Layers tree, exactly the groups and frames that hold layer `id` (its row is
/// then drawn), and nothing else: other folded groups stay as the person left them. A rename
/// whose row isn't drawn is finished as losing focus does (`layers`), which a layer's name taken
/// up from another view never may be (R86-02, run 197).
pub fn reveal_layer(ed: &Editor, st: &mut UiState, id: NodeId) {
    let mut parent = ed.doc.get(id).and_then(|n| n.parent);
    while let Some(p) = parent {
        st.collapsed.remove(&p);
        parent = ed.doc.get(p).and_then(|n| n.parent);
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

/// The layer row that shows what the pointer is over on the canvas (DESIGN.md, linked hover):
/// the canvas's own hit (`Editor::hover`) while the pointer is in the canvas rectangle last
/// laid out and over no floating UI, so it clears the moment the pointer leaves the canvas. It
/// is the node's own row, or inside a collapsed group the nearest row shown (the tree is never
/// expanded for a hover). Only a highlight: no selection, document, history or scroll changes.
fn canvas_hover_row(
    ctx: &egui::Context,
    ed: &Editor,
    st: &UiState,
    canvas: Option<egui::Rect>,
) -> Option<NodeId> {
    let at = ctx.pointer_hover_pos()?;
    if !canvas?.contains(at) || over_floating_ui(ctx, st, at) {
        return None;
    }
    let id = ed.hover?;
    let mut chain = vec![id];
    let mut parent = ed.doc.get(id)?.parent;
    while let Some(p) = parent {
        chain.push(p);
        parent = ed.doc.get(p).and_then(|n| n.parent);
    }
    // From the root down, the first collapsed group holding it is the deepest row drawn.
    let collapsed = chain
        .iter()
        .rev()
        .find(|n| **n != id && st.collapsed.contains(n));
    Some(collapsed.copied().unwrap_or(id))
}

/// One sidebar row, drawn like an NSOutlineView row: 24pt tall (the lists stay compact; the
/// inspector's sections are the airy part), `theme::RADIUS` selection as an accent tint, subtle
/// hover (the pointer's over the row, or the canvas's over its layer), disclosure chevron,
/// 14pt symbol, 12pt name, and eye/lock on hover.
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
    // Selection is a soft accent tint (the name keeps its text colour), as the open panel
    // toggles are; the current Paper's row is grey instead (it is no selection). The canvas's
    // hover only lights the row, as the pointer does.
    let linked = st.chrome.hover_row == Some(id);
    if selected {
        ui.painter()
            .rect_filled(row, theme::RADIUS, theme::selected_fill(ui));
    } else if hovered || linked {
        ui.painter().rect_filled(row, theme::RADIUS, p.hover);
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
    // Symbols are quiet; components and instances keep the accent that tells them apart.
    let icon = node_icon(&n);
    let tint = if matches!(icon, Icon::Component | Icon::Instance) {
        theme::accent()
    } else {
        fg2
    };
    theme::paint_icon(ui.painter(), sq(x, 14.0), icon, tint);
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
            ui.painter().rect_filled(r, theme::RADIUS, p.hover);
        }
        let color = if b.hovered() { p.text } else { fg2 };
        theme::paint_icon(ui.painter(), r.shrink(3.0), icon, color);
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
        } else if !r.has_focus() && !other_name_field_focused(ui.ctx(), st, r.id) {
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
/// lower half, where the line sits and wraps upward: the rail (on a short canvas it runs down
/// the whole left side); the zoom, in the right bar since revision 20, counts only if it ever
/// reaches that half. It never covers them, at any height, and has the rest of the canvas's
/// width.
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
