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

/// Page `id` named `name` with one rectangle at `rect` (world); camera at zoom 1, no pan.
fn page(id: &str, name: &str, rect: crate::geom::Rect) -> PageData {
    let mut doc = Document::default();
    let n = doc.alloc_id();
    doc.nodes.insert(n, Node::new(n, NodeKind::Rect, rect));
    doc.root.push(n);
    PageData {
        id: id.into(),
        name: name.into(),
        doc,
        camera: Some(Default::default()),
    }
}

/// The canvas as the server sends it (`/api/canvas`): `pages` in list order, `active` shown.
fn canvas(pages: &[PageData], active: &str) -> String {
    let list: Vec<serde_json::Value> = pages
        .iter()
        .map(|p| {
            serde_json::json!({"id": p.id, "name": p.name, "revision": "\"r\"", "text": page_to_json(p)})
        })
        .collect();
    serde_json::json!({"canvas": {"name": "Canvas"}, "revision": "\"c\"", "active_page": active, "pages": list})
        .to_string()
}

/// Every page of the editor as it is now (what a server would serve once all of it was saved).
fn pages_now(s: &WebState) -> Vec<PageData> {
    (0..s.ed.pages.len()).map(|i| s.ed.page_data(i)).collect()
}

/// A canvas save as the editor sends it: the active page, the page list `(id, name)` and the
/// pages whose content it carries `(id, page file)`.
type Save = (String, Vec<(String, String)>, Vec<(String, PageData)>);

fn parse_save(snap: &str) -> Save {
    let v: serde_json::Value = serde_json::from_str(snap).unwrap();
    let list = v["pages"].as_array().unwrap();
    let id = |p: &serde_json::Value| p["id"].as_str().unwrap().to_string();
    let names = list
        .iter()
        .map(|p| (id(p), p["name"].as_str().unwrap().to_string()))
        .collect();
    let sent = list
        .iter()
        .filter_map(|p| {
            let text = p.get("text")?.as_str()?;
            Some((id(p), page_from_json(text).unwrap()))
        })
        .collect();
    (v["active_page"].as_str().unwrap().to_string(), names, sent)
}

/// The one page file a canvas save carries.
fn sent_page(snap: &str) -> PageData {
    let (_, _, mut sent) = parse_save(snap);
    assert_eq!(sent.len(), 1, "one page's content");
    sent.remove(0).1
}

/// One 200×100 rectangle at world (100, 100) on the canvas's one page; camera at zoom 1, no
/// pan; laid out once.
fn state() -> WebState {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let pages = [page("p", "Page 1", rect)];
    let mut s = WebState::open(&canvas(&pages, "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    s
}

/// Two pages: "Alpha" (a 200×100 rectangle at (100, 100)) and "Beta" (a 50×50 one at
/// (400, 300)), as the server has them.
fn two_pages_data() -> Vec<PageData> {
    vec![
        page(
            "alpha1",
            "Alpha",
            crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0),
        ),
        page(
            "beta22",
            "Beta",
            crate::geom::Rect::new(400.0, 300.0, 50.0, 50.0),
        ),
    ]
}

/// [`two_pages_data`] opened on its remembered active page, "Beta"; laid out.
fn two_pages() -> WebState {
    let mut s = WebState::open(&canvas(&two_pages_data(), "beta22"), false, W, H).unwrap();
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

/// The canvas with node `id` of the first page renamed to `name`, as an agent saved it.
fn renamed_elsewhere(s: &WebState, id: crate::model::NodeId, name: &str) -> String {
    let mut pages = pages_now(s);
    pages[0].doc.nodes.get_mut(&id).unwrap().name = name.into();
    canvas(&pages, s.ed.active_page_id())
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

fn shape_text(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::Shape::Vec(v) => {
            for s in v {
                shape_text(s, out);
            }
        }
        egui::Shape::Text(t) => out.push((
            t.galley.job.text.clone(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        _ => {}
    }
}

/// One headless frame, returning every text it paints with its bounds (CSS px).
fn painted_text(s: &mut WebState) -> Vec<(String, egui::Rect)> {
    let mut l = s.layout();
    let mut text = Vec::new();
    for shape in &l.output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    l.output.textures_delta.clear();
    s.drawn();
    text
}

fn paints(s: &mut WebState, wanted: &str) -> bool {
    painted_text(s).iter().any(|(t, _)| t.trim() == wanted)
}

/// Selects the rectangle, clicks upstream's inspector number field whose painted text starts
/// with `label` ("X  ", "Y  ") at its painted bounds, and selects its text (Ctrl+A).
/// Adapted from the R19 review's probe helper.
fn open_number_field(s: &mut WebState, label: &str) -> crate::model::NodeId {
    click_at(s, 150.0, 150.0, 0.0);
    step(s);
    step(s);
    let id = s.ed.selection[0];
    let text = painted_text(s);
    let (_, bounds) = text
        .iter()
        .find(|(t, _)| t.starts_with(label))
        .unwrap_or_else(|| panic!("{label:?} field not painted: {text:?}"));
    let p = bounds.center();
    click(s, (p.x as f64, p.y as f64), 1000.0);
    step(s);
    step(s);
    assert!(
        s.field_has_keyboard(),
        "the {label:?} field is in its text mode"
    );
    s.modifiers(false, true, false, false);
    press(s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(s);
    id
}

/// The rectangle's position on the axis a field edits.
fn axis(s: &WebState, id: crate::model::NodeId, label: &str) -> f64 {
    let r = s.ed.doc.get(id).unwrap().rect;
    if label.starts_with('X') { r.x } else { r.y }
}

#[test]
fn an_inspector_number_draft_is_unsaved_work_until_upstream_finishes_it() {
    for label in ["X  ", "Y  "] {
        let mut s = state();
        let id = open_number_field(&mut s, label);
        let old = axis(&s, id, label);
        // An open number field may hold a draft: busy from the moment it has the keyboard.
        assert!(s.unsaved_work() && !s.can_reload(), "{label}: open field");
        // "-" before a negative number: queued, then laid out and painted, not a value yet.
        s.paste("-");
        assert!(!s.can_reload(), "{label}: queued");
        step(&mut s);
        assert!(paints(&mut s, "-"), "{label}: the field shows the draft");
        assert_eq!(axis(&s, id, label), old, "{label}: not in the document");
        assert!(!s.dirty(), "{label}: nothing to save yet");
        assert!(s.save_snapshot().is_none());
        assert!(s.unsaved_work(), "{label}: the leave guard sees it");
        assert!(!s.can_reload(), "{label}: no clean reload or leave over it");
        // The page loses focus (a leave starts): refused, and the draft stays.
        s.set_focused(false);
        step(&mut s);
        assert!(paints(&mut s, "-"), "{label}: kept through the blur");
        assert!(
            !s.can_reload() && s.unsaved_work(),
            "{label}: still unfinished"
        );
        s.set_focused(true);
        step(&mut s);
        // Completed, upstream applies it; Enter finishes the field; the change saves.
        s.paste("150");
        step(&mut s);
        assert_eq!(axis(&s, id, label), -150.0, "{label}: completed");
        press(&mut s, "Enter", "Enter");
        step(&mut s);
        step(&mut s);
        assert!(!s.field_has_keyboard(), "{label}: finished");
        assert!(s.dirty());
        let _snap = s.save_snapshot().expect("the new position is saved");
        s.save_succeeded();
        assert!(
            s.can_reload() && !s.unsaved_work(),
            "{label}: clean after the save"
        );
    }
}

#[test]
fn escape_cancels_an_inspector_number_draft_and_the_field_is_clean_again() {
    let mut s = state();
    let id = open_number_field(&mut s, "X  ");
    s.paste("-");
    step(&mut s);
    assert!(paints(&mut s, "-"));
    press(&mut s, "Escape", "Escape");
    step(&mut s);
    step(&mut s);
    assert!(!s.field_has_keyboard(), "upstream cancelled the field");
    assert!(
        !paints(&mut s, "-"),
        "the draft is gone, as upstream cancels it"
    );
    assert_eq!(axis(&s, id, "X  "), 100.0);
    assert!(!s.dirty() && !s.unsaved_work() && s.can_reload());
}

#[test]
fn an_agent_s_change_never_meets_a_stale_number_draft() {
    // A "-" draft: the poll can't reload over it; the explicit discard (the conflict banner's
    // reload) loads the agent's page and nothing of the draft comes back.
    let mut s = state();
    let id = open_number_field(&mut s, "X  ");
    s.paste("-");
    step(&mut s);
    let agent = renamed_elsewhere(&s, id, "Agent");
    assert!(!s.can_reload(), "the poll doesn't replace the page");
    s.reload(&agent).unwrap();
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.doc.get(id).unwrap().name, "Agent");
    assert_eq!(axis(&s, id, "X  "), 100.0);
    assert!(
        !paints(&mut s, "-"),
        "the discarded draft isn't shown again"
    );
    assert!(!s.field_has_keyboard() && !s.unsaved_work() && s.can_reload());
    // A completed value saved on the old revision: refused (409) and kept; the explicit
    // discard loads the agent's page, and the discarded value isn't applied again.
    let mut s = state();
    let id = open_number_field(&mut s, "X  ");
    // The agent's page, saved from the revision this view loaded (X 100).
    let agent = renamed_elsewhere(&s, id, "Agent");
    s.paste("-150");
    step(&mut s);
    assert_eq!(axis(&s, id, "X  "), -150.0);
    let _snap = s.save_snapshot().expect("the local position");
    s.save_failed();
    assert!(
        s.dirty() && !s.can_reload(),
        "a conflict keeps the local change"
    );
    s.reload(&agent).unwrap();
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.doc.get(id).unwrap().name, "Agent");
    assert_eq!(
        axis(&s, id, "X  "),
        100.0,
        "the discarded value stays discarded"
    );
    assert!(!s.dirty() && !s.unsaved_work());
}

/// Opens upstream's command palette (Ctrl+K); its search field takes the keyboard.
fn open_palette(s: &mut WebState) {
    s.modifiers(false, true, false, false);
    press(s, "k", "KeyK");
    s.modifiers(false, false, false, false);
    step(s);
    step(s);
    assert!(s.ui.palette.open && s.field_has_keyboard());
}

/// Selects the rectangle, opens the palette, then clicks the painted inspector X field and
/// selects its text: the palette stays open, the field has the keyboard. Adapted from the R20
/// review's probe.
fn open_x_with_palette_open(s: &mut WebState) -> crate::model::NodeId {
    click_at(s, 150.0, 150.0, 0.0);
    step(s);
    step(s);
    let id = s.ed.selection[0];
    open_palette(s);
    let search = s.ctx.memory(|m| m.focused());
    assert_eq!(
        search, s.ui.palette.search_id,
        "the search has the keyboard"
    );
    let text = painted_text(s);
    let (_, bounds) = text
        .iter()
        .find(|(t, _)| t.starts_with("X  "))
        .unwrap_or_else(|| panic!("X field not painted: {text:?}"));
    let p = bounds.center();
    click(s, (p.x as f64, p.y as f64), 1000.0);
    step(s);
    step(s);
    assert!(s.ui.palette.open, "the palette stays open");
    assert!(s.field_has_keyboard(), "the X field has the keyboard");
    assert_ne!(s.ctx.memory(|m| m.focused()), search, "not the search");
    s.modifiers(false, true, false, false);
    press(s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(s);
    id
}

#[test]
fn a_palette_search_is_ephemeral_and_never_taken_for_a_document_field() {
    let mut s = state();
    open_palette(&mut s);
    s.paste("no matching command qzqz");
    assert!(!s.can_reload(), "queued typing for the field");
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ui.palette.query, "no matching command qzqz");
    assert!(s.field_has_keyboard(), "the search still has the keyboard");
    assert!(!s.dirty() && !s.unsaved_work() && s.can_reload());
    press(&mut s, "Escape", "Escape");
    step(&mut s);
    step(&mut s);
    assert!(!s.ui.palette.open && !s.unsaved_work() && s.can_reload());
}

#[test]
fn an_open_palette_never_exempts_a_number_draft_in_the_inspector() {
    let mut s = state();
    let id = open_x_with_palette_open(&mut s);
    assert!(
        s.unsaved_work() && !s.can_reload(),
        "the inspector field, not the search, has the keyboard"
    );
    s.paste("-");
    step(&mut s);
    assert!(paints(&mut s, "-"), "the field shows the draft");
    assert_eq!(axis(&s, id, "X  "), 100.0);
    assert!(!s.dirty() && s.save_snapshot().is_none());
    assert!(
        s.unsaved_work() && !s.can_reload(),
        "laid out: still guarded"
    );
    s.set_focused(false);
    step(&mut s);
    assert!(paints(&mut s, "-"));
    assert!(s.ui.palette.open);
    assert!(
        s.unsaved_work() && !s.can_reload(),
        "after the blur: still guarded"
    );
    // So no clean reload can drop it; the explicit discard (the conflict banner) does, once.
    let agent = renamed_elsewhere(&s, id, "Agent");
    s.reload(&agent).unwrap();
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.doc.get(id).unwrap().name, "Agent");
    assert_eq!(axis(&s, id, "X  "), 100.0);
    assert!(
        !paints(&mut s, "-"),
        "the discarded draft isn't shown again"
    );
}

#[test]
fn a_number_draft_beside_an_open_palette_finishes_like_any_other() {
    // Completed and finished with Enter: upstream applies it and it saves.
    let mut s = state();
    let id = open_x_with_palette_open(&mut s);
    s.paste("-150");
    step(&mut s);
    assert_eq!(axis(&s, id, "X  "), -150.0);
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert!(s.dirty());
    let _snap = s.save_snapshot().expect("the new position is saved");
    s.save_succeeded();
    assert!(!s.unsaved_work() && s.can_reload(), "clean after the save");
    // Cancelled with Escape: upstream drops the draft (and closes the palette); clean again.
    let mut s = state();
    let id = open_x_with_palette_open(&mut s);
    s.paste("-");
    step(&mut s);
    press(&mut s, "Escape", "Escape");
    step(&mut s);
    step(&mut s);
    assert_eq!(axis(&s, id, "X  "), 100.0);
    assert!(!s.dirty() && !s.unsaved_work() && s.can_reload());
    // Finished by a click on the canvas: focus leaves the field; clean again.
    let mut s = state();
    let id = open_x_with_palette_open(&mut s);
    s.paste("-");
    step(&mut s);
    click_at(&mut s, 700.0, 600.0, 2000.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(axis(&s, id, "X  "), 100.0);
    assert!(!s.field_has_keyboard());
    assert!(!s.dirty() && !s.unsaved_work() && s.can_reload());
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
    let saved = sent_page(&snap);
    assert_eq!(saved.doc.get(id).unwrap().rect.w, 230.0);
    s.save_succeeded();
    s.run(Action::Undo);
    assert_eq!(
        s.ed.doc.get(id).unwrap().rect.w,
        200.0,
        "one coalesced undo step for the whole field edit"
    );
}

// ---- pages: upstream's pages panel on the whole canvas, saved as one canvas

/// Where text `wanted` is painted (its centre, CSS px) in the region `within` accepts.
fn painted_at(s: &mut WebState, wanted: &str, within: impl Fn(egui::Rect) -> bool) -> (f64, f64) {
    let text = painted_text(s);
    let (_, r) = text
        .iter()
        .find(|(t, r)| t.trim() == wanted && within(*r))
        .unwrap_or_else(|| panic!("{wanted:?} isn't painted there: {text:?}"));
    (r.center().x as f64, r.center().y as f64)
}

/// The row of page `name` in upstream's pages panel (the left sidebar).
fn page_row(s: &mut WebState, name: &str) -> (f64, f64) {
    let left = s.canvas_rect().min.x;
    painted_at(s, name, |r| r.max.x <= left)
}

/// A click on page `name`'s row, laid out.
fn click_page(s: &mut WebState, name: &str, t: f64) {
    let p = page_row(s, name);
    click(s, p, t);
    step(s);
    step(s);
}

/// Item `item` of page `name`'s context menu (upstream's Rename, Duplicate, Delete), opening
/// the menu with a secondary click on the row if it isn't shown.
fn menu_item(s: &mut WebState, name: &str, item: &str, t: f64) -> (f64, f64) {
    if !paints(s, item) {
        let (x, y) = page_row(s, name);
        s.pointer_down(x, y, Button::Secondary, t);
        s.pointer_up(x, y, Button::Secondary, t + 10.0);
        step(s);
        step(s);
    }
    painted_at(s, item, |_| true)
}

#[test]
fn every_page_opens_on_the_remembered_one_and_a_switch_saves_as_the_page_list() {
    let mut s = two_pages();
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Alpha", "Beta"], "every page, in list order");
    assert_eq!(s.ed.page_name(), "Beta", "the remembered active page");
    assert!(!s.dirty() && s.can_reload());
    assert!(s.save_snapshot().is_none(), "nothing to save");
    // A click on Alpha's row in upstream's pages panel switches to it.
    click_page(&mut s, "Alpha", 100.0);
    assert_eq!(s.ed.page_name(), "Alpha");
    assert!(
        s.dirty() && !s.can_reload(),
        "the active page is the canvas's to save"
    );
    let snap = s.save_snapshot().expect("a page-list save");
    let (active, list, sent) = parse_save(&snap);
    assert_eq!(active, "alpha1");
    assert_eq!(
        list,
        [
            ("alpha1".to_string(), "Alpha".to_string()),
            ("beta22".to_string(), "Beta".to_string())
        ]
    );
    assert!(sent.is_empty(), "no page's content changed");
    s.save_succeeded();
    assert!(!s.dirty() && s.can_reload() && !s.unsaved_work());
}

#[test]
fn each_page_keeps_its_own_history_selection_and_camera_across_switches() {
    let mut s = two_pages();
    // Beta: its rectangle selected and moved (one undoable edit), its camera panned.
    click_at(&mut s, 425.0, 325.0, 0.0);
    let b = s.ed.selection[0];
    ui::inspector::apply(&mut s.ed, "x", &[b], |n| n.rect.x = 410.0);
    s.ed.camera.pan = v2(-30.0, 20.0);
    step(&mut s);
    // Alpha, through the pages panel: its own (empty) selection and view; delete its rectangle.
    click_page(&mut s, "Alpha", 200.0);
    assert_eq!(s.ed.page_name(), "Alpha");
    assert!(s.ed.selection.is_empty(), "Alpha's own selection");
    assert_eq!(s.ed.camera.pan, V2::ZERO, "Alpha's own camera");
    click_at(&mut s, 150.0, 150.0, 400.0);
    assert_eq!(s.ed.selection.len(), 1);
    assert!(s.key_down("Delete", "Delete", false));
    assert!(s.ed.doc.nodes.is_empty());
    // Back on Beta: its edit, selection and camera as they were; its undo is its own.
    click_page(&mut s, "Beta", 600.0);
    assert_eq!(s.ed.page_name(), "Beta");
    assert_eq!(s.ed.selection, vec![b]);
    assert_eq!(s.ed.camera.pan, v2(-30.0, 20.0));
    assert_eq!(s.ed.doc.get(b).unwrap().rect.x, 410.0);
    s.run(Action::Undo);
    assert_eq!(s.ed.doc.get(b).unwrap().rect.x, 400.0, "Beta's own undo");
    assert_eq!(s.ed.page_doc(0).nodes.len(), 0, "Alpha's deletion is kept");
    // Both pages changed since they were loaded: both are saved, as they are now.
    let snap = s.save_snapshot().expect("both pages");
    let (active, _, sent) = parse_save(&snap);
    assert_eq!(active, "beta22");
    let ids: Vec<&str> = sent.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["alpha1", "beta22"]);
    assert!(sent[0].1.doc.nodes.is_empty(), "Alpha's deletion");
    assert_eq!(
        sent[1].1.doc.get(b).unwrap().rect.x,
        400.0,
        "Beta as undone"
    );
    s.save_succeeded();
    assert!(!s.dirty() && s.can_reload());
}

#[test]
fn the_pages_panel_renames_on_enter_or_a_click_away_and_escape_or_a_discard_drops_it() {
    for finish in ["Enter", "click", "Escape", "discard"] {
        let mut s = two_pages();
        // A double-click on the row opens upstream's rename field.
        let p = page_row(&mut s, "Beta");
        click(&mut s, p, 100.0);
        step(&mut s);
        click(&mut s, p, 150.0);
        step(&mut s);
        step(&mut s);
        assert!(s.ui.pages.renaming.is_some(), "{finish}: the field is open");
        assert!(s.field_has_keyboard(), "{finish}: with the keyboard");
        assert!(s.can_reload(), "{finish}: nothing typed, nothing to lose");
        press(&mut s, "q", "KeyQ");
        assert!(!s.can_reload(), "{finish}: queued typing");
        step(&mut s);
        assert_eq!(s.ed.page_name(), "Beta", "{finish}: a draft");
        assert!(
            !s.dirty() && s.unsaved_work() && !s.can_reload(),
            "{finish}: the draft is guarded"
        );
        assert!(s.save_snapshot().is_none(), "{finish}: nothing to save yet");
        s.set_focused(false);
        step(&mut s);
        assert!(
            s.ui.pages.renaming.is_some() && s.unsaved_work() && !s.can_reload(),
            "{finish}: kept through a blur"
        );
        s.set_focused(true);
        step(&mut s);
        match finish {
            "Enter" => press(&mut s, "Enter", "Enter"),
            "click" => click_at(&mut s, 700.0, 500.0, 2000.0),
            "Escape" => press(&mut s, "Escape", "Escape"),
            // The conflict banner's explicit discard: the server's canvas, in place.
            _ => s.reload(&canvas(&two_pages_data(), "beta22")).unwrap(),
        }
        step(&mut s);
        step(&mut s);
        assert!(s.ui.pages.renaming.is_none(), "{finish}: the field closed");
        assert!(!s.field_has_keyboard(), "{finish}");
        if matches!(finish, "Escape" | "discard") {
            // Enter now has no field to finish: the draft never comes back.
            s.key_down("Enter", "Enter", false);
            s.key_up("Enter", "Enter");
            step(&mut s);
            step(&mut s);
            assert_eq!(s.ed.page_name(), "Beta", "{finish}: dropped");
            assert!(
                !s.dirty() && !s.unsaved_work() && s.can_reload(),
                "{finish}"
            );
            continue;
        }
        let name = s.ed.page_name().to_string();
        assert!(name != "Beta" && name.contains('q'), "{finish}: {name}");
        assert!(s.dirty() && !s.can_reload(), "{finish}: a change to save");
        let snap = s.save_snapshot().expect("the rename is saved");
        let (_, list, sent) = parse_save(&snap);
        assert_eq!(list[1], ("beta22".to_string(), name.clone()), "{finish}");
        assert!(sent.is_empty(), "{finish}: a name is the page list's");
        s.save_succeeded();
        assert!(!s.dirty() && s.can_reload(), "{finish}: saved");
    }
}

#[test]
fn the_pages_panel_adds_duplicates_reorders_and_deletes_and_keeps_the_last_page() {
    let mut s = state();
    // The only page: upstream's Delete is disabled, so it stays.
    let d = menu_item(&mut s, "Page 1", "Delete", 100.0);
    click(&mut s, d, 200.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.pages.len(), 1, "the last page stays");
    assert!(!s.dirty());
    // Duplicate: a copy after it, shown.
    let d = menu_item(&mut s, "Page 1", "Duplicate", 300.0);
    click(&mut s, d, 400.0);
    step(&mut s);
    step(&mut s);
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Page 1", "Page 1 copy"]);
    assert_eq!(s.ed.page_name(), "Page 1 copy");
    assert_eq!(s.ed.page_doc(1).nodes.len(), 1, "with the page's content");
    // Delete the copy from its menu.
    let d = menu_item(&mut s, "Page 1 copy", "Delete", 500.0);
    click(&mut s, d, 600.0);
    step(&mut s);
    step(&mut s);
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Page 1"]);
    // The header's + (Add page): pressed where it is, right of the "Pages" title.
    let left = s.canvas_rect().min.x as f64;
    let (title_x, title_y) = painted_at(&mut s, "Pages", |r| (r.max.x as f64) < left);
    let mut x = left - 4.0;
    let mut t = 1000.0;
    while s.ed.pages.len() == 1 && x > title_x {
        click(&mut s, (x, title_y), t);
        step(&mut s);
        x -= 4.0;
        t += 500.0;
    }
    step(&mut s);
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Page 1", "Page 2"], "added after the active page");
    assert_eq!(s.ed.page_name(), "Page 2");
    // Drag Page 2's row onto Page 1's: reordered.
    let (from, to) = (page_row(&mut s, "Page 2"), page_row(&mut s, "Page 1"));
    s.pointer_move(from.0, from.1, t);
    s.pointer_down(from.0, from.1, Button::Primary, t);
    step(&mut s);
    for k in 1..=2 {
        let y = from.1 + (to.1 - from.1) * k as f64 / 2.0;
        s.pointer_move(from.0, y, t + k as f64 * 20.0);
        step(&mut s);
    }
    s.pointer_up(to.0, to.1, Button::Primary, t + 100.0);
    step(&mut s);
    step(&mut s);
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Page 2", "Page 1"], "reordered");
    assert_eq!(s.ed.page_name(), "Page 2", "the shown page stays");
    // One save: the new list and active page, and the new page's content (never saved).
    let new = s.ed.active_page_id().to_string();
    let snap = s.save_snapshot().expect("the page changes");
    let (active, list, sent) = parse_save(&snap);
    assert_eq!(active, new);
    assert_eq!(
        list,
        [
            (new.clone(), "Page 2".to_string()),
            ("p".to_string(), "Page 1".to_string())
        ]
    );
    let ids: Vec<&str> = sent.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, [new.as_str()], "only the new page's content");
    s.save_succeeded();
    assert!(!s.dirty() && s.can_reload());
}

#[test]
fn a_saved_canvas_reopens_with_every_page_its_order_names_active_page_and_content() {
    let mut s = two_pages();
    // Upstream's page operations, as the pages panel and inspector run them.
    let added = s.ed.add_page();
    s.ed.rename_page(added, "Gamma");
    let n = s.ed.edit("Add", |tx| {
        let id = tx.alloc_id();
        let r = crate::geom::Rect::new(7.0, 8.0, 9.0, 10.0);
        tx.insert(Node::new(id, NodeKind::Ellipse, r), None, None)
    });
    s.ed.duplicate_page(0);
    s.ed.move_page(0, 3);
    s.ed.rename_page(0, "Alpha again");
    let snap = s.save_snapshot().expect("the page changes");
    s.save_succeeded();
    assert!(!s.dirty());
    let (active, list, _) = parse_save(&snap);
    // What the server has now is exactly what the editor holds: reopened, nothing is lost.
    let reopened = WebState::open(&canvas(&pages_now(&s), &active), false, W, H).unwrap();
    let ids = |s: &WebState| -> Vec<(String, String)> {
        s.ed.pages
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect()
    };
    assert_eq!(ids(&reopened), list, "order, ids and names");
    assert_eq!(ids(&reopened), ids(&s));
    assert_eq!(reopened.ed.active_page_id(), s.ed.active_page_id());
    for i in 0..s.ed.pages.len() {
        assert_eq!(
            reopened.ed.page_doc(i).nodes,
            s.ed.page_doc(i).nodes,
            "page {i}"
        );
    }
    let gamma = reopened
        .ed
        .pages
        .iter()
        .position(|p| p.name == "Gamma")
        .unwrap();
    assert!(reopened.ed.page_doc(gamma).nodes.contains_key(&n));
    assert!(!reopened.dirty() && reopened.can_reload());
}

#[test]
fn an_old_save_answer_never_marks_newer_edits_on_any_page_or_the_list_saved() {
    let mut s = two_pages();
    // Beta's rectangle deleted; the save of that is taken.
    click_at(&mut s, 425.0, 325.0, 0.0);
    assert!(s.key_down("Delete", "Delete", false));
    let snap = s.save_snapshot().expect("Beta's change");
    let (_, _, sent) = parse_save(&snap);
    assert_eq!(sent.len(), 1);
    // While it is in flight: a switch to Alpha, an edit there and a new page.
    s.ed.switch_page(0);
    let a = s.ed.doc.root[0];
    ui::inspector::apply(&mut s.ed, "x", &[a], |n| n.rect.x = 5.0);
    let added = s.ed.add_page();
    assert!(s.save_snapshot().is_none(), "one save at a time");
    // The first save's answer: only Beta's content as it was then counts as saved.
    s.save_succeeded();
    assert!(s.dirty() && !s.can_reload(), "the newer changes stay");
    let beta = s.ed.pages.iter().position(|p| p.id == "beta22").unwrap();
    let dirty = s.ed.dirty_pages();
    assert!(dirty.contains(&0) && dirty.contains(&added), "{dirty:?}");
    assert!(!dirty.contains(&beta), "Beta was saved");
    let snap = s.save_snapshot().expect("the rest");
    let (active, list, sent) = parse_save(&snap);
    assert_eq!(active, s.ed.active_page_id());
    assert_eq!(list.len(), 3);
    let ids: Vec<&str> = sent.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["alpha1", s.ed.pages[added].id.as_str()]);
    assert_eq!(sent[0].1.doc.get(a).unwrap().rect.x, 5.0);
    s.save_succeeded();
    assert!(!s.dirty() && s.can_reload());
}

#[test]
fn an_undo_or_rename_after_a_save_was_taken_stays_to_save() {
    let mut s = state();
    click_at(&mut s, 150.0, 150.0, 0.0);
    assert!(s.key_down("Delete", "Delete", false));
    let snap = s.save_snapshot().unwrap();
    assert!(sent_page(&snap).doc.nodes.is_empty());
    // Undone and the page renamed before the answer: the page differs from what was saved.
    s.run(Action::Undo);
    s.ed.rename_page(0, "Renamed");
    s.save_succeeded();
    assert!(s.dirty(), "the undo and the rename are newer than the save");
    let snap = s.save_snapshot().expect("saved next");
    let (_, list, _) = parse_save(&snap);
    assert_eq!(list, [("p".to_string(), "Renamed".to_string())]);
    assert_eq!(sent_page(&snap).doc.nodes.len(), 1, "the undone state");
    s.save_succeeded();
    assert!(!s.dirty() && s.can_reload());
}

#[test]
fn a_refused_save_keeps_every_page_change_and_only_an_explicit_discard_drops_it() {
    let mut s = two_pages();
    click_at(&mut s, 425.0, 325.0, 0.0);
    assert!(s.key_down("Delete", "Delete", false));
    s.ed.switch_page(0);
    s.ed.rename_page(0, "First");
    s.ed.add_page();
    let snap = s.save_snapshot().unwrap();
    // Refused (a conflict, or no answer): everything stays unsaved and is sent again as it was.
    s.save_failed();
    assert!(s.dirty() && !s.can_reload() && s.unsaved_work());
    assert_eq!(s.save_snapshot().as_deref(), Some(snap.as_str()));
    s.save_failed();
    assert_eq!(s.ed.pages.len(), 3);
    // The explicit discard loads the server's canvas in place: every local change goes.
    s.reload(&canvas(&two_pages_data(), "beta22")).unwrap();
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Alpha", "Beta"]);
    assert_eq!(s.ed.page_name(), "Beta");
    assert_eq!(s.ed.doc.nodes.len(), 1, "Beta's rectangle is back");
    assert!(!s.dirty() && s.can_reload() && s.save_snapshot().is_none());
}

#[test]
fn a_clean_reload_takes_the_whole_canvas_and_keeps_each_page_s_view() {
    let mut s = two_pages();
    s.ed.camera.pan = v2(-30.0, 20.0);
    // The rectangle's centre (425, 325), where the panned camera shows it.
    click_at(&mut s, 425.0 + 30.0, 325.0 - 20.0, 0.0);
    assert_eq!(s.ed.selection.len(), 1);
    let b = s.ed.selection[0];
    // Elsewhere, Beta's rectangle was moved and a page added (an agent).
    let mut pages = two_pages_data();
    pages[1].doc.nodes.get_mut(&b).unwrap().rect.x = 420.0;
    pages.push(page(
        "gamma3",
        "Gamma",
        crate::geom::Rect::new(0.0, 0.0, 10.0, 10.0),
    ));
    assert!(s.can_reload());
    s.reload(&canvas(&pages, "beta22")).unwrap();
    assert_eq!(s.ed.pages.len(), 3);
    assert_eq!(s.ed.page_name(), "Beta");
    assert_eq!(s.ed.camera.pan, v2(-30.0, 20.0), "Beta's camera kept");
    assert_eq!(s.ed.selection, vec![b], "and its selection");
    assert_eq!(s.ed.doc.get(b).unwrap().rect.x, 420.0);
    assert!(!s.dirty() && s.can_reload());
    // A change of the active page made elsewhere shows rather than hides.
    s.reload(&canvas(&pages, "gamma3")).unwrap();
    assert_eq!(s.ed.page_name(), "Gamma");
    assert!(s.ed.selection.is_empty());
    assert!(!s.dirty());
}

// ---- R22: page actions while the pages panel draws, and fields opened on another page.
// The first four tests are the independent R22 review's probes (r22-probes.rs), adopted with
// their fixtures and assertions unchanged (only wrapped in this file's style).

#[test]
fn r22_deleting_the_first_of_two_pages_keeps_the_editor_running() {
    let mut s = two_pages();
    let button = menu_item(&mut s, "Alpha", "Delete", 100.0);
    click(&mut s, button, 200.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(
        s.ed.pages.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["beta22"]
    );
    assert_eq!(s.ed.active_page_id(), "beta22");
}

#[test]
fn r22_deleting_the_middle_of_three_pages_keeps_the_editor_running() {
    let mut pages = two_pages_data();
    pages.push(page(
        "gamma3",
        "Gamma",
        crate::geom::Rect::new(10.0, 10.0, 20.0, 20.0),
    ));
    let mut s = WebState::open(&canvas(&pages, "beta22"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    let button = menu_item(&mut s, "Beta", "Delete", 100.0);
    click(&mut s, button, 200.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(
        s.ed.pages.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["alpha1", "gamma3"]
    );
    assert_eq!(s.ed.active_page_id(), "gamma3");
}

#[test]
fn r22_deleting_the_last_row_is_a_positive_control() {
    let mut s = two_pages();
    let button = menu_item(&mut s, "Beta", "Delete", 100.0);
    click(&mut s, button, 200.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(
        s.ed.pages.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["alpha1"]
    );
    assert_eq!(s.ed.active_page_id(), "alpha1");
}

#[test]
fn r22_a_layer_rename_draft_never_renames_a_different_page() {
    let mut s = two_pages();
    click_page(&mut s, "Alpha", 100.0);
    let id = open_rename(&mut s);
    let beta_name = s.ed.page_doc(1).get(id).unwrap().name.clone();
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    let draft = s.ui.renaming.as_ref().unwrap().1.clone();
    assert!(draft.contains('q'));
    assert!(s.unsaved_work() && !s.can_reload());
    click_page(&mut s, "Beta", 3000.0);
    eprintln!(
        "R22 names after switch: Alpha={:?} Beta={:?} draft={:?}",
        s.ed.page_doc(0).get(id).map(|n| &n.name),
        s.ed.page_doc(1).get(id).map(|n| &n.name),
        s.ui.renaming
    );
    assert_eq!(s.ed.active_page_id(), "beta22");
    assert_eq!(
        s.ed.page_doc(1).get(id).unwrap().name,
        beta_name,
        "a draft started on Alpha must not edit Beta's same-numbered node"
    );
    assert_eq!(
        s.ed.page_doc(0).get(id).unwrap().name,
        draft,
        "click-away should commit the layer rename to Alpha"
    );
}

/// The ids of the canvas's pages, in order.
fn page_ids(s: &WebState) -> Vec<String> {
    s.ed.pages.iter().map(|p| p.id.clone()).collect()
}

/// The index of page `id`.
fn page_index(s: &WebState, id: &str) -> usize {
    s.ed.pages.iter().position(|p| p.id == id).unwrap()
}

#[test]
fn r77_menu_deletes_of_any_row_active_or_not_keep_the_right_pages_and_shown_page() {
    // (shown page, deleted row, pages left, page shown after).
    for (shown, deleted, left, after) in [
        ("Beta", "Alpha", ["beta22", "gamma3"], "beta22"),
        ("Alpha", "Alpha", ["beta22", "gamma3"], "beta22"),
        ("Alpha", "Beta", ["alpha1", "gamma3"], "alpha1"),
        ("Beta", "Gamma", ["alpha1", "beta22"], "beta22"),
        ("Gamma", "Gamma", ["alpha1", "beta22"], "beta22"),
    ] {
        let mut pages = two_pages_data();
        pages.push(page(
            "gamma3",
            "Gamma",
            crate::geom::Rect::new(10.0, 10.0, 20.0, 20.0),
        ));
        let mut s = WebState::open(&canvas(&pages, "beta22"), false, W, H).unwrap();
        settle(&mut s, 0.0);
        if shown != "Beta" {
            click_page(&mut s, shown, 50.0);
        }
        let button = menu_item(&mut s, deleted, "Delete", 100.0);
        click(&mut s, button, 200.0);
        step(&mut s);
        step(&mut s);
        let why = format!("{deleted} deleted while {shown} is shown");
        assert_eq!(page_ids(&s), left, "{why}");
        assert_eq!(s.ed.active_page_id(), after, "{why}");
        // Each page left still has its own content, and the panel still draws and works.
        for id in left {
            let i = page_index(&s, id);
            assert_eq!(s.ed.page_doc(i).nodes.len(), 1, "{why}: {id}");
        }
        let snap = s.save_snapshot().expect("the deletion is saved");
        let (active, list, _) = parse_save(&snap);
        assert_eq!(active, after, "{why}");
        let listed: Vec<&str> = list.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(listed, left, "{why}");
    }
    // The last page stays (upstream's Delete is disabled for it), as before.
    let mut s = state();
    let d = menu_item(&mut s, "Page 1", "Delete", 100.0);
    click(&mut s, d, 200.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(page_ids(&s), ["p"]);
}

#[test]
fn r77_a_layer_rename_is_finished_on_its_page_before_any_page_action() {
    // Each page action, chosen while a layer rename draft is open on Alpha.
    for action in ["switch", "add", "duplicate", "delete other"] {
        let mut s = two_pages();
        click_page(&mut s, "Alpha", 100.0);
        let id = open_rename(&mut s);
        assert!(
            s.ed.page_doc(1).get(id).is_some(),
            "Beta has a node with the same id"
        );
        let beta_name = s.ed.page_doc(1).get(id).unwrap().name.clone();
        press(&mut s, "q", "KeyQ");
        step(&mut s);
        let draft = s.ui.renaming.as_ref().unwrap().1.clone();
        match action {
            "switch" => click_page(&mut s, "Beta", 3000.0),
            "add" => {
                // The header's +, found as the pages panel test finds it.
                let left = s.canvas_rect().min.x as f64;
                let (title_x, title_y) = painted_at(&mut s, "Pages", |r| (r.max.x as f64) < left);
                let mut x = left - 4.0;
                let mut t = 3000.0;
                while s.ed.pages.len() == 2 && x > title_x {
                    click(&mut s, (x, title_y), t);
                    step(&mut s);
                    x -= 4.0;
                    t += 500.0;
                }
                step(&mut s);
            }
            "duplicate" => {
                let d = menu_item(&mut s, "Beta", "Duplicate", 3000.0);
                click(&mut s, d, 3100.0);
                step(&mut s);
                step(&mut s);
            }
            _ => {
                let d = menu_item(&mut s, "Beta", "Delete", 3000.0);
                click(&mut s, d, 3100.0);
                step(&mut s);
                step(&mut s);
            }
        }
        let alpha = page_index(&s, "alpha1");
        assert_eq!(
            s.ed.page_doc(alpha).get(id).unwrap().name,
            draft,
            "{action}: finished on Alpha"
        );
        if let Some(beta) = s.ed.pages.iter().position(|p| p.id == "beta22") {
            assert_eq!(
                s.ed.page_doc(beta).get(id).unwrap().name,
                beta_name,
                "{action}: Beta untouched"
            );
        }
        for (i, p) in s.ed.pages.iter().enumerate() {
            if p.id != "alpha1" && p.id != "beta22" {
                // A new page is empty; a duplicate of Beta has Beta's names.
                assert!(
                    s.ed.page_doc(i).get(id).is_none_or(|n| n.name == beta_name),
                    "{action}: {}",
                    p.name
                );
            }
        }
        assert!(s.ui.renaming.is_none(), "{action}: the field is finished");
        assert!(!s.field_has_keyboard(), "{action}");
    }
}

#[test]
fn r77_the_finished_rename_saves_reopens_and_undoes_on_its_own_page_only() {
    let mut s = two_pages();
    click_page(&mut s, "Alpha", 100.0);
    let id = open_rename(&mut s);
    let old = s.ed.page_doc(0).get(id).unwrap().name.clone();
    let beta_name = s.ed.page_doc(1).get(id).unwrap().name.clone();
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    let draft = s.ui.renaming.as_ref().unwrap().1.clone();
    click_page(&mut s, "Beta", 3000.0);
    // The save carries Alpha's content with the new name, and nothing of Beta's.
    let snap = s.save_snapshot().expect("Alpha's rename and the switch");
    let (active, _, sent) = parse_save(&snap);
    assert_eq!(active, "beta22");
    let ids: Vec<&str> = sent.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(ids, ["alpha1"], "only Alpha's content changed");
    assert_eq!(sent[0].1.doc.get(id).unwrap().name, draft);
    s.save_succeeded();
    // Reopened as saved: the name is on Alpha, Beta's is as it was.
    let reopened = WebState::open(&canvas(&pages_now(&s), "beta22"), false, W, H).unwrap();
    assert_eq!(reopened.ed.page_doc(0).get(id).unwrap().name, draft);
    assert_eq!(reopened.ed.page_doc(1).get(id).unwrap().name, beta_name);
    // Undo belongs to Alpha's history: on Beta nothing is undone, on Alpha the rename is.
    s.run(Action::Undo);
    assert_eq!(s.ed.page_doc(1).get(id).unwrap().name, beta_name);
    assert_eq!(s.ed.page_doc(0).get(id).unwrap().name, draft);
    click_page(&mut s, "Alpha", 4000.0);
    s.run(Action::Undo);
    assert_eq!(s.ed.page_doc(0).get(id).unwrap().name, old, "Alpha's undo");
    assert_eq!(s.ed.page_doc(1).get(id).unwrap().name, beta_name);
}

#[test]
fn r77_a_page_rename_draft_is_finished_on_its_own_page_before_another_row_is_deleted() {
    let mut s = two_pages();
    // A double-click on Beta's row opens its rename; type a draft.
    let p = page_row(&mut s, "Beta");
    click(&mut s, p, 100.0);
    step(&mut s);
    click(&mut s, p, 150.0);
    step(&mut s);
    step(&mut s);
    assert!(s.ui.pages.renaming.is_some());
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    // Alpha (the row before it) is deleted from its menu: the draft names Beta, not row 1.
    let d = menu_item(&mut s, "Alpha", "Delete", 3000.0);
    click(&mut s, d, 3100.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(page_ids(&s), ["beta22"]);
    let name = s.ed.pages[0].name.clone();
    assert!(name != "Beta" && name.contains('q'), "{name}");
    assert!(s.ui.pages.renaming.is_none());
}

// ---- R23: a rename whose row is hidden, palette page commands, and the field guard. The
// first five tests are the independent R23 review's probes (r23-probes.rs and
// r23-draft-guard-probes.rs), adopted with their fixtures, input and final assertions. One
// intermediate assertion of the guard probe is adjusted, see there.

fn r23_hidden_rename_setup() -> (WebState, crate::model::NodeId, String, String) {
    let mut s = two_pages();
    click_page(&mut s, "Alpha", 100.0);
    let id = open_rename(&mut s);
    let beta = s.ed.page_doc(1).get(id).unwrap().name.clone();
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    let draft = s.ui.renaming.as_ref().unwrap().1.clone();
    let left = s.canvas_rect().min.x;
    let p = painted_at(&mut s, "Library", |r| r.max.x <= left);
    click(&mut s, p, 2000.0);
    step(&mut s);
    step(&mut s);
    assert!(s.ui.show_library, "actual Library tab hides the layer rows");
    eprintln!(
        "R23 after Library: rename={:?} focus={:?}",
        s.ui.renaming,
        s.ctx.memory(|m| m.focused())
    );
    (s, id, draft, beta)
}

fn r23_go_beta_from_palette(s: &mut WebState) {
    open_palette(s);
    s.paste("Go to page: Beta");
    step(s);
    step(s);
    assert_eq!(s.ui.palette.query, "Go to page: Beta");
    assert!(paints(s, "Go to page: Beta"));
    press(s, "Enter", "Enter");
    step(s);
    step(s);
    assert_eq!(
        s.ed.active_page_id(),
        "beta22",
        "actual palette page command switched"
    );
}

fn r23_finish_visible_layer_field(s: &mut WebState) {
    let left = s.canvas_rect().min.x;
    let p = painted_at(s, "Layers", |r| r.max.x <= left);
    click(s, p, 4000.0);
    step(s);
    step(s);
    assert!(!s.ui.show_library);
    if s.ui.renaming.is_some() {
        press(s, "Enter", "Enter");
        step(s);
        step(s);
    }
}

#[test]
fn r23_hidden_layer_draft_survives_palette_page_switch_on_its_origin() {
    let (mut s, id, draft, beta) = r23_hidden_rename_setup();
    r23_go_beta_from_palette(&mut s);
    r23_finish_visible_layer_field(&mut s);
    eprintln!(
        "R23 palette hidden rename: Alpha={:?} Beta={:?} draft={:?}",
        s.ed.page_doc(0).get(id).map(|n| &n.name),
        s.ed.page_doc(1).get(id).map(|n| &n.name),
        s.ui.renaming
    );
    assert_eq!(
        s.ed.page_doc(1).get(id).unwrap().name,
        beta,
        "a hidden Alpha draft must not edit Beta"
    );
    assert_eq!(
        s.ed.page_doc(0).get(id).unwrap().name,
        draft,
        "a local switch finishes the rename on Alpha"
    );
}

#[test]
fn r23_hidden_layer_draft_pages_panel_switch_is_a_positive_control() {
    let (mut s, id, draft, beta) = r23_hidden_rename_setup();
    click_page(&mut s, "Beta", 3000.0);
    r23_finish_visible_layer_field(&mut s);
    assert_eq!(s.ed.page_doc(1).get(id).unwrap().name, beta);
    assert_eq!(s.ed.page_doc(0).get(id).unwrap().name, draft);
}

#[test]
fn r23_visible_layer_rename_finishes_before_palette_switch_control() {
    let mut s = two_pages();
    click_page(&mut s, "Alpha", 100.0);
    let id = open_rename(&mut s);
    let beta = s.ed.page_doc(1).get(id).unwrap().name.clone();
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    let draft = s.ui.renaming.as_ref().unwrap().1.clone();
    // Click the canvas as ordinary click-away, then use the actual palette command.
    click_at(&mut s, 700.0, 500.0, 2000.0);
    step(&mut s);
    step(&mut s);
    assert!(s.ui.renaming.is_none());
    r23_go_beta_from_palette(&mut s);
    assert_eq!(s.ed.page_doc(1).get(id).unwrap().name, beta);
    assert_eq!(s.ed.page_doc(0).get(id).unwrap().name, draft);
}

fn r23_library_tab(s: &mut WebState) {
    let left = s.canvas_rect().min.x;
    let p = painted_at(s, "Library", |r| r.max.x <= left);
    click(s, p, 2000.0);
    step(s);
    step(s);
    assert!(s.ui.show_library);
}

#[test]
fn r23_hidden_unchanged_layer_rename_does_not_exempt_an_inspector_draft() {
    let mut s = state();
    let id = open_rename(&mut s);
    assert!(s.ui.renaming.is_some(), "unchanged layer rename is open");
    r23_library_tab(&mut s);
    // Adjusted from the original probe, which asserted here that the rename "stays in hidden
    // Library state" (true BEFORE the R23 fix, see r23-before.log). Hiding the layer rows now
    // finishes the rename as losing focus does (unchanged, so no edit), so no rename is stored
    // any more. The same input follows, and the final protection assertions are unchanged;
    // `field_editing` also no longer lets a stored rename exempt another focused field.
    assert!(
        s.ui.renaming.is_none(),
        "hiding the layer rows finishes the rename"
    );
    assert!(!s.dirty() && s.can_reload(), "nothing typed yet");
    let field_id = open_number_field(&mut s, "X  ");
    assert_eq!(field_id, id);
    s.paste("-");
    step(&mut s);
    step(&mut s);
    assert!(
        paints(&mut s, "-"),
        "the actual inspector paints the incomplete number"
    );
    assert_eq!(
        axis(&s, id, "X  "),
        100.0,
        "the document has no number to save"
    );
    assert!(s.field_has_keyboard());
    eprintln!(
        "R23 hidden rename guard: renaming={:?} dirty={} unsaved={} can_reload={}",
        s.ui.renaming,
        s.dirty(),
        s.unsaved_work(),
        s.can_reload()
    );
    assert!(
        s.unsaved_work() && !s.can_reload(),
        "an unrelated hidden unchanged rename must not exempt an actual inspector draft"
    );
}

#[test]
fn r23_library_without_hidden_rename_protects_inspector_draft_control() {
    let mut s = state();
    r23_library_tab(&mut s);
    let id = open_number_field(&mut s, "X  ");
    s.paste("-");
    step(&mut s);
    step(&mut s);
    assert!(paints(&mut s, "-"));
    assert_eq!(axis(&s, id, "X  "), 100.0);
    assert!(s.unsaved_work() && !s.can_reload());
}

#[test]
fn r78_a_hidden_layer_draft_is_finished_on_its_page_and_saves_reopens_and_undoes_there() {
    let (mut s, id, draft, beta) = r23_hidden_rename_setup();
    // Hiding the rows finished the draft on Alpha, where it was typed.
    assert!(s.ui.renaming.is_none());
    assert_eq!(s.ed.page_doc(0).get(id).unwrap().name, draft);
    r23_go_beta_from_palette(&mut s);
    r23_finish_visible_layer_field(&mut s);
    let snap = s.save_snapshot().expect("Alpha's rename and the switch");
    let (active, _, sent) = parse_save(&snap);
    assert_eq!(active, "beta22");
    let ids: Vec<&str> = sent.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(ids, ["alpha1"], "only Alpha's content changed");
    assert_eq!(sent[0].1.doc.get(id).unwrap().name, draft);
    s.save_succeeded();
    let reopened = WebState::open(&canvas(&pages_now(&s), "beta22"), false, W, H).unwrap();
    assert_eq!(reopened.ed.page_doc(0).get(id).unwrap().name, draft);
    assert_eq!(reopened.ed.page_doc(1).get(id).unwrap().name, beta);
    // Undo on Beta changes nothing of Alpha's; on Alpha it undoes the rename.
    s.run(Action::Undo);
    assert_eq!(s.ed.page_doc(1).get(id).unwrap().name, beta);
    click_page(&mut s, "Alpha", 5000.0);
    s.run(Action::Undo);
    assert_ne!(
        s.ed.page_doc(0).get(id).unwrap().name,
        draft,
        "Alpha's undo"
    );
    assert_eq!(s.ed.page_doc(1).get(id).unwrap().name, beta);
}

#[test]
fn r78_collapsing_the_pages_panel_finishes_a_page_rename_on_its_page() {
    let mut s = two_pages();
    // A double-click on Beta's row opens its rename; type a draft.
    let p = page_row(&mut s, "Beta");
    click(&mut s, p, 100.0);
    step(&mut s);
    click(&mut s, p, 150.0);
    step(&mut s);
    step(&mut s);
    assert!(s.ui.pages.renaming.is_some());
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    assert!(!s.can_reload(), "a changed page-rename draft");
    // One click on the header's collapse chevron: the sidebar's first item, 16 px wide, inside
    // its 10 px margin, on the "Pages" title's line. (Any other click there would finish the
    // field by ordinary click-away first, which wouldn't test the collapse.)
    let left = s.canvas_rect().min.x as f64;
    let (_, title_y) = painted_at(&mut s, "Pages", |r| (r.max.x as f64) < left);
    click(&mut s, (18.0, title_y), 3000.0);
    step(&mut s);
    step(&mut s);
    assert!(s.ui.pages.collapsed, "the panel collapsed");
    assert!(
        s.ui.pages.renaming.is_none(),
        "the hidden rename was finished"
    );
    let beta = page_index(&s, "beta22");
    let name = s.ed.pages[beta].name.clone();
    assert!(name != "Beta" && name.contains('q'), "{name}");
    assert_eq!(s.ed.pages[page_index(&s, "alpha1")].name, "Alpha");
    assert!(s.dirty() && s.unsaved_work(), "a page-list change to save");
}

#[test]
fn r77_an_inspector_number_draft_never_continues_on_the_next_page() {
    for typed in ["-150", "-"] {
        let mut s = two_pages();
        // Beta's rectangle selected, so the next page shows the same X field.
        click_at(&mut s, 425.0, 325.0, 0.0);
        click_page(&mut s, "Alpha", 100.0);
        let id = open_number_field(&mut s, "X  ");
        assert!(s.ed.page_doc(1).get(id).is_some(), "the same id on Beta");
        s.paste(typed);
        step(&mut s);
        let alpha_x = s.ed.doc.get(id).unwrap().rect.x;
        click_page(&mut s, "Beta", 3000.0);
        step(&mut s);
        assert_eq!(s.ed.active_page_id(), "beta22");
        assert_eq!(s.ed.selection, vec![id], "{typed}: Beta's own selection");
        assert_eq!(
            s.ed.doc.get(id).unwrap().rect.x,
            400.0,
            "{typed}: Beta's X untouched"
        );
        assert_eq!(
            s.ed.page_doc(0).get(id).unwrap().rect.x,
            alpha_x,
            "{typed}: Alpha keeps what upstream applied"
        );
        assert!(!s.field_has_keyboard(), "{typed}: the field ended");
        assert!(!paints(&mut s, typed), "{typed}: no draft shown on Beta");
    }
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
fn the_inspector_page_name_renames_the_page_and_saves_it_with_the_page_list() {
    // The inspector's page name field (nothing selected), found where it is painted: the
    // owner's click, select all, "X" and Enter rename the page, as upstream does.
    let x = inspector_edge(&state());
    let mut found = None;
    let mut y = 40.0;
    while found.is_none() && y < H - 40.0 {
        let mut s = state();
        type_page_name(&mut s, (x, y));
        if s.ed.page_name() == "X" {
            found = Some(s);
        }
        y += 8.0;
    }
    let mut s = found.expect("the field renames the page");
    assert!(!s.field_has_keyboard(), "Enter finished the field");
    assert!(s.ed.meta_dirty && s.dirty() && !s.can_reload());
    let snap = s.save_snapshot().expect("the new name is saved");
    let (active, list, sent) = parse_save(&snap);
    assert_eq!(active, "p");
    assert_eq!(list, [("p".to_string(), "X".to_string())]);
    assert!(sent.is_empty(), "the page's content is unchanged");
    s.save_succeeded();
    assert!(!s.dirty() && s.can_reload() && !s.unsaved_work());
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

/// The canvas with the first page's rectangle moved to x and a second rectangle added, as
/// another writer saved it.
fn changed_elsewhere(s: &WebState, x: f64) -> String {
    let mut pages = pages_now(s);
    let page = &mut pages[0];
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
    canvas(&pages, s.ed.active_page_id())
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
    let saved = sent_page(&snap);
    let n = saved.doc.nodes.values().next().unwrap();
    assert!(n.rect.x > 100.0 && n.rect.y > 100.0, "moved: {:?}", n.rect);
}
