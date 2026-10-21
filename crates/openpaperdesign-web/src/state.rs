//! Everything the browser page does with the upstream application, apart from the GPU surface
//! and the JS bindings (`web.rs`). It is plain Rust, so it is tested on the host.
//!
//! The page hosts upstream's own workspace: `ui::draw` (layers, inspector, pages, floating
//! tools, palette, menus, help) laid out by `render::frame::layout`, exactly as the native
//! app's `App` does with winit. DOM input goes to egui first; pointer input reaches the editor
//! only over the canvas rectangle that layout returned, and not over a floating egui area,
//! like `App::over_canvas`; keys reach the editor only while no egui text field has keyboard
//! focus. What upstream asks the app for (`AppRequest`) is handled here, one case each.
//!
//! Redraw rule: the scene is a function of editor state plus time (only the laser trail
//! animates, see `Editor::animating`), and egui asks for its own repaints. Editor state only
//! changes inside the methods below, so every method that can change it marks a redraw.
//! `Editor::revision` alone is not enough: selection, hover, marquee, handles and modifier
//! changes don't bump it.

use crate::editor::shortcuts::{self, Action};
use crate::editor::{AppRequest, Button, Editor, InputEvent, Key};
use crate::geom::v2;
use crate::io::export::{self, Format, Scope, Spec};
use crate::keys;
use crate::model::{Color, Document};
use crate::project::PageData;
use crate::project::format::{page_from_json, page_to_json};
use crate::render::frame::{self, Layout};
use crate::ui::{self, UiState};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// What the page itself must do for the editor (see [`WebState::take_request`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRequest {
    /// Save now (upstream's Save): the page runs its usual checked save.
    Save,
    /// Back to the canvases (upstream's Home, New and Open): the page's own navigation, which
    /// saves and drains first, or says where that is when it has none.
    Home,
    /// The person chose another appearance (Menu ▸ Appearance): the page keeps the view
    /// preference ([`WebState::appearance`]) where it can. Nothing about the canvas changed.
    Appearance,
    /// The person chose another canvas background (the canvas's menu, Menu ▸ Background, View,
    /// the palette or the grid shortcut): the page keeps the view preference
    /// ([`WebState::background`]) where it can. Nothing about the canvas changed.
    Background,
}

impl HostRequest {
    pub fn name(self) -> &'static str {
        match self {
            HostRequest::Save => "save",
            HostRequest::Home => "home",
            HostRequest::Appearance => "appearance",
            HostRequest::Background => "background",
        }
    }
}

/// An appearance preference by its stored name: `system`, `light` or `dark`.
pub fn appearance_from_name(name: &str) -> Option<egui::ThemePreference> {
    match name {
        "system" => Some(egui::ThemePreference::System),
        "light" => Some(egui::ThemePreference::Light),
        "dark" => Some(egui::ThemePreference::Dark),
        _ => None,
    }
}

pub fn appearance_name(p: egui::ThemePreference) -> &'static str {
    match p {
        egui::ThemePreference::System => "system",
        egui::ThemePreference::Light => "light",
        egui::ThemePreference::Dark => "dark",
    }
}

/// An export the page offers as a browser download.
#[derive(Debug, Clone, PartialEq)]
pub struct Download {
    pub name: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// The page's clipboard as upstream sees it (`io::clipboard::Provider`): text the page hands
/// in from a `paste` event, and text the editor wrote for the page to put on the clipboard
/// from a `copy`/`cut` event (or, best effort, the async clipboard API). No OS clipboard.
#[derive(Default)]
struct ClipboardBuf {
    /// Text of the `paste` event being handled.
    incoming: Option<String>,
    /// Text the editor wrote, not yet given to the page.
    outgoing: Option<String>,
    /// The last text the editor wrote (a menu Paste without a paste event uses it).
    last: Option<String>,
}

struct WebClipboard(Rc<RefCell<ClipboardBuf>>);

impl crate::io::clipboard::Provider for WebClipboard {
    fn read_text(&mut self) -> Option<String> {
        let mut b = self.0.borrow_mut();
        b.incoming.take().or_else(|| b.last.clone())
    }
    fn write_text(&mut self, s: &str) {
        let mut b = self.0.borrow_mut();
        b.outgoing = Some(s.to_string());
        b.last = Some(s.to_string());
    }
}

const NO_PNG: &str = "PNG export and Copy as PNG aren't available in this browser editor (they \
    need a GPU read-back it can't do yet). Export SVG or PDF instead. Nothing was exported.";
const NO_SAVE_AS: &str =
    "Duplicating a canvas isn't available in this editor. Nothing was changed.";
const NO_IMAGE: &str =
    "Placing images from files isn't available in this editor yet. Nothing was changed.";
const LIBRARY_SESSION: &str = "Library items stay only while this editor is open; they aren't \
    saved with the canvas.";

/// The workspace width (CSS px) below which upstream's narrow layout is used: under it the
/// ordinary layout's side panels (240 + 260 px by default, 180 + 220 at least) would leave the
/// canvas less than about 500 px, too little room to draw (DESIGN.md).
pub const NARROW_BELOW: f32 = 1024.0;
/// The width (CSS px) from which the narrow layout's one panel is docked beside the canvas at
/// its usual width; narrower, it fills the surface (there is no header above it).
pub const DOCK_FROM: f32 = 600.0;

/// The page list (ids and names, in order) and the active page's id.
type Meta = (Vec<(String, String)>, String);

fn meta(ed: &Editor) -> Meta {
    (
        ed.pages
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect(),
        ed.active_page_id().to_string(),
    )
}

/// What a save in flight was taken from (see [`WebState::save_snapshot`]): the document stamp,
/// each page whose content it carries with that page's history generation then, and the page
/// list and active page it names. Its answer acknowledges exactly these, never a change made
/// after the snapshot was taken.
struct Inflight {
    stamp: u64,
    pages: Vec<(String, u64)>,
    meta: Meta,
    /// The file name it carries, when it renames the file.
    name: Option<String>,
}

/// A canvas as the server sent it: its pages, its active page and its file name.
struct Canvas {
    pages: Vec<PageData>,
    active: Option<String>,
    name: Option<String>,
}

/// The canvas the server sent (`/api/canvas`, or the in-panel canvas read): `{canvas: {name},
/// active_page, pages: [{id, name, text}]}` (its other fields are the transport's), each `text`
/// an upstream page file. The list's ids and names win over the files' (the manifest owns
/// them, as in upstream `Project::load_page`). `canvas.name` is the file's name (the project's,
/// in upstream's terms), separate from any page's.
fn parse_canvas(json: &str) -> Result<Canvas, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("the canvas can't be read: {e}"))?;
    let list = v
        .get("pages")
        .and_then(|p| p.as_array())
        .filter(|l| !l.is_empty())
        .ok_or("the canvas has no pages")?;
    let mut pages = Vec::with_capacity(list.len());
    for p in list {
        let field = |k: &str| {
            p.get(k)
                .and_then(|x| x.as_str())
                .ok_or_else(|| format!("a page of the canvas has no {k}"))
        };
        let (id, name) = (field("id")?, field("name")?);
        let mut page = page_from_json(field("text")?).map_err(|e| format!("page {name:?}: {e}"))?;
        page.id = id.to_string();
        page.name = name.to_string();
        page.doc.name = name.to_string();
        pages.push(page);
    }
    let active = v
        .get("active_page")
        .and_then(|a| a.as_str())
        .map(str::to_string);
    let name = v
        .get("canvas")
        .and_then(|c| c.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_string);
    Ok(Canvas {
        pages,
        active,
        name,
    })
}

pub struct WebState {
    pub ed: Editor,
    /// Upstream's workspace state (panels, palette, renaming, export settings...).
    pub ui: UiState,
    pub ctx: egui::Context,
    mac: bool,
    /// `doc_stamp` of the content last confirmed written by the server.
    saved_stamp: u64,
    /// The snapshot being written, if any.
    inflight: Option<Inflight>,
    /// Something that can change the picture happened since the last draw.
    redraw: bool,
    /// egui input gathered since the last layout.
    events: Vec<egui::Event>,
    mods: egui::Modifiers,
    focused: bool,
    /// The canvas element's size in CSS px, and device pixels per CSS px.
    screen: egui::Vec2,
    ppp: f32,
    /// The canvas rectangle of the last layout (CSS px in the element).
    canvas: egui::Rect,
    /// A pointer press on the canvas went to the editor; its moves and release follow it.
    captured: bool,
    now_ms: f64,
    /// When egui asked to be laid out again (page clock, ms).
    repaint_at: Option<f64>,
    cursor: egui::CursorIcon,
    clipboard: Rc<RefCell<ClipboardBuf>>,
    /// Text an egui text field copied (best effort for the page to write).
    field_copy: Option<String>,
    requests: VecDeque<HostRequest>,
    downloads: VecDeque<Download>,
    library_noted: bool,
    /// The OS appearance as the page reports it (egui's system theme; see
    /// [`WebState::set_system_dark`]); `None` until it does.
    system_theme: Option<egui::Theme>,
    /// The appearance the page last knew of, so a change made in the menu is reported once.
    reported_appearance: egui::ThemePreference,
    /// The background the page last knew of, so a change made in a menu is reported once.
    reported_background: crate::render::scene::Background,
    /// The file's name (`canvas.name`) as the server last confirmed it: loaded, reloaded or
    /// saved. Upstream's top-bar title (`UiState::project_name`) differing from it is a rename
    /// to save, in the same canvas save as the pages.
    saved_name: String,
}

/// Shortcuts the browser keeps: copy, cut and paste arrive as the page's own clipboard events
/// (the only supported way to reach the clipboard from a page), which then run them. Every
/// other upstream shortcut is the workspace's.
pub fn browser_keeps(a: Action) -> bool {
    matches!(a, Action::Copy | Action::Cut | Action::Paste)
}

/// Whether upstream `Editor::key_down` acts on `key` in the editor's current state (same
/// order of checks), minus the shortcuts the browser keeps. Decided before the key is handled.
pub fn editor_takes(ed: &Editor, key: &Key) -> bool {
    let cmd_char = ed.mods.command() && matches!(key, Key::Char(_));
    let select_all = ed.mods.command() && matches!(key, Key::Char('a' | 'A'));
    if ed.text_edit.is_some() && (!cmd_char || select_all) {
        return true;
    }
    if ed.pen.is_some() && matches!(key, Key::Enter | Key::Escape) {
        return true;
    }
    if ed.path_edit.is_some()
        && matches!(key, Key::Enter | Key::Escape | Key::Delete | Key::Backspace)
    {
        return true;
    }
    if let Some(a) = shortcuts::lookup(key, ed.mods) {
        return !browser_keeps(a);
    }
    *key == Key::Space
}

fn egui_button(b: Button) -> egui::PointerButton {
    match b {
        Button::Primary => egui::PointerButton::Primary,
        Button::Middle => egui::PointerButton::Middle,
        Button::Secondary => egui::PointerButton::Secondary,
    }
}

/// The CSS cursor for what egui shows.
pub fn css_cursor(c: egui::CursorIcon) -> &'static str {
    use egui::CursorIcon as C;
    match c {
        C::PointingHand => "pointer",
        C::Text | C::VerticalText => "text",
        C::Crosshair => "crosshair",
        C::Move | C::AllScroll => "move",
        C::Grab => "grab",
        C::Grabbing => "grabbing",
        C::NotAllowed | C::NoDrop => "not-allowed",
        C::Wait => "wait",
        C::Progress => "progress",
        C::Help => "help",
        C::ContextMenu => "context-menu",
        C::Copy => "copy",
        C::Alias => "alias",
        C::Cell => "cell",
        C::ResizeHorizontal | C::ResizeColumn | C::ResizeEast | C::ResizeWest => "ew-resize",
        C::ResizeVertical | C::ResizeRow | C::ResizeNorth | C::ResizeSouth => "ns-resize",
        C::ResizeNeSw | C::ResizeNorthEast | C::ResizeSouthWest => "nesw-resize",
        C::ResizeNwSe | C::ResizeNorthWest | C::ResizeSouthEast => "nwse-resize",
        C::ZoomIn => "zoom-in",
        C::ZoomOut => "zoom-out",
        C::None => "none",
        _ => "default",
    }
}

impl WebState {
    /// Open the whole canvas the server sent (see [`parse_canvas`]): every page, in list
    /// order, on the active page it names, through upstream `Editor::open_pages`, so the pages
    /// panel switches, adds, renames, duplicates, reorders and deletes pages as upstream does.
    /// The file's name is upstream's own top-bar title (`UiState::project_name`), shown above
    /// the page's name; its "All projects" button is the host's way back (`HostRequest::Home`).
    pub fn open(
        canvas_json: &str,
        mac: bool,
        css_width: f64,
        css_height: f64,
    ) -> Result<WebState, String> {
        let canvas = parse_canvas(canvas_json)?;
        let mut ed = Editor::default();
        ed.viewport = v2(css_width.max(1.0), css_height.max(1.0));
        ed.open_pages(canvas.pages, canvas.active.as_deref());
        let clipboard = Rc::new(RefCell::new(ClipboardBuf::default()));
        ed.clipboard_io = Some(Box::new(WebClipboard(clipboard.clone())));
        let ctx = egui::Context::default();
        ui::setup_style(&ctx);
        // Ctrl/⌘ with +, = or - is upstream's canvas zoom (`Editor`). egui would also apply it
        // as its own UI zoom (`Options::zoom_with_keyboard`), scaling what it paints while the
        // page's input and the canvas rectangle stay in CSS px: positions would no longer match
        // what is drawn. The UI scale here is the page's device pixel ratio only.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let name = canvas.name.unwrap_or_default();
        let mut s = WebState {
            saved_stamp: ed.doc_stamp,
            ed,
            ui: UiState {
                project_name: name.clone(),
                // The title renames the file, saved with the canvas (see `save_snapshot`).
                file_rename: true,
                // The granted surface can be a narrow sidebar: below this width upstream's
                // narrow layout (one side panel at a time, the top bar across) keeps the canvas,
                // tools, All projects and panels reachable. Wider, the ordinary layout.
                narrow: ui::NarrowLayout::with_dock(NARROW_BELOW, DOCK_FROM),
                ..Default::default()
            },
            ctx,
            mac,
            inflight: None,
            redraw: true,
            events: Vec::new(),
            mods: egui::Modifiers::NONE,
            focused: true,
            screen: egui::vec2(1.0, 1.0),
            ppp: 1.0,
            canvas: egui::Rect::NOTHING,
            captured: false,
            now_ms: 0.0,
            repaint_at: None,
            cursor: egui::CursorIcon::Default,
            clipboard,
            field_copy: None,
            requests: VecDeque::new(),
            downloads: VecDeque::new(),
            library_noted: false,
            system_theme: None,
            reported_appearance: egui::ThemePreference::System,
            reported_background: Default::default(),
            saved_name: name,
        };
        s.resize(css_width, css_height, 1.0);
        Ok(s)
    }

    /// The canvas element's size in CSS px and the device pixel ratio. The editor's viewport
    /// is the canvas rectangle inside the workspace, set by the next layout.
    pub fn resize(&mut self, css_width: f64, css_height: f64, dpr: f64) {
        self.screen = egui::vec2(css_width.max(1.0) as f32, css_height.max(1.0) as f32);
        self.ppp = if dpr.is_finite() && dpr > 0.0 {
            dpr as f32
        } else {
            1.0
        };
        self.redraw = true;
    }

    /// The OS appearance, as the page reports it (the browser's colour-scheme preference, else
    /// the host's theme where the browser can't say). The native app's egui gets it from
    /// egui-winit; here it reaches egui the same way, as its raw input's system theme, which the
    /// System appearance follows. Presentation only: the document, its save state, undo, focus
    /// and any field's draft are untouched. Redraws only when it changes.
    pub fn set_system_dark(&mut self, dark: bool) {
        let theme = egui::Theme::from_dark_mode(dark);
        if self.system_theme != Some(theme) {
            self.system_theme = Some(theme);
            self.redraw = true;
        }
    }

    /// The appearance preference the page kept (`system`, `light` or `dark`; see
    /// [`appearance_from_name`]): applied before the next frame, without being reported back.
    /// Anything else is refused and changes nothing (the page then keeps System). Presentation
    /// only, as [`WebState::set_system_dark`].
    pub fn set_appearance(&mut self, name: &str) -> bool {
        let Some(p) = appearance_from_name(name) else {
            return false;
        };
        self.ui.appearance = p;
        self.reported_appearance = p;
        if self.ctx.options(|o| o.theme_preference) != p {
            self.ctx.set_theme(p);
            self.redraw = true;
        }
        true
    }

    /// The Canvas's name as this editor has it now: upstream's title (`UiState::project_name`),
    /// which a finished rename sets at once and a reload sets to the server's. Never a draft
    /// still being typed. The page names its editor with it (its region and title, R61-F2).
    pub fn canvas_name(&self) -> &str {
        &self.ui.project_name
    }

    /// The person's appearance preference now: `system`, `light` or `dark`.
    pub fn appearance(&self) -> &'static str {
        appearance_name(self.ui.appearance)
    }

    /// The background the page kept (`none`, `dots` or `lines`): shown from the next frame,
    /// without being reported back. Anything else is refused and changes nothing. A view
    /// preference: the document, its save state and undo are untouched.
    pub fn set_background(&mut self, name: &str) -> bool {
        let Some(b) = crate::render::scene::Background::from_name(name) else {
            return false;
        };
        self.reported_background = b;
        if self.ui.background != b {
            self.ui.set_background(b);
            self.redraw = true;
        }
        true
    }

    /// The canvas background now: `none`, `dots` or `lines`.
    pub fn background(&self) -> &'static str {
        self.ui.background.name()
    }

    /// Whether the workspace is shown dark now (the preference resolved with the OS
    /// appearance), as the chrome and the canvas display are.
    pub fn dark(&self) -> bool {
        self.ctx.theme() == egui::Theme::Dark
    }

    /// Advance time; returns whether a frame should be laid out and drawn now.
    pub fn tick(&mut self, now_ms: f64) -> bool {
        self.now_ms = now_ms;
        let trail = self.ed.laser.len();
        self.ed.tick(now_ms);
        // Expired laser points leave the scene: that change needs a frame of its own (after
        // the last point expires `animating()` is already false). Stays pending until drawn.
        if self.ed.laser.len() != trail {
            self.redraw = true;
        }
        let egui_due = self.repaint_at.is_some_and(|t| now_ms >= t);
        self.redraw || !self.events.is_empty() || egui_due || self.ed.animating(now_ms)
    }

    fn raw_input(&mut self) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.screen)),
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(self.ppp),
                    focused: Some(self.focused),
                    ..Default::default()
                },
            ))
            .collect(),
            time: Some(self.now_ms / 1000.0),
            // Modifier changes reach egui as `Event::ModifiersChanged`, in order (`modifiers`).
            events: std::mem::take(&mut self.events),
            focused: self.focused,
            system_theme: self.system_theme,
            ..Default::default()
        }
    }

    /// Lay out the workspace for one frame (upstream `render::frame::layout`) and handle what
    /// it asked for. The caller paints the returned layout (`Painter::paint`) and then calls
    /// [`WebState::drawn`].
    pub fn layout(&mut self) -> Layout {
        let input = self.raw_input();
        let mut l = frame::layout(&self.ctx, input, &mut self.ed, &mut self.ui);
        self.canvas = l.canvas;
        let platform = std::mem::take(&mut l.output.platform_output);
        self.cursor = platform.cursor_icon;
        for c in platform.commands {
            if let egui::OutputCommand::CopyText(t) = c {
                self.field_copy = Some(t);
            }
        }
        let delay = l
            .output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |v| v.repaint_delay);
        self.repaint_at = if delay < std::time::Duration::from_secs(3600) {
            Some(self.now_ms + delay.as_secs_f64() * 1000.0)
        } else {
            None
        };
        self.handle_requests();
        if self.ui.appearance != self.reported_appearance {
            self.reported_appearance = self.ui.appearance;
            self.requests.push_back(HostRequest::Appearance);
            self.redraw = true;
        }
        if self.ui.background != self.reported_background {
            self.reported_background = self.ui.background;
            self.requests.push_back(HostRequest::Background);
            self.redraw = true;
        }
        if self.ui.library.dirty {
            // Nothing persists the library from a page (it is the native app's own file).
            self.ui.library.dirty = false;
            if !self.library_noted {
                self.library_noted = true;
                self.ed.status = LIBRARY_SESSION.into();
            }
        }
        l
    }

    /// The frame for the current state was drawn.
    pub fn drawn(&mut self) {
        self.redraw = false;
    }

    /// The canvas rectangle of the last layout (CSS px in the canvas element).
    pub fn canvas_rect(&self) -> egui::Rect {
        self.canvas
    }

    /// What the cursor should look like now (a CSS `cursor` value).
    pub fn cursor(&self) -> &'static str {
        css_cursor(self.cursor)
    }

    /// The next thing the page must do for the editor, if any.
    pub fn take_request(&mut self) -> Option<HostRequest> {
        self.requests.pop_front()
    }

    pub fn take_download(&mut self) -> Option<Download> {
        self.downloads.pop_front()
    }

    /// Text for the page to put on the clipboard: what the editor wrote (a copy outside a
    /// `copy` event, e.g. from a menu) or what an egui text field copied.
    pub fn take_clipboard_text(&mut self) -> Option<String> {
        let out = self.clipboard.borrow_mut().outgoing.take();
        out.or_else(|| self.field_copy.take())
    }

    /// Like the native `App::over_canvas`: inside the canvas rectangle, not over floating egui
    /// UI (tools, palette, help, menus, tooltips; the shared `ui::over_floating_ui`, which
    /// doesn't count the canvas's own menu once it is closed), and no egui drag under way.
    fn over_canvas(&self, p: egui::Pos2) -> bool {
        self.canvas.contains(p)
            && !ui::over_floating_ui(&self.ctx, &self.ui, p)
            && !self.ctx.egui_is_using_pointer()
    }

    fn to_canvas(&self, p: egui::Pos2) -> crate::geom::V2 {
        v2(
            (p.x - self.canvas.min.x) as f64,
            (p.y - self.canvas.min.y) as f64,
        )
    }

    pub fn set_focused(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            self.events.push(egui::Event::WindowFocused(focused));
        }
        if !focused {
            self.modifiers(false, false, false, false);
        }
        self.redraw = true;
    }

    pub fn modifiers(&mut self, shift: bool, ctrl: bool, alt: bool, meta: bool) {
        let mods = egui::Modifiers {
            alt,
            ctrl,
            shift,
            mac_cmd: self.mac && meta,
            command: if self.mac { meta } else { ctrl },
        };
        if mods != self.mods {
            self.mods = mods;
            self.events.push(egui::Event::ModifiersChanged(mods));
            self.redraw = true;
        }
        let m = keys::modifiers(self.mac, shift, ctrl, alt, meta);
        if m != self.ed.mods {
            self.ed.handle(InputEvent::Modifiers(m));
            self.redraw = true;
        }
    }

    /// Positions are CSS px in the canvas element (the whole workspace). While a tool flyout
    /// or the canvas's own menu is open the press is theirs (it closes them), never a drawing
    /// on the canvas. A press egui gives a panel's resize handle (its edge reaches into the
    /// canvas) is the handle's alone: the panel resizes and nothing behind it moves.
    pub fn pointer_down(&mut self, x: f64, y: f64, button: Button, time_ms: f64) {
        let p = egui::pos2(x as f32, y as f32);
        self.events.push(egui::Event::PointerMoved(p));
        self.events.push(egui::Event::PointerButton {
            pos: p,
            button: egui_button(button),
            pressed: true,
            modifiers: self.mods,
        });
        // An open egui popup's press closes it (egui) and never also reaches the canvas, as an
        // open flyout's or canvas menu's doesn't (run147: before, this route had no popup check,
        // so the press that closed a menu went to the editor too, with the current tool).
        let theirs = self.ui.captures_input()
            || ui::popup_open(&self.ctx)
            || self.ui.chrome.on_panel_edge(p);
        if self.over_canvas(p) && !theirs {
            // Clicking the canvas takes keyboard focus away from text fields.
            self.ctx.memory_mut(|m| {
                if let Some(f) = m.focused() {
                    m.surrender_focus(f);
                }
            });
            self.captured = true;
            self.ed.handle(InputEvent::PointerDown {
                pos: self.to_canvas(p),
                button,
                pressure: None,
                time_ms,
            });
        }
        self.redraw = true;
    }

    pub fn pointer_move(&mut self, x: f64, y: f64, time_ms: f64) {
        let p = egui::pos2(x as f32, y as f32);
        self.events.push(egui::Event::PointerMoved(p));
        if self.captured || self.over_canvas(p) {
            self.ed.handle(InputEvent::PointerMove {
                pos: self.to_canvas(p),
                pressure: None,
                time_ms,
            });
        } else {
            self.ed.hover = None;
        }
        self.redraw = true;
    }

    pub fn pointer_up(&mut self, x: f64, y: f64, button: Button, time_ms: f64) {
        let p = egui::pos2(x as f32, y as f32);
        self.events.push(egui::Event::PointerMoved(p));
        self.events.push(egui::Event::PointerButton {
            pos: p,
            button: egui_button(button),
            pressed: false,
            modifiers: self.mods,
        });
        if self.captured {
            self.captured = false;
            self.ed.handle(InputEvent::PointerUp {
                pos: self.to_canvas(p),
                button,
                time_ms,
            });
        }
        self.redraw = true;
    }

    /// The pointer left the canvas element.
    pub fn pointer_leave(&mut self) {
        self.events.push(egui::Event::PointerGone);
        if !self.captured {
            self.ed.hover = None;
        }
        self.redraw = true;
    }

    /// DOM wheel delta in CSS px (positive y = scroll down). Over the canvas the editor pans
    /// or (with Ctrl/⌘, which is how browsers report a trackpad pinch) zooms; over a panel
    /// egui scrolls it.
    pub fn wheel(&mut self, dx: f64, dy: f64, x: f64, y: f64) {
        let p = egui::pos2(x as f32, y as f32);
        self.events.push(egui::Event::PointerMoved(p));
        if self.over_canvas(p) {
            let d = if self.ed.mods.shift && dx == 0.0 {
                v2(dy, 0.0)
            } else {
                v2(dx, dy)
            };
            self.ed.handle(InputEvent::Scroll {
                delta: d,
                pos: self.to_canvas(p),
            });
        } else {
            self.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(-dx as f32, -dy as f32),
                // A DOM wheel event has no gesture phase (egui: "if unknown, Move").
                phase: egui::TouchPhase::Move,
                modifiers: self.mods,
            });
        }
        self.redraw = true;
    }

    /// Whether an egui text field (inspector, rename, palette...) has the keyboard.
    pub fn field_has_keyboard(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// Returns whether the key was taken (the page then prevents the browser default). While
    /// an egui text field has the keyboard, keys are its own (copy, cut and paste still come
    /// as clipboard events); otherwise keys the editor wouldn't act on are not sent and stay
    /// the browser's.
    pub fn key_down(&mut self, key: &str, code: &str, repeat: bool) -> bool {
        let named = egui::Key::from_name(key);
        if let Some(k) = named {
            self.events.push(egui::Event::Key {
                key: k,
                physical_key: None,
                pressed: true,
                repeat,
                modifiers: self.mods,
            });
        }
        self.redraw = true;
        // An open tool flyout or canvas menu takes Escape (it closes, the tool stays as it is).
        if named == Some(egui::Key::Escape) && self.ui.captures_input() {
            return true;
        }
        // A rail control with egui's keyboard focus (a family's button or chevron, or a flyout
        // row, reached with Tab) takes Enter or Space: egui chooses it, and the editor never
        // also gets the key (Enter would edit the selected text, Space would pan).
        let focused = self.ctx.memory(|m| m.focused());
        if named.is_some_and(|k| self.ui.rail.takes_key(k, focused)) {
            return true;
        }
        if self.field_has_keyboard() {
            let clipboard_chord = self.mods.command
                && matches!(named, Some(egui::Key::C | egui::Key::X | egui::Key::V));
            if clipboard_chord {
                return false;
            }
            if key.chars().count() == 1 && !self.mods.command && !self.mods.ctrl {
                self.events.push(egui::Event::Text(key.to_string()));
            }
            return true;
        }
        // An open egui popup (a menu, a submenu, a choice's list; run147) owns the keys too:
        // egui moves its focus, chooses and closes it (Escape), and the editor never also gets
        // them. Before, with the All Projects menu open and nothing focused, ArrowDown nudged
        // the selected rectangle 1 px (an undoable, saved edit behind the menu).
        if ui::popup_open(&self.ctx) {
            return true;
        }
        // A rename just asked for (Rename Paper…, a layer's Rename) owns the keyboard before its
        // field has it: the Papers or layer rows may still be opening (a frame or two). Its keys
        // go to egui only, never to the editor's shortcuts (R57: an "r" typed then chose the
        // Rectangle tool). A rename whose rows aren't coming is finished at once by `ui::draw`,
        // so this never holds keys for a field that won't be shown.
        if self.ui.pages.renaming.is_some() || self.ui.renaming.is_some() {
            return true;
        }
        let editing = self.ed.text_edit.is_some();
        let Some(k) = keys::key(self.ed.mods, editing, key, code) else {
            return false;
        };
        if !editor_takes(&self.ed, &k) {
            return false;
        }
        self.ed.handle(InputEvent::KeyDown { key: k, repeat });
        if editing && !self.ed.mods.command() && key.chars().count() == 1 {
            self.ed.handle(InputEvent::Text(key.to_string()));
        }
        true
    }

    pub fn key_up(&mut self, key: &str, code: &str) {
        if let Some(k) = egui::Key::from_name(key) {
            self.events.push(egui::Event::Key {
                key: k,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: self.mods,
            });
        }
        // Every release reaches the editor, whoever has the keyboard now (R61-F1): upstream acts
        // on a release only to let go of a key it holds (Space's pan, `Editor::space_down`), so a
        // key pressed on the canvas and released after a field or a menu took the keyboard still
        // ends. Before, such a release stayed with egui (a menu item had the focus) and the next
        // Select drag panned. Presses keep their owners (`key_down`).
        if let Some(k) = keys::key(self.ed.mods, self.ed.text_edit.is_some(), key, code) {
            self.ed.handle(InputEvent::KeyUp { key: k });
        }
        self.redraw = true;
    }

    /// A `paste` event's text: into the egui text field that has the keyboard, else the
    /// editor's Paste (upstream reads it through its clipboard provider).
    pub fn paste(&mut self, text: &str) {
        // An open popup's too (`ui::popup_open`): nothing is pasted onto the canvas behind it.
        if self.field_has_keyboard() || ui::popup_open(&self.ctx) {
            self.events.push(egui::Event::Paste(text.to_string()));
        } else {
            self.clipboard.borrow_mut().incoming = Some(text.to_string());
            self.run(Action::Paste);
            self.clipboard.borrow_mut().incoming = None;
        }
        self.redraw = true;
    }

    /// A `copy` (or `cut`) event: the editor copies its selection, and the text it wrote is
    /// returned for the event's clipboard data. In an egui text field the field handles it
    /// on the next frame (its text then comes from `take_clipboard_text`).
    pub fn copy(&mut self, cut: bool) -> Option<String> {
        self.redraw = true;
        // An open popup's too: no Cut deletes the selection behind it.
        if self.field_has_keyboard() || ui::popup_open(&self.ctx) {
            self.events.push(if cut {
                egui::Event::Cut
            } else {
                egui::Event::Copy
            });
            return None;
        }
        self.clipboard.borrow_mut().outgoing = None;
        self.run(if cut { Action::Cut } else { Action::Copy });
        self.clipboard.borrow_mut().outgoing.take()
    }

    pub fn tool(&self) -> String {
        format!("{:?}", self.ed.tool).to_lowercase()
    }

    pub fn run(&mut self, a: Action) {
        self.ed.run(a);
        self.redraw = true;
    }

    /// Every `AppRequest`, explicitly: the page's own save and navigation, in-memory SVG and
    /// PDF exports offered as downloads, and a visible reason for what this editor can't do.
    fn handle_requests(&mut self) {
        for req in std::mem::take(&mut self.ed.requests) {
            match req {
                AppRequest::Save => self.requests.push_back(HostRequest::Save),
                AppRequest::Home | AppRequest::New | AppRequest::Open => {
                    self.requests.push_back(HostRequest::Home)
                }
                AppRequest::SaveAs => self.ed.status = NO_SAVE_AS.into(),
                AppRequest::PlaceImage | AppRequest::ImageFill(..) => {
                    self.ed.status = NO_IMAGE.into()
                }
                AppRequest::CopyAsPng => self.ed.status = NO_PNG.into(),
                AppRequest::Export(spec) => self.export(spec),
                AppRequest::ExportFormat(f) => {
                    let spec = Spec {
                        format: f,
                        ..self.ui.export
                    };
                    self.export(spec);
                }
            }
            self.redraw = true;
        }
    }

    /// Upstream's export for SVG and PDF, in memory (`io::svg::export`, `io::pdf`), as one
    /// file. PNG needs a GPU read-back and is refused with a reason.
    fn export(&mut self, spec: Spec) {
        if spec.format == Format::Png {
            self.ed.status = NO_PNG.into();
            return;
        }
        let made: Result<Download, String> = {
            let names: Vec<String> = self.ed.pages.iter().map(|p| p.name.clone()).collect();
            let pages: Vec<(&str, &Document)> = names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.as_str(), self.ed.page_doc(i)))
                .collect();
            let sel = self.ed.top_selection();
            let items = export::items(&spec, &pages, self.ed.active_page, &sel);
            let bg = (!spec.transparent).then_some(Color::WHITE);
            let base = match items.as_slice() {
                [one] => one.name.clone(),
                _ => self.ed.page_name().to_string(),
            };
            let name = format!(
                "{}.{}",
                match crate::project::folder_name(&base) {
                    s if s.is_empty() => "export".to_string(),
                    s => s,
                },
                spec.format.ext()
            );
            if items.is_empty() {
                Err("Nothing to export.".into())
            } else if spec.format == Format::Pdf {
                let mut out = Vec::new();
                for it in &items {
                    out.extend(crate::io::pdf::pages_for(
                        it.doc,
                        Some(&it.roots),
                        spec.scope == Scope::Frames,
                        bg,
                    ));
                }
                if out.is_empty() {
                    Err("Nothing to export.".into())
                } else {
                    Ok(Download {
                        name,
                        mime: "application/pdf",
                        bytes: crate::io::pdf::export(&out),
                    })
                }
            } else if items.len() > 1 {
                Err(
                    "SVG export makes one file here: export one frame or the page, or use PDF \
                     for several. Nothing was exported."
                        .into(),
                )
            } else {
                crate::io::svg::export(items[0].doc, Some(&items[0].roots), bg)
                    .map(|s| Download {
                        name,
                        mime: "image/svg+xml",
                        bytes: s.into_bytes(),
                    })
                    .ok_or_else(|| "Nothing to export.".to_string())
            }
        };
        match made {
            Ok(d) => {
                self.ed.status = format!(
                    "Exported {} (a download; the browser decides where it goes)",
                    d.name
                );
                self.downloads.push_back(d);
            }
            Err(e) => self.ed.status = e,
        }
    }

    /// Top-level nodes as JSON `[{id, name, kind, children, world: [x,y,w,h], screen: [x,y,w,h]}]`
    /// (screen = CSS px in the canvas element), for status displays and automated checks.
    pub fn root_nodes(&self) -> String {
        let doc = &self.ed.doc;
        let off = if self.canvas.is_finite() {
            (self.canvas.min.x as f64, self.canvas.min.y as f64)
        } else {
            (0.0, 0.0)
        };
        let list: Vec<_> = doc
            .root
            .iter()
            .filter_map(|&id| {
                let n = doc.get(id)?;
                let w = doc.world_rect(id)?;
                let s = self.ed.camera.world_rect_to_screen(&w);
                let kind = serde_json::to_value(&n.kind)
                    .ok()
                    .and_then(|v| v.get("type").cloned())
                    .unwrap_or_default();
                Some(serde_json::json!({
                    "id": id.0,
                    "name": n.name,
                    "kind": kind,
                    "children": n.children.len(),
                    "world": [w.x, w.y, w.w, w.h],
                    "screen": [s.x + off.0, s.y + off.1, s.w, s.h],
                }))
            })
            .collect();
        serde_json::Value::from(list).to_string()
    }

    /// Something the canvas would lose, by upstream's own bookkeeping: a page whose content
    /// changed since its last confirmed save (`Editor::dirty_pages`, inactive pages included),
    /// a page-list, name or active-page change (`Editor::meta_dirty`), or a document change
    /// since the last confirmed snapshot (`doc_stamp`). A save in flight is not a confirmed
    /// one, so it stays dirty until its answer.
    pub fn dirty(&self) -> bool {
        self.ed.doc_stamp != self.saved_stamp || self.ed.project_dirty() || self.name_dirty()
    }

    /// The file's name was changed here (the title's finished rename) since the server last
    /// confirmed it: it goes with the next canvas save.
    fn name_dirty(&self) -> bool {
        self.ui.project_name != self.saved_name
    }

    /// An egui text field has the keyboard, other than the command palette's search: an
    /// inspector number field in its text mode (upstream `DragValue`), a colour's hex value, a
    /// library name... Its text can be an unfinished draft that the document doesn't have yet
    /// (`-` before a negative X; upstream applies a number only once it parses, or drops it on
    /// Escape). Told by egui's public text-edit state for the focused widget, not by its text.
    /// Only the palette's own search field is exempt, told by the id upstream's palette reports
    /// for it (`PaletteUi::search_id`): the palette stays open when a document field is
    /// clicked, and that field keeps its protection. An open rename's own field is judged by
    /// its draft instead, told the same way (`UiState::rename_field`, `PagesUi::rename_field`).
    fn field_editing(&self) -> bool {
        let Some(id) = self.ctx.memory(|m| m.focused()) else {
            return false;
        };
        if self.ui.palette.open && self.ui.palette.search_id == Some(id) {
            return false;
        }
        // An open rename's own field (layer or page): its draft is judged by its text
        // (`ui_edit_pending`). Only that field, told by the id upstream reports for it; a stored
        // rename never exempts another focused field.
        let layer = self.ui.renaming.is_some() && self.ui.rename_field == Some(id);
        let page = self.ui.pages.renaming.is_some() && self.ui.pages.rename_field == Some(id);
        let title = self.ui.file_title_draft.is_some() && self.ui.file_title_field == Some(id);
        if layer || page || title {
            return false;
        }
        egui::widgets::text_edit::TextEditState::load(&self.ctx, id).is_some()
    }

    /// An upstream UI edit that isn't in the document yet. Nothing can save it yet, and a
    /// reload would strand it:
    /// - the layer-rename or page-rename field: a changed draft (upstream commits it on Enter or
    ///   when the field loses focus; an unchanged one has nothing to lose, and a reload closes
    ///   it);
    /// - any other egui text field while it has the keyboard, changed or not, until upstream
    ///   finishes it (Enter, or focus elsewhere such as a click on the canvas) or cancels it
    ///   (Escape). It is told apart only by focus, so an untouched field counts too: that is
    ///   the conservative side;
    /// - typing queued for an egui text field that the next layout will apply.
    fn ui_edit_pending(&self) -> bool {
        let changed = |text: &str, name: Option<&str>| {
            let text = text.trim();
            !text.is_empty() && name.is_some_and(|n| n != text)
        };
        let layer = self
            .ui
            .renaming
            .as_ref()
            .map(|(id, text)| changed(text, self.ed.doc.get(*id).map(|n| n.name.as_str())));
        let page = self
            .ui
            .pages
            .renaming
            .as_ref()
            .map(|(i, text)| changed(text, self.ed.pages.get(*i).map(|p| p.name.as_str())));
        // The file title's rename: changed against the name shown before it opened.
        let title = self
            .ui
            .file_title_draft
            .as_ref()
            .map(|text| changed(text, Some(self.ui.project_name.as_str())));
        let renaming = layer.is_some() || page.is_some() || title.is_some();
        // A stored rename speaks only for its own field: another focused field is judged on
        // its own (`field_editing` exempts exactly the rename's field).
        let draft = layer.unwrap_or(false)
            || page.unwrap_or(false)
            || title.unwrap_or(false)
            || self.field_editing();
        let typing = (renaming || self.field_has_keyboard())
            && self.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                        | egui::Event::Key { pressed: true, .. }
                )
            });
        draft || typing
    }

    /// Anything a leave would lose: unsaved content or an unfinished UI edit (for the page's
    /// leave guard).
    pub fn unsaved_work(&self) -> bool {
        self.dirty() || self.ui_edit_pending()
    }

    /// The canvas save to send now, or `None` when there is nothing to save, a save is already
    /// in flight, or a gesture/text edit still owns the open transaction. It is
    /// `{active_page, pages: [{id, name, text?}]}`: the whole page list in order, the active
    /// page, and an upstream page file (`page_to_json` of `Editor::page_data`) for each page
    /// upstream counts as changed (`Editor::dirty_pages`, a new page included); the server
    /// writes them and the manifest as one commit. What it was taken from is kept for its
    /// answer (see [`WebState::save_succeeded`]).
    pub fn save_snapshot(&mut self) -> Option<String> {
        if self.inflight.is_some() || self.ed.is_busy() || self.ed.text_edit.is_some() {
            return None;
        }
        self.ed.commit_pending();
        if !self.dirty() {
            return None;
        }
        let changed = self.ed.dirty_pages();
        let mut sent = Vec::with_capacity(changed.len());
        let mut pages = Vec::with_capacity(self.ed.pages.len());
        for (i, slot) in self.ed.pages.iter().enumerate() {
            let mut p = serde_json::json!({"id": slot.id, "name": slot.name});
            if changed.contains(&i) {
                p["text"] = page_to_json(&self.ed.page_data(i)).into();
                sent.push((slot.id.clone(), self.ed.page_gen(i)));
            }
            pages.push(p);
        }
        // The file's own name, when the title renamed it: in this same save, so the name and
        // the pages are one commit, one revision and one answer.
        let name = self.name_dirty().then(|| self.ui.project_name.clone());
        self.inflight = Some(Inflight {
            stamp: self.ed.doc_stamp,
            pages: sent,
            meta: meta(&self.ed),
            name: name.clone(),
        });
        let mut save = serde_json::json!({"active_page": self.ed.active_page_id(), "pages": pages});
        if let Some(n) = name {
            save["name"] = n.into();
        }
        Some(save.to_string())
    }

    /// Whether a newer revision from the server may replace the canvas now: nothing unsaved on
    /// any page or in the page list, no save in flight, no gesture, text edit or unfinished UI
    /// edit (a changed rename draft, queued typing) under way. Otherwise nothing is reloaded:
    /// the local change's save is refused as a conflict (nothing merges). Leaving
    /// (`Session.drain`) is clean only then too.
    pub fn can_reload(&self) -> bool {
        !self.dirty()
            && self.inflight.is_none()
            && !self.ed.is_busy()
            && self.ed.text_edit.is_none()
            && !self.ui_edit_pending()
    }

    /// Load the canvas as the server has it now, in place: its pages, list and active page (a
    /// change made elsewhere to any of them shows), keeping the view: the tool, each remaining
    /// page's camera, and on the page still shown the selection where those nodes still exist.
    /// Undo history starts over. Only when `can_reload`, or as the explicit discard of every
    /// local change. Open rename fields (layer or page) close and a page drag ends: they were
    /// for the replaced canvas, so their text (unchanged after a clean reload, or deliberately
    /// dropped by the conflict banner's discard) never lands on the new one. The file's name
    /// follows the server's (a rename from the library shows).
    pub fn reload(&mut self, canvas_json: &str) -> Result<(), String> {
        let Canvas {
            mut pages,
            active,
            name,
        } = parse_canvas(canvas_json)?;
        if let Some(name) = name {
            self.ui.project_name = name.clone();
            self.saved_name = name;
        } else {
            // No name from the server: whatever is shown counts as its name.
            self.saved_name = self.ui.project_name.clone();
        }
        self.ui.renaming = None;
        // The title's rename field was for the replaced canvas too.
        self.ui.file_title_draft = None;
        let collapsed = self.ui.pages.collapsed;
        self.ui.pages = Default::default();
        self.ui.pages.collapsed = collapsed;
        self.ctx.memory_mut(|m| {
            if let Some(f) = m.focused() {
                m.surrender_focus(f);
            }
        });
        for p in &mut pages {
            let mine = self.ed.pages.iter().position(|s| s.id == p.id);
            if let Some(c) = mine.and_then(|i| self.camera_of(i)) {
                p.camera = Some(c);
            }
        }
        let shown = self.ed.active_page_id().to_string();
        let selection = self.ed.selection.clone();
        let tool = self.ed.tool;
        self.ed.open_pages(pages, active.as_deref());
        self.ed.tool = tool;
        if self.ed.active_page_id() == shown {
            let kept: Vec<_> = selection
                .into_iter()
                .filter(|id| self.ed.doc.nodes.contains_key(id))
                .collect();
            self.ed.selection = kept;
        }
        self.saved_stamp = self.ed.doc_stamp;
        self.inflight = None;
        self.redraw = true;
        Ok(())
    }

    /// Page `i`'s camera, if it has one of its own (a page never shown yet is fit when shown).
    fn camera_of(&self, i: usize) -> Option<crate::geom::Camera> {
        if i == self.ed.active_page {
            return Some(self.ed.camera);
        }
        let stash = self.ed.pages[i].stash.as_ref()?;
        (!stash.needs_fit).then_some(stash.camera)
    }

    /// The server stored the snapshot in flight: exactly what it was taken from counts as
    /// saved. A page sent then is marked saved (`Editor::mark_page_saved`) only if its history
    /// generation is still the one sent; one edited, undone or redone since stays to save,
    /// recorded as saved at the generation that was sent. The page list and active page count
    /// as saved only if they are still the ones sent. Whatever changed after the snapshot (an
    /// edit on any page, a new, renamed, moved or deleted page, a switch) stays unsaved.
    pub fn save_succeeded(&mut self) {
        let Some(f) = self.inflight.take() else {
            return;
        };
        self.saved_stamp = f.stamp;
        for (id, generation) in f.pages {
            let Some(i) = self.ed.pages.iter().position(|p| p.id == id) else {
                continue;
            };
            if self.ed.page_gen(i) == generation {
                self.ed.mark_page_saved(i);
            } else {
                self.ed.pages[i].saved_gen = generation;
            }
        }
        if meta(&self.ed) == f.meta {
            self.ed.meta_dirty = false;
        }
        // The name it carried is the server's now; a later rename (or one made while it was in
        // flight) stays to save.
        if let Some(n) = f.name {
            self.saved_name = n;
        }
    }

    /// The snapshot in flight was not stored (conflict, refusal or no answer): everything it
    /// held stays unsaved, to be sent again or explicitly discarded.
    pub fn save_failed(&mut self) {
        self.inflight = None;
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
