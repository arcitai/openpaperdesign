//! The browser host of upstream's workspace, on the host (no GPU: `render::frame::layout` is
//! egui layout only). Positions are CSS px in the canvas element; `at` turns a point in the
//! canvas rectangle (which is the editor's own coordinate space) into one.

use super::*;
use crate::editor::Tool;
use crate::geom::V2;
use crate::model::{Node, NodeKind};
use crate::project::PageData;
use crate::render::scene::{self, SceneCache};

const W: f64 = 1280.0;
const H: f64 = 800.0;

/// One 200×100 rectangle at world (100, 100); camera at zoom 1, no pan; laid out once.
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
    let mut s = WebState::open(&json, "p".into(), "Page 1".into(), false, W, H).unwrap();
    settle(&mut s, 0.0);
    s
}

/// Lay out and "draw" until neither the editor nor egui asks for another frame.
fn settle(s: &mut WebState, mut t: f64) -> f64 {
    for _ in 0..30 {
        if !s.tick(t) {
            return t;
        }
        step(s);
        t += 16.0;
    }
    panic!("the workspace never went idle");
}

/// One frame (input since the last one is laid out), headless: nothing is painted, so egui's
/// texture uploads in this layout are discarded on purpose (`TexturesDelta::clear`, as egui
/// asks for an intentional drop). This fixture shows nothing about GPU texture delivery; in the
/// page, `web.rs` hands the whole layout to upstream `Painter::paint`.
fn step(s: &mut WebState) {
    let mut l = s.layout();
    l.output.textures_delta.clear();
    s.drawn();
}

/// A point of the canvas rectangle (editor coordinates) in canvas-element coordinates.
fn at(s: &WebState, x: f64, y: f64) -> (f64, f64) {
    let c = s.canvas_rect();
    (x + c.min.x as f64, y + c.min.y as f64)
}

fn click(s: &mut WebState, (x, y): (f64, f64), t: f64) {
    s.pointer_down(x, y, Button::Primary, t);
    s.pointer_up(x, y, Button::Primary, t + 10.0);
}

/// A click at a point of the canvas rectangle.
fn click_at(s: &mut WebState, x: f64, y: f64, t: f64) {
    let p = at(s, x, y);
    click(s, p, t);
}

fn drag(s: &mut WebState, (x0, y0): (f64, f64), (x1, y1): (f64, f64), t: f64) {
    s.pointer_move(x0, y0, t);
    s.pointer_down(x0, y0, Button::Primary, t);
    s.pointer_move((x0 + x1) / 2.0, (y0 + y1) / 2.0, t + 10.0);
    s.pointer_move(x1, y1, t + 20.0);
    s.pointer_up(x1, y1, Button::Primary, t + 30.0);
}

fn overlay_vertices(s: &WebState) -> usize {
    let f = scene::canvas(&s.ed, &mut SceneCache::default(), V2::ZERO, true);
    f.over.iter().map(|b| b.mesh.vertices.len()).sum()
}

// ---- the workspace and where input goes

#[test]
fn the_upstream_workspace_takes_its_panels_and_the_editor_gets_the_rest() {
    let s = state();
    let c = s.canvas_rect();
    // Upstream's layers (≥ 180 px), design inspector (≥ 220 px) and top bar.
    assert!(c.min.x >= 180.0, "layers panel on the left: {c:?}");
    assert!(c.max.x <= W as f32 - 220.0, "inspector on the right: {c:?}");
    assert!(c.min.y > 0.0, "top bar: {c:?}");
    assert_eq!(
        (s.ed.viewport.x, s.ed.viewport.y),
        (c.width() as f64, c.height() as f64),
        "the editor's viewport is the canvas rectangle, not the element"
    );
}

#[test]
fn a_click_in_the_canvas_selects_in_canvas_coordinates() {
    let mut s = state();
    let rev = s.ed.revision;
    let before = overlay_vertices(&s);
    // (150, 150) is inside the rectangle only in the canvas rectangle's own coordinates.
    click_at(&mut s, 150.0, 150.0, 20.0);
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
}

#[test]
fn panel_presses_never_reach_the_canvas() {
    let mut s = state();
    s.run(Action::Tool(Tool::Rect));
    step(&mut s);
    let nodes = s.ed.doc.nodes.len();
    // A drag over the layers panel (left of the canvas) and over the inspector (right).
    let c = s.canvas_rect();
    let left = (c.min.x as f64 / 2.0, 400.0);
    drag(&mut s, left, (left.0 + 60.0, 460.0), 0.0);
    step(&mut s);
    let right = (c.max.x as f64 + 40.0, 400.0);
    drag(&mut s, right, (right.0 + 60.0, 460.0), 100.0);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes.len(), nodes, "nothing drawn from a panel");
    // The same drag in the canvas draws one rectangle where it was dragged.
    let (from, to) = (at(&s, 400.0, 300.0), at(&s, 480.0, 360.0));
    drag(&mut s, from, to, 200.0);
    assert_eq!(s.ed.doc.nodes.len(), nodes + 1);
    let id = s.ed.selection[0];
    let r = s.ed.doc.world_rect(id).unwrap();
    assert_eq!((r.x, r.y, r.w, r.h), (400.0, 300.0, 80.0, 60.0), "{r:?}");
    assert!(s.dirty());
}

#[test]
fn hover_and_modifier_changes_redraw() {
    let mut s = state();
    let (x, y) = at(&s, 150.0, 150.0);
    s.pointer_move(x, y, 10.0);
    assert!(s.ed.hover.is_some());
    assert!(s.tick(16.0));
    step(&mut s);
    let t = settle(&mut s, 32.0);
    s.modifiers(false, false, true, false);
    assert!(s.tick(t), "Alt shows distance guides");
    let t = settle(&mut s, t);
    s.modifiers(false, false, true, false);
    assert!(!s.tick(t), "same modifiers: no change");
    // Leaving the canvas for a panel clears the hover.
    let c = s.canvas_rect();
    s.pointer_move(c.min.x as f64 / 2.0, 300.0, t);
    assert!(s.ed.hover.is_none());
}

// ---- upstream's layers panel: rename, with the keyboard owned by its field

#[test]
fn rename_in_the_layers_panel_keeps_keys_from_the_canvas_and_saves() {
    let mut s = state();
    click_at(&mut s, 150.0, 150.0, 0.0);
    let id = s.ed.selection[0];
    let old = s.ed.doc.get(id).unwrap().name.clone();
    // Upstream's Rename shortcut (Ctrl+R here) is the workspace's now, not the browser's.
    s.modifiers(false, true, false, false);
    assert!(s.key_down("r", "KeyR", false), "Ctrl+R renames");
    s.key_up("r", "KeyR");
    s.modifiers(false, false, false, false);
    step(&mut s);
    step(&mut s);
    assert!(
        s.ui.renaming.is_some(),
        "the layers panel's rename field is open"
    );
    assert!(s.field_has_keyboard(), "and it has the keyboard");
    // "e" is the Eraser shortcut and "x" deletes nothing here: both are the field's.
    for k in ["e", "x"] {
        assert!(s.key_down(k, &format!("Key{}", k.to_uppercase()), false));
        s.key_up(k, &format!("Key{}", k.to_uppercase()));
        step(&mut s);
    }
    assert_eq!(
        s.ed.tool,
        Tool::Select,
        "no tool shortcut reached the editor"
    );
    assert_eq!(
        s.ed.doc.get(id).unwrap().name,
        old,
        "not renamed before Enter"
    );
    assert!(s.key_down("Enter", "Enter", false));
    s.key_up("Enter", "Enter");
    step(&mut s);
    step(&mut s);
    let renamed = s.ed.doc.get(id).unwrap().name.clone();
    assert!(
        renamed != old && renamed.contains("ex"),
        "{old} -> {renamed}"
    );
    assert!(!s.field_has_keyboard());
    // One undoable edit of the canonical page, saved like any other.
    let snap = s.save_snapshot().expect("the rename is a change");
    assert!(snap.contains(&renamed));
    s.save_succeeded();
    s.modifiers(false, true, false, false);
    assert!(s.key_down("z", "KeyZ", false), "Ctrl+Z undoes it");
    assert_eq!(s.ed.doc.get(id).unwrap().name, old);
    assert!(s.dirty(), "the undo is a change to save too");
}

#[test]
fn a_click_on_the_canvas_commits_the_layers_panel_rename() {
    let mut s = state();
    click_at(&mut s, 150.0, 150.0, 0.0);
    let id = s.ed.selection[0];
    let old = s.ed.doc.get(id).unwrap().name.clone();
    s.modifiers(false, true, false, false);
    assert!(s.key_down("r", "KeyR", false));
    s.key_up("r", "KeyR");
    s.modifiers(false, false, false, false);
    step(&mut s);
    step(&mut s);
    assert!(s.field_has_keyboard(), "the rename field has the keyboard");
    assert!(s.key_down("q", "KeyQ", false));
    s.key_up("q", "KeyQ");
    step(&mut s);
    // Clicking empty canvas takes the keyboard from the field; losing it commits the name.
    click_at(&mut s, 700.0, 500.0, 100.0);
    step(&mut s);
    step(&mut s);
    let renamed = s.ed.doc.get(id).unwrap().name.clone();
    assert!(
        renamed != old && renamed.contains('q'),
        "{old} -> {renamed}"
    );
    assert!(s.ui.renaming.is_none(), "the field is closed");
    assert!(!s.field_has_keyboard());
    let snap = s.save_snapshot().expect("the rename is a change to save");
    assert!(snap.contains(&renamed));
}

/// Selects the rectangle and opens upstream's layer rename field (Ctrl+R), laid out.
fn open_rename(s: &mut WebState) -> crate::model::NodeId {
    click_at(s, 150.0, 150.0, 0.0);
    let id = s.ed.selection[0];
    s.modifiers(false, true, false, false);
    assert!(s.key_down("r", "KeyR", false));
    s.key_up("r", "KeyR");
    s.modifiers(false, false, false, false);
    step(s);
    step(s);
    assert!(s.field_has_keyboard(), "the rename field has the keyboard");
    id
}

fn press(s: &mut WebState, key: &str, code: &str) {
    assert!(s.key_down(key, code, false));
    s.key_up(key, code);
}

/// The page with node `id` renamed to `name`, as an agent saved it.
fn renamed_elsewhere(s: &WebState, id: crate::model::NodeId, name: &str) -> String {
    let mut page = s.ed.page_data(0);
    page.doc.nodes.get_mut(&id).unwrap().name = name.into();
    page_to_json(&page)
}

#[test]
fn an_unfinished_layer_rename_is_unsaved_work_until_it_is_committed_and_saved() {
    let mut s = state();
    let id = open_rename(&mut s);
    let old = s.ed.doc.get(id).unwrap().name.clone();
    assert!(
        s.can_reload(),
        "an open field with nothing typed loses nothing"
    );
    assert!(!s.unsaved_work());
    // Typed, not laid out yet: the typing is queued for the field.
    press(&mut s, "z", "KeyZ");
    assert!(!s.can_reload(), "queued typing for the field");
    assert!(s.unsaved_work());
    step(&mut s);
    // Laid out: a draft, not yet in the document.
    assert_eq!(s.ed.doc.get(id).unwrap().name, old, "still a draft");
    assert!(!s.dirty(), "the document itself is unchanged");
    assert!(s.unsaved_work(), "the page's leave guard sees the draft");
    assert!(!s.can_reload(), "no clean reload (or clean leave) over it");
    assert!(
        s.save_snapshot().is_none(),
        "nothing in the document to save yet"
    );
    // The page loses focus (a leave starts, or the panel's Back is clicked): the draft stays.
    s.set_focused(false);
    step(&mut s);
    assert!(s.ui.renaming.is_some());
    assert!(!s.can_reload(), "a refused leave keeps it unfinished");
    assert!(s.unsaved_work());
    s.set_focused(true);
    step(&mut s);
    // Enter commits it through upstream's rename; it saves like any other edit.
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    let renamed = s.ed.doc.get(id).unwrap().name.clone();
    assert!(
        renamed != old && renamed.contains('z'),
        "{old} -> {renamed}"
    );
    assert!(s.ui.renaming.is_none());
    assert!(s.dirty() && !s.can_reload(), "a change to save now");
    let snap = s.save_snapshot().expect("the rename is saved");
    assert!(snap.contains(&renamed));
    s.save_succeeded();
    assert!(s.can_reload(), "clean again after the save");
    assert!(!s.unsaved_work());
}

#[test]
fn an_agent_s_change_never_meets_a_stale_rename_draft() {
    // A typed draft: the agent's newer page isn't loaded over it. Committed, it saves on the
    // old revision, which the server refuses (409): a conflict, nothing reloads, and the
    // banner's explicit discard loads the agent's page and drops the local name.
    let mut s = state();
    let id = open_rename(&mut s);
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    let agent = renamed_elsewhere(&s, id, "Agent");
    assert!(!s.can_reload(), "the poll doesn't replace the page");
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert!(s.ed.doc.get(id).unwrap().name.contains('q'));
    let _snap = s.save_snapshot().expect("the local rename");
    s.save_failed();
    assert!(
        s.dirty() && !s.can_reload(),
        "a conflict keeps the local change"
    );
    s.reload(&agent).unwrap();
    assert_eq!(s.ed.doc.get(id).unwrap().name, "Agent");
    assert!(!s.unsaved_work());
    // An open field with nothing typed: a clean reload takes the agent's name, and the field
    // (for the replaced page) closes, so Enter afterwards can't write the old name back.
    let mut s = state();
    let id = open_rename(&mut s);
    assert!(s.can_reload());
    let agent = renamed_elsewhere(&s, id, "Agent");
    s.reload(&agent).unwrap();
    assert!(
        s.ui.renaming.is_none(),
        "the field closed with the old page"
    );
    s.key_down("Enter", "Enter", false);
    s.key_up("Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert_eq!(
        s.ed.doc.get(id).unwrap().name,
        "Agent",
        "the agent's name stays"
    );
    assert!(!s.dirty());
}

/// The egui pixel scale the next frame is painted at (headless).
fn painted_scale(s: &mut WebState) -> f32 {
    let mut l = s.layout();
    l.output.textures_delta.clear();
    s.drawn();
    l.output.pixels_per_point
}

#[test]
fn zoom_shortcuts_zoom_the_canvas_and_keep_painting_and_input_in_css_px() {
    for (mac, dpr) in [(false, 1.0), (false, 2.0), (true, 1.0), (true, 2.0)] {
        let mut s = state();
        s.mac = mac;
        s.resize(W, H, dpr);
        step(&mut s);
        step(&mut s);
        let canvas = s.canvas_rect();
        assert_eq!(painted_scale(&mut s), dpr as f32);
        let zoom = s.ed.camera.zoom;
        // ⌘ on a Mac, Ctrl elsewhere: two "=" (zoom in) and one "-" (zoom out).
        for (k, code) in [("=", "Equal"), ("=", "Equal"), ("-", "Minus")] {
            if mac {
                s.modifiers(false, false, false, true);
            } else {
                s.modifiers(false, true, false, false);
            }
            press(&mut s, k, code);
            s.modifiers(false, false, false, false);
            step(&mut s);
            step(&mut s);
        }
        assert!(
            s.ed.camera.zoom > zoom,
            "upstream's canvas zoom (mac {mac})"
        );
        let scale = painted_scale(&mut s);
        assert_eq!(
            scale, dpr as f32,
            "egui paints at the page's DPR only (mac {mac}, dpr {dpr})"
        );
        assert_eq!(s.canvas_rect(), canvas, "panels keep their CSS size");
        // A click where the rectangle is painted (device px / DPR = CSS px) selects it.
        let id = s.ed.doc.root[0];
        let r =
            s.ed.camera
                .world_rect_to_screen(&s.ed.doc.world_rect(id).unwrap());
        let device = (
            (canvas.min.x as f64 + r.x + r.w / 2.0) * scale as f64,
            (canvas.min.y as f64 + r.y + r.h / 2.0) * scale as f64,
        );
        click(&mut s, (device.0 / dpr, device.1 / dpr), 900.0);
        assert_eq!(s.ed.selection, vec![id], "mac {mac}, dpr {dpr}");
    }
}

#[test]
fn inspector_edits_are_coalesced_undoable_changes_of_the_saved_page() {
    let mut s = state();
    click_at(&mut s, 150.0, 150.0, 0.0);
    step(&mut s);
    let id = s.ed.selection[0];
    // What the inspector's width field does on every change (upstream `inspector::apply`).
    for w in [210.0, 220.0, 230.0] {
        ui::inspector::apply(&mut s.ed, "width", &[id], |n| n.rect.w = w);
        step(&mut s);
    }
    assert_eq!(s.ed.doc.get(id).unwrap().rect.w, 230.0);
    let snap = s.save_snapshot().expect("a change to save");
    let saved = page_from_json(&snap).unwrap();
    assert_eq!(saved.doc.get(id).unwrap().rect.w, 230.0);
    s.save_succeeded();
    s.run(Action::Undo);
    assert_eq!(
        s.ed.doc.get(id).unwrap().rect.w,
        200.0,
        "one coalesced undo step for the whole field edit"
    );
}

// ---- pages: shown, never changed

/// Page ids and names, active page, document stamp, the page file, selection, undo depth.
type PageState = (
    Vec<(String, String)>,
    usize,
    u64,
    String,
    Vec<crate::model::NodeId>,
    usize,
);

/// Everything a page action could disturb.
fn page_state(s: &WebState) -> PageState {
    (
        s.ed.pages
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect(),
        s.ed.active_page,
        s.ed.doc_stamp,
        page_to_json(&s.ed.page_data(s.ed.active_page)),
        s.ed.selection.clone(),
        s.ed.history.undo_len(),
    )
}

/// Clicks and double-clicks over the top of the left sidebar: the pages panel's header (with
/// Add page) and first row, above the layers tabs and their arrange buttons.
fn press_the_pages_panel(s: &mut WebState) {
    let c = s.canvas_rect();
    let mut t = 1000.0;
    let mut y = 4.0;
    while y < 56.0 {
        let mut x = 4.0;
        while x < c.min.x as f64 {
            for _ in 0..2 {
                click(s, (x, y), t);
                t += 30.0;
            }
            step(s);
            t += 500.0;
            x += 12.0;
        }
        y += 6.0;
    }
}

#[test]
fn page_actions_are_disabled_and_change_nothing() {
    // The control: the same presses with page actions available do change pages.
    let mut open = state();
    open.ui.pages_locked = false;
    step(&mut open);
    let before = page_state(&open);
    press_the_pages_panel(&mut open);
    assert!(
        open.ed.pages.len() > 1 || open.ui.pages.renaming.is_some() || page_state(&open) != before,
        "the presses reach the pages panel's controls"
    );
    // This host: page actions are off, so exactly nothing changes.
    let mut s = state();
    click_at(&mut s, 150.0, 150.0, 0.0);
    step(&mut s);
    assert!(s.ui.pages_locked);
    let before = page_state(&s);
    press_the_pages_panel(&mut s);
    assert!(s.ui.pages.renaming.is_none());
    assert_eq!(
        page_state(&s),
        before,
        "page list, document, history and selection kept"
    );
    assert!(!s.dirty());
    // Next/previous page from the keyboard: one page, so the active page stays.
    s.key_down("PageDown", "PageDown", false);
    s.key_down("PageUp", "PageUp", false);
    step(&mut s);
    assert_eq!(page_state(&s), before);
}

/// What the owner did in a real browser, at one point of the inspector: click, select all,
/// type "X", Enter.
fn type_page_name(s: &mut WebState, (x, y): (f64, f64)) {
    click(s, (x, y), 5000.0);
    step(s);
    s.modifiers(false, true, false, false);
    s.key_down("a", "KeyA", false);
    s.key_up("a", "KeyA");
    s.modifiers(false, false, false, false);
    step(s);
    for (k, code) in [("X", "KeyX"), ("Enter", "Enter")] {
        s.key_down(k, code, false);
        s.key_up(k, code);
        step(s);
    }
    step(s);
}

/// The inspector's left edge: labels, the full-width page name field, checkboxes and small
/// buttons (not the sliders, swatches or colour button further right).
fn inspector_edge(s: &WebState) -> f64 {
    s.canvas_rect().max.x as f64 + 24.0
}

#[test]
fn the_inspector_page_name_is_read_only_where_pages_are() {
    // The control: where page changes can be stored (the native default), the inspector's page
    // name field (nothing selected) takes that input and renames the page.
    let x = inspector_edge(&state());
    let mut found = None;
    let mut y = 40.0;
    while found.is_none() && y < H - 40.0 {
        let mut open = state();
        open.ui.pages_locked = false;
        step(&mut open);
        type_page_name(&mut open, (x, y));
        if open.ed.page_name() == "X" {
            found = Some(y);
        }
        y += 8.0;
    }
    let y = found.expect("the field renames the page where pages can change");
    // This host: the same click and keys at the same place change nothing about the page.
    let mut s = state();
    assert!(s.ui.pages_locked);
    let page = |s: &WebState| {
        (
            s.ed.pages
                .iter()
                .map(|p| (p.id.clone(), p.name.clone()))
                .collect::<Vec<_>>(),
            s.ed.active_page,
            page_to_json(&s.ed.page_data(s.ed.active_page)),
            s.ed.history.undo_len(),
        )
    };
    let before = page(&s);
    click(&mut s, (x, y), 5000.0);
    step(&mut s);
    assert!(
        !s.field_has_keyboard(),
        "the page name field is disabled, so it takes no keyboard"
    );
    type_page_name(&mut s, (x, y));
    assert_eq!(s.ed.page_name(), "Page 1");
    assert!(!s.ed.meta_dirty, "no page change is pending");
    assert_eq!(page(&s), before, "page list, page file and history kept");
    assert!(!s.dirty(), "nothing changed, so nothing is shown as saved");
    assert!(s.save_snapshot().is_none());
}

// ---- keys

#[test]
fn editing_keys_are_consumed_and_act() {
    let mut s = state();
    assert!(s.key_down("r", "KeyR", false));
    assert_eq!(s.ed.tool, Tool::Rect);
    assert!(s.key_down("Escape", "Escape", false));
    assert_eq!(s.ed.tool, Tool::Select);
    click_at(&mut s, 150.0, 150.0, 0.0);
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
fn clipboard_keys_and_unbound_keys_stay_with_the_browser() {
    let mut s = state();
    s.modifiers(false, true, false, false);
    for (k, code) in [("c", "KeyC"), ("x", "KeyX"), ("v", "KeyV")] {
        assert!(
            !s.key_down(k, code, false),
            "Ctrl+{k}: the page's clipboard event runs it"
        );
    }
    // Workspace shortcuts that the old toolbar page left to the browser are the editor's.
    for (k, code) in [("k", "KeyK"), ("s", "KeyS")] {
        assert!(s.key_down(k, code, false), "Ctrl+{k} is the workspace's");
    }
    s.modifiers(false, false, false, false);
    assert!(!s.key_down("F5", "F5", false));
    assert!(!s.key_down("Shift", "ShiftLeft", false));
}

#[test]
fn the_command_palette_opens_and_owns_the_keyboard() {
    let mut s = state();
    s.modifiers(false, true, false, false);
    assert!(s.key_down("k", "KeyK", false));
    s.modifiers(false, false, false, false);
    step(&mut s);
    step(&mut s);
    assert!(s.ui.palette.open, "upstream's palette");
    assert!(s.field_has_keyboard(), "its search field has the keyboard");
    assert!(s.key_down("r", "KeyR", false));
    assert_eq!(
        s.ed.tool,
        Tool::Select,
        "typing searches, it doesn't switch tools"
    );
}

#[test]
fn help_and_hide_ui_are_the_workspace_s_own() {
    let mut s = state();
    s.modifiers(false, true, false, false);
    assert!(
        s.key_down("/", "Slash", false),
        "Ctrl+/ shows the shortcuts"
    );
    s.modifiers(false, false, false, false);
    step(&mut s);
    assert!(s.ui.show_help);
    s.modifiers(false, true, false, false);
    assert!(
        s.key_down("\\", "Backslash", false),
        "Ctrl+\\ hides the panels"
    );
    s.modifiers(false, false, false, false);
    step(&mut s);
    assert!(s.ui.hide_ui);
    step(&mut s);
    let c = s.canvas_rect();
    assert_eq!(
        (c.min.x, c.min.y),
        (0.0, 0.0),
        "the canvas takes the whole element"
    );
}

// ---- requests

#[test]
fn save_and_home_go_to_the_page_and_nothing_is_dropped() {
    let mut s = state();
    s.modifiers(false, true, false, false);
    assert!(s.key_down("s", "KeyS", false));
    s.modifiers(false, false, false, false);
    step(&mut s);
    assert_eq!(s.take_request(), Some(HostRequest::Save));
    for r in [AppRequest::Home, AppRequest::New, AppRequest::Open] {
        s.ed.requests.push(r);
        step(&mut s);
        assert_eq!(s.take_request(), Some(HostRequest::Home));
    }
    assert_eq!(s.take_request(), None);
    let before = page_to_json(&s.ed.page_data(0));
    for (r, why) in [
        (AppRequest::SaveAs, "Duplicating"),
        (AppRequest::PlaceImage, "Placing images"),
        (AppRequest::CopyAsPng, "PNG"),
        (AppRequest::ExportFormat(Format::Png), "PNG"),
    ] {
        s.ed.requests.push(r);
        step(&mut s);
        assert!(s.ed.status.contains(why), "{}", s.ed.status);
        assert!(s.take_download().is_none());
    }
    assert_eq!(page_to_json(&s.ed.page_data(0)), before, "nothing changed");
    assert!(s.ed.requests.is_empty());
}

#[test]
fn svg_and_pdf_exports_are_real_files_of_the_page() {
    let mut s = state();
    s.ed.requests.push(AppRequest::ExportFormat(Format::Svg));
    step(&mut s);
    let svg = s.take_download().expect("an SVG download");
    assert_eq!(svg.mime, "image/svg+xml");
    assert!(svg.name.ends_with(".svg"));
    let text = String::from_utf8(svg.bytes).unwrap();
    assert!(text.contains("<svg"), "{text}");
    s.ed.requests.push(AppRequest::ExportFormat(Format::Pdf));
    step(&mut s);
    let pdf = s.take_download().expect("a PDF download");
    assert_eq!(pdf.mime, "application/pdf");
    assert!(pdf.bytes.starts_with(b"%PDF"), "a PDF file");
    assert!(!s.dirty(), "exporting changes nothing");
}

// ---- clipboard

#[test]
fn copy_and_paste_go_through_the_page_s_clipboard_events() {
    let mut s = state();
    click_at(&mut s, 150.0, 150.0, 0.0);
    let text = s
        .copy(false)
        .expect("the copy event gets the editor's text");
    assert!(text.contains("paper-rs/clipboard"), "{text}");
    assert!(s.key_down("Delete", "Delete", false));
    assert_eq!(s.ed.doc.nodes.len(), 0);
    s.paste(&text);
    assert_eq!(s.ed.doc.nodes.len(), 1, "pasted from the event's text");
    assert!(s.dirty());
}

// ---- redraw rules (from the toolbar page, kept)

/// Laser points at canvas (400, 400) .. (420, 410), drawn at t = 0..20 ms, then a frame.
fn with_laser_trail() -> WebState {
    let mut s = state();
    s.run(Action::Tool(Tool::Laser));
    let (x0, y0) = at(&s, 400.0, 400.0);
    s.pointer_down(x0, y0, Button::Primary, 0.0);
    s.pointer_move(x0 + 10.0, y0 + 5.0, 10.0);
    s.pointer_up(x0 + 20.0, y0 + 10.0, Button::Primary, 20.0);
    assert!(!s.ed.laser.is_empty());
    assert!(s.tick(100.0));
    step(&mut s);
    assert!(overlay_vertices(&s) > 0, "trail on screen");
    s
}

#[test]
fn laser_expiry_draws_the_clearing_frame() {
    let mut s = with_laser_trail();
    let mut t = 100.0;
    while !s.ed.laser.is_empty() {
        t += 16.0;
        assert!(s.tick(t), "frame at {t} ms while the trail fades");
        step(&mut s);
        assert!(t < 5000.0);
    }
    assert_eq!(overlay_vertices(&s), 0, "nothing left to draw");
}

#[test]
fn laser_expiry_after_a_long_gap_still_draws() {
    let mut s = with_laser_trail();
    assert!(s.tick(60_000.0), "the stale trail must be cleared");
    assert!(s.ed.laser.is_empty());
    assert_eq!(overlay_vertices(&s), 0);
    // Not drawn (e.g. the surface was unavailable): still pending on the next frame.
    assert!(s.tick(60_016.0), "pending until a frame is actually drawn");
}

// ---- saving and reloading (unchanged rules)

/// The page with the rectangle moved to x and a second rectangle, as another writer saved it.
fn changed_elsewhere(s: &WebState, x: f64) -> String {
    let mut page = s.ed.page_data(0);
    let id = *page.doc.nodes.keys().next().unwrap();
    page.doc.nodes.get_mut(&id).unwrap().rect.x = x;
    let n = page.doc.alloc_id();
    page.doc.nodes.insert(
        n,
        Node::new(
            n,
            NodeKind::Rect,
            crate::geom::Rect::new(0.0, 0.0, 5.0, 5.0),
        ),
    );
    page.doc.root.push(n);
    page_to_json(&page)
}

#[test]
fn reload_only_when_clean_and_it_keeps_the_view() {
    let mut s = state();
    s.ed.camera.pan = v2(-40.0, 10.0);
    click_at(&mut s, 150.0 + 40.0, 150.0 - 10.0, 0.0);
    assert_eq!(s.ed.selection.len(), 1);
    let selected = s.ed.selection[0];
    assert!(s.can_reload(), "clean: may reload");
    let newer = changed_elsewhere(&s, 300.0);
    s.reload(&newer).unwrap();
    assert_eq!(s.ed.doc.nodes.len(), 2);
    assert_eq!(s.ed.doc.nodes[&selected].rect.x, 300.0);
    assert_eq!(s.ed.camera.pan, v2(-40.0, 10.0), "camera kept");
    assert_eq!(s.ed.selection, vec![selected], "selection kept");
    assert!(!s.dirty(), "the reloaded page is the saved state");
    assert!(s.tick(100.0), "redraws");
}

#[test]
fn never_reload_over_unsaved_or_in_flight_edits() {
    let mut s = state();
    let (x, y) = at(&s, 150.0, 150.0);
    s.pointer_move(x, y, 0.0);
    s.pointer_down(x, y, Button::Primary, 0.0);
    s.pointer_move(x + 20.0, y + 10.0, 10.0);
    assert!(!s.can_reload(), "dragging");
    s.pointer_up(x + 20.0, y + 10.0, Button::Primary, 20.0);
    assert!(!s.can_reload(), "unsaved move");
    let _snap = s.save_snapshot().unwrap();
    assert!(!s.can_reload(), "save in flight");
    s.save_succeeded();
    assert!(s.can_reload(), "saved: clean again");
    click(&mut s, (x + 20.0, y + 10.0), 30.0);
    assert!(
        s.can_reload(),
        "a selection on its own is not an unsaved change"
    );
}

#[test]
fn saves_follow_document_changes_only() {
    let mut s = state();
    assert!(s.save_snapshot().is_none(), "freshly opened");
    let (x, y) = at(&s, 150.0, 150.0);
    click(&mut s, (x, y), 0.0);
    assert!(
        s.save_snapshot().is_none(),
        "selection alone isn't a change"
    );
    s.pointer_down(x, y, Button::Primary, 20.0);
    s.pointer_move(x + 40.0, y + 20.0, 30.0);
    assert!(s.save_snapshot().is_none(), "mid-drag");
    s.pointer_up(x + 40.0, y + 20.0, Button::Primary, 40.0);
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
