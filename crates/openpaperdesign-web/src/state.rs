//! Everything the browser page does with the upstream editor, apart from the GPU surface and
//! the JS bindings (`web.rs`). It is plain Rust, so it is tested on the host.
//!
//! Redraw rule: the scene is a function of editor state plus time (only the laser trail
//! animates, see `Editor::animating`). Editor state only changes inside the methods below, so
//! every method that can change it marks a redraw. `Editor::revision` alone is not enough:
//! selection, hover, marquee, handles and modifier changes don't bump it.

use crate::editor::shortcuts::{self, Action};
use crate::editor::{Button, Editor, InputEvent, Key, Tool};
use crate::geom::v2;
use crate::keys;
use crate::project::format::{page_from_json, page_to_json};

pub struct WebState {
    pub ed: Editor,
    mac: bool,
    /// `doc_stamp` of the content last confirmed written by the server.
    saved_stamp: u64,
    /// `doc_stamp` of the snapshot being written, if any.
    inflight: Option<u64>,
    /// Something that can change the picture happened since the last draw.
    redraw: bool,
}

/// Shortcuts whose action only asks the app chrome to do something (dialogs, export,
/// palette, rename, page list...). That chrome doesn't exist in this slice, so these keys
/// are neither sent to the editor nor consumed: the browser keeps them (⌘R reloads, ⌘S/⌘P
/// stay the browser's).
pub fn chrome_only(a: Action) -> bool {
    matches!(
        a,
        Action::New
            | Action::Open
            | Action::Save
            | Action::SaveAs
            | Action::ExportPng
            | Action::ExportSvg
            | Action::ExportPdf
            | Action::CopyAsPng
            | Action::CopyAsSvg
            | Action::PlaceImage
            | Action::CommandPalette
            | Action::Rename
            | Action::ToggleUi
            | Action::ToggleHelp
            | Action::ToggleGrid
            | Action::Home
    )
}

/// Whether upstream `Editor::key_down` acts on `key` in the editor's current state (same
/// order of checks), minus chrome-only shortcuts. Decided before the key is handled.
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
        return !chrome_only(a);
    }
    *key == Key::Space
}

impl WebState {
    /// Open an upstream page file listed in the manifest as `page_id`/`page_name`.
    pub fn open(
        page_json: &str,
        page_id: String,
        page_name: String,
        mac: bool,
        css_width: f64,
        css_height: f64,
    ) -> Result<WebState, String> {
        let mut page = page_from_json(page_json).map_err(|e| e.to_string())?;
        // The manifest owns page ids and names (a migrated legacy file gets a fresh random id).
        page.id = page_id;
        page.name = page_name.clone();
        page.doc.name = page_name;
        let mut ed = Editor::default();
        ed.viewport = v2(css_width.max(1.0), css_height.max(1.0));
        ed.open_pages(vec![page], None);
        let mut s = WebState {
            saved_stamp: ed.doc_stamp,
            ed,
            mac,
            inflight: None,
            redraw: true,
        };
        s.resize(css_width, css_height);
        Ok(s)
    }

    pub fn resize(&mut self, css_width: f64, css_height: f64) {
        self.ed.viewport = v2(css_width.max(1.0), css_height.max(1.0));
        if self.ed.pending_fit && css_width > 1.0 {
            self.ed.pending_fit = false;
            self.ed.zoom_to_fit();
        }
        self.redraw = true;
    }

    /// Advance time; returns whether a frame should be drawn now.
    pub fn tick(&mut self, now_ms: f64) -> bool {
        let trail = self.ed.laser.len();
        self.ed.tick(now_ms);
        // Expired laser points leave the scene: that change needs a frame of its own (after
        // the last point expires `animating()` is already false). Stays pending until drawn.
        if self.ed.laser.len() != trail {
            self.redraw = true;
        }
        // Requests for app chrome (export, open, palette, rename...) have no handler in this
        // slice; drop them instead of letting them pile up.
        self.ed.requests.clear();
        self.ed.ui_events.clear();
        self.redraw || self.ed.animating(now_ms)
    }

    /// The frame for the current state was drawn.
    pub fn drawn(&mut self) {
        self.redraw = false;
    }

    pub fn modifiers(&mut self, shift: bool, ctrl: bool, alt: bool, meta: bool) {
        let m = keys::modifiers(self.mac, shift, ctrl, alt, meta);
        if m != self.ed.mods {
            self.ed.handle(InputEvent::Modifiers(m));
            self.redraw = true;
        }
    }

    pub fn pointer_down(&mut self, x: f64, y: f64, button: Button, time_ms: f64) {
        self.ed.handle(InputEvent::PointerDown {
            pos: v2(x, y),
            button,
            pressure: None,
            time_ms,
        });
        self.redraw = true;
    }

    pub fn pointer_move(&mut self, x: f64, y: f64, time_ms: f64) {
        self.ed.handle(InputEvent::PointerMove {
            pos: v2(x, y),
            pressure: None,
            time_ms,
        });
        self.redraw = true;
    }

    pub fn pointer_up(&mut self, x: f64, y: f64, button: Button, time_ms: f64) {
        self.ed.handle(InputEvent::PointerUp {
            pos: v2(x, y),
            button,
            time_ms,
        });
        self.redraw = true;
    }

    /// DOM wheel delta in CSS px (positive y = scroll down). With Ctrl/⌘ held (which is how
    /// browsers report a trackpad pinch) upstream zooms instead of panning.
    pub fn wheel(&mut self, dx: f64, dy: f64, x: f64, y: f64) {
        let d = if self.ed.mods.shift && dx == 0.0 {
            v2(dy, 0.0)
        } else {
            v2(dx, dy)
        };
        self.ed.handle(InputEvent::Scroll {
            delta: d,
            pos: v2(x, y),
        });
        self.redraw = true;
    }

    /// Returns whether the key was taken by the editor (the page then prevents the browser
    /// default). Keys the editor wouldn't act on are not sent and stay the browser's.
    pub fn key_down(&mut self, key: &str, code: &str, repeat: bool) -> bool {
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
        self.redraw = true;
        true
    }

    pub fn key_up(&mut self, key: &str, code: &str) {
        if let Some(k) = keys::key(self.ed.mods, self.ed.text_edit.is_some(), key, code) {
            self.ed.handle(InputEvent::KeyUp { key: k });
            self.redraw = true;
        }
    }

    /// Toolbar: "select", "hand", "rect", "ellipse".
    pub fn set_tool(&mut self, name: &str) {
        let tool = match name {
            "select" => Tool::Select,
            "hand" => Tool::Hand,
            "rect" => Tool::Rect,
            "ellipse" => Tool::Ellipse,
            _ => return,
        };
        self.run(Action::Tool(tool));
    }

    pub fn tool(&self) -> String {
        format!("{:?}", self.ed.tool).to_lowercase()
    }

    pub fn run(&mut self, a: Action) {
        self.ed.run(a);
        self.redraw = true;
    }

    /// Top-level nodes as JSON `[{id, name, kind, children, world: [x,y,w,h], screen: [x,y,w,h]}]`
    /// (screen = CSS px in the canvas), for status displays and automated checks.
    pub fn root_nodes(&self) -> String {
        let doc = &self.ed.doc;
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
                    "screen": [s.x, s.y, s.w, s.h],
                }))
            })
            .collect();
        serde_json::Value::from(list).to_string()
    }

    /// Content changed since the last confirmed save (or a save is in flight).
    pub fn dirty(&self) -> bool {
        self.ed.doc_stamp != self.saved_stamp
    }

    /// The page file to write now, or `None` when there is nothing to save, a save is
    /// already in flight, or a gesture/text edit still owns the open transaction.
    pub fn save_snapshot(&mut self) -> Option<String> {
        if self.inflight.is_some() || self.ed.is_busy() || self.ed.text_edit.is_some() {
            return None;
        }
        self.ed.commit_pending();
        if self.ed.doc_stamp == self.saved_stamp {
            return None;
        }
        self.inflight = Some(self.ed.doc_stamp);
        Some(page_to_json(&self.ed.page_data(self.ed.active_page)))
    }

    /// The server stored the last snapshot.
    pub fn save_succeeded(&mut self) {
        if let Some(s) = self.inflight.take() {
            self.saved_stamp = s;
        }
    }

    /// The last snapshot was not stored (conflict or network error); it stays dirty.
    pub fn save_failed(&mut self) {
        self.inflight = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::V2;
    use crate::model::{Document, Node, NodeKind};
    use crate::project::PageData;
    use crate::render::scene::{self, SceneCache};

    /// One 200×100 rectangle at world (100, 100); camera at zoom 1, no pan.
    fn state() -> WebState {
        let mut doc = Document::default();
        let id = doc.alloc_id();
        let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
        doc.nodes.insert(id, Node::new(id, NodeKind::Rect, rect));
        doc.root.push(id);
        let page = PageData {
            id: "p".into(),
            name: "Page 1".into(),
            doc,
            camera: Some(Default::default()),
        };
        let json = page_to_json(&page);
        let mut s =
            WebState::open(&json, "p".into(), "Page 1".into(), false, 800.0, 600.0).unwrap();
        assert!(s.tick(0.0), "first frame draws");
        s.drawn();
        s
    }

    fn overlay_vertices(s: &WebState) -> usize {
        let f = scene::canvas(&s.ed, &mut SceneCache::default(), V2::ZERO, true);
        f.over.iter().map(|b| b.mesh.vertices.len()).sum()
    }

    #[test]
    fn selection_click_redraws_although_revision_is_unchanged() {
        let mut s = state();
        let rev = s.ed.revision;
        let before = overlay_vertices(&s);
        assert!(!s.tick(16.0), "idle: nothing to draw");
        s.pointer_down(150.0, 150.0, Button::Primary, 20.0);
        s.pointer_up(150.0, 150.0, Button::Primary, 30.0);
        assert_eq!(s.ed.selection.len(), 1);
        assert_eq!(
            s.ed.revision, rev,
            "upstream revision doesn't track selection"
        );
        assert_ne!(
            overlay_vertices(&s),
            before,
            "selection changes the overlay"
        );
        assert!(s.tick(40.0), "so the adapter must draw");
        s.drawn();
        assert!(!s.tick(56.0));
    }

    /// Laser points at world (400, 400) .. (420, 410), drawn at t = 0..20 ms, then a frame.
    fn with_laser_trail() -> WebState {
        let mut s = state();
        s.run(Action::Tool(Tool::Laser));
        s.pointer_down(400.0, 400.0, Button::Primary, 0.0);
        s.pointer_move(410.0, 405.0, 10.0);
        s.pointer_up(420.0, 410.0, Button::Primary, 20.0);
        assert!(!s.ed.laser.is_empty());
        assert!(s.tick(100.0));
        s.drawn();
        assert!(overlay_vertices(&s) > 0, "trail on screen");
        s
    }

    #[test]
    fn laser_expiry_draws_the_clearing_frame() {
        let mut s = with_laser_trail();
        let mut t = 100.0;
        // Animate frame by frame until the trail is gone; the frame that removes it must draw.
        while !s.ed.laser.is_empty() {
            t += 16.0;
            assert!(s.tick(t), "frame at {t} ms while the trail fades");
            s.drawn();
            assert!(t < 5000.0);
        }
        assert_eq!(overlay_vertices(&s), 0, "nothing left to draw");
        assert!(!s.tick(t + 16.0), "idle afterwards");
    }

    #[test]
    fn laser_expiry_after_a_long_gap_still_draws() {
        let mut s = with_laser_trail();
        // E.g. a background tab: the next frame comes long after every point expired.
        assert!(s.tick(60_000.0), "the stale trail must be cleared");
        assert!(s.ed.laser.is_empty());
        assert_eq!(overlay_vertices(&s), 0);
        // Not drawn (e.g. the surface was unavailable): still pending on the next frame.
        assert!(s.tick(60_016.0), "pending until a frame is actually drawn");
        s.drawn();
        assert!(!s.tick(60_032.0));
    }

    #[test]
    fn hover_and_modifier_changes_redraw() {
        let mut s = state();
        s.pointer_move(150.0, 150.0, 10.0);
        assert!(s.ed.hover.is_some());
        assert!(s.tick(16.0));
        s.drawn();
        s.modifiers(false, false, true, false);
        assert!(s.tick(32.0), "Alt shows distance guides");
        s.drawn();
        s.modifiers(false, false, true, false);
        assert!(!s.tick(48.0), "same modifiers: no change");
    }

    #[test]
    fn select_all_is_consumed() {
        let mut s = state();
        s.modifiers(false, true, false, false);
        assert!(s.key_down("a", "KeyA", false), "Ctrl+A");
        assert_eq!(s.ed.selection.len(), 1);
        let mut mac = state();
        mac.mac = true;
        mac.modifiers(false, false, false, true);
        assert!(mac.key_down("a", "KeyA", false), "⌘A on a Mac");
    }

    #[test]
    fn editing_keys_are_consumed_and_act() {
        let mut s = state();
        assert!(s.key_down("r", "KeyR", false));
        assert_eq!(s.ed.tool, Tool::Rect);
        assert!(s.key_down("Escape", "Escape", false));
        assert_eq!(s.ed.tool, Tool::Select);
        s.pointer_down(150.0, 150.0, Button::Primary, 0.0);
        s.pointer_up(150.0, 150.0, Button::Primary, 10.0);
        assert!(s.key_down("Delete", "Delete", false));
        assert_eq!(s.ed.doc.nodes.len(), 0);
        s.modifiers(false, true, false, false);
        assert!(s.key_down("z", "KeyZ", false), "Ctrl+Z");
        assert_eq!(s.ed.doc.nodes.len(), 1);
        s.modifiers(true, true, false, false);
        assert!(s.key_down("Z", "KeyZ", false), "Ctrl+Shift+Z");
        assert_eq!(s.ed.doc.nodes.len(), 0);
        s.modifiers(false, false, false, false);
        assert!(s.key_down(" ", "Space", false), "Space (pan)");
    }

    #[test]
    fn chrome_shortcuts_and_unbound_keys_stay_with_the_browser() {
        let mut s = state();
        s.modifiers(false, true, false, false);
        for (k, code) in [("r", "KeyR"), ("s", "KeyS"), ("o", "KeyO"), ("k", "KeyK")] {
            let a = shortcuts::lookup(&Key::Char(k.chars().next().unwrap()), s.ed.mods);
            assert!(
                a.is_some_and(chrome_only),
                "Ctrl+{k} is a chrome action: {a:?}"
            );
            assert!(!s.key_down(k, code, false), "Ctrl+{k} not consumed");
        }
        assert!(s.ed.ui_events.is_empty() && s.ed.requests.is_empty());
        s.modifiers(false, false, false, false);
        assert!(!s.key_down("F5", "F5", false));
        assert!(!s.key_down("Shift", "ShiftLeft", false));
    }

    #[test]
    fn saves_follow_document_changes_only() {
        let mut s = state();
        assert!(s.save_snapshot().is_none(), "freshly opened");
        s.pointer_down(150.0, 150.0, Button::Primary, 0.0);
        s.pointer_up(150.0, 150.0, Button::Primary, 10.0);
        assert!(
            s.save_snapshot().is_none(),
            "selection alone isn't a change"
        );
        s.pointer_down(150.0, 150.0, Button::Primary, 20.0);
        s.pointer_move(190.0, 170.0, 30.0);
        assert!(s.save_snapshot().is_none(), "mid-drag");
        s.pointer_up(190.0, 170.0, Button::Primary, 40.0);
        let snap = s.save_snapshot().expect("drag moved the node");
        assert!(s.save_snapshot().is_none(), "one in flight");
        s.save_failed();
        assert!(s.dirty());
        assert_eq!(s.save_snapshot().as_deref(), Some(snap.as_str()));
        s.save_succeeded();
        assert!(!s.dirty());
        let saved = page_from_json(&snap).unwrap();
        let n = saved.doc.nodes.values().next().unwrap();
        assert!(n.rect.x > 100.0 && n.rect.y > 100.0, "moved: {:?}", n.rect);
    }
}
