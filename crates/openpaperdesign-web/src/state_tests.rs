//! The browser host of upstream's workspace, on the host (no GPU: `render::frame::layout` is
//! egui layout only). Positions are CSS px in the canvas element; `at` turns a point in the
//! canvas rectangle (which is the editor's own coordinate space) into one.

use super::*;
use crate::editor::Tool;
use crate::geom::V2;
use crate::model::{Node, NodeKind};
use crate::project::PageData;
use crate::render::scene::{self, SceneCache};
use crate::ui::NarrowPanel;

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
    canvas_named(pages, active, "Canvas")
}

/// [`canvas`], for a file named `name`.
fn canvas_named(pages: &[PageData], active: &str, name: &str) -> String {
    let list: Vec<serde_json::Value> = pages
        .iter()
        .map(|p| {
            serde_json::json!({"id": p.id, "name": p.name, "revision": "\"r\"", "text": page_to_json(p)})
        })
        .collect();
    serde_json::json!({"canvas": {"name": name}, "revision": "\"c\"", "active_page": active, "pages": list})
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
/// pan; laid out, with the Layers panel opened as a person opens it (a fresh view starts with
/// it closed, see the revision 2 workspace tests), so its rows and the pages panel are there.
fn state() -> WebState {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let pages = [page("p", "Page 1", rect)];
    let mut s = WebState::open(&canvas(&pages, "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    open_layers(&mut s);
    s
}

/// The centre of a laid-out control (CSS px).
fn centre(r: egui::Rect) -> (f64, f64) {
    (r.center().x as f64, r.center().y as f64)
}

/// A click on the Layers toggle (floating at the canvas's top left while the panel is closed),
/// then the arrangement settles.
fn toggle_layers(s: &mut WebState) {
    let r = s.ui.chrome.layers.expect("the Layers toggle is laid out");
    click(s, centre(r), 1.0);
    for _ in 0..4 {
        step(s);
    }
}

/// [`toggle_layers`] when the Layers panel isn't shown yet.
fn open_layers(s: &mut WebState) {
    if !s.ui.narrow.shown().is_some_and(|a| a.layers) {
        toggle_layers(s);
    }
    assert!(
        s.ui.narrow.shown().is_some_and(|a| a.layers),
        "the Layers panel opened"
    );
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
    open_layers(&mut s);
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
    // Upstream's layers (≥ 180 px) and design inspector (≥ 220 px), both full height: there is
    // no header across the surface (DESIGN.md revision 18), so the canvas starts at the top.
    assert!(c.min.x >= 180.0, "layers panel on the left: {c:?}");
    assert!(c.max.x <= W as f32 - 220.0, "inspector on the right: {c:?}");
    assert_eq!(c.min.y, 0.0, "no full-width header above the canvas: {c:?}");
    assert_eq!(
        c.max.y, H as f32,
        "the canvas runs the surface's height: {c:?}"
    );
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
    // The page's clock goes on from where the fixture left it: egui's time never runs
    // backwards (it already knows the pointer, from the fixture's click on Layers).
    let t0 = s.now_ms;
    let (x, y) = at(&s, 150.0, 150.0);
    s.pointer_move(x, y, t0 + 10.0);
    assert!(s.ed.hover.is_some());
    assert!(s.tick(t0 + 16.0));
    step(&mut s);
    let t = settle(&mut s, t0 + 32.0);
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

/// One headless frame: whether text `wanted` is painted inside `within` and whole inside the
/// clip rectangle it is painted with (so it is visible, not scrolled or clipped away).
fn painted_whole(s: &mut WebState, wanted: &str, within: egui::Rect) -> bool {
    let mut l = s.layout();
    let mut whole = false;
    for clipped in &l.output.shapes {
        let mut text = Vec::new();
        shape_text(&clipped.shape, &mut text);
        whole |= text.iter().any(|(t, r)| {
            t.trim() == wanted && within.contains_rect(*r) && clipped.clip_rect.contains_rect(*r)
        });
    }
    l.output.textures_delta.clear();
    s.drawn();
    whole
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

/// The row of page `name` in upstream's pages panel (the left sidebar's Papers list), not the
/// Canvas identity above it, whose Paper line names the shown page too.
fn page_row(s: &mut WebState, name: &str) -> (f64, f64) {
    let left = s.canvas_rect().min.x;
    let identity = s.ui.chrome.identity.expect("the identity is laid out");
    painted_at(s, name, |r| r.max.x <= left && !identity.intersects(r))
}

/// A click on page `name`'s row, laid out.
fn click_page(s: &mut WebState, name: &str, t: f64) {
    let p = page_row(s, name);
    click(s, p, t);
    step(s);
    step(s);
}

/// Item `item` of page `name`'s context menu (Rename Paper, Duplicate Paper, Delete Paper),
/// opening the menu with a secondary click on the row if it isn't shown.
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

/// A double-click at `p`, as egui counts one, then laid out. egui's clock only moves with the
/// page's frames (`tick`), so clicks without a frame between them happen at one moment, and
/// egui counts a click within 0.3 s of the last one as a double-click and within 0.6 s of the
/// one before as a triple-click (which isn't a double-click). The fixture's own click on the
/// Layers toggle ([`state`], [`two_pages`]) is such an earlier click: the page's clock first
/// passes that window, as a person's time does between opening the panel and the row; the
/// second click follows the first 120 ms later.
fn double_click(s: &mut WebState, p: (f64, f64), t: f64) {
    let later = s.now_ms + 1000.0;
    let now = settle(s, later);
    click(s, p, t);
    step(s);
    s.tick(now + 120.0);
    click(s, p, t + 120.0);
    step(s);
    step(s);
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
        double_click(&mut s, p, 100.0);
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
    let d = menu_item(&mut s, "Page 1", "Delete Paper", 100.0);
    click(&mut s, d, 200.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.pages.len(), 1, "the last page stays");
    assert!(!s.dirty());
    // Duplicate: a copy after it, shown.
    let d = menu_item(&mut s, "Page 1", "Duplicate Paper", 300.0);
    click(&mut s, d, 400.0);
    step(&mut s);
    step(&mut s);
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Page 1", "Page 1 copy"]);
    assert_eq!(s.ed.page_name(), "Page 1 copy");
    assert_eq!(s.ed.page_doc(1).nodes.len(), 1, "with the page's content");
    // Delete the copy from its menu.
    let d = menu_item(&mut s, "Page 1 copy", "Delete Paper", 500.0);
    click(&mut s, d, 600.0);
    step(&mut s);
    step(&mut s);
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Page 1"]);
    // The list header's + (Add Paper): pressed where it is, right of the "Papers" title.
    let left = s.canvas_rect().min.x as f64;
    let (title_x, title_y) = painted_at(&mut s, "Papers", |r| (r.max.x as f64) < left);
    let mut x = left - 4.0;
    let mut t = 1000.0;
    while s.ed.pages.len() == 1 && x > title_x {
        click(&mut s, (x, title_y), t);
        step(&mut s);
        x -= 4.0;
        t += 500.0;
    }
    step(&mut s);
    // A new Paper is named as people see pages; the stored legacy "Page 1" stays exactly as
    // it was (no rename of existing names).
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Page 1", "Paper 2"], "added after the active page");
    assert_eq!(s.ed.page_name(), "Paper 2");
    // Drag Paper 2's row onto Page 1's: reordered.
    let (from, to) = (page_row(&mut s, "Paper 2"), page_row(&mut s, "Page 1"));
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
    assert_eq!(names, ["Paper 2", "Page 1"], "reordered");
    assert_eq!(s.ed.page_name(), "Paper 2", "the shown page stays");
    // One save: the new list and active page, and the new page's content (never saved).
    let new = s.ed.active_page_id().to_string();
    let snap = s.save_snapshot().expect("the page changes");
    let (active, list, sent) = parse_save(&snap);
    assert_eq!(active, new);
    assert_eq!(
        list,
        [
            (new.clone(), "Paper 2".to_string()),
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

// ---- one app: the file's identity and the way back live in upstream's own chrome (the left
// panel's top, or the controls floating over the canvas while it is closed)

/// Whether text `wanted` is painted inside the Canvas identity (its name and Paper line).
fn in_identity(s: &mut WebState, wanted: &str) -> bool {
    let id = s.ui.chrome.identity.expect("the identity is laid out");
    painted_text(s)
        .iter()
        .any(|(t, r)| t.trim() == wanted && id.expand(0.5).contains_rect(*r))
}

#[test]
fn the_file_name_is_the_one_canvas_title_and_follows_a_rename() {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let pages = [page("p", "Page 1", rect)];
    let mut s = WebState::open(&canvas_named(&pages, "p", "Landing"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    // A fresh view keeps Layers closed: the title floats at the canvas's top left.
    let float =
        s.ui.chrome
            .left_float
            .expect("the closed panel's row floats");
    assert!(float.contains_rect(s.ui.chrome.identity.unwrap()));
    let in_bar = |s: &mut WebState, wanted: &str| in_identity(s, wanted);
    assert!(
        in_bar(&mut s, "Landing"),
        "the file's name is the Canvas title"
    );
    // Revision 20: one title, no Paper line under or beside it (the Papers list names Papers).
    assert!(
        !in_bar(&mut s, "Page 1"),
        "no extra Paper line in the title"
    );
    assert!(
        !paints(&mut s, "Page 1"),
        "nor anywhere while the list is closed"
    );
    assert_eq!(
        s.ed.page_name(),
        "Page 1",
        "the file name is not a page name"
    );
    // A rename in the library reaches a clean editor with its next reload.
    let now = pages_now(&s);
    s.reload(&canvas_named(&now, "p", "Launch plan")).unwrap();
    step(&mut s);
    assert!(in_bar(&mut s, "Launch plan"));
    assert!(!in_bar(&mut s, "Landing"));
    assert!(!s.dirty(), "a name from the server is no local change");
}

#[test]
fn upstream_s_all_projects_button_is_the_host_s_way_back() {
    let mut s = state();
    let left = s.canvas_rect().min.x;
    // No header across the surface: the left panel's own top row starts with All Projects, at
    // the surface's top left; the Canvas's title follows it and the Layers toggle ends the row
    // (revision 20: the menu is All Projects' own, no separate `···`).
    let home = s.ui.chrome.home.expect("All Projects is laid out");
    let layers = s.ui.chrome.layers.expect("the Layers toggle is laid out");
    let id = s.ui.chrome.identity.expect("the title is laid out");
    assert_eq!(s.ui.chrome.menu, Some(home), "one control for both");
    assert!(home.min.x < 12.0 && home.max.y <= 40.0, "{home:?}");
    assert!(home.max.x <= id.min.x && id.max.x <= layers.min.x, "{id:?}");
    assert!(
        (home.center().y - id.center().y).abs() < 0.5,
        "the title is in the same row"
    );
    assert!(
        layers.max.x <= left && layers.max.x > left - 20.0,
        "the toggle at the panel's right end: {layers:?}"
    );
    assert!(!paints(&mut s, "···"), "no separate menu glyph");
    assert!(left >= 180.0, "the layers panel beside the canvas");
    // A click opens the menu (going nowhere yet); Back to All Projects comes first and is the
    // same request home as before.
    open_menu(&mut s, 1000.0);
    assert_eq!(s.take_request(), None, "opening the menu goes nowhere");
    let text = painted_text(&mut s);
    let at = |wanted: &str| {
        let found = text.iter().rev().find(|(t, _)| t.trim() == wanted);
        found.map(|(_, r)| r.min.y)
    };
    let back = at("Back to All Projects").expect("Back to All Projects in the menu");
    for item in [
        "Rename Canvas…",
        "Rename Paper…",
        "Go to Paper",
        "About & notices",
    ] {
        let y = at(item).unwrap_or_else(|| panic!("{item} in the menu: {text:?}"));
        assert!(back < y, "Back to All Projects comes before {item}");
    }
    click_front_label(&mut s, "Back to All Projects", 2000.0);
    assert_eq!(
        s.take_request(),
        Some(HostRequest::Home),
        "the editor asks its host to go Home"
    );
    assert!(!s.dirty(), "asking changes nothing in the document");
}

// ---- R25-F3: the granted surface can be a narrow sidebar (320 px). Upstream's narrow layout
// (`UiState::narrow`, opted into by this host below `NARROW_BELOW`) keeps the canvas, the
// tools, All projects and one side panel at a time reachable; wider, the ordinary layout.

const NARROW: f64 = 320.0;
const TALL: f64 = 760.0;

/// The one-rectangle canvas of [`state`] on a 320 × 760 surface, laid out.
fn narrow_state() -> WebState {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let pages = [page("p", "Page 1", rect)];
    let mut s = WebState::open(&canvas_named(&pages, "p", "Landing"), false, NARROW, TALL).unwrap();
    settle(&mut s, 0.0);
    s
}

/// Click the panel toggle `label` ("Layers", "Design") where it is laid out (in its panel's
/// top row, or floating over the canvas while that panel is closed) and let the arrangement
/// settle (it changes after one frame without a focused text field).
fn toggle(s: &mut WebState, label: &str, t: f64) {
    let r = match label {
        "Layers" => s.ui.chrome.layers,
        _ => s.ui.chrome.design,
    }
    .unwrap_or_else(|| panic!("the {label} toggle is laid out"));
    click(s, centre(r), t);
    for _ in 0..4 {
        step(s);
    }
}

/// The single panel a narrow or medium surface shows now.
fn shown_panel(s: &WebState) -> NarrowPanel {
    s.ui.narrow.shown().expect("laid out").panel()
}

/// Every tool through the rail, as a person reaches it: each family's main button, then each
/// row of its flyout (opened with the family's own chevron). Each flyout lies inside the
/// canvas; choosing a row closes it; nothing reaches the canvas. Returns the tools reached.
fn reach_every_tool(s: &mut WebState, mut t: f64) -> std::collections::BTreeSet<String> {
    use crate::ui::rail::FAMILIES;
    let mut reached = std::collections::BTreeSet::new();
    let canvas = s.canvas_rect();
    for (fi, (_, tools)) in FAMILIES.iter().enumerate() {
        let (main, chevron) = s.ui.rail.rects.rows[fi];
        assert!(
            canvas.contains_rect(main),
            "family {fi}: {main:?} in {canvas:?}"
        );
        click(s, centre(main), t);
        step(s);
        t += 500.0;
        assert_eq!(
            crate::ui::rail::family_of(s.ed.tool),
            fi,
            "the main button runs its family's member"
        );
        reached.insert(format!("{:?}", s.ed.tool));
        let Some(chevron) = chevron else {
            assert_eq!(tools.len(), 1, "only a one-tool family has no chevron");
            continue;
        };
        // Revision 18's slim rail: the arrow's own 12 px column (not revision 2's 24 px), a
        // whole row high (`ROW_H`, or on a short room the rows' smaller height, never below
        // `MIN_ROW_H`), beside the main button and never over it.
        assert!(
            chevron.min.x >= main.max.x - 0.5
                && chevron.width() >= crate::ui::rail::CHEVRON_W - 0.5
                && (chevron.height() - main.height()).abs() < 0.5
                && chevron.height() >= crate::ui::rail::MIN_ROW_H - 0.5,
            "the chevron is its own target, beside the main button: {chevron:?}"
        );
        for (k, tool) in tools.iter().enumerate() {
            let before = s.ed.tool;
            click(s, centre(chevron), t);
            step(s);
            step(s);
            t += 500.0;
            assert_eq!(s.ui.rail.flyout, Some(fi), "the chevron opens the flyout");
            assert_eq!(s.ed.tool, before, "opening the flyout chooses no tool");
            let fly = s.ui.rail.rects.flyout.expect("the flyout is laid out");
            assert!(
                fly.min.x >= canvas.min.x && fly.max.x <= canvas.max.x + 0.5,
                "inside the canvas: {fly:?} {canvas:?}"
            );
            let (row_tool, row) = s.ui.rail.rects.flyout_rows[k];
            assert_eq!(row_tool, *tool, "the family's tools, in order");
            assert!(
                painted_whole(s, tool.label(), row),
                "{tool:?}: its row's name is painted whole, not scrolled or clipped away"
            );
            click(s, centre(row), t);
            step(s);
            t += 500.0;
            assert_eq!(s.ed.tool, *tool, "a whole row chooses its tool");
            assert_eq!(s.ui.rail.flyout, None, "and closes the flyout");
            assert_eq!(
                s.ui.rail.members[fi], *tool,
                "it is the family's member now"
            );
            reached.insert(format!("{tool:?}"));
        }
    }
    reached
}

#[test]
fn r25_a_narrow_surface_keeps_the_canvas_all_projects_and_every_tool_reachable() {
    let mut s = narrow_state();
    assert_eq!(shown_panel(&s), NarrowPanel::Canvas);
    let c = s.canvas_rect();
    assert_eq!(c.min.x, 0.0, "no side panel beside the canvas: {c:?}");
    assert!(
        c.width() >= NARROW as f32 - 1.0,
        "the canvas spans the surface: {c:?}"
    );
    assert!(
        c.min.y == 0.0 && c.height() >= TALL as f32 - 1.0,
        "the whole surface, with no header: {c:?}"
    );
    assert_eq!(
        (s.ed.viewport.x, s.ed.viewport.y),
        (c.width() as f64, c.height() as f64)
    );
    // Both panels are closed: their top rows float over the canvas's top corners. The left one
    // carries All Projects (with the menu), the file's name and the Layers toggle; the right one
    // Export, the zoom and the Design toggle (an icon, the mirror of Layers). All inside the
    // surface, at its top, and clear of each other and of the rail.
    let left = s.ui.chrome.left_float.expect("the left row floats");
    let right = s.ui.chrome.right_float.expect("the right toggle floats");
    let painted = painted_text(&mut s);
    let (_, r) = painted
        .iter()
        .find(|(t, _)| t.trim() == "Landing")
        .unwrap_or_else(|| panic!("Landing not painted: {painted:?}"));
    assert!(left.contains_rect(*r), "{r:?} in {left:?}");
    let zoom = s.ui.chrome.zoom.expect("the zoom is in the right bar");
    assert!(right.contains_rect(zoom), "{zoom:?} in {right:?}");
    for r in [s.ui.chrome.home, s.ui.chrome.layers] {
        let r = r.expect("laid out");
        assert!(left.contains_rect(r) && r.max.y <= 40.0, "{r:?}");
    }
    let design = s.ui.chrome.design.expect("laid out");
    assert!(
        right.contains_rect(design) && design.max.x <= NARROW as f32,
        "{design:?}"
    );
    let rail = s.ui.rail.rects.rail.expect("the rail");
    assert!(
        !overlap(left, right) && !overlap(left, rail),
        "{left:?} {right:?} {rail:?}"
    );
    assert!(left.max.x <= NARROW as f32 && c.contains_rect(left) && c.contains_rect(right));
    // All Projects' Back to All Projects asks the host to go Home.
    open_menu(&mut s, 1000.0);
    click_front_label(&mut s, "Back to All Projects", 1100.0);
    assert_eq!(
        s.take_request(),
        Some(HostRequest::Home),
        "All Projects is reachable"
    );
    // Every tool through the rail: main buttons and the families' flyouts, inside the surface.
    let reached = reach_every_tool(&mut s, 2000.0);
    let all: std::collections::BTreeSet<String> =
        Tool::ALL.iter().map(|t| format!("{t:?}")).collect();
    assert_eq!(reached, all, "every tool is on the surface");
    assert_eq!(s.ed.doc.nodes.len(), 1, "probing the tools drew nothing");
    s.run(Action::Tool(Tool::Select));
    let t = 100_000.0;
    // The canvas itself takes input: a drag with the rectangle tool draws where it went.
    s.run(Action::Tool(Tool::Rect));
    let (from, to) = (at(&s, 40.0, 60.0), at(&s, 120.0, 140.0));
    drag(&mut s, from, to, t);
    assert_eq!(s.ed.doc.nodes.len(), 2);
    let id = s.ed.selection[0];
    let r = s.ed.doc.world_rect(id).unwrap();
    assert_eq!((r.x, r.y, r.w, r.h), (40.0, 60.0, 80.0, 80.0), "{r:?}");
}

#[test]
fn r25_narrow_panels_open_one_at_a_time_as_sheets_and_the_wide_layout_follows_the_choice() {
    let mut s = narrow_state();
    toggle(&mut s, "Layers", 1000.0);
    assert_eq!(shown_panel(&s), NarrowPanel::Layers);
    assert_eq!(s.ui.narrow.shown().unwrap().tier, crate::ui::Tier::Sheet);
    assert!(
        paints(&mut s, "Papers") && paints(&mut s, "Page 1"),
        "upstream's pages and layers"
    );
    // The sheet covers the canvas, so its top row (under its way back) carries the way home
    // and both toggles; nothing floats.
    let back = s.ui.chrome.sheet_back.expect("the sheet's way back");
    let home = s.ui.chrome.home.expect("home on the sheet");
    let design = s.ui.chrome.design.expect("the Design toggle on the sheet");
    for r in [home, design] {
        assert!(
            r.min.y >= back.max.y && r.max.y <= back.max.y + 48.0,
            "{r:?}"
        );
    }
    assert!((home.center().y - design.center().y).abs() < 0.5 && design.max.x <= NARROW as f32);
    assert!(s.ui.chrome.left_float.is_none() && s.ui.chrome.right_float.is_none());
    assert!(
        s.canvas_rect().width() < 2.0,
        "the sheet takes the room: no canvas sliver"
    );
    assert!(
        s.ui.rail.rects.rail.is_none(),
        "no rail behind the opaque sheet"
    );
    // A press over the sheet never reaches the canvas.
    s.run(Action::Tool(Tool::Rect));
    step(&mut s);
    drag(&mut s, (100.0, 400.0), (180.0, 460.0), 1500.0);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes.len(), 1, "nothing drawn from the panel");
    press(&mut s, "Escape", "Escape");
    toggle(&mut s, "Design", 2000.0);
    assert_eq!(shown_panel(&s), NarrowPanel::Design);
    assert!(paints(&mut s, "Export"), "upstream's inspector");
    assert!(!paints(&mut s, "Papers"), "one panel at a time");
    // The sheet's own way back to the canvas.
    let back =
        s.ui.chrome
            .sheet_back
            .expect("the sheet says how to get back");
    click(&mut s, centre(back), 3000.0);
    for _ in 0..4 {
        step(&mut s);
    }
    assert_eq!(shown_panel(&s), NarrowPanel::Canvas);
    assert!(s.canvas_rect().width() >= NARROW as f32 - 1.0);
    // Wide: the person's choice so far (both closed) is kept; a fresh view's default Design
    // isn't forced back over it.
    s.resize(W, H, 1.0);
    let t = settle(&mut s, 4000.0);
    let a = s.ui.narrow.shown().unwrap();
    assert_eq!(
        (a.tier, a.layers, a.design),
        (crate::ui::Tier::Wide, false, false)
    );
    let c = s.canvas_rect();
    assert!(c.min.x < 1.0 && c.max.x >= W as f32 - 1.0, "{c:?}");
    // Both opened there sit beside the canvas, as before.
    toggle(&mut s, "Layers", 5000.0);
    toggle(&mut s, "Design", 6000.0);
    let c = s.canvas_rect();
    assert!(c.min.x >= 180.0 && c.max.x <= W as f32 - 220.0, "{c:?}");
    assert!(paints(&mut s, "Papers") && paints(&mut s, "Export"));
    // Narrow again: one panel at a time, the one opened last.
    s.resize(NARROW, TALL, 1.0);
    settle(&mut s, t);
    assert_eq!(shown_panel(&s), NarrowPanel::Design);
    assert!(
        !s.dirty(),
        "arranging the panels changes nothing in the document"
    );
}

#[test]
fn r25_the_narrow_layout_waits_for_a_focused_number_field_and_never_drops_its_draft() {
    for (typed, finish) in [("-", "Escape"), ("-150", "Enter")] {
        let mut s = narrow_state();
        click_at(&mut s, 150.0, 150.0, 0.0);
        step(&mut s);
        let id = s.ed.selection[0];
        toggle(&mut s, "Design", 500.0);
        assert_eq!(shown_panel(&s), NarrowPanel::Design);
        let sheet = s.ui.narrow.shown();
        let text = painted_text(&mut s);
        let (_, bounds) = text
            .iter()
            .find(|(t, _)| t.starts_with("X  "))
            .unwrap_or_else(|| panic!("X field not painted: {text:?}"));
        let p = bounds.center();
        click(&mut s, (p.x as f64, p.y as f64), 1000.0);
        step(&mut s);
        step(&mut s);
        assert!(
            s.field_has_keyboard(),
            "{typed}: the X field has the keyboard"
        );
        s.modifiers(false, true, false, false);
        press(&mut s, "a", "KeyA");
        s.modifiers(false, false, false, false);
        step(&mut s);
        s.paste(typed);
        step(&mut s);
        // The surface widens and narrows while the field has the keyboard: nothing moves.
        for (w, h) in [(W, H), (NARROW, TALL), (W, H)] {
            s.resize(w, h, 1.0);
            for _ in 0..4 {
                step(&mut s);
            }
            assert_eq!(s.ui.narrow.shown(), sheet, "{typed}: waits");
            assert!(paints(&mut s, typed), "{typed}: the draft is still shown");
            assert!(
                s.field_has_keyboard() && s.unsaved_work() && !s.can_reload(),
                "{typed}"
            );
        }
        // Finished by upstream's own keys, then the arrangement follows the surface.
        press(&mut s, finish, finish);
        for _ in 0..4 {
            step(&mut s);
        }
        assert!(!s.field_has_keyboard(), "{typed}: finished");
        assert_eq!(
            s.ui.narrow.shown().map(|a| (a.tier, a.design)),
            Some((crate::ui::Tier::Wide, true)),
            "{typed}: wide now, with the Design panel the person opened"
        );
        let want = if typed == "-" { 100.0 } else { -150.0 };
        assert_eq!(
            axis(&s, id, "X  "),
            want,
            "{typed}: as upstream finishes it"
        );
        assert_eq!(s.dirty(), typed != "-", "{typed}");
    }
}

#[test]
fn r25_a_narrow_rename_opens_the_layers_panel_and_a_toggle_finishes_it_as_click_away_does() {
    let mut s = narrow_state();
    click_at(&mut s, 150.0, 150.0, 0.0);
    let id = s.ed.selection[0];
    let old = s.ed.doc.get(id).unwrap().name.clone();
    s.modifiers(false, true, false, false);
    press(&mut s, "r", "KeyR");
    s.modifiers(false, false, false, false);
    for _ in 0..4 {
        step(&mut s);
    }
    assert_eq!(
        shown_panel(&s),
        NarrowPanel::Layers,
        "Rename shows the rows"
    );
    // (Run145: the click's new selection and the Rename reached the same frame, and the
    // reveal the selection started then swapped the Layers sheet for Design. The explicit
    // Rename wins: the reveal ends, and nothing brings it back while the rename is open.)
    assert!(!s.ui.panels.revealed(), "the explicit Rename won");
    assert!(
        s.ui.renaming.is_some(),
        "the rename stays open while they appear"
    );
    assert!(s.field_has_keyboard(), "its field has the keyboard");
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    assert_eq!(s.ed.doc.get(id).unwrap().name, old, "still a draft");
    timed_frames(&mut s, 30);
    assert_eq!(shown_panel(&s), NarrowPanel::Layers, "no later reveal");
    assert!(
        s.field_has_keyboard() && s.unsaved_work(),
        "the draft stays"
    );
    // Choosing Design is a click elsewhere: the field finishes (commits), then Design shows.
    toggle(&mut s, "Design", 2000.0);
    let renamed = s.ed.doc.get(id).unwrap().name.clone();
    assert!(
        renamed != old && renamed.contains('q'),
        "{old} -> {renamed}"
    );
    assert!(s.ui.renaming.is_none());
    assert_eq!(shown_panel(&s), NarrowPanel::Design);
    assert!(s.dirty(), "one ordinary edit to save");
}

#[test]
fn r25_the_narrow_layout_is_opt_in_and_off_by_default() {
    assert_eq!(
        crate::ui::UiState::default().narrow.below,
        0.0,
        "native windows: off"
    );
    let mut s = narrow_state();
    s.ui.narrow = Default::default();
    step(&mut s);
    step(&mut s);
    let a = s.ui.narrow.shown().unwrap();
    assert_eq!(
        a.tier,
        crate::ui::Tier::Wide,
        "the wide tier at every width"
    );
    assert!(
        !a.layers && a.design,
        "a fresh view's wide defaults: Layers closed, Design open"
    );
}

// ---- the docked tier (DESIGN.md): from DOCK_FROM to NARROW_BELOW, the chosen panel sits at
// its usual width beside the canvas, which keeps its drawing room and the floating tools.

#[test]
fn a_medium_surface_docks_one_panel_beside_the_canvas_and_keeps_a_draft_across_resizes() {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let pages = [page("p", "Page 1", rect)];
    let mut s = WebState::open(&canvas_named(&pages, "p", "Landing"), false, 800.0, TALL).unwrap();
    settle(&mut s, 0.0);
    assert_eq!(shown_panel(&s), NarrowPanel::Canvas);
    assert!(s.canvas_rect().width() >= 799.0, "the canvas first");
    // Design opened by its toggle (the person's own choice: pinned, not a reveal), docked on
    // the right; then the rectangle selected. (Revision 20 reveals Design for a selection here
    // too, so the person's open choice comes first; the reveal has its own tests.)
    toggle(&mut s, "Design", 500.0);
    click_at(&mut s, 150.0, 150.0, 600.0);
    step(&mut s);
    let id = s.ed.selection[0];
    assert!(!s.ui.panels.revealed(), "pinned, not revealed");
    assert_eq!(shown_panel(&s), NarrowPanel::Design);
    assert_eq!(s.ui.narrow.shown().unwrap().tier, crate::ui::Tier::Docked);
    let c = s.canvas_rect();
    assert_eq!(c.min.x, 0.0, "{c:?}");
    assert!(
        (c.width() - (800.0 - 260.0)).abs() < 4.0,
        "the inspector at its usual width beside the canvas: {c:?}"
    );
    // The canvas still draws: a drag with the rectangle tool where the canvas is.
    s.run(Action::Tool(Tool::Rect));
    let (from, to) = (at(&s, 300.0, 300.0), at(&s, 360.0, 340.0));
    drag(&mut s, from, to, 700.0);
    assert_eq!(s.ed.doc.nodes.len(), 2, "drawn beside the docked panel");
    s.run(Action::Tool(Tool::Select));
    // A number draft in the docked inspector, then the surface narrows below DOCK_FROM: the
    // panel fills the room, the same field keeps the keyboard and the draft.
    click_at(&mut s, 150.0, 150.0, 900.0);
    step(&mut s);
    assert_eq!(s.ed.selection, vec![id]);
    let text = painted_text(&mut s);
    let (_, bounds) = text
        .iter()
        .find(|(t, _)| t.starts_with("X  "))
        .unwrap_or_else(|| panic!("X field not painted: {text:?}"));
    let p = bounds.center();
    click(&mut s, (p.x as f64, p.y as f64), 1000.0);
    step(&mut s);
    step(&mut s);
    assert!(s.field_has_keyboard(), "the X field has the keyboard");
    s.modifiers(false, true, false, false);
    press(&mut s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(&mut s);
    s.paste("-");
    step(&mut s);
    // Docked ↔ sheet is the same panel (DESIGN.md): it follows the surface at once, the field
    // focused, and nothing waits.
    for (w, filled) in [(500.0, true), (800.0, false), (500.0, true)] {
        s.resize(w, TALL, 1.0);
        for _ in 0..4 {
            step(&mut s);
        }
        assert_eq!(shown_panel(&s), NarrowPanel::Design, "{w}");
        let tier = if filled {
            crate::ui::Tier::Sheet
        } else {
            crate::ui::Tier::Docked
        };
        assert_eq!(s.ui.narrow.shown().unwrap().tier, tier, "{w}");
        assert_eq!(
            s.canvas_rect().width() < 2.0,
            filled,
            "{w}: filled or docked"
        );
        assert!(paints(&mut s, "-"), "{w}: the draft is still shown");
        assert!(
            s.field_has_keyboard() && s.unsaved_work() && !s.can_reload(),
            "{w}"
        );
    }
    press(&mut s, "Escape", "Escape");
    step(&mut s);
    step(&mut s);
    assert_eq!(
        axis(&s, id, "X  "),
        100.0,
        "cancelled as upstream cancels it"
    );
}

#[test]
fn the_identity_keeps_the_file_name_and_the_controls_apart_at_every_width() {
    // A fresh view keeps Layers closed, so the left panel's row floats over the canvas with the
    // Canvas's name (truncated to its slot if need be). From the narrow sidebar to a wide window
    // the name stays inside the identity and the surface, and overlaps no control (home, menu,
    // either toggle), the rail or any other text painted beside it.
    for w in [320.0, 480.0, 640.0, 800.0, 1100.0, 1600.0] {
        let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
        let pages = [page("p", "Page 1", rect)];
        let name = "A rather long file name for the top bar";
        let mut s = WebState::open(&canvas_named(&pages, "p", name), false, w, TALL).unwrap();
        settle(&mut s, 0.0);
        let id = s.ui.chrome.identity.expect("the identity is laid out");
        let text = painted_text(&mut s);
        let (_, title) = text
            .iter()
            .find(|(t, _)| t.trim() == name)
            .unwrap_or_else(|| panic!("{w}: the file name is painted: {text:?}"));
        assert!(
            title.max.x <= w as f32 && id.expand(0.5).contains_rect(*title),
            "{w}: inside the identity and the surface: {title:?} {id:?}"
        );
        let chrome = &s.ui.chrome;
        let controls = [chrome.home, chrome.menu, chrome.layers, chrome.design];
        for c in controls.into_iter().chain([s.ui.rail.rects.rail]).flatten() {
            assert!(
                !overlap(*title, c),
                "{w}: {c:?} overlaps the name {title:?}"
            );
        }
        for (t, r) in text
            .iter()
            .filter(|(t, r)| t.trim() != name && overlap(*r, id))
        {
            assert!(
                !overlap(*r, *title),
                "{w}: {t:?} {r:?} overlaps the name {title:?}"
            );
        }
    }
}

/// Open the All Projects menu (revision 20: the square control's own menu, which replaced the
/// separate `···`), hovered first as a person reaches it, then laid out twice.
fn open_menu(s: &mut WebState, t: f64) {
    let home = s.ui.chrome.home.expect("All Projects is laid out");
    let p = centre(home);
    s.pointer_move(p.0, p.1, t);
    step(s);
    click(s, p, t + 5.0);
    step(s);
    step(s);
}

/// Rename Paper… from the All Projects menu: the shown Paper's one rename field, in the Papers
/// list (revision 20 took the inspector's copy of this entry away; this is the same
/// `start_paper_rename` the inspector's button called).
fn menu_rename_paper(s: &mut WebState, t: f64) {
    open_menu(s, t);
    click_front_label(s, "Rename Paper…", t + 100.0);
}

/// Click the painted text `label` (hovered first, so a submenu opens), then lay out twice.
fn click_label(s: &mut WebState, label: &str, t: f64) {
    let text = painted_text(s);
    let (_, r) = text
        .iter()
        .find(|(x, _)| x.trim() == label)
        .unwrap_or_else(|| panic!("{label:?} not painted: {text:?}"));
    let p = r.center();
    s.pointer_move(p.x as f64, p.y as f64, t);
    step(s);
    click(s, (p.x as f64, p.y as f64), t + 5.0);
    step(s);
    step(s);
}

#[test]
fn menu_appearance_switches_the_chrome_and_the_canvas_and_is_reported_once() {
    let mut s = state();
    let authored = page_to_json(&s.ed.page_data(0));
    assert_eq!(s.appearance(), "system");
    open_menu(&mut s, 100.0);
    click_label(&mut s, "Appearance", 200.0);
    click_label(&mut s, "Dark", 300.0);
    assert_eq!(s.appearance(), "dark");
    assert_eq!(s.take_request(), Some(HostRequest::Appearance));
    assert_eq!(s.take_request(), None, "reported once");
    settle(&mut s, 400.0);
    assert!(s.dark() && s.ctx.global_style().visuals.dark_mode);
    // The canvas is shown with the same resolved appearance (what `Painter::paint` picks):
    // a dark work surface; an export still draws the document as authored.
    let display = scene::Display::for_theme(s.ctx.theme());
    assert_eq!(display, scene::Display::Dark);
    let mut cache = SceneCache::default();
    cache.set_display(display);
    let f = scene::canvas(&s.ed, &mut cache, V2::ZERO, true);
    assert_eq!(
        f.under[0].mesh.vertices[0].color,
        scene::rgba(scene::CANVAS_BG_DARK)
    );
    // Presentation only.
    assert_eq!(page_to_json(&s.ed.page_data(0)), authored);
    assert!(!s.dirty() && s.can_reload() && s.save_snapshot().is_none());
    // Back to System, through the same menu.
    open_menu(&mut s, 500.0);
    click_label(&mut s, "Appearance", 600.0);
    click_label(&mut s, "System", 700.0);
    assert_eq!(s.appearance(), "system");
    assert_eq!(s.take_request(), Some(HostRequest::Appearance));
    settle(&mut s, 800.0);
    assert!(
        !s.dark(),
        "System with no OS report is light (egui's light fallback)"
    );
}

#[test]
fn the_appearance_follows_the_os_unless_chosen_and_keeps_the_document_and_a_draft() {
    // Light until the page reports a dark OS (egui's light fallback); System follows the OS
    // live; an explicit Light or Dark wins. Presentation only.
    let mut s = state();
    let dark = |s: &WebState| s.ctx.global_style().visuals.dark_mode;
    assert!(!dark(&s) && !s.dark());
    s.set_system_dark(false);
    assert!(s.redraw, "the first OS report is drawn");
    step(&mut s);
    s.set_system_dark(false);
    assert!(!s.redraw, "the same OS appearance asks for no frame");
    // A number draft in the inspector ("-" before a negative X).
    click_at(&mut s, 150.0, 150.0, 100.0);
    step(&mut s);
    let id = s.ed.selection[0];
    let text = painted_text(&mut s);
    let (_, bounds) = text
        .iter()
        .find(|(t, _)| t.starts_with("X  "))
        .unwrap_or_else(|| panic!("X field not painted: {text:?}"));
    let p = bounds.center();
    click(&mut s, (p.x as f64, p.y as f64), 200.0);
    step(&mut s);
    step(&mut s);
    assert!(s.field_has_keyboard(), "the X field has the keyboard");
    s.modifiers(false, true, false, false);
    press(&mut s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(&mut s);
    s.paste("-");
    step(&mut s);
    let (stamp, revision, dirty) = (s.ed.doc_stamp, s.ed.revision, s.dirty());
    let authored = page_to_json(&s.ed.page_data(0));
    // (OS dark?, kept preference or none, shown dark?)
    for (os, pref, want) in [
        (true, None, true),
        (true, Some("light"), false),
        (false, Some("dark"), true),
        (false, Some("system"), false),
        (true, None, true),
    ] {
        s.set_system_dark(os);
        if let Some(p) = pref {
            assert!(s.set_appearance(p));
        }
        assert!(s.redraw, "{os} {pref:?}: a change of appearance is drawn");
        step(&mut s);
        step(&mut s);
        assert_eq!(dark(&s), want, "{os} {pref:?}");
        assert_eq!(s.dark(), want);
        assert_eq!(
            s.take_request(),
            None,
            "a kept preference isn't reported back"
        );
        assert_eq!(page_to_json(&s.ed.page_data(0)), authored);
        assert!(paints(&mut s, "-"), "{want}: the draft is still shown");
        assert!(
            s.field_has_keyboard() && s.unsaved_work() && !s.can_reload(),
            "{want}: the draft keeps the keyboard and its protection"
        );
        assert_eq!(
            (s.ed.doc_stamp, s.ed.revision, s.dirty()),
            (stamp, revision, dirty),
            "{want}: the document is untouched"
        );
    }
    assert_eq!(s.ed.selection, vec![id]);
    press(&mut s, "Escape", "Escape");
    step(&mut s);
    step(&mut s);
    assert_eq!(
        axis(&s, id, "X  "),
        100.0,
        "cancelled as upstream cancels it"
    );
    assert!(dark(&s), "and the appearance stays");
}

// ---- R22: page actions while the pages panel draws, and fields opened on another page.
// The first four tests are the independent R22 review's probes (r22-probes.rs), adopted with
// their fixtures and assertions unchanged (only wrapped in this file's style).

#[test]
fn r22_deleting_the_first_of_two_pages_keeps_the_editor_running() {
    let mut s = two_pages();
    let button = menu_item(&mut s, "Alpha", "Delete Paper", 100.0);
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
    open_layers(&mut s);
    let button = menu_item(&mut s, "Beta", "Delete Paper", 100.0);
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
    let button = menu_item(&mut s, "Beta", "Delete Paper", 100.0);
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
        open_layers(&mut s);
        if shown != "Beta" {
            click_page(&mut s, shown, 50.0);
        }
        let button = menu_item(&mut s, deleted, "Delete Paper", 100.0);
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
    let d = menu_item(&mut s, "Page 1", "Delete Paper", 100.0);
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
                // The list header's +, found as the pages panel test finds it.
                let left = s.canvas_rect().min.x as f64;
                let (title_x, title_y) = painted_at(&mut s, "Papers", |r| (r.max.x as f64) < left);
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
                let d = menu_item(&mut s, "Beta", "Duplicate Paper", 3000.0);
                click(&mut s, d, 3100.0);
                step(&mut s);
                step(&mut s);
            }
            _ => {
                let d = menu_item(&mut s, "Beta", "Delete Paper", 3000.0);
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
    double_click(&mut s, p, 100.0);
    assert!(s.ui.pages.renaming.is_some());
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    // Alpha (the row before it) is deleted from its menu: the draft names Beta, not row 1.
    let d = menu_item(&mut s, "Alpha", "Delete Paper", 3000.0);
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
    s.paste("Go to Paper: Beta");
    step(s);
    step(s);
    assert_eq!(s.ui.palette.query, "Go to Paper: Beta");
    assert!(paints(s, "Go to Paper: Beta"));
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
    double_click(&mut s, p, 100.0);
    assert!(s.ui.pages.renaming.is_some());
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    assert!(!s.can_reload(), "a changed page-rename draft");
    // One click on the list header's collapse chevron: its first item, 16 px wide, inside the
    // sidebar's 10 px margin, on the "Papers" title's line. (Any other click there would finish
    // the field by ordinary click-away first, which wouldn't test the collapse.)
    let left = s.canvas_rect().min.x as f64;
    let (_, title_y) = painted_at(&mut s, "Papers", |r| (r.max.x as f64) < left);
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

/// Hover then click the frontmost painted text `label` (the last painted: egui paints its menus
/// and popups after the panels), then lay out twice, as [`click_label`]; returns where it was.
/// For a menu item whose text a panel paints too (the compact menu's View and the inspector's).
fn click_front_label(s: &mut WebState, label: &str, t: f64) -> egui::Rect {
    let text = painted_text(s);
    let r = text
        .iter()
        .rev()
        .find(|(x, _)| x.trim() == label)
        .map(|(_, r)| *r)
        .unwrap_or_else(|| panic!("{label:?} not painted: {text:?}"));
    let p = r.center();
    s.pointer_move(p.x as f64, p.y as f64, t);
    step(s);
    click(s, (p.x as f64, p.y as f64), t + 5.0);
    step(s);
    step(s);
    r
}

/// Hover then click painted text `wanted` where `within` accepts it, then lay out twice (as
/// [`click_label`], for text painted in more than one place).
fn click_label_in(s: &mut WebState, wanted: &str, within: impl Fn(egui::Rect) -> bool, t: f64) {
    let p = painted_at(s, wanted, within);
    s.pointer_move(p.0, p.1, t);
    step(s);
    click(s, p, t + 5.0);
    step(s);
    step(s);
}

#[test]
fn the_menu_s_rename_paper_opens_the_one_paper_rename_and_saves_it_once() {
    // Revision 20: the Design panel no longer repeats the Paper's name or a Rename Paper…
    // (revision 18 had a read-only copy there); Rename Paper… in the All Projects menu opens
    // that Paper's own rename in the Papers list, the same field, Enter and page-list save as
    // the list's own rename.
    let mut s = state();
    let text = inspector_text(&mut s);
    assert!(text.iter().any(|t| t == "Paper"), "{text:?}");
    assert!(
        !text.iter().any(|t| t == "Page 1" || t == "Rename Paper…"),
        "no Paper name or rename duplicated in the Design panel: {text:?}"
    );
    assert!(!s.field_has_keyboard(), "no field is open for the name");
    menu_rename_paper(&mut s, 5000.0);
    for _ in 0..3 {
        step(&mut s);
    }
    assert_eq!(s.ui.pages.renaming.as_ref().map(|(i, _)| *i), Some(0));
    assert_eq!(s.ui.pages.rename_field, s.ctx.memory(|m| m.focused()));
    assert!(
        s.field_has_keyboard(),
        "the Papers list's field has the keyboard"
    );
    s.modifiers(false, true, false, false);
    press(&mut s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(&mut s);
    press(&mut s, "X", "KeyX");
    step(&mut s);
    assert_eq!(s.ed.page_name(), "Page 1", "a draft until it is finished");
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.page_name(), "X");
    assert!(!s.field_has_keyboard(), "Enter finished the field");
    assert!(s.ed.meta_dirty && s.dirty() && !s.can_reload());
    let snap = s.save_snapshot().expect("the new name is saved");
    let (active, list, sent) = parse_save(&snap);
    assert_eq!(active, "p");
    assert_eq!(list, [("p".to_string(), "X".to_string())]);
    assert!(sent.is_empty(), "the page's content is unchanged");
    assert_eq!(saved_name(&snap), None, "not a Canvas rename");
    s.save_succeeded();
    assert!(!s.dirty() && s.can_reload() && !s.unsaved_work());
    // The Papers list shows the new name.
    let left = s.canvas_rect().min.x;
    painted_at(&mut s, "X", |r| r.max.x <= left);
}

/// R57's desired-behaviour regression (independent-probes.rs, adopted with its fixture and
/// assertions; only formatted in this file's style): from a fresh view, Papers closed, a Rename
/// Paper… outside the list opens it on the first click with the shown Paper's (Beta's) rename
/// focused; the draft holds through a resize that waits for it; Enter renames that Paper only,
/// the legacy "Page 1" and both Papers' content stay exact, in one page-list save. Revision 20
/// moved that entry from the inspector to the All Projects menu (the same `start_paper_rename`),
/// so only the click that starts it changed.
#[test]
fn r57_inspector_rename_opens_closed_papers_and_keeps_target_across_resize() {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let initial = [page("alpha1", "Page 1", rect), page("beta22", "Beta", rect)];
    let authored: Vec<_> = initial
        .iter()
        .map(|p| (p.doc.nodes.clone(), p.doc.root.clone()))
        .collect();
    let mut s = WebState::open(&canvas(&initial, "beta22"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    assert!(!s.ui.narrow.shown().unwrap().layers);
    menu_rename_paper(&mut s, 1000.0);
    timed_frames(&mut s, 5);
    assert!(
        s.ui.narrow.shown().unwrap().layers,
        "opens the previously closed Papers list"
    );
    assert_eq!(s.ui.pages.renaming.as_ref().map(|(i, _)| *i), Some(1));
    assert!(s.field_has_keyboard());
    s.modifiers(false, true, false, false);
    press(&mut s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(&mut s);
    s.paste("Revised Beta");
    timed_frames(&mut s, 2);
    s.resize(800.0, H, 1.0);
    timed_frames(&mut s, 5);
    assert_eq!(
        s.ui.narrow.shown().unwrap().tier,
        crate::ui::Tier::Wide,
        "the resize waits for the field"
    );
    assert_eq!(s.ed.pages[1].name, "Beta", "still a draft");
    assert!(s.unsaved_work() && !s.can_reload());
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 5);
    assert_eq!(s.ui.narrow.shown().unwrap().tier, crate::ui::Tier::Docked);
    assert_eq!(s.ed.pages[1].name, "Revised Beta");
    assert_eq!(s.ed.pages[0].name, "Page 1", "legacy name stays exact");
    assert_eq!(s.ed.active_page_id(), "beta22");
    let snap = s.save_snapshot().expect("one metadata save");
    let (active, names, sent) = parse_save(&snap);
    assert_eq!(active, "beta22");
    assert_eq!(
        names,
        vec![
            ("alpha1".to_string(), "Page 1".to_string()),
            ("beta22".to_string(), "Revised Beta".to_string())
        ]
    );
    assert!(sent.is_empty(), "neither Paper's authored content changed");
    for (i, want) in authored.iter().enumerate() {
        let data = s.ed.page_data(i);
        assert_eq!(&(data.doc.nodes, data.doc.root), want, "Paper {i}");
    }
    s.save_succeeded();
    assert!(s.can_reload() && s.save_snapshot().is_none());
}

/// Ten Papers ("Paper 1" … "Paper 10", ids p00000 … p00009), the tenth shown, Papers closed:
/// more rows than the Papers list's capped window holds.
fn ten_papers() -> WebState {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let pages: Vec<_> = (0..10)
        .map(|i| page(&format!("p{i:05}"), &format!("Paper {}", i + 1), rect))
        .collect();
    let mut s = WebState::open(&canvas(&pages, "p00009"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    s
}

/// R58's desired-behaviour regression (independent-scroll-window.rs, adopted with its fixture,
/// geometry thresholds and assertions; only formatted and its fixture shared): a Rename Paper…
/// outside the list (the inspector's until revision 20, now the All Projects menu's, the same
/// `start_paper_rename`) for the tenth Paper, below the list's capped window, opens a field the
/// person can see and hit, not an offscreen draft.
#[test]
fn r58_inspector_rename_reveals_the_focused_paper_beyond_the_initial_scroll_window() {
    let mut s = ten_papers();
    menu_rename_paper(&mut s, 1000.0);
    timed_frames(&mut s, 8);
    assert_eq!(s.ui.pages.renaming.as_ref().map(|(i, _)| *i), Some(9));
    assert!(s.field_has_keyboard());
    let id = s.ui.pages.rename_field.expect("the intended row's field");
    let response = s.ctx.read_response(id).expect("actual rename response");
    assert!(
        response.interact_rect.height() >= 18.0 && response.interact_rect.width() >= 40.0,
        "the focused Paper rename must be visible/hittable, not an offscreen draft: {response:?}"
    );
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 3);
    assert!(s.ui.pages.renaming.is_none() && !s.field_has_keyboard());
    assert!(s.can_reload() && s.save_snapshot().is_none());
}

#[test]
fn r58_a_revealed_rename_commits_on_its_own_paper_and_never_pins_the_scroll() {
    let mut s = ten_papers();
    let names =
        |s: &WebState| -> Vec<String> { s.ed.pages.iter().map(|p| p.name.clone()).collect() };
    let before = names(&s);
    let contents: Vec<_> = (0..10).map(|i| s.ed.page_doc(i).nodes.clone()).collect();
    let (selection, camera) = (s.ed.selection.clone(), s.ed.camera);
    menu_rename_paper(&mut s, 1000.0);
    timed_frames(&mut s, 8);
    let id = s.ui.pages.rename_field.expect("the tenth Paper's field");
    let shown = s.ctx.read_response(id).expect("its response").interact_rect;
    assert!(shown.height() >= 18.0, "revealed: {shown:?}");
    // The person scrolls the list back up (the wheel over it): the field moves down with its
    // row, out of the window, and stays there; nothing scrolls it back while it is focused.
    let at = (shown.center().x as f64, shown.center().y as f64);
    for k in 0..4 {
        s.wheel(0.0, -120.0, at.0, at.1);
        timed_frames(&mut s, 1);
        assert!(s.field_has_keyboard(), "{k}: the field keeps the keyboard");
    }
    // Long enough for egui's smoothed wheel scrolling to finish.
    timed_frames(&mut s, 40);
    let after = s.ctx.read_response(id).expect("its response").rect;
    assert!(
        after.min.y > shown.max.y,
        "the list scrolled up: {shown:?} → {after:?}"
    );
    timed_frames(&mut s, 10);
    let later = s.ctx.read_response(id).expect("its response").rect;
    assert_eq!(later, after, "no pinning while focused");
    // Typing still goes into it, and Enter renames that Paper only.
    s.modifiers(false, true, false, false);
    press(&mut s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(&mut s);
    s.paste("Final");
    timed_frames(&mut s, 2);
    assert_eq!(s.ed.tool, Tool::Select, "no key reached a tool");
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 3);
    let mut want = before.clone();
    want[9] = "Final".into();
    assert_eq!(names(&s), want, "only the tenth Paper is renamed");
    assert_eq!(s.ed.active_page_id(), "p00009");
    for (i, nodes) in contents.iter().enumerate() {
        assert_eq!(&s.ed.page_doc(i).nodes, nodes, "Paper {i}'s content");
    }
    assert_eq!((s.ed.selection.clone(), s.ed.camera), (selection, camera));
    let snap = s.save_snapshot().expect("one page-list save");
    let (active, list, sent) = parse_save(&snap);
    assert_eq!(active, "p00009");
    assert_eq!(list[9], ("p00009".to_string(), "Final".to_string()));
    assert!(sent.is_empty(), "no Paper's content is sent");
    s.save_succeeded();
    assert!(s.can_reload() && s.save_snapshot().is_none());
}

#[test]
fn r57_a_first_click_rename_with_papers_closed_owns_the_keys_until_its_field_has_them() {
    // Fresh, Papers closed. Right after the first click (before the list has opened and its
    // field taken the keyboard) a typed "r" is the rename's, never the Rectangle shortcut; then
    // the field has the keyboard and typing goes into it. Escape drops the draft, and the Paper,
    // the tool and the save state are as they were. Control: with Papers already open the same
    // click focuses the field at once, as before.
    // (Revision 20: the first click is the All Projects menu's Rename Paper…, the same
    // `start_paper_rename` the inspector's button called.)
    let mut s = fresh(W, H);
    assert!(!s.ui.narrow.shown().unwrap().layers);
    menu_rename_paper(&mut s, 1000.0);
    assert_eq!(
        s.ui.pages.renaming.as_ref().map(|(i, _)| *i),
        Some(0),
        "the requested rename survives the frame that asked for it"
    );
    assert!(s.key_down("r", "KeyR", false), "taken for the rename");
    s.key_up("r", "KeyR");
    assert_eq!(s.ed.tool, Tool::Select, "no tool shortcut while it opens");
    timed_frames(&mut s, 5);
    assert!(
        s.ui.narrow.shown().unwrap().layers,
        "the Papers list opened"
    );
    assert!(
        s.ui.pages.rename_field.is_some(),
        "its row's field is laid out"
    );
    assert_eq!(s.ui.pages.rename_field, s.ctx.memory(|m| m.focused()));
    assert!(s.field_has_keyboard());
    press(&mut s, "e", "KeyE");
    timed_frames(&mut s, 1);
    assert_eq!(s.ed.tool, Tool::Select, "typing is the field's");
    let draft =
        s.ui.pages
            .renaming
            .as_ref()
            .map(|(_, n)| n.clone())
            .unwrap();
    assert!(
        draft.ends_with('e') && !draft.contains('r'),
        "the typed e is in the draft, the held r never went anywhere: {draft:?}"
    );
    assert_eq!(s.ed.page_name(), "Page 1", "still a draft");
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 2);
    assert!(s.ui.pages.renaming.is_none() && !s.field_has_keyboard());
    assert_eq!(s.ed.page_name(), "Page 1", "Escape dropped it");
    assert!(!s.dirty() && s.can_reload() && s.save_snapshot().is_none());
    // With nothing requested, the same key is the editor's again.
    assert!(s.key_down("r", "KeyR", false));
    s.key_up("r", "KeyR");
    assert_eq!(
        s.ed.tool,
        Tool::Rect,
        "the shortcut works when no rename is pending"
    );
    s.run(Action::Tool(Tool::Select));
    // Control: Papers already open, the field has the keyboard after the click's own frames.
    menu_rename_paper(&mut s, 5000.0);
    timed_frames(&mut s, 2);
    assert_eq!(s.ui.pages.renaming.as_ref().map(|(i, _)| *i), Some(0));
    assert!(s.field_has_keyboard(), "open-panel control");
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 2);
    assert_eq!(s.ed.page_name(), "Page 1");
    assert!(!s.dirty());
}

// ---- revision 2, slice 1: the header's safe area, panel states, Collapse panels and zoom.
// These drive the real upstream UI (`ui::draw`) through this host's input; geometry asserts
// read what it laid out (`UiState::chrome`, `rail.rects`), not a copy of its constants.

/// A fresh view of the one-rectangle canvas on a `w` × `h` surface, laid out (no panel opened).
fn fresh(w: f64, h: f64) -> WebState {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let pages = [page("p", "Page 1", rect)];
    let mut s = WebState::open(&canvas_named(&pages, "p", "Landing"), false, w, h).unwrap();
    settle(&mut s, 0.0);
    s
}

fn overlap(a: egui::Rect, b: egui::Rect) -> bool {
    a.intersects(b) && a.intersect(b).area() > 0.5
}

/// Whether `a` and `b` lie at the same place, within `tol` px on every edge.
fn same_place(a: egui::Rect, b: egui::Rect, tol: f32) -> bool {
    (a.min - b.min).length() <= tol && (a.max - b.max).length() <= tol
}

#[test]
fn rev18_a_fresh_view_has_no_header_and_each_control_keeps_its_place_when_a_panel_opens() {
    // (width, tier, Design shown in a fresh view)
    for (w, tier, design) in [
        (1280.0, crate::ui::Tier::Wide, true),
        (1024.0, crate::ui::Tier::Wide, true),
        (800.0, crate::ui::Tier::Docked, false),
        (600.0, crate::ui::Tier::Docked, false),
        (599.0, crate::ui::Tier::Sheet, false),
        (320.0, crate::ui::Tier::Sheet, false),
    ] {
        let mut s = fresh(w, H);
        let a = s.ui.narrow.shown().unwrap();
        assert_eq!(a.tier, tier, "{w}");
        assert!(!a.layers, "{w}: Layers starts closed");
        assert_eq!(a.design, design, "{w}: Design");
        let c = s.canvas_rect();
        // No header: the canvas (and any open panel) starts at the surface's top.
        assert_eq!(c.min.y, 0.0, "{w}: no full-width header: {c:?}");
        assert!(c.min.x < 1.0, "{w}: no left panel: {c:?}");
        if design {
            assert!(
                c.max.x <= w as f32 - 220.0,
                "{w}: the quiet inspector: {c:?}"
            );
        } else {
            assert!(c.width() >= w as f32 - 1.0, "{w}: canvas only: {c:?}");
        }
        // The closed left panel's row floats at the canvas's top left (home first); the right
        // toggle is in the open inspector's top row, or floats at the canvas's top right.
        let home = s.ui.chrome.home.unwrap();
        let layers = s.ui.chrome.layers.unwrap();
        let design_toggle = s.ui.chrome.design.unwrap();
        let float = s.ui.chrome.left_float.expect("the left row floats");
        assert!(
            float.contains_rect(home) && float.contains_rect(layers),
            "{w}"
        );
        assert!(home.min.x < 12.0 && home.max.y <= 40.0, "{w}: {home:?}");
        assert!(
            design_toggle.max.x > w as f32 - 20.0 && design_toggle.max.y <= 40.0,
            "{w}: {design_toggle:?}"
        );
        assert_eq!(s.ui.chrome.right_float.is_some(), !design, "{w}");
        // The two toggles are one mirrored pair: the same size, on one line.
        assert_eq!(layers.size(), design_toggle.size(), "{w}");
        assert!(
            (layers.center().y - design_toggle.center().y).abs() < 0.5,
            "{w}"
        );
        // Neither floating row covers the rail.
        let rail = s.ui.rail.rects.rail.expect("the rail");
        assert!(!overlap(float, rail), "{w}: {float:?} {rail:?}");
        // Opening Layers puts its panel at the left, full height; home and the toggle keep the
        // places they had while floating, and the floating row is gone.
        toggle(&mut s, "Layers", 1000.0);
        assert!(s.ui.narrow.shown().unwrap().layers, "{w}");
        if tier != crate::ui::Tier::Sheet {
            let (h2, l2) = (s.ui.chrome.home.unwrap(), s.ui.chrome.layers.unwrap());
            assert!(
                same_place(h2, home, 1.5),
                "{w}: home moved {home:?} → {h2:?}"
            );
            assert!(
                same_place(l2, layers, 1.5),
                "{w}: toggle moved {layers:?} → {l2:?}"
            );
            assert_eq!(s.canvas_rect().min.y, 0.0, "{w}: still no header");
        }
        assert!(
            s.ui.chrome.left_float.is_none(),
            "{w}: nothing floats on the left"
        );
        assert!(paints(&mut s, "Papers"), "{w}: the pages and layers panel");
        assert!(!s.dirty(), "{w}: arranging changes nothing");
    }
}

#[test]
fn rev18_the_right_toggle_keeps_its_place_and_both_panels_reach_the_top() {
    // Wide, both panels open: each runs the surface's full height with its own top row; the
    // right toggle sits at the inspector's top right, where it floated while it was closed.
    let mut s = fresh(W, H);
    s.run(Action::TogglePanels);
    for _ in 0..4 {
        step(&mut s);
    }
    let floating = s.ui.chrome.design.expect("the floating right toggle");
    assert!(s.ui.chrome.right_float.unwrap().contains_rect(floating));
    s.run(Action::TogglePanels);
    for _ in 0..4 {
        step(&mut s);
    }
    toggle(&mut s, "Layers", 1000.0);
    let a = s.ui.narrow.shown().unwrap();
    assert!(a.layers && a.design, "{a:?}");
    let docked = s.ui.chrome.design.unwrap();
    assert!(
        same_place(docked, floating, 1.5),
        "{floating:?} → {docked:?}"
    );
    let c = s.canvas_rect();
    assert!(docked.min.x >= c.max.x && s.ui.chrome.right_float.is_none());
    let (home, layers) = (s.ui.chrome.home.unwrap(), s.ui.chrome.layers.unwrap());
    assert!(layers.max.x <= c.min.x && home.max.y <= 40.0);
    assert!(
        (home.center().y - docked.center().y).abs() < 0.5,
        "one top line"
    );
    assert_eq!((c.min.y, c.max.y), (0.0, H as f32), "{c:?}");
    assert!(!s.dirty());
}

#[test]
fn rev2_collapse_panels_hides_both_and_restore_brings_them_back_and_hide_ui_stays_separate() {
    let mut s = state();
    let both = s.canvas_rect();
    assert!(
        both.min.x >= 180.0 && both.max.x <= W as f32 - 220.0,
        "{both:?}"
    );
    let before = page_to_json(&s.ed.page_data(0));
    // Collapse through the All Projects menu, as a person finds it: View ▸ Collapse.
    open_menu(&mut s, 100.0);
    // With nothing selected the inspector (right of the canvas) paints a View section title
    // too; the menu's View is the one in front, left of the inspector.
    let right = s.canvas_rect().max.x;
    let views: Vec<egui::Rect> = painted_text(&mut s)
        .into_iter()
        .filter(|(t, _)| t.trim() == "View")
        .map(|(_, r)| r)
        .collect();
    assert!(
        views.len() >= 2 && views.iter().any(|r| r.min.x >= right),
        "the menu's and the inspector's View: {views:?}"
    );
    let view = click_front_label(&mut s, "View", 200.0);
    assert!(
        view.max.x <= right,
        "the menu's View, not the inspector's: {view:?}"
    );
    assert!(
        paints(&mut s, "Collapse / Restore Panels"),
        "View's submenu is open"
    );
    click_front_label(&mut s, "Collapse / Restore Panels", 300.0);
    for _ in 0..4 {
        step(&mut s);
    }
    assert!(s.ui.panels.collapsed());
    let a = s.ui.narrow.shown().unwrap();
    assert!(!a.layers && !a.design, "both hidden");
    let c = s.canvas_rect();
    assert!(c.min.x < 1.0 && c.max.x >= W as f32 - 1.0, "{c:?}");
    assert!(
        s.ui.chrome.home.is_some() && s.ui.rail.rects.rail.is_some() && s.ui.chrome.zoom.is_some(),
        "the way home, the rail and the zoom stay"
    );
    // Collapsed, both panels' top rows float over the canvas: the toggles and the identity
    // stay reachable without a header.
    assert!(s.ui.chrome.left_float.is_some() && s.ui.chrome.right_float.is_some());
    assert!(s.ui.chrome.identity.is_some() && s.ui.chrome.design.is_some());
    // Restore (the palette's and menu's same action): the same two panels again.
    s.run(Action::TogglePanels);
    for _ in 0..4 {
        step(&mut s);
    }
    assert_eq!(s.canvas_rect(), both, "the arrangement before the collapse");
    // Collapsed again, then one panel opened: the collapse ends, with that panel only.
    s.run(Action::TogglePanels);
    for _ in 0..4 {
        step(&mut s);
    }
    toggle(&mut s, "Design", 400.0);
    assert!(!s.ui.panels.collapsed());
    let a = s.ui.narrow.shown().unwrap();
    assert!(a.design && !a.layers, "{a:?}");
    // Hide UI is its own mode: everything goes, and comes back as it was.
    let design_only = s.canvas_rect();
    s.run(Action::ToggleUi);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.canvas_rect().min, egui::Pos2::ZERO);
    s.run(Action::ToggleUi);
    for _ in 0..4 {
        step(&mut s);
    }
    assert_eq!(s.canvas_rect(), design_only);
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
    assert!(!s.dirty(), "panel states are no drawing edits");
}

#[test]
fn rev2_collapse_waits_for_a_focused_number_draft_and_never_drops_it() {
    let mut s = state();
    let id = open_number_field(&mut s, "X  ");
    s.paste("-");
    step(&mut s);
    let before = s.ui.narrow.shown();
    s.run(Action::TogglePanels);
    for _ in 0..4 {
        step(&mut s);
    }
    assert_eq!(
        s.ui.narrow.shown(),
        before,
        "the arrangement waits for the field"
    );
    assert!(paints(&mut s, "-"), "the draft is still shown");
    assert!(s.field_has_keyboard() && s.unsaved_work() && !s.can_reload());
    press(&mut s, "Escape", "Escape");
    for _ in 0..4 {
        step(&mut s);
    }
    assert!(!s.field_has_keyboard());
    let a = s.ui.narrow.shown().unwrap();
    assert!(!a.layers && !a.design, "then the collapse applies");
    assert_eq!(
        axis(&s, id, "X  "),
        100.0,
        "cancelled as upstream cancels it"
    );
}

#[test]
fn rev20_zoom_sits_in_the_right_bar_beside_the_design_toggle_and_offers_fit() {
    // Revision 20 moved the zoom from the canvas's lower left to the right bar: the open Design
    // panel's top row, left of its toggle, or the floating bar at the canvas's top right; it
    // keeps its place against the toggle in both.
    let mut s = state();
    let c = s.canvas_rect();
    let zoom = s.ui.chrome.zoom.unwrap();
    let design = s.ui.chrome.design.unwrap();
    let rail = s.ui.rail.rects.rail.unwrap();
    assert!(zoom.min.x >= c.max.x, "in the Design panel's row: {zoom:?}");
    assert!(zoom.max.x <= design.min.x && zoom.max.y <= 40.0, "{zoom:?}");
    assert!((zoom.center().y - design.center().y).abs() < 0.5, "one row");
    let export = s.ui.chrome.export.expect("Export in the same bar");
    assert!(export.max.x <= zoom.min.x, "{export:?} {zoom:?}");
    assert!(!overlap(zoom, rail), "{zoom:?} {rail:?}");
    // The Design panel closed: the same group floats at the canvas's top right, the same
    // distance from the toggle.
    let gap = design.min.x - zoom.max.x;
    toggle(&mut s, "Design", 50.0);
    let (c2, floating) = (s.canvas_rect(), s.ui.chrome.zoom.unwrap());
    let design2 = s.ui.chrome.design.unwrap();
    assert!(c2.contains_rect(floating), "{floating:?} in {c2:?}");
    assert!(same_place(design2, design, 1.5), "{design:?} → {design2:?}");
    assert!(
        ((design2.min.x - floating.max.x) - gap).abs() < 1.5,
        "{floating:?}"
    );
    assert!(s.ui.chrome.right_float.unwrap().contains_rect(floating));
    toggle(&mut s, "Design", 60.0);
    let zoom = s.ui.chrome.zoom.unwrap();
    let z = s.ed.camera.zoom;
    let zoom_in = centre(s.ui.chrome.zoom_in.unwrap());
    click(&mut s, zoom_in, 100.0);
    step(&mut s);
    assert!(s.ed.camera.zoom > z, "zoom in");
    let zoom_out = centre(s.ui.chrome.zoom_out.unwrap());
    click(&mut s, zoom_out, 200.0);
    step(&mut s);
    assert!((s.ed.camera.zoom - z).abs() < 1e-9, "and out again");
    // The level opens the fit commands: zoomed far in, Zoom to Fit shows the rectangle again.
    let id = s.ed.doc.root[0];
    let world = s.ed.doc.world_rect(id).unwrap();
    s.ed.camera.zoom_about(v2(10.0, 10.0), 20.0);
    assert!(
        !s.ed
            .camera
            .visible_world(s.ed.viewport)
            .contains_rect(&world)
    );
    let level = centre(s.ui.chrome.zoom_level.unwrap());
    click(&mut s, level, 300.0);
    step(&mut s);
    step(&mut s);
    click_label(&mut s, "Zoom to Fit", 400.0);
    assert!(
        s.ed.camera
            .visible_world(s.ed.viewport)
            .contains_rect(&world),
        "fitted at {}",
        s.ed.camera.zoom
    );
    // A transient status never covers the zoom. Its area is new here: egui's first frame of a
    // new area is an invisible sizing pass (which asks for the next frame); the next one shows
    // it.
    s.ed.status = "Snapping off".into();
    step(&mut s);
    let text = painted_text(&mut s);
    let (_, r) = text
        .iter()
        .find(|(t, _)| t == "Snapping off")
        .expect("the status is shown");
    assert!(!overlap(*r, zoom), "{r:?} {zoom:?}");
    assert!(c.contains_rect(*r), "on the canvas: {r:?}");
    assert!(!s.dirty(), "zooming is a view change");
}

#[test]
fn rev2_the_rail_and_zoom_fit_every_canonical_width_and_short_height_without_overlap() {
    for (w, h) in [
        (320.0, TALL),
        (599.0, TALL),
        (600.0, TALL),
        (800.0, TALL),
        (1024.0, TALL),
        (1280.0, TALL),
        (1280.0, 380.0),
        (1280.0, 270.0),
        (320.0, 380.0),
        (320.0, 270.0),
    ] {
        let mut s = fresh(w, h);
        let c = s.canvas_rect();
        let rail = s.ui.rail.rects.rail.expect("the rail");
        let zoom = s.ui.chrome.zoom.expect("the zoom");
        assert!(
            c.contains_rect(rail) || c.height() < rail.height(),
            "{w}×{h}: {rail:?}"
        );
        assert!(!overlap(rail, zoom), "{w}×{h}: {rail:?} {zoom:?}");
        // Revision 20: the zoom is in the right bar, the open Design panel's top row or the
        // floating bar at the canvas's top right; either way inside the surface, at its top.
        assert!(
            zoom.max.x <= w as f32 && zoom.min.y >= 0.0 && zoom.max.y <= 40.0,
            "{w}×{h}: {zoom:?}"
        );
        // A fresh view keeps Layers closed: its row floats at the canvas's top left, and the
        // rail keeps to the room under it (the canvas itself starts at the top).
        let float = s.ui.chrome.left_float.expect("the left row floats");
        assert!(
            !overlap(rail, float) && !overlap(zoom, float),
            "{w}×{h}: {float:?}"
        );
        if let Some(right) = s.ui.chrome.right_float {
            assert!(c.contains_rect(zoom), "{w}×{h}: {zoom:?} in {c:?}");
            assert!(right.contains_rect(zoom), "{w}×{h}: in the floating bar");
            assert!(
                !overlap(rail, right) && !overlap(float, right),
                "{w}×{h}: {right:?}"
            );
        } else {
            assert!(zoom.min.x >= c.max.x, "{w}×{h}: in the Design panel's row");
        }
        let room_top = float.max.y;
        if c.max.y - room_top < crate::ui::rail::SHORT_H {
            assert!(
                (rail.min.y - room_top - crate::ui::rail::INSET).abs() < 0.5,
                "{w}×{h}: a short room anchors the rail at its top"
            );
        }
        // The largest family's flyout fits inside the canvas, beside the rail where it can.
        let (_, chevron) = s.ui.rail.rects.rows[3];
        click(&mut s, centre(chevron.unwrap()), 100.0);
        step(&mut s);
        step(&mut s);
        let fly = s.ui.rail.rects.flyout.expect("the Shapes flyout");
        assert!(
            fly.min.x >= c.min.x && fly.max.x <= c.max.x + 0.5,
            "{w}×{h}: {fly:?} {c:?}"
        );
        assert!(
            fly.min.y >= c.min.y && fly.max.y <= c.max.y + 0.5,
            "{w}×{h}: clamped (it scrolls when short): {fly:?} {c:?}"
        );
        if c.width() - (rail.max.x - c.min.x) - 4.0 - crate::ui::rail::INSET
            >= crate::ui::rail::FLYOUT_MIN
        {
            assert!(
                fly.min.x >= rail.max.x,
                "{w}×{h}: opens inward beside the rail"
            );
        }
        press(&mut s, "Escape", "Escape");
        step(&mut s);
    }
}

// ---- revision 2, slice 2: one rail, all sixteen tools (seven families since revision 20,
// which gives Hand its own button; Shapes is family 3, Text 5)

#[test]
fn rev2_the_rail_reaches_all_sixteen_tools_and_each_family_remembers_its_member() {
    let mut s = fresh(W, H);
    assert_eq!(s.ui.rail.rects.rows.len(), 7, "seven families");
    let reached = reach_every_tool(&mut s, 1000.0);
    assert_eq!(reached.len(), 16, "{reached:?}");
    assert_eq!(s.ed.doc.nodes.len(), 1, "probing the tools drew nothing");
    // A shortcut makes its tool the family's member: S is Sticky note, in Text.
    press(&mut s, "Escape", "Escape");
    assert!(s.key_down("s", "KeyS", false));
    step(&mut s);
    assert_eq!(s.ed.tool, Tool::Sticky);
    assert_eq!(s.ui.rail.members[5], Tool::Sticky);
    // Escape outside a flyout is back to Select; the Text family keeps Sticky note.
    press(&mut s, "Escape", "Escape");
    step(&mut s);
    assert_eq!(s.ed.tool, Tool::Select);
    assert_eq!(s.ui.rail.members[5], Tool::Sticky);
    let (text_main, _) = s.ui.rail.rects.rows[5];
    click(&mut s, centre(text_main), 9000.0);
    step(&mut s);
    assert_eq!(
        s.ed.tool,
        Tool::Sticky,
        "the family's main button runs its member"
    );
    // On the canonical short canvas too (the largest flyout still fits there), narrow and
    // wide: every row is painted whole and a click on it chooses its tool.
    for (w, h) in [(NARROW, 270.0), (W, 270.0)] {
        let mut s = fresh(w, h);
        let reached = reach_every_tool(&mut s, 1000.0);
        assert_eq!(reached.len(), 16, "{w}×{h}: {reached:?}");
        assert_eq!(s.ed.doc.nodes.len(), 1, "{w}×{h}: probing drew nothing");
    }
}

#[test]
fn rev2_a_focused_flyout_row_is_chosen_with_enter_and_the_key_never_reaches_the_editor() {
    // A text layer selected: an Enter the editor got would start editing its text.
    let mut doc = Document::default();
    let label = doc.alloc_id();
    let kind = NodeKind::Text {
        text: "Label".into(),
        font_size: 16.0,
        auto_width: true,
        align: crate::model::TextAlign::Left,
    };
    let r = crate::geom::Rect::new(100.0, 100.0, 60.0, 20.0);
    doc.nodes.insert(label, Node::new(label, kind, r));
    doc.root.push(label);
    let p = PageData {
        id: "p".into(),
        name: "Page 1".into(),
        doc,
        camera: Some(Default::default()),
    };
    let mut s = WebState::open(&canvas(&[p], "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    click_at(&mut s, 110.0, 110.0, 0.0);
    step(&mut s);
    assert_eq!(s.ed.selection, vec![label]);
    assert!(s.ed.text_edit.is_none());
    // The Shapes flyout, opened with its chevron; then egui's own keyboard navigation (Tab)
    // until its Polygon row has the focus.
    let (_, chevron) = s.ui.rail.rects.rows[3];
    click(&mut s, centre(chevron.unwrap()), 100.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ui.rail.flyout, Some(3));
    let mut tabs = 0;
    while s.ui.rail.rects.focus.map(|(_, t)| t) != Some(Some(Tool::Polygon)) {
        assert!(tabs < 300, "Tab reaches the Polygon row");
        s.key_down("Tab", "Tab", false);
        s.key_up("Tab", "Tab");
        step(&mut s);
        step(&mut s);
        tabs += 1;
    }
    assert_eq!(s.ui.rail.flyout, Some(3), "Tab keeps the flyout open");
    assert_eq!(s.ed.selection, vec![label], "Tab cycled the one layer");
    assert!(s.ed.text_edit.is_none());
    // Enter chooses the focused row; the editor never sees it.
    assert!(s.key_down("Enter", "Enter", false), "the row's key");
    s.key_up("Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.tool, Tool::Polygon, "Enter chose the focused row");
    assert_eq!(s.ui.rail.flyout, None, "and closed the flyout");
    assert_eq!(s.ui.rail.members[3], Tool::Polygon, "its member now");
    assert!(s.ed.text_edit.is_none(), "no text editing from the Enter");
    // Control: the text layer selected again from the canvas (a click there also takes egui's
    // focus away), Enter is the editor's: it edits the text.
    s.run(Action::Tool(Tool::Select));
    click_at(&mut s, 700.0, 500.0, 5000.0);
    step(&mut s);
    click_at(&mut s, 110.0, 110.0, 6000.0);
    step(&mut s);
    assert_eq!(s.ed.selection, vec![label]);
    assert!(s.ed.text_edit.is_none());
    assert!(s.key_down("Enter", "Enter", false));
    assert!(s.ed.text_edit.is_some(), "the editor's Enter edits it");
}

#[test]
fn rev2_escape_or_an_outside_press_closes_a_flyout_and_never_reaches_the_canvas() {
    let mut s = fresh(W, H);
    assert!(s.key_down("r", "KeyR", false));
    step(&mut s);
    assert_eq!(s.ed.tool, Tool::Rect);
    let open = |s: &mut WebState, t: f64| {
        let (_, chevron) = s.ui.rail.rects.rows[3];
        click(s, centre(chevron.unwrap()), t);
        step(s);
        step(s);
        assert_eq!(s.ui.rail.flyout, Some(3));
    };
    // Escape closes the flyout only: the tool stays (it isn't the editor's Escape).
    open(&mut s, 100.0);
    assert!(s.key_down("Escape", "Escape", false), "taken");
    s.key_up("Escape", "Escape");
    step(&mut s);
    assert_eq!(s.ui.rail.flyout, None);
    assert_eq!(s.ed.tool, Tool::Rect);
    // A press on the canvas closes it and draws nothing, though the Rectangle tool is active.
    open(&mut s, 200.0);
    let nodes = s.ed.doc.nodes.len();
    let (from, to) = (at(&s, 600.0, 400.0), at(&s, 680.0, 460.0));
    drag(&mut s, from, to, 300.0);
    step(&mut s);
    assert_eq!(s.ui.rail.flyout, None);
    assert_eq!(
        s.ed.doc.nodes.len(),
        nodes,
        "no click-through to the canvas"
    );
    // Control: the next drag draws.
    drag(&mut s, from, to, 400.0);
    assert_eq!(s.ed.doc.nodes.len(), nodes + 1);
}

#[test]
fn rev2_a_focused_field_keeps_tool_keys_and_the_families_keep_their_members() {
    let mut s = state();
    let _ = open_number_field(&mut s, "X  ");
    let members = s.ui.rail.members;
    for (k, code) in [("o", "KeyO"), ("e", "KeyE"), (".", "Period"), ("s", "KeyS")] {
        assert!(s.key_down(k, code, false), "{k}: the field's");
        s.key_up(k, code);
        step(&mut s);
    }
    assert_eq!(
        s.ed.tool,
        Tool::Select,
        "no tool shortcut reached the editor"
    );
    assert_eq!(
        s.ui.rail.members, members,
        "and no family changed its member"
    );
    press(&mut s, "Escape", "Escape");
}

// ---- revision 2, slice 3: the quiet contextual inspector and honest mixed values

/// Painted text with each number field read as a person reads it: upstream's `DragValue`
/// paints its prefix ("W  ") and its value ("200") as two texts, the value right after the
/// prefix on the same row; here they are one text ("W  200") with their joint bounds. A
/// Mixed placeholder ("W  Mixed") is one text already.
fn fields_read(text: Vec<(String, egui::Rect)>) -> Vec<(String, egui::Rect)> {
    let mut out: Vec<(String, egui::Rect)> = Vec::new();
    for (t, r) in text {
        if let Some((prefix, pr)) = out.last_mut()
            && prefix.ends_with("  ")
            && (r.min.x - pr.max.x).abs() < 1.0
            && (r.center().y - pr.center().y).abs() < 2.0
        {
            prefix.push_str(&t);
            *pr = pr.union(r);
            continue;
        }
        out.push((t, r));
    }
    out
}

/// Painted text in the inspector (right of the canvas), number fields read whole
/// ([`fields_read`]).
fn inspector_text(s: &mut WebState) -> Vec<String> {
    let right = s.canvas_rect().max.x;
    fields_read(painted_text(s))
        .into_iter()
        .filter(|(_, r)| r.min.x >= right)
        .map(|(t, _)| t)
        .collect()
}

#[test]
fn rev2_an_empty_inspector_is_quiet_and_a_creation_tool_shows_only_its_own_defaults() {
    let mut s = state();
    let text = inspector_text(&mut s);
    // The shown Paper's context (revision 18), not a generic hint: what it holds and the view's
    // grid. Revision 20: its name and rename live in the Papers list only, not repeated here.
    for wanted in ["Paper", "1 layer", "View", "Grid"] {
        assert!(text.iter().any(|t| t == wanted), "{wanted}: {text:?}");
    }
    for repeated in ["Page 1", "Rename Paper…"] {
        assert!(!text.iter().any(|t| t == repeated), "{repeated}: {text:?}");
    }
    assert!(
        !text.iter().any(|t| t.starts_with("Select a layer")),
        "{text:?}"
    );
    for absent in [
        "For new shapes",
        "For new arrows",
        "For new sticky notes",
        "For new pencil strokes",
        "Width",
        "Smoothing",
        "Transparent background",
    ] {
        assert!(!text.iter().any(|t| t == absent), "{absent}: {text:?}");
    }
    assert!(
        text.iter().any(|t| t == "Export"),
        "export stays reachable, folded"
    );
    // (key, code, shift, wanted, unwanted)
    for (key, code, shift, wanted, unwanted) in [
        ("b", "KeyB", false, "For new pencil strokes", "Route"),
        ("L", "KeyL", true, "For new arrows", "Smoothing"),
        ("s", "KeyS", false, "For new sticky notes", "Route"),
        ("r", "KeyR", false, "For new shapes", "Width"),
    ] {
        s.modifiers(shift, false, false, false);
        assert!(s.key_down(key, code, false), "{key}");
        s.modifiers(false, false, false, false);
        step(&mut s);
        step(&mut s);
        let text = inspector_text(&mut s);
        assert!(text.iter().any(|t| t == wanted), "{wanted}: {text:?}");
        assert!(!text.iter().any(|t| t == unwanted), "{unwanted}: {text:?}");
    }
    assert!(!s.dirty(), "looking at defaults changes nothing");
}

/// Two rectangles with different positions, heights and fills and the same width: A at
/// (100, 100) 200 × 100, B at (400, 300) 200 × 50. Both selected (Ctrl+A), laid out.
fn two_rects() -> (WebState, crate::model::NodeId, crate::model::NodeId) {
    let mut doc = Document::default();
    let a = doc.alloc_id();
    let mut na = Node::new(
        a,
        NodeKind::Rect,
        crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0),
    );
    na.style = crate::model::Style::with_fill(Color::rgb(0xe6, 0x4a, 0x3b));
    doc.nodes.insert(a, na);
    let b = doc.alloc_id();
    let mut nb = Node::new(
        b,
        NodeKind::Rect,
        crate::geom::Rect::new(400.0, 300.0, 200.0, 50.0),
    );
    nb.style = crate::model::Style::with_fill(Color::rgb(0x2f, 0x6f, 0xf3));
    doc.nodes.insert(b, nb);
    doc.root = vec![a, b];
    let page = PageData {
        id: "p".into(),
        name: "Page 1".into(),
        doc,
        camera: Some(Default::default()),
    };
    let mut s = WebState::open(&canvas(&[page], "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    s.modifiers(false, true, false, false);
    assert!(s.key_down("a", "KeyA", false));
    s.modifiers(false, false, false, false);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.selection.len(), 2);
    (s, a, b)
}

#[test]
fn rev2_a_mixed_selection_never_shows_the_first_layer_s_values_as_shared() {
    let (mut s, _, _) = two_rects();
    let text = inspector_text(&mut s);
    assert!(text.iter().any(|t| t == "X  Mixed"), "{text:?}");
    assert!(text.iter().any(|t| t == "H  Mixed"), "{text:?}");
    assert!(
        !text.iter().any(|t| t == "X  100" || t == "H  100"),
        "the first layer's values aren't shown as everyone's: {text:?}"
    );
    assert!(
        text.iter().any(|t| t.starts_with("W  200")),
        "the shared width is shown: {text:?}"
    );
    assert!(
        text.iter().any(|t| t.contains("different fills")),
        "the fills differ: {text:?}"
    );
    assert!(
        !text.iter().any(|t| t == "#e64a3b"),
        "not the first fill as if shared"
    );
}

#[test]
fn rev19_paired_inspector_fields_fill_the_row_in_two_equal_columns() {
    // Revision 19: X and Y (and W and H) are one row of two equal fields filling the
    // inspector's width, Mixed drawn in its own field's place, the columns lined up row to row.
    // Before, both were 92 px at the row's left, which left the right third of it empty.
    let (mut s, _, _) = two_rects();
    let c = s.canvas_rect();
    let inspector: Vec<(String, egui::Rect)> = fields_read(painted_text(&mut s))
        .into_iter()
        .filter(|(_, r)| r.min.x >= c.max.x)
        .collect();
    let at = |wanted: &str| {
        let found = inspector.iter().find(|(t, _)| t.starts_with(wanted));
        let (_, r) = found.unwrap_or_else(|| panic!("{wanted:?}"));
        *r
    };
    let (x, y, w, h) = (at("X  "), at("Y  "), at("W  "), at("H  "));
    // The inspector's content runs from its 14 px inset to the surface's.
    let mid = (c.max.x + W as f32) / 2.0;
    for (left, right) in [(x, y), (w, h)] {
        let dy = (left.center().y - right.center().y).abs();
        assert!(dy < 1.0, "one row: {left:?} {right:?}");
        assert!(
            left.max.x < mid && right.min.x > mid,
            "two columns: {left:?} {right:?}"
        );
        let (l, r) = (mid - left.center().x, right.center().x - mid);
        assert!((l - r).abs() < 3.0, "equal columns: {l} {r}");
        assert!(right.max.x <= W as f32 - 13.5, "inset: {right:?}");
    }
    assert!((x.center().x - w.center().x).abs() < 1.5, "{x:?} {w:?}");
    assert!((y.center().x - h.center().x).abs() < 1.5, "{y:?} {h:?}");
    assert!(!s.dirty(), "looking changes nothing");
}

#[test]
fn rev2_a_shared_value_edits_every_selected_layer_as_one_undoable_change() {
    let (mut s, a, b) = two_rects();
    let right = s.canvas_rect().max.x;
    let text = fields_read(painted_text(&mut s));
    let (_, w) = text
        .iter()
        .find(|(t, r)| t.starts_with("W  200") && r.min.x >= right)
        .unwrap_or_else(|| panic!("the shared W field: {text:?}"));
    click(&mut s, centre(*w), 1000.0);
    step(&mut s);
    step(&mut s);
    assert!(s.field_has_keyboard(), "the W field is in its text mode");
    s.modifiers(false, true, false, false);
    press(&mut s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(&mut s);
    // Typed key by key: each keystroke is a change of the same field.
    for (k, code) in [("2", "Digit2"), ("5", "Digit5"), ("0", "Digit0")] {
        press(&mut s, k, code);
        step(&mut s);
    }
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    let width = |s: &WebState, id| s.ed.doc.get(id).unwrap().rect.w;
    assert_eq!((width(&s, a), width(&s, b)), (250.0, 250.0), "both layers");
    assert_eq!(
        s.ed.doc.get(b).unwrap().rect.h,
        50.0,
        "only the edited value"
    );
    assert!(s.dirty());
    s.run(Action::Undo);
    assert_eq!(
        (width(&s, a), width(&s, b)),
        (200.0, 200.0),
        "one coalesced undo step for the whole edit"
    );
}

#[test]
fn rev2_different_kinds_show_only_what_they_share_and_sketch_and_export_are_folded() {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let mut p = page("p", "Page 1", rect);
    let t = p.doc.alloc_id();
    p.doc.nodes.insert(
        t,
        Node::new(
            t,
            NodeKind::Text {
                text: "Label".into(),
                font_size: 16.0,
                auto_width: true,
                align: crate::model::TextAlign::Left,
            },
            crate::geom::Rect::new(400.0, 300.0, 60.0, 20.0),
        ),
    );
    p.doc.root.push(t);
    let mut s = WebState::open(&canvas(&[p], "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    // One rectangle: its sections, with Sketch and Export folded.
    click_at(&mut s, 150.0, 150.0, 0.0);
    step(&mut s);
    step(&mut s);
    let text = inspector_text(&mut s);
    assert!(text.iter().any(|t| t == "Sketch"), "{text:?}");
    assert!(
        !text
            .iter()
            .any(|t| t == "Hand-drawn style" || t == "Sloppiness"),
        "folded by default: {text:?}"
    );
    assert!(text.iter().any(|t| t == "Export"));
    assert!(
        !text.iter().any(|t| t == "Transparent background"),
        "folded"
    );
    // Rectangle and text together: no typography, no fill or stroke lists, and a word on why.
    s.modifiers(false, true, false, false);
    assert!(s.key_down("a", "KeyA", false));
    s.modifiers(false, false, false, false);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.selection.len(), 2);
    let text = inspector_text(&mut s);
    assert!(
        text.iter()
            .any(|t| t.starts_with("Only what every selected layer has")),
        "{text:?}"
    );
    for absent in ["Text", "Fill", "Stroke", "Sketch"] {
        assert!(!text.iter().any(|t| t == absent), "{absent}: {text:?}");
    }
    assert!(
        text.iter().any(|t| t == "Layer"),
        "shared layer properties stay"
    );
    assert!(!s.dirty());
}

// ---- revision 2, slice 4: the canvas background (a view preference) and the canvas's own
// menu. They drive the real upstream UI through this host's input, as above.

/// A secondary click (pressed and released in place, no drag) at `p`, then two frames (the menu
/// is a new egui area: its first frame is egui's invisible sizing pass).
fn secondary_click(s: &mut WebState, (x, y): (f64, f64), t: f64) {
    s.pointer_move(x, y, t);
    s.pointer_down(x, y, Button::Secondary, t);
    s.pointer_up(x, y, Button::Secondary, t + 10.0);
    step(s);
    step(s);
}

/// The requests the page has to answer now.
fn requests(s: &mut WebState) -> Vec<HostRequest> {
    std::iter::from_fn(|| s.take_request()).collect()
}

#[test]
fn rev2_the_canvas_menu_switches_the_background_as_a_view_preference_only() {
    let mut s = state();
    assert_eq!(s.ui.background, scene::Background::None, "a fresh view");
    assert_eq!(s.background(), "none");
    let authored = page_to_json(&s.ed.page_data(0));
    let svg = crate::io::svg::export(&s.ed.doc, None, None);
    let snapping = s.ed.snapping;
    // Blank canvas (the rectangle is at world 100..300 × 100..200).
    let blank = at(&s, 600.0, 450.0);
    secondary_click(&mut s, blank, 100.0);
    assert!(s.ui.canvas_menu.is_some(), "the canvas's own menu opened");
    assert!(s.ed.selection.is_empty(), "nothing selected");
    assert_eq!(s.ed.doc.nodes.len(), 1, "nothing drawn");
    // The menu's own labels, inside the canvas (the inspector's View row says Grid too).
    let c = s.canvas_rect();
    let in_menu = |s: &mut WebState, label: &str| {
        painted_text(s)
            .iter()
            .any(|(t, r)| t.trim() == label && c.contains_rect(*r))
    };
    for label in [
        "Background",
        "None",
        "Dots",
        "Grid",
        "Zoom to Fit",
        "Collapse Panels",
    ] {
        assert!(in_menu(&mut s, label), "{label}");
    }
    // Revision 18 names the line pattern Grid everywhere; its stored name stays `lines`, so a
    // kept preference from before still loads.
    assert!(!in_menu(&mut s, "Lines"), "the old label is gone");
    click_label_in(&mut s, "Grid", |r| c.contains_rect(r), 200.0);
    assert_eq!(s.ui.background, scene::Background::Lines);
    assert_eq!(scene::Background::Lines.label(), "Grid");
    assert!(s.ui.canvas_menu.is_none(), "a choice closes it");
    assert_eq!(requests(&mut s), [HostRequest::Background]);
    assert_eq!(s.background(), "lines");
    assert!(s.set_background("lines"), "the stored name is still taken");
    assert!(
        requests(&mut s).is_empty(),
        "a kept preference isn't reported back"
    );
    // Presentation only: the document, its save state, undo, snapping and exports are untouched.
    assert_eq!(page_to_json(&s.ed.page_data(0)), authored);
    assert_eq!(crate::io::svg::export(&s.ed.doc, None, None), svg);
    assert_eq!(s.ed.snapping, snapping);
    assert!(!s.ed.is_dirty(), "no undo step");
    assert!(!s.dirty() && s.can_reload() && s.save_snapshot().is_none());
    // The grid shortcut (Ctrl/⌘+Shift+', the workspace's): None against the last pattern.
    let grid = |s: &mut WebState| {
        s.modifiers(true, true, false, false);
        assert!(s.key_down("\"", "Quote", false), "taken");
        s.key_up("\"", "Quote");
        s.modifiers(false, false, false, false);
        step(s);
        step(s);
    };
    grid(&mut s);
    assert_eq!(s.ui.background, scene::Background::None);
    grid(&mut s);
    assert_eq!(s.ui.background, scene::Background::Lines, "not Dots");
    let both = [HostRequest::Background, HostRequest::Background];
    assert_eq!(requests(&mut s), both);
    // The palette offers the same patterns.
    open_palette(&mut s);
    s.paste("Background: Dots");
    step(&mut s);
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ui.background, scene::Background::Dots);
    assert_eq!(requests(&mut s), [HostRequest::Background]);
    // A kept preference from the page applies without being reported back; anything else is
    // refused and changes nothing.
    assert!(s.set_background("none"));
    step(&mut s);
    assert_eq!(s.ui.background, scene::Background::None);
    assert!(requests(&mut s).is_empty(), "not reported back");
    assert!(!s.set_background("grid"));
    assert_eq!(s.background(), "none");
    assert_eq!(page_to_json(&s.ed.page_data(0)), authored);
    assert!(!s.dirty());
}

#[test]
fn rev2_the_canvas_menu_opens_only_on_a_still_secondary_click_on_blank_canvas_and_never_draws() {
    let mut s = state();
    s.run(Action::Tool(Tool::Rect));
    step(&mut s);
    let nodes = s.ed.doc.nodes.len();
    // On the rectangle: no menu (an object keeps its own behaviour).
    let on_rect = at(&s, 150.0, 150.0);
    secondary_click(&mut s, on_rect, 100.0);
    assert!(s.ui.canvas_menu.is_none(), "not over a node");
    // A secondary drag on blank canvas: no menu.
    let (x, y) = at(&s, 600.0, 450.0);
    s.pointer_down(x, y, Button::Secondary, 200.0);
    s.pointer_move(x + 60.0, y + 40.0, 210.0);
    s.pointer_up(x + 60.0, y + 40.0, Button::Secondary, 220.0);
    step(&mut s);
    step(&mut s);
    assert!(s.ui.canvas_menu.is_none(), "a drag isn't a click");
    // Over a panel: never the canvas's menu.
    let panel = (s.canvas_rect().max.x as f64 + 40.0, 400.0);
    secondary_click(&mut s, panel, 300.0);
    assert!(s.ui.canvas_menu.is_none(), "not over a panel");
    assert_eq!(s.ed.doc.nodes.len(), nodes, "none of it drew");
    // On blank canvas it opens; a press elsewhere on the canvas closes it and draws nothing,
    // though the Rectangle tool is active.
    let blank = (x, y);
    secondary_click(&mut s, blank, 400.0);
    assert!(s.ui.canvas_menu.is_some());
    let (from, to) = (at(&s, 300.0, 300.0), at(&s, 380.0, 360.0));
    drag(&mut s, from, to, 500.0);
    step(&mut s);
    assert!(s.ui.canvas_menu.is_none(), "an outside press closes it");
    assert_eq!(s.ed.doc.nodes.len(), nodes, "no click-through");
    // Escape closes it too, and is the menu's: the tool stays.
    secondary_click(&mut s, blank, 600.0);
    assert!(s.ui.canvas_menu.is_some());
    assert!(s.key_down("Escape", "Escape", false), "taken");
    s.key_up("Escape", "Escape");
    step(&mut s);
    assert!(s.ui.canvas_menu.is_none());
    assert_eq!(s.ed.tool, Tool::Rect, "not the editor's Escape");
    // Controls: the next drag draws, and the middle button still pans.
    drag(&mut s, from, to, 700.0);
    assert_eq!(s.ed.doc.nodes.len(), nodes + 1);
    let pan = s.ed.camera.pan;
    s.pointer_down(x, y, Button::Middle, 800.0);
    s.pointer_move(x + 50.0, y + 30.0, 810.0);
    s.pointer_up(x + 50.0, y + 30.0, Button::Middle, 820.0);
    step(&mut s);
    assert_ne!(s.ed.camera.pan, pan, "middle-button panning");
    assert!(s.ui.canvas_menu.is_none());
}

#[test]
fn rev2_an_open_canvas_menu_keeps_its_press_and_the_next_press_where_it_was_is_the_canvas_s() {
    let mut s = state();
    let blank = at(&s, 600.0, 450.0);
    let elsewhere = at(&s, 250.0, 450.0);
    let middle_drag = |s: &mut WebState, (x, y): (f64, f64), t: f64| {
        s.pointer_down(x, y, Button::Middle, t);
        s.pointer_move(x + 50.0, y + 30.0, t + 10.0);
        s.pointer_up(x + 50.0, y + 30.0, Button::Middle, t + 20.0);
    };
    secondary_click(&mut s, blank, 100.0);
    assert!(s.ui.canvas_menu.is_some());
    let pan = s.ed.camera.pan;
    // While it is open a press is the menu's: it closes it, and the canvas doesn't pan.
    middle_drag(&mut s, elsewhere, 200.0);
    step(&mut s);
    assert!(s.ui.canvas_menu.is_none(), "the outside press closed it");
    assert_eq!(s.ed.camera.pan, pan, "no pan from the menu's press");
    // The very next press, where the frame that closed the menu still drew it (egui counts it
    // visible for one more frame), is the canvas's.
    middle_drag(&mut s, blank, 300.0);
    step(&mut s);
    assert_ne!(s.ed.camera.pan, pan, "the canvas pans");
    assert_eq!(s.ed.doc.nodes.len(), 1, "nothing drawn");
}

// ---- revision 2, slice 6: the file's real name, renamed from its identity (revision 18: at
// the left panel's top) and saved with the canvas (the same canvas save as the pages), and the
// shown page's own actions

/// Where text starting with `wanted` is painted in the Canvas identity (its centre, CSS px).
fn in_header(s: &mut WebState, wanted: &str) -> (f64, f64) {
    let id = s.ui.chrome.identity.expect("the identity is laid out");
    let text = painted_text(s);
    let (_, r) = text
        .iter()
        .find(|(t, r)| t.trim().starts_with(wanted) && id.expand(0.5).contains_rect(*r))
        .unwrap_or_else(|| panic!("{wanted:?} isn't in the identity {id:?}: {text:?}"));
    centre(*r)
}

/// Click the identity's Canvas name (`name`) to open its rename field, select its text and
/// type `text`, laid out: a draft until it is finished.
fn rename_title(s: &mut WebState, name: &str, text: &str, t: f64) {
    let p = in_header(s, name);
    click(s, p, t);
    for _ in 0..3 {
        step(s);
    }
    assert!(s.ui.file_title_draft.is_some(), "the field is open");
    assert!(s.field_has_keyboard(), "with the keyboard");
    s.modifiers(false, true, false, false);
    press(s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(s);
    s.paste(text);
    step(s);
}

/// The file name a canvas save carries, if it renames the file.
fn saved_name(snap: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(snap).unwrap();
    v.get("name").and_then(|n| n.as_str()).map(str::to_string)
}

#[test]
fn rev2_the_title_renames_the_file_on_enter_or_a_click_away_and_escape_drops_it() {
    // An open field with nothing typed loses nothing.
    let mut s = state();
    assert!(s.ui.file_rename, "this host saves the name");
    let p = in_header(&mut s, "Canvas");
    click(&mut s, p, 50.0);
    for _ in 0..3 {
        step(&mut s);
    }
    assert!(s.field_has_keyboard());
    assert!(s.can_reload() && !s.unsaved_work(), "nothing to lose");
    for finish in ["Enter", "click", "Escape"] {
        let mut s = state();
        rename_title(&mut s, "Canvas", "  Launch plan ", 100.0);
        assert_eq!(s.ui.project_name, "Canvas", "{finish}: a draft");
        assert!(!s.dirty(), "{finish}: nothing to save yet");
        assert!(
            s.unsaved_work() && !s.can_reload(),
            "{finish}: the draft is guarded"
        );
        assert!(s.save_snapshot().is_none(), "{finish}");
        match finish {
            "Enter" => press(&mut s, "Enter", "Enter"),
            "click" => click_at(&mut s, 700.0, 500.0, 2000.0),
            _ => press(&mut s, "Escape", "Escape"),
        }
        step(&mut s);
        step(&mut s);
        assert!(s.ui.file_title_draft.is_none(), "{finish}: closed");
        assert!(!s.field_has_keyboard(), "{finish}");
        if finish == "Escape" {
            assert_eq!(s.ui.project_name, "Canvas", "dropped");
            assert!(!s.dirty() && !s.unsaved_work() && s.can_reload());
            continue;
        }
        assert_eq!(s.ui.project_name, "Launch plan", "{finish}: trimmed");
        // The header shows it (panics if not).
        in_header(&mut s, "Launch plan");
        assert!(s.dirty() && !s.can_reload(), "{finish}: to save");
        let snap = s.save_snapshot().expect("the rename is saved");
        let name = saved_name(&snap);
        assert_eq!(name.as_deref(), Some("Launch plan"), "{finish}");
        let (_, list, sent) = parse_save(&snap);
        assert_eq!(list.len(), 1, "{finish}: the page list as it is");
        assert!(sent.is_empty(), "{finish}: no page content changed");
        s.save_succeeded();
        assert!(!s.dirty() && s.can_reload(), "{finish}: saved");
        assert!(s.save_snapshot().is_none(), "{finish}: nothing more");
    }
}

#[test]
fn rev2_rename_and_content_share_one_save_and_an_older_answer_keeps_a_later_rename() {
    let mut s = state();
    // A drawing change and a rename, both unsaved: one save carries both.
    s.run(Action::Tool(Tool::Rect));
    let (from, to) = (at(&s, 400.0, 300.0), at(&s, 480.0, 360.0));
    drag(&mut s, from, to, 100.0);
    step(&mut s);
    rename_title(&mut s, "Canvas", "First", 1000.0);
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    let snap = s.save_snapshot().expect("one save");
    assert_eq!(saved_name(&snap).as_deref(), Some("First"));
    assert_eq!(sent_page(&snap).doc.nodes.len(), 2, "and the drawing");
    // Renamed again while that save is in flight.
    rename_title(&mut s, "First", "Second", 3000.0);
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert!(s.save_snapshot().is_none(), "one save at a time");
    s.save_succeeded();
    assert!(s.dirty(), "the later rename stays to save");
    let snap = s.save_snapshot().expect("the later rename");
    assert_eq!(saved_name(&snap).as_deref(), Some("Second"));
    assert!(parse_save(&snap).2.is_empty(), "the drawing is saved");
    // Refused (a conflict: the name was changed elsewhere meanwhile): everything is kept.
    s.save_failed();
    assert!(s.dirty() && !s.can_reload(), "the rename is kept");
    assert_eq!(s.ui.project_name, "Second");
    assert_eq!(s.ed.doc.nodes.len(), 2);
    // Only the explicit discard (the conflict banner's reload) takes the server's name.
    s.reload(&canvas_named(&pages_now(&s), "p", "Renamed elsewhere"))
        .unwrap();
    assert_eq!(s.ui.project_name, "Renamed elsewhere");
    assert!(!s.dirty() && s.can_reload());
}

#[test]
fn rev20_the_all_projects_menu_offers_the_shown_paper_s_own_actions() {
    // Revision 18's Paper line is gone (one Canvas title); its actions are in the All Projects
    // menu, run the same way.
    let mut s = state();
    open_menu(&mut s, 100.0);
    click_front_label(&mut s, "Duplicate Paper", 200.0);
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Page 1", "Page 1 copy"]);
    assert_eq!(s.ed.page_name(), "Page 1 copy");
    // Rename Paper… opens the shown page's own rename in the Papers list: the same field, the
    // same Enter, and the same page-list save.
    menu_rename_paper(&mut s, 300.0);
    for _ in 0..4 {
        step(&mut s);
    }
    assert_eq!(s.ui.pages.renaming.as_ref().map(|(i, _)| *i), Some(1));
    assert!(s.field_has_keyboard(), "its field has the keyboard");
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert!(s.ed.page_name().contains('q'), "{}", s.ed.page_name());
    assert_eq!(s.ed.pages[0].name, "Page 1", "only the shown page");
    let snap = s.save_snapshot().expect("the page list is saved");
    assert_eq!(saved_name(&snap), None, "not a file rename");
}

// ---- revision 2 corrections (R33): a panel edge's press, the typography the characters
// have, one undo step per field interaction, a short surface's status, the zoom's room and
// independent corners. Real input with time going on, as a browser delivers it.

/// `n` frames, 16 ms apart.
fn timed_frames(s: &mut WebState, n: usize) {
    for _ in 0..n {
        s.tick(s.now_ms + 16.0);
        step(s);
    }
}

/// A click at `p` a second after the last input (no double click), with its frames.
fn timed_click(s: &mut WebState, (x, y): (f64, f64)) {
    s.tick(s.now_ms + 1000.0);
    s.pointer_move(x, y, s.now_ms);
    s.pointer_down(x, y, Button::Primary, s.now_ms);
    timed_frames(s, 1);
    s.pointer_up(x, y, Button::Primary, s.now_ms);
    timed_frames(s, 4);
}

/// A primary drag from `from` to `to` in 20 moves, a frame after each, as a person drags.
fn timed_drag(s: &mut WebState, from: (f64, f64), to: (f64, f64)) {
    s.tick(s.now_ms + 1000.0);
    s.pointer_move(from.0, from.1, s.now_ms);
    timed_frames(s, 2);
    s.pointer_down(from.0, from.1, Button::Primary, s.now_ms);
    timed_frames(s, 1);
    for i in 1..=20 {
        let f = i as f64 / 20.0;
        let x = from.0 + (to.0 - from.0) * f;
        let y = from.1 + (to.1 - from.1) * f;
        s.pointer_move(x, y, s.now_ms);
        timed_frames(s, 1);
    }
    s.pointer_up(to.0, to.1, Button::Primary, s.now_ms);
    timed_frames(s, 5);
}

#[test]
fn rev2_a_panel_edge_drag_resizes_the_panel_and_never_moves_the_shape_behind_it() {
    // R33-F1: Layers and Design open at 1024 × 380, a rectangle reaching behind Design's edge
    // (its resize handle reaches into the canvas).
    let open = || {
        let rect = crate::geom::Rect::new(450.0, 60.0, 200.0, 100.0);
        let pages = [page("p", "Page 1", rect)];
        let mut s = WebState::open(&canvas(&pages, "p"), false, 1024.0, 380.0).unwrap();
        settle(&mut s, 0.0);
        let layers = s.ui.chrome.layers.unwrap();
        timed_click(&mut s, centre(layers));
        assert!(s.ui.narrow.shown().is_some_and(|a| a.layers && a.design));
        s
    };
    // Just inside the canvas, exactly on its edge, and just on the panel's side.
    for offset in [-1.0, 0.0, 1.0] {
        let mut s = open();
        let before = page_to_json(&s.ed.page_data(0));
        let edge = s.canvas_rect().max.x as f64;
        let from = (edge + offset, 150.0);
        timed_drag(&mut s, from, (from.0 - 100.0, 150.0));
        let after = s.canvas_rect().max.x as f64;
        assert!(after < edge - 70.0, "{offset}: resized to {after}");
        assert_eq!(
            page_to_json(&s.ed.page_data(0)),
            before,
            "{offset}: nothing behind the edge moved"
        );
        assert!(!s.dirty(), "{offset}: nothing to save");
    }
    // Control: a drag that starts on the rectangle, clear of the edge, moves it.
    let mut s = open();
    let id = s.ed.doc.root[0];
    let from = at(&s, 480.0, 110.0);
    timed_drag(&mut s, from, (from.0 - 50.0, from.1));
    let x = s.ed.doc.get(id).unwrap().rect.x;
    assert!(x < 420.0, "the canvas drag moved the rectangle: {x}");
    assert!(s.dirty());
}

/// Two text layers "Label" at base size 24, both selected; with `span`, the first one's
/// whole text is a bold 48 range and the second stays plain.
fn two_labels(span: bool) -> WebState {
    let mut doc = Document::default();
    for (i, (x, y)) in [(100.0, 100.0), (400.0, 300.0)].into_iter().enumerate() {
        let id = doc.alloc_id();
        let kind = NodeKind::Text {
            text: "Label".into(),
            font_size: 24.0,
            auto_width: true,
            align: crate::model::TextAlign::Left,
        };
        let mut n = Node::new(id, kind, crate::geom::Rect::new(x, y, 80.0, 30.0));
        if span && i == 0 {
            n.text_style.spans.push(crate::model::TextSpan {
                start: 0,
                end: 5,
                size: Some(48.0),
                bold: Some(true),
                ..Default::default()
            });
        }
        doc.nodes.insert(id, n);
        doc.root.push(id);
    }
    let page = PageData {
        id: "p".into(),
        name: "Page 1".into(),
        doc,
        camera: Some(Default::default()),
    };
    let mut s = WebState::open(&canvas(&[page], "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    s.modifiers(false, true, false, false);
    assert!(s.key_down("a", "KeyA", false));
    s.modifiers(false, false, false, false);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.selection.len(), 2);
    s
}

/// The Size field's value as the inspector shows it (the number painted after its "Size "
/// prefix); `None` when there is no number there (Mixed).
fn size_shown(s: &mut WebState) -> Option<f64> {
    let text = inspector_text(s);
    text.windows(2)
        .find(|p| p[0] == "Size ")
        .and_then(|p| p[1].parse().ok())
}

/// The font sizes of the page's layers, in order.
fn label_sizes(s: &WebState) -> Vec<f64> {
    let size = |id: &crate::model::NodeId| s.ed.doc.get(*id).unwrap().kind.font_size().unwrap();
    s.ed.doc.root.iter().map(size).collect()
}

#[test]
fn rev2_shared_typography_is_what_the_characters_have_ranges_included() {
    // R33-F2: equal base sizes, but the first layer's text is a 48 range.
    let mut s = two_labels(true);
    let drawn = |s: &WebState, i: usize| {
        let n = s.ed.doc.get(s.ed.doc.root[i]).unwrap();
        let base = n.kind.font_size().unwrap();
        n.text_style.char_style(0, base, Color::INK).size
    };
    assert_eq!(
        (drawn(&s, 0), drawn(&s, 1)),
        (48.0, 24.0),
        "the layout draws 48 and 24"
    );
    let text = inspector_text(&mut s);
    assert!(text.iter().any(|t| t == "Size Mixed"), "{text:?}");
    assert_eq!(size_shown(&mut s), None, "never a shared 24");
    assert!(
        text.iter().any(|t| t.starts_with("Size or colour differs")),
        "and why: {text:?}"
    );
    assert!(!s.dirty(), "looking changes nothing");
    // Control: plain texts of one size share it, as a value to edit.
    let mut s = two_labels(false);
    assert_eq!(size_shown(&mut s), Some(24.0));
    let text = inspector_text(&mut s);
    assert!(!text.iter().any(|t| t == "Size Mixed"), "{text:?}");
}

/// Click the inspector's Size field, select its text and type `keys`, then Enter.
fn type_in_size(s: &mut WebState, keys: &[(&str, &str)]) {
    let right = s.canvas_rect().max.x;
    let text = painted_text(s);
    let (_, r) = text
        .iter()
        .find(|(t, r)| t == "Size " && r.min.x >= right)
        .unwrap_or_else(|| panic!("the Size field: {text:?}"));
    timed_click(s, centre(*r));
    assert!(s.field_has_keyboard(), "the Size field is in its text mode");
    s.modifiers(false, true, false, false);
    press(s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    timed_frames(s, 1);
    for (k, code) in keys {
        press(s, k, code);
        timed_frames(s, 1);
    }
    press(s, "Enter", "Enter");
    timed_frames(s, 2);
}

#[test]
fn rev2_one_size_interaction_is_one_undo_step_and_a_later_one_is_its_own() {
    // R33-F3: typed key by key (each valid keystroke changes both layers).
    let mut s = two_labels(false);
    type_in_size(&mut s, &[("3", "Digit3"), ("0", "Digit0")]);
    assert_eq!(label_sizes(&s), [30.0, 30.0], "both layers");
    s.run(Action::Undo);
    assert_eq!(label_sizes(&s), [24.0, 24.0], "one undo, the whole of it");
    s.run(Action::Redo);
    assert_eq!(label_sizes(&s), [30.0, 30.0], "one redo");
    // A later interaction is a step of its own.
    type_in_size(&mut s, &[("4", "Digit4"), ("0", "Digit0")]);
    assert_eq!(label_sizes(&s), [40.0, 40.0]);
    s.run(Action::Undo);
    assert_eq!(label_sizes(&s), [30.0, 30.0], "only the later interaction");
    // Dragging the value is one step too.
    let mut s = two_labels(false);
    let right = s.canvas_rect().max.x;
    let text = painted_text(&mut s);
    let (_, r) = text
        .iter()
        .find(|(t, r)| t == "Size " && r.min.x >= right)
        .unwrap_or_else(|| panic!("the Size field: {text:?}"));
    let from = centre(*r);
    timed_drag(&mut s, from, (from.0 + 40.0, from.1));
    let dragged = label_sizes(&s);
    assert!(
        dragged[0] > 24.0 && dragged[0] == dragged[1],
        "both, by the drag: {dragged:?}"
    );
    s.run(Action::Undo);
    assert_eq!(label_sizes(&s), [24.0, 24.0], "one undo for the drag");
}

#[test]
fn rev2_a_status_on_a_short_surface_stays_clear_of_the_rail_and_the_zoom() {
    // R33-F4: the real refusal of Copy as PNG at 320 × 270, where the rail runs down the
    // canvas's left side and the zoom sits in the floating right bar at the top (revision 20).
    let mut s = fresh(320.0, 270.0);
    let before = page_to_json(&s.ed.page_data(0));
    s.run(Action::CopyAsPng);
    timed_frames(&mut s, 3);
    let canvas = s.canvas_rect();
    // The canvas now reaches the top (no header); the tools' room under the floating row is
    // what is short.
    let room_top = s.ui.chrome.left_float.unwrap().max.y;
    assert!(
        canvas.max.y - room_top < crate::ui::rail::SHORT_H,
        "{canvas:?}"
    );
    let rail = s.ui.rail.rects.rail.unwrap();
    let zoom = s.ui.chrome.zoom.unwrap();
    let text = painted_text(&mut s);
    let (_, r) = text
        .iter()
        .find(|(t, _)| t.starts_with("PNG export and Copy as PNG"))
        .unwrap_or_else(|| panic!("the refusal is shown: {text:?}"));
    assert!(!overlap(*r, rail), "clear of the rail: {r:?} {rail:?}");
    assert!(!overlap(*r, zoom), "clear of the zoom: {r:?} {zoom:?}");
    assert!(canvas.contains_rect(*r), "inside the canvas: {r:?}");
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
}

#[test]
fn rev2_the_zoom_stays_inside_a_canvas_narrowed_by_widened_panels() {
    // R33-F5: at 1024 × 270 both panels dragged to their widest (420). Revision 18 has no
    // 38 px header, so the same 232 px short canvas is now a 1024 × 232 surface.
    let mut s = fresh(1024.0, 232.0);
    let layers = s.ui.chrome.layers.unwrap();
    timed_click(&mut s, centre(layers));
    let doc = s.ed.doc.clone();
    let old = s.canvas_rect();
    timed_drag(&mut s, (old.min.x as f64, 150.0), (420.0, 150.0));
    let mid = s.canvas_rect();
    assert!(mid.min.x > old.min.x + 100.0, "Layers widened: {mid:?}");
    timed_drag(&mut s, (mid.max.x as f64, 150.0), (604.0, 150.0));
    let canvas = s.canvas_rect();
    assert!(canvas.max.x < mid.max.x - 100.0, "Design: {canvas:?}");
    assert_eq!(s.ed.doc, doc, "resizing the panels drew nothing");
    // Revision 20: the zoom is no longer on the canvas (R33 had it beside the rail, where a
    // canvas this narrow left it no room); it is in the Design panel's top row, left of the
    // toggle, so the widened panels don't push it anywhere. It stays inside that row at every
    // percentage, clear of the toggle, Export and the rail.
    let inside = |s: &mut WebState, why: &str| {
        timed_frames(s, 3);
        let canvas = s.canvas_rect();
        let rail = s.ui.rail.rects.rail.unwrap();
        let zoom = s.ui.chrome.zoom.expect("a zoom control");
        let design = s.ui.chrome.design.unwrap();
        let export = s.ui.chrome.export.unwrap();
        assert!(
            zoom.min.x >= canvas.max.x && zoom.max.x <= design.min.x,
            "{why}: {zoom:?} between the canvas {canvas:?} and the toggle {design:?}"
        );
        assert!(export.max.x <= zoom.min.x, "{why}: Export before it");
        assert!(!overlap(zoom, rail), "{why}: clear of the rail");
    };
    inside(&mut s, "100%");
    while s.ed.camera.zoom < 10.0 {
        s.run(Action::ZoomIn);
    }
    inside(&mut s, "1000%");
    // The panel's row has room for the whole group even then: its own + zooms in.
    let zoom_in =
        s.ui.chrome
            .zoom_in
            .expect("the whole group in the panel's row");
    let zoom = s.ed.camera.zoom;
    timed_click(&mut s, centre(zoom_in));
    assert!(s.ed.camera.zoom > zoom, "the group's Zoom In");
    // Longer and shorter percentages keep it inside.
    for _ in 0..12 {
        s.run(Action::ZoomIn);
    }
    inside(&mut s, "zoomed in");
    for _ in 0..24 {
        s.run(Action::ZoomOut);
    }
    inside(&mut s, "zoomed out");
    assert_eq!(s.ed.doc, doc, "zooming drew nothing");
    // The rail's own targets still work on the narrow canvas.
    let chevron = s.ui.rail.rects.rows[3].1.unwrap();
    timed_click(&mut s, centre(chevron));
    assert_eq!(s.ui.rail.flyout, Some(3));
}

/// The inspector's text with two rectangles (radius 10) selected, their independent corners
/// `a` and `b`.
fn corners_selected(a: Option<[f64; 4]>, b: Option<[f64; 4]>) -> Vec<String> {
    let mut doc = Document::default();
    for (radii, (x, y)) in [(a, (100.0, 100.0)), (b, (400.0, 300.0))] {
        let id = doc.alloc_id();
        let r = crate::geom::Rect::new(x, y, 200.0, 100.0);
        let mut n = Node::new(id, NodeKind::Rect, r);
        n.style.corner_radius = 10.0;
        n.style.radii = radii;
        doc.nodes.insert(id, n);
        doc.root.push(id);
    }
    let page = PageData {
        id: "p".into(),
        name: "Page 1".into(),
        doc,
        camera: Some(Default::default()),
    };
    let mut s = WebState::open(&canvas(&[page], "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    s.modifiers(false, true, false, false);
    assert!(s.key_down("a", "KeyA", false));
    s.modifiers(false, false, false, false);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.selection.len(), 2);
    inspector_text(&mut s)
}

/// Whether the corner field with prefix `p` shows 10 in `text`.
fn corner_shows_ten(text: &[String], p: &str) -> bool {
    text.windows(2)
        .any(|w| w[0] == p && w[1].parse::<f64>() == Ok(10.0))
}

#[test]
fn rev2_independent_corners_are_mixed_only_when_they_differ() {
    // R33-F6: every corner of both layers is 10.
    let text = corners_selected(Some([10.0; 4]), Some([10.0; 4]));
    assert!(!text.iter().any(|t| t == "Radius  Mixed"), "{text:?}");
    for p in ["TL ", "TR ", "BR ", "BL "] {
        assert!(corner_shows_ten(&text, p), "{p}10: {text:?}");
    }
    // One with independent corners, one without: the same corners still.
    let text = corners_selected(Some([10.0; 4]), None);
    assert!(!text.iter().any(|t| t == "Radius  Mixed"), "{text:?}");
    // Control: one corner differs, so Mixed.
    let text = corners_selected(Some([10.0; 4]), Some([10.0, 10.0, 10.0, 4.0]));
    assert!(text.iter().any(|t| t == "Radius  Mixed"), "{text:?}");
}

// ---- revision 18: linked hover, the slim rail's arrows, the identity's renames and Paper
// navigation, and the inspector's states. The real upstream UI through this host's input; the
// geometry read is what it laid out (`UiState::chrome`, `rail.rects`, `layer_rows`).

#[test]
fn rev18_a_canvas_hover_lights_its_layer_row_and_changes_nothing() {
    let mut s = state();
    let id = s.ed.doc.root[0];
    let before = (page_to_json(&s.ed.page_data(0)), s.ed.history.undo_len());
    let t0 = s.now_ms + 1000.0;
    let hover = |s: &mut WebState, (x, y): (f64, f64), t: f64| {
        s.pointer_move(x, y, t);
        step(s);
        step(s);
    };
    // Over the rectangle on the canvas: its row in the layers panel shows it, nothing more.
    let on_rect = at(&s, 150.0, 150.0);
    hover(&mut s, on_rect, t0);
    assert_eq!(s.ed.hover, Some(id));
    assert_eq!(
        s.ui.chrome.hover_row,
        Some(id),
        "the row shows the canvas's hover"
    );
    let row = s.ui.layer_rows.iter().find(|(r, _, _)| *r == id).unwrap().1;
    assert!(
        row.max.x <= s.canvas_rect().min.x,
        "a row of the layers panel"
    );
    assert!(s.ed.selection.is_empty(), "nothing selected");
    // Blank canvas: no row is lit.
    let blank = at(&s, 600.0, 450.0);
    hover(&mut s, blank, t0 + 20.0);
    assert_eq!(s.ui.chrome.hover_row, None, "blank canvas");
    // Over it again, then off the canvas onto the inspector, then the rail: cleared each time,
    // never left stale.
    hover(&mut s, on_rect, t0 + 40.0);
    assert_eq!(s.ui.chrome.hover_row, Some(id));
    let right = s.canvas_rect().max.x as f64;
    hover(&mut s, (right + 60.0, 400.0), t0 + 60.0);
    assert_eq!(
        s.ui.chrome.hover_row, None,
        "cleared as the pointer leaves the canvas"
    );
    hover(&mut s, on_rect, t0 + 80.0);
    let rail = centre(s.ui.rail.rects.rows[0].0);
    hover(&mut s, rail, t0 + 100.0);
    assert_eq!(
        s.ui.chrome.hover_row, None,
        "a floating control isn't the canvas"
    );
    // The panel's own row hover still shows on the canvas, as before.
    let row_at = (row.center().x as f64, row.center().y as f64);
    hover(&mut s, row_at, t0 + 120.0);
    assert_eq!(s.ed.hover, Some(id), "the row lights its layer");
    assert_eq!(
        s.ui.chrome.hover_row, None,
        "the pointer isn't on the canvas"
    );
    assert!(s.ed.selection.is_empty());
    let after = (page_to_json(&s.ed.page_data(0)), s.ed.history.undo_len());
    assert_eq!(after, before, "no document or history change");
    assert!(!s.dirty() && s.can_reload());
}

#[test]
fn rev18_a_hover_inside_a_collapsed_group_lights_the_group_s_row_without_expanding_it() {
    let mut s = state();
    let id = s.ed.doc.root[0];
    s.ed.selection = vec![id];
    s.run(Action::Group);
    let g = s.ed.selection[0];
    assert_ne!(g, id, "a group holds the rectangle");
    s.ed.selection.clear();
    s.ui.collapsed.insert(g);
    step(&mut s);
    let undo = s.ed.history.undo_len();
    let (x, y) = at(&s, 150.0, 150.0);
    s.pointer_move(x, y, s.now_ms + 1000.0);
    step(&mut s);
    step(&mut s);
    assert!(s.ed.hover.is_some(), "the canvas hovers the drawing");
    // Its own row isn't drawn (the group is collapsed): the group's row, the one shown.
    assert_eq!(s.ui.chrome.hover_row, Some(g));
    assert!(s.ui.collapsed.contains(&g), "not expanded for a hover");
    assert!(
        !s.ui.layer_rows.iter().any(|(r, _, _)| *r == id),
        "its rows stay hidden"
    );
    assert!(s.ed.selection.is_empty() && s.ed.history.undo_len() == undo);
}

#[test]
fn rev18_the_slim_rail_shows_an_arrow_only_where_wanted_and_never_moves_the_canvas() {
    use crate::ui::rail;
    let mut s = fresh(W, H);
    let c = s.canvas_rect();
    let r = s.ui.rail.rects.rail.unwrap();
    assert_eq!(rail::RAIL_W, 52.0, "slimmer than revision 2's 64 px");
    assert_eq!(r.width(), rail::RAIL_W);
    let rows = s.ui.rail.rects.rows.clone();
    // Revision 20: Select and Hand are separate, always-visible buttons; Hand (one tool) has no
    // arrow, Pointer keeps Laser pointer behind its arrow.
    assert_eq!(rows.len(), 7);
    assert_eq!(s.ui.rail.members[..2], [Tool::Select, Tool::Hand]);
    assert!(rows[0].1.is_some() && rows[1].1.is_none(), "{rows:?}");
    // At rest only the current tool's family (Pointer: Select) shows its arrow. On a surface
    // without hover a family is chosen first; its arrow then shows and opens its flyout.
    let rest = [true, false, false, false, false, false, false];
    assert_eq!(s.ui.rail.rects.arrows, rest);
    let t0 = s.now_ms + 1000.0;
    // Hovering the Shapes row shows its arrow; nothing moves and no tool is chosen.
    let shapes = centre(rows[3].0);
    s.pointer_move(shapes.0, shapes.1, t0);
    step(&mut s);
    step(&mut s);
    assert!(s.ui.rail.rects.arrows[3], "the hovered row's arrow");
    assert_eq!(s.canvas_rect(), c, "the canvas keeps its rectangle");
    assert_eq!(s.ui.rail.rects.rail, Some(r));
    assert_eq!(
        s.ui.rail.rects.rows, rows,
        "every target stays where it was"
    );
    assert_eq!(s.ed.tool, Tool::Select, "hovering chooses nothing");
    // Away from the rail it goes again.
    let blank = at(&s, 600.0, 450.0);
    s.pointer_move(blank.0, blank.1, t0 + 20.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ui.rail.rects.arrows, rest);
    // Keyboard focus (Tab) on another family's control shows that family's arrow too.
    let mut tabs = 0;
    while !s.ui.rail.rects.arrows[1..].contains(&true) {
        assert!(tabs < 300, "Tab reaches the rail");
        s.key_down("Tab", "Tab", false);
        s.key_up("Tab", "Tab");
        step(&mut s);
        step(&mut s);
        tabs += 1;
    }
    assert!(
        s.ui.rail.rects.focus.is_some(),
        "shown for the focused rail control"
    );
    assert_eq!(s.ed.tool, Tool::Select, "focus chooses nothing");
    s.ctx.memory_mut(|m| {
        if let Some(f) = m.focused() {
            m.surrender_focus(f);
        }
    });
    step(&mut s);
    // Choosing Shapes by its main button (a touch's way): its arrow shows at rest, Pointer's
    // goes, and the arrow then opens the flyout without choosing anything else.
    click(&mut s, shapes, t0 + 100.0);
    step(&mut s);
    s.pointer_move(blank.0, blank.1, t0 + 200.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.tool, Tool::Rect);
    assert_eq!(
        s.ui.rail.rects.arrows,
        [false, false, false, true, false, false, false]
    );
    click(&mut s, centre(rows[3].1.unwrap()), t0 + 300.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ui.rail.flyout, Some(3));
    assert_eq!(s.ed.tool, Tool::Rect, "the arrow opened the menu only");
    assert_eq!(s.canvas_rect(), c);
    press(&mut s, "Escape", "Escape");
    step(&mut s);
    assert_eq!(s.ui.rail.flyout, None);
    assert_eq!(s.ed.doc.nodes.len(), 1, "nothing drawn");
}

/// Every rail target as laid out (egui's actual responses): each family's main button, then its
/// arrow where it has one.
fn rail_targets(s: &WebState) -> Vec<egui::Rect> {
    let rows = &s.ui.rail.rects.rows;
    rows.iter()
        .flat_map(|(main, arrow)| std::iter::once(*main).chain(*arrow))
        .collect()
}

#[test]
fn rev18_a_short_surface_keeps_every_rail_control_clear_of_the_floating_row_and_hittable() {
    use crate::ui::rail;
    // The owner's actual short surface (the lead's run133 probe): 320 × 240, panels closed.
    // Before, the requested rail (y 45–255) ran past the 240 px canvas and the shown rail
    // started at y 33, under the floating row (y 1–37). Its rows now take the room there is.
    let mut s = fresh(320.0, 240.0);
    timed_frames(&mut s, 4);
    let c = s.canvas_rect();
    let top = s.ui.chrome.left_float.expect("floating Home/identity");
    let right = s.ui.chrome.right_float.expect("floating Design toggle");
    let rows = s.ui.rail.rects.rows.clone();
    assert_eq!(rows.len(), 7, "seven families (revision 20: Hand its own)");
    // The probe's own assertion, unchanged in meaning: the first button as egui laid it out.
    let first = rows[0].0;
    assert!(
        !overlap(top, first),
        "first rail button geometrically overlaps floating controls: top={top:?}; button={first:?}"
    );
    // Every target: clear of both floating rows, inside the canvas, at least MIN_ROW_H high,
    // each distinct from every other.
    let targets = rail_targets(&s);
    assert_eq!(targets.len(), 12, "seven main buttons and five arrows");
    for (i, t) in targets.iter().enumerate() {
        assert!(!overlap(*t, top) && !overlap(*t, right), "{i}: {t:?}");
        assert!(c.contains_rect(*t), "{i}: {t:?} in {c:?}");
        assert!(t.height() >= rail::MIN_ROW_H - 0.5, "{i}: {t:?}");
        for u in &targets[i + 1..] {
            assert!(!overlap(*t, *u), "{i}: {t:?} and {u:?}");
        }
    }
    // The requested rail is where egui showed it: every actual target lies inside it, and it
    // is clear of the floating row and the zoom.
    let shown = s.ui.rail.rects.rail.expect("the rail");
    for t in &targets {
        assert!(
            shown.expand(1.5).contains_rect(*t),
            "{t:?} outside {shown:?}"
        );
    }
    let zoom = s.ui.chrome.zoom.expect("the zoom");
    assert!(!overlap(shown, top) && !overlap(zoom, top) && !overlap(zoom, shown));
    assert!(c.contains_rect(zoom), "{zoom:?} in {c:?}");
    // Hits: a press just inside each main button's top and bottom edges chooses that family's
    // member, never a neighbour's, and none reaches the floating row's Home; Home itself is
    // still the way home.
    let mut t = s.now_ms + 1000.0;
    for (fi, (main, _)) in rows.iter().enumerate() {
        for y in [main.min.y + 1.0, main.max.y - 1.0] {
            s.run(Action::Tool(if fi == 0 {
                Tool::Rect
            } else {
                Tool::Select
            }));
            step(&mut s);
            click(&mut s, (main.center().x as f64, y as f64), t);
            step(&mut s);
            t += 500.0;
            let got = crate::ui::rail::family_of(s.ed.tool);
            assert_eq!(got, fi, "a press at y {y} on family {fi}'s button");
        }
    }
    assert_eq!(s.take_request(), None, "no press on the rail reached Home");
    assert_eq!(s.ui.rail.flyout, None, "nor opened a flyout");
    // All Projects is still the way home: its menu's first item.
    open_menu(&mut s, t + 500.0);
    click_front_label(&mut s, "Back to All Projects", t + 600.0);
    assert_eq!(s.take_request(), Some(HostRequest::Home));
    // And every one of the sixteen tools through its family's button and arrow and flyout.
    let reached = reach_every_tool(&mut s, t + 1000.0);
    assert_eq!(reached.len(), 16, "{reached:?}");
    assert_eq!(s.ed.doc.nodes.len(), 1, "probing drew nothing");
}

#[test]
fn rev18_ordinary_heights_keep_the_rail_s_full_rows() {
    use crate::ui::rail;
    // Control: a tall surface and a short one whose room under the floating row still fits
    // seven 32 px rows keep the rail's full rows. (Revision 20's seventh row needs 256 px of
    // room: the canonical 270 px surface now has 233 and gets 28 px rows, so 300 is the
    // control; the short-room test covers 240.)
    for (w, h) in [(W, H), (NARROW, 300.0)] {
        let mut s = fresh(w, h);
        timed_frames(&mut s, 2);
        let shown = s.ui.rail.rects.rail.expect("the rail");
        assert_eq!(shown.width(), rail::RAIL_W, "{w}×{h}");
        assert_eq!(shown.height(), rail::rail_height(), "{w}×{h}");
        let rows = &s.ui.rail.rects.rows;
        for (main, arrow) in rows {
            assert_eq!(
                main.size(),
                egui::vec2(rail::MAIN_W, rail::ROW_H),
                "{w}×{h}"
            );
            if let Some(a) = arrow {
                assert_eq!(
                    a.size(),
                    egui::vec2(rail::CHEVRON_W, rail::ROW_H),
                    "{w}×{h}"
                );
            }
        }
        let float = s.ui.chrome.left_float.expect("the left row floats");
        for t in rail_targets(&s) {
            assert!(!overlap(t, float), "{w}×{h}: {t:?} {float:?}");
        }
    }
}

#[test]
fn rev18_rename_canvas_opens_in_the_name_s_own_slot_and_survives_collapse_panels() {
    let mut s = state();
    let identity = s.ui.chrome.identity.unwrap();
    // From the All Projects menu's Rename Canvas…: the field takes the title's place.
    open_menu(&mut s, 100.0);
    click_label(&mut s, "Rename Canvas…", 200.0);
    for _ in 0..3 {
        step(&mut s);
    }
    assert!(s.ui.file_title_draft.is_some() && s.field_has_keyboard());
    let field = s.ui.file_title_field.expect("the field is laid out");
    let r = s.ctx.read_response(field).expect("its response").rect;
    assert_eq!(
        s.ui.chrome.identity,
        Some(identity),
        "nothing moved or resized"
    );
    assert!(
        identity.contains_rect(r),
        "in the name's slot: {r:?} {identity:?}"
    );
    s.modifiers(false, true, false, false);
    press(&mut s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(&mut s);
    s.paste("Draft name");
    step(&mut s);
    // Collapse Panels while the field has the keyboard: the arrangement waits; the draft stays.
    let before = s.ui.narrow.shown();
    s.run(Action::TogglePanels);
    for _ in 0..4 {
        step(&mut s);
    }
    assert_eq!(s.ui.narrow.shown(), before, "it waits for the field");
    assert!(s.unsaved_work() && !s.can_reload(), "the draft is guarded");
    assert_eq!(s.ui.project_name, "Canvas", "still a draft");
    // Enter finishes it, once; then the collapse applies and the floating identity names it.
    press(&mut s, "Enter", "Enter");
    for _ in 0..4 {
        step(&mut s);
    }
    assert_eq!(s.ui.project_name, "Draft name");
    let a = s.ui.narrow.shown().unwrap();
    assert!(!a.layers && !a.design, "then the collapse applies");
    assert!(in_identity(&mut s, "Draft name"), "the floating identity");
    let snap = s.save_snapshot().expect("the rename is saved");
    assert_eq!(saved_name(&snap).as_deref(), Some("Draft name"));
    s.save_succeeded();
    assert!(s.save_snapshot().is_none(), "one rename, saved once");
    // In the floating identity too the field opens in the name's slot; a blank name changes
    // nothing and says why.
    let compact = s.ui.chrome.identity.unwrap();
    let name = in_header(&mut s, "Draft name");
    click(&mut s, name, 3000.0);
    for _ in 0..3 {
        step(&mut s);
    }
    let field = s.ui.file_title_field.expect("the field is laid out");
    let r = s.ctx.read_response(field).expect("its response").rect;
    assert_eq!(
        s.ui.chrome.identity,
        Some(compact),
        "the floating row keeps its size"
    );
    assert!(compact.contains_rect(r), "{r:?} {compact:?}");
    s.modifiers(false, true, false, false);
    press(&mut s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(&mut s);
    s.paste("   ");
    step(&mut s);
    press(&mut s, "Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ui.project_name, "Draft name", "a blank name isn't taken");
    assert!(s.ed.status.contains("1 to 60"), "{}", s.ed.status);
    assert!(!s.dirty() && s.save_snapshot().is_none());
}

#[test]
fn rev20_go_to_paper_navigates_papers_and_a_paper_rename_ends_on_its_own_paper() {
    let mut s = two_pages();
    // Rename Paper… from the title's own menu (a secondary click on the Canvas name) opens
    // the shown Paper's (Beta's) rename in the Papers list.
    let name = in_header(&mut s, "Canvas");
    s.pointer_down(name.0, name.1, Button::Secondary, 100.0);
    s.pointer_up(name.0, name.1, Button::Secondary, 110.0);
    step(&mut s);
    step(&mut s);
    let right = s.canvas_rect().max.x;
    click_label_in(&mut s, "Rename Paper…", |r| r.max.x <= right, 200.0);
    for _ in 0..3 {
        step(&mut s);
    }
    let beta = page_index(&s, "beta22");
    assert_eq!(s.ui.pages.renaming.as_ref().map(|(i, _)| *i), Some(beta));
    assert!(s.field_has_keyboard());
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    // The All Projects menu's Go to Paper lists every Paper (revision 20; revision 18 had a
    // Paper line under the title for it); choosing Alpha there finishes the draft on Beta
    // first (opening the menu is a click elsewhere), then shows Alpha.
    open_menu(&mut s, 1000.0);
    click_front_label(&mut s, "Go to Paper", 1100.0);
    let text = painted_text(&mut s);
    // The menu is painted over the panel, after its rows: the last "Alpha" is the menu's.
    let (_, alpha) = text
        .iter()
        .rev()
        .find(|(t, _)| t.trim() == "Alpha")
        .expect("Alpha in the Paper menu");
    let p = centre(*alpha);
    s.pointer_move(p.0, p.1, 2000.0);
    step(&mut s);
    click(&mut s, p, 2005.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.active_page_id(), "alpha1", "the menu showed Alpha");
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names[0], "Alpha", "Alpha untouched");
    assert!(
        names[1] != "Beta" && names[1].contains('q'),
        "finished on Beta: {names:?}"
    );
    assert!(s.ui.pages.renaming.is_none() && !s.field_has_keyboard());
    let snap = s.save_snapshot().expect("the rename and the switch");
    let (active, list, sent) = parse_save(&snap);
    assert_eq!(active, "alpha1");
    assert_eq!(list[1].0, "beta22");
    assert!(sent.is_empty(), "no page's content changed");
}

#[test]
fn rev18_an_empty_canvas_click_returns_the_inspector_to_the_paper_context() {
    let mut s = state();
    let right = s.canvas_rect().max.x;
    click_at(&mut s, 150.0, 150.0, 0.0);
    step(&mut s);
    step(&mut s);
    let text = inspector_text(&mut s);
    assert!(
        text.iter().any(|t| t.starts_with("X  ")),
        "the layer's own sections: {text:?}"
    );
    assert!(
        !text.iter().any(|t| t == "Rename Paper…" || t == "View"),
        "{text:?}"
    );
    // A click on empty canvas: no selection, and the shown Paper's context again.
    click_at(&mut s, 700.0, 500.0, 1000.0);
    step(&mut s);
    step(&mut s);
    assert!(s.ed.selection.is_empty());
    let text = inspector_text(&mut s);
    for wanted in ["Paper", "View", "Grid"] {
        assert!(text.iter().any(|t| t == wanted), "{wanted}: {text:?}");
    }
    assert!(!text.iter().any(|t| t.starts_with("X  ")), "{text:?}");
    // Its Grid is the view's (the same preference as the canvas's menu): no document change.
    let authored = page_to_json(&s.ed.page_data(0));
    click_label_in(&mut s, "None", |r| r.min.x >= right, 2000.0);
    click_label_in(&mut s, "Dots", |r| r.min.x >= right, 3000.0);
    assert_eq!(s.ui.background, scene::Background::Dots);
    assert_eq!(requests(&mut s), [HostRequest::Background]);
    assert_eq!(page_to_json(&s.ed.page_data(0)), authored);
    assert!(!s.dirty() && s.can_reload() && s.save_snapshot().is_none());
}

// ---- revision 20: the contextual Design reveal and the left panel's Design/Theme switch.
// The real upstream UI through this host's input. The lead's before capture (frozen d0): with
// the Design panel closed by the person, an ordinary click selected a rectangle and the panel
// stayed closed; these say what revision 20 does instead.

/// A wide fresh view (Layers closed) whose Design panel the person closed with its own toggle.
fn design_closed() -> WebState {
    let mut s = fresh(W, H);
    toggle(&mut s, "Design", 100.0);
    assert!(!s.ui.narrow.shown().unwrap().design, "closed by the person");
    assert_eq!(s.ui.panels.design_choice(), Some(false));
    s
}

fn design_shown(s: &WebState) -> bool {
    s.ui.narrow.shown().unwrap().design
}

#[test]
fn rev20_a_selection_reveals_a_closed_design_panel_and_clearing_it_restores_the_choice() {
    let mut s = design_closed();
    let before = page_to_json(&s.ed.page_data(0));
    let undo = s.ed.history.undo_len();
    let closed = s.canvas_rect();
    // An ordinary click on the rectangle: the Design panel shows, for it.
    click_at(&mut s, 150.0, 150.0, 1000.0);
    timed_frames(&mut s, 3);
    assert_eq!(s.ed.selection.len(), 1);
    assert!(design_shown(&s) && s.ui.panels.revealed(), "revealed");
    assert_eq!(
        s.ui.panels.design_choice(),
        Some(false),
        "the person's closed choice isn't rewritten"
    );
    assert!(s.canvas_rect().max.x < closed.max.x - 200.0);
    assert!(inspector_text(&mut s).iter().any(|t| t.starts_with("X  ")));
    // A click on blank canvas clears the selection: closed again, as the person left it.
    click_at(&mut s, 700.0, 500.0, 2000.0);
    timed_frames(&mut s, 3);
    assert!(s.ed.selection.is_empty());
    assert!(!design_shown(&s) && !s.ui.panels.revealed(), "restored");
    assert_eq!(s.canvas_rect(), closed);
    assert_eq!(s.ui.panels.design_choice(), Some(false));
    // The toggle on a revealed panel closes it until the selection clears.
    click_at(&mut s, 150.0, 150.0, 3000.0);
    timed_frames(&mut s, 3);
    assert!(design_shown(&s));
    toggle(&mut s, "Design", 4000.0);
    timed_frames(&mut s, 2);
    assert!(!design_shown(&s), "dismissed");
    assert_eq!(
        s.ui.panels.design_choice(),
        Some(false),
        "still the person's"
    );
    click_at(&mut s, 160.0, 160.0, 5000.0);
    timed_frames(&mut s, 3);
    assert_eq!(s.ed.selection.len(), 1);
    assert!(!design_shown(&s), "stays dismissed while selected");
    click_at(&mut s, 700.0, 500.0, 6000.0);
    timed_frames(&mut s, 2);
    assert!(s.ed.selection.is_empty());
    click_at(&mut s, 150.0, 150.0, 7000.0);
    timed_frames(&mut s, 3);
    assert!(design_shown(&s), "the next selection reveals it again");
    // Selecting, revealing and hiding changed nothing in the document or its history.
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
    assert_eq!(s.ed.history.undo_len(), undo);
    assert!(!s.dirty() && s.can_reload());
}

#[test]
fn rev20_an_open_design_panel_stays_open_and_collapse_never_reveals() {
    // Pinned: open (the wide default) through a selection and its clearing, never "revealed".
    let mut s = fresh(W, H);
    click_at(&mut s, 150.0, 150.0, 1000.0);
    timed_frames(&mut s, 3);
    assert!(design_shown(&s) && !s.ui.panels.revealed());
    click_at(&mut s, 700.0, 500.0, 2000.0);
    timed_frames(&mut s, 3);
    assert!(design_shown(&s), "the open panel stays open");
    // Collapsed: Collapse Panels still hides both, selection or not.
    s.run(Action::TogglePanels);
    timed_frames(&mut s, 4);
    click_at(&mut s, 150.0, 150.0, 3000.0);
    timed_frames(&mut s, 3);
    assert_eq!(s.ed.selection.len(), 1);
    assert!(!design_shown(&s), "no reveal while collapsed");
}

#[test]
fn rev20_a_docked_reveal_sits_beside_the_canvas_or_the_open_left_panel_and_restores_it() {
    // Canvas only, 800 px (docked): the Design panel docks on the right for the selection; the
    // canvas keeps its origin; clearing restores the canvas-only view.
    let mut s = fresh(800.0, TALL);
    let c = s.canvas_rect();
    let before = (page_to_json(&s.ed.page_data(0)), s.ed.history.undo_len());
    click_at(&mut s, 150.0, 150.0, 1000.0);
    timed_frames(&mut s, 3);
    assert_eq!(s.ed.selection.len(), 1);
    assert!(design_shown(&s) && s.ui.panels.revealed());
    assert_eq!(s.ui.narrow.shown().unwrap().tier, crate::ui::Tier::Docked);
    let narrowed = s.canvas_rect();
    assert_eq!(narrowed.min, c.min, "the canvas keeps its origin");
    assert!(narrowed.width() < c.width() - 200.0);
    assert_eq!(s.ui.panels.design_choice(), None, "no choice written");
    // A click on blank canvas clears it. (Run145 fixture fix: the first version clicked at
    // x 600, blank canvas before the reveal but inside the revealed panel, 540–800, after it;
    // the point is now checked against the canvas as laid out.)
    let blank = at(&s, 450.0, 500.0);
    let blank_pos = egui::pos2(blank.0 as f32, blank.1 as f32);
    assert!(narrowed.contains(blank_pos), "{blank:?} in {narrowed:?}");
    click(&mut s, blank, 2000.0);
    timed_frames(&mut s, 3);
    assert!(s.ed.selection.is_empty(), "cleared by the click");
    assert_eq!(shown_panel(&s), NarrowPanel::Canvas);
    assert_eq!(s.canvas_rect(), c);
    assert_eq!(s.ui.panels.design_choice(), None, "still no choice");
    let after = (page_to_json(&s.ed.page_data(0)), s.ed.history.undo_len());
    assert_eq!(after, before, "no document or history change");
    // The left panel open on Theme, 800 px (both fit): choosing a Theme row (the rectangle's
    // colour) shows the Design panel beside it; clearing the selection leaves Theme exactly as
    // it was.
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let mut p = page("p", "Page 1", rect);
    for n in p.doc.nodes.values_mut() {
        n.style = crate::model::Style::with_fill(Color::rgb(0xe5, 0xed, 0xff));
    }
    let mut s = WebState::open(&canvas(&[p], "p"), false, 800.0, TALL).unwrap();
    settle(&mut s, 0.0);
    toggle(&mut s, "Layers", 100.0);
    let left = s.canvas_rect().min.x;
    click_label_in(&mut s, "Theme", |r| r.max.x <= left, 200.0);
    let rows = painted_text(&mut s);
    let (_, colour) = rows
        .iter()
        .find(|(t, r)| t.starts_with('#') && r.max.x <= left)
        .expect("a colour row in Theme");
    click(&mut s, centre(*colour), 300.0);
    timed_frames(&mut s, 3);
    assert_eq!(s.ed.selection.len(), 1, "the colour's layer is selected");
    let a = s.ui.narrow.shown().unwrap();
    assert!(a.layers && a.design, "beside the left panel: {a:?}");
    assert_eq!(s.ui.left_tab, crate::ui::LeftTab::Theme, "Theme stays open");
    assert_eq!(s.canvas_rect().min.x, left, "the canvas keeps its origin");
    // A click on blank canvas (inside the 300 px left between the panels) clears it.
    click_at(&mut s, 250.0, 500.0, 400.0);
    timed_frames(&mut s, 3);
    assert!(s.ed.selection.is_empty());
    let a = s.ui.narrow.shown().unwrap();
    assert!(a.layers && !a.design, "Theme alone again: {a:?}");
    assert_eq!(s.ui.left_tab, crate::ui::LeftTab::Theme);
}

#[test]
fn rev20_a_reveal_never_moves_the_canvas_under_a_drag_and_follows_its_release() {
    // 800 px, canvas only: pressing on the rectangle selects it, and the drag that follows
    // moves it. The Design panel (which narrows the canvas) waits for the release, so nothing
    // changes under the pointer while it is held; then it shows, the move done as dragged.
    let mut s = fresh(800.0, TALL);
    let c = s.canvas_rect();
    let id = s.ed.doc.root[0];
    let (x, y) = at(&s, 150.0, 150.0);
    s.tick(s.now_ms + 1000.0);
    s.pointer_move(x, y, s.now_ms);
    s.pointer_down(x, y, Button::Primary, s.now_ms);
    timed_frames(&mut s, 2);
    assert_eq!(s.ed.selection, vec![id], "selected by the press");
    assert!(s.ed.is_busy(), "the press owns the canvas");
    for k in 1..=4 {
        s.pointer_move(x + 20.0 * k as f64, y + 10.0 * k as f64, s.now_ms);
        timed_frames(&mut s, 1);
        assert_eq!(s.canvas_rect(), c, "{k}: no change under the drag");
        assert!(!design_shown(&s), "{k}: the reveal waits");
    }
    s.pointer_up(x + 80.0, y + 40.0, Button::Primary, s.now_ms);
    timed_frames(&mut s, 3);
    assert!(design_shown(&s), "then the Design panel shows");
    // Moved by the drag (80, 40), give or take snapping: the canvas never moved under it.
    let r = s.ed.doc.get(id).unwrap().rect;
    assert!(
        (r.x - 180.0).abs() < 8.0 && (r.y - 140.0).abs() < 8.0,
        "moved as dragged: {r:?}"
    );
}

#[test]
fn rev20_where_only_one_panel_fits_the_reveal_stands_in_for_it_and_gives_it_back() {
    // Docked at 700 px with Layers open: a selection shows the Design panel in the left
    // panel's place (both don't fit beside a usable canvas); clearing it gives Layers back,
    // the person's choices untouched. Its left toggle brings Layers back at once too.
    let mut s = fresh(700.0, TALL);
    toggle(&mut s, "Layers", 100.0);
    click_at(&mut s, 150.0, 150.0, 1000.0);
    timed_frames(&mut s, 3);
    assert_eq!(shown_panel(&s), NarrowPanel::Design);
    assert!(s.ui.panels.swapped() && s.ui.panels.revealed());
    assert!(
        s.ui.panels.wanted(crate::ui::Tier::Docked).layers,
        "Layers still chosen"
    );
    click_at(&mut s, 400.0, 500.0, 2000.0);
    timed_frames(&mut s, 3);
    assert!(s.ed.selection.is_empty());
    assert_eq!(shown_panel(&s), NarrowPanel::Layers, "given back");
    assert!(paints(&mut s, "Papers"));
    click_at(&mut s, 150.0, 150.0, 3000.0);
    timed_frames(&mut s, 3);
    assert_eq!(shown_panel(&s), NarrowPanel::Design);
    toggle(&mut s, "Layers", 4000.0);
    timed_frames(&mut s, 2);
    assert_eq!(
        shown_panel(&s),
        NarrowPanel::Layers,
        "the left toggle gives it back"
    );
    assert_eq!(s.ed.selection.len(), 1, "the selection stays");
    // A sheet (320 px) with the Layers sheet open: a row's selection shows the Design sheet in
    // its place, whose way back returns to Layers (not the canvas). (Over the canvas alone, see
    // the next test.)
    let mut s = narrow_state();
    toggle(&mut s, "Layers", 2000.0);
    let (_, row, _) = s.ui.layer_rows[0];
    click(&mut s, centre(row), 3000.0);
    timed_frames(&mut s, 3);
    assert_eq!(shown_panel(&s), NarrowPanel::Design);
    assert!(
        paints(&mut s, "← Layers"),
        "its way back names the sheet it stands in for"
    );
    let back = s.ui.chrome.sheet_back.expect("the way back");
    click(&mut s, centre(back), 4000.0);
    timed_frames(&mut s, 3);
    assert_eq!(shown_panel(&s), NarrowPanel::Layers);
    assert_eq!(s.ed.selection.len(), 1, "the selection stays");
    assert!(!s.dirty(), "arranging changed nothing in the document");
    // From the Theme sheet: its colour row selects the rectangle, the Design sheet stands in
    // for Theme ("← Theme"), and clearing the selection gives Theme back as it was.
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let mut p = page("p", "Page 1", rect);
    for n in p.doc.nodes.values_mut() {
        n.style = crate::model::Style::with_fill(Color::rgb(0xe5, 0xed, 0xff));
    }
    let mut s = WebState::open(&canvas(&[p], "p"), false, NARROW, TALL).unwrap();
    settle(&mut s, 0.0);
    toggle(&mut s, "Layers", 100.0);
    click_label(&mut s, "Theme", 200.0);
    // The setup is the person's own choice: opening the left sheet closes Design (one panel at
    // a time), so Design's choice is now "closed", not the fresh default. (Run145 fixture fix:
    // the first version expected `None` at the end, which the setup itself had ruled out.) The
    // reveal must leave exactly this choice.
    let chosen = s.ui.panels.design_choice();
    assert_eq!(chosen, Some(false), "closed by opening the left sheet");
    let colour = painted_at(&mut s, "#e5edff", |_| true);
    click(&mut s, colour, 300.0);
    timed_frames(&mut s, 3);
    assert_eq!(s.ed.selection.len(), 1, "the colour's layer is selected");
    assert_eq!(shown_panel(&s), NarrowPanel::Design);
    assert!(s.ui.panels.revealed() && s.ui.panels.swapped());
    assert_eq!(
        s.ui.panels.design_choice(),
        chosen,
        "not written by the reveal"
    );
    assert!(paints(&mut s, "← Theme"), "its way back names Theme");
    s.ed.selection.clear();
    timed_frames(&mut s, 3);
    assert_eq!(shown_panel(&s), NarrowPanel::Layers, "Theme again");
    assert_eq!(s.ui.left_tab, crate::ui::LeftTab::Theme);
    assert_eq!(s.ui.panels.design_choice(), chosen, "nor by its end");
    // Its way back ("← Theme") restores the same, the selection kept.
    click(&mut s, colour, 400.0);
    timed_frames(&mut s, 3);
    assert_eq!(shown_panel(&s), NarrowPanel::Design);
    let back = s.ui.chrome.sheet_back.expect("the way back");
    click(&mut s, centre(back), 500.0);
    timed_frames(&mut s, 3);
    assert_eq!(shown_panel(&s), NarrowPanel::Layers);
    assert_eq!(s.ui.left_tab, crate::ui::LeftTab::Theme);
    assert_eq!(s.ed.selection.len(), 1, "the selection stays");
    assert_eq!(s.ui.panels.design_choice(), chosen, "nor by its way back");
}

#[test]
fn rev20_a_sheet_reveal_covers_the_canvas_after_the_selection_and_its_way_back_keeps_it() {
    // Run145 correction: the owner approved no sheet exception. On a 320 px canvas-only view
    // (Design closed) a selection or a creation shows the same Design sheet once the canvas is
    // done with it (a double-click's window, an open text draft); "← Canvas" goes back with the
    // selection and the choices kept; clearing the selection restores the canvas.
    let mut s = narrow_state();
    let before = page_to_json(&s.ed.page_data(0));
    let id = s.ed.doc.root[0];
    s.tick(s.now_ms + 1000.0);
    let t = s.now_ms;
    click_at(&mut s, 150.0, 150.0, t);
    timed_frames(&mut s, 2);
    assert_eq!(s.ed.selection, vec![id]);
    assert_eq!(
        shown_panel(&s),
        NarrowPanel::Canvas,
        "a double-click's second press still finds the canvas"
    );
    assert!(!s.ui.panels.revealed());
    timed_frames(&mut s, 30);
    assert_eq!(
        shown_panel(&s),
        NarrowPanel::Design,
        "then the Design sheet"
    );
    assert!(s.ui.panels.revealed() && !s.ui.panels.swapped());
    assert_eq!(s.ui.panels.design_choice(), None, "no choice written");
    let c = s.canvas_rect();
    assert!(c.width() < 2.0, "the same sheet: {c:?}");
    assert!(inspector_text(&mut s).iter().any(|t| t.starts_with("X  ")));
    assert!(paints(&mut s, "← Canvas"), "its way back");
    // The way back: the canvas again, the selection kept; it stays dismissed while selected.
    let back = s.ui.chrome.sheet_back.expect("the way back");
    let t = s.now_ms;
    click(&mut s, centre(back), t);
    timed_frames(&mut s, 3);
    assert_eq!(shown_panel(&s), NarrowPanel::Canvas);
    assert_eq!(s.ed.selection, vec![id], "the selection stays");
    assert!(s.canvas_rect().width() >= NARROW as f32 - 1.0);
    timed_frames(&mut s, 30);
    assert_eq!(shown_panel(&s), NarrowPanel::Canvas, "dismissed");
    // A click on blank canvas clears it; nothing in the document changed.
    let t = s.now_ms;
    click_at(&mut s, 250.0, 600.0, t);
    timed_frames(&mut s, 3);
    assert!(s.ed.selection.is_empty());
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
    assert!(!s.dirty());
    // A creation reveals it too, after the drawing gesture and the window.
    s.run(Action::Tool(Tool::Rect));
    step(&mut s);
    let (from, to) = (at(&s, 150.0, 400.0), at(&s, 230.0, 480.0));
    let t = s.now_ms;
    drag(&mut s, from, to, t);
    timed_frames(&mut s, 2);
    assert_eq!(s.ed.doc.nodes.len(), 2, "drawn");
    assert_eq!(s.ed.tool, Tool::Select);
    assert_eq!(shown_panel(&s), NarrowPanel::Canvas);
    timed_frames(&mut s, 30);
    assert_eq!(
        shown_panel(&s),
        NarrowPanel::Design,
        "the new shape's Design"
    );
    // Clearing the selection (as a command or an agent's change can) restores the canvas.
    s.ed.selection.clear();
    timed_frames(&mut s, 3);
    let a = s.ui.narrow.shown().unwrap();
    assert!(!a.layers && !a.design, "the canvas as it was: {a:?}");
    assert_eq!(s.ui.panels.design_choice(), None);
    // A text draft on the canvas keeps it: no sheet covers the text being typed.
    s.run(Action::Tool(Tool::Text));
    step(&mut s);
    let t = s.now_ms;
    click_at(&mut s, 150.0, 600.0, t);
    timed_frames(&mut s, 30);
    assert!(s.ed.text_edit.is_some(), "editing the new text");
    assert_eq!(
        shown_panel(&s),
        NarrowPanel::Canvas,
        "the draft keeps the canvas"
    );
}

#[test]
fn rev20_an_open_menu_owns_its_keys_and_never_edits_the_drawing_behind_it() {
    // Run147, the lead's probe on frozen 145 (adopted): a rectangle selected, the All Projects
    // menu open and painted with nothing focused, then ArrowDown: the rectangle moved from
    // y 100 to 101, an undoable, saved edit behind the menu. The open menu now owns its keys.
    let mut s = fresh(W, H);
    click_at(&mut s, 150.0, 150.0, 1000.0);
    timed_frames(&mut s, 3);
    assert_eq!(s.ed.selection.len(), 1);
    let before = page_to_json(&s.ed.page_data(0));
    let selection = s.ed.selection.clone();
    let undo = s.ed.history.undo_len();
    open_menu(&mut s, 2000.0);
    assert!(paints(&mut s, "Back to All Projects"), "the menu is open");
    assert!(!s.field_has_keyboard(), "nothing focused, as in the probe");
    for (key, code) in [
        ("ArrowDown", "ArrowDown"),
        ("ArrowUp", "ArrowUp"),
        ("ArrowLeft", "ArrowLeft"),
        ("Delete", "Delete"),
        ("r", "KeyR"),
    ] {
        let taken = s.key_down(key, code, false);
        s.key_up(key, code);
        timed_frames(&mut s, 2);
        assert!(taken, "the open menu owns {key}");
        let page = page_to_json(&s.ed.page_data(0));
        assert_eq!(page, before, "{key} must not change the Paper");
        assert_eq!(s.ed.history.undo_len(), undo, "{key}: no undo step");
        assert_eq!(s.ed.selection, selection, "{key}: the selection stays");
        assert_eq!(s.ed.tool, Tool::Select, "{key}: no tool shortcut");
    }
    assert!(paints(&mut s, "Back to All Projects"), "still open");
    // A Cut or Paste arriving behind it changes nothing either.
    assert_eq!(s.copy(true), None);
    s.paste("plain text");
    timed_frames(&mut s, 1);
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
    // Escape is the menu's: it closes, and the canvas keeps its selection.
    let taken = s.key_down("Escape", "Escape", false);
    s.key_up("Escape", "Escape");
    timed_frames(&mut s, 2);
    assert!(taken, "the open menu owns Escape");
    assert!(!paints(&mut s, "Back to All Projects"), "closed");
    assert_eq!(s.ed.selection, selection, "closing it never deselects");
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
    assert_eq!(s.ed.history.undo_len(), undo);
    assert!(!s.dirty());
    // Control: with no menu open the same key is the canvas's nudge, one undo step.
    assert!(s.key_down("ArrowDown", "ArrowDown", false));
    s.key_up("ArrowDown", "ArrowDown");
    let id = selection[0];
    assert_eq!(s.ed.doc.get(id).unwrap().rect.y, 101.0, "nudged");
    assert_eq!(s.ed.history.undo_len(), undo + 1);
    s.run(Action::Undo);
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
}

/// The rect of the widget with egui's keyboard focus, as last laid out, and whether it is on
/// a popup's layer (a menu, a submenu).
fn focused(s: &WebState) -> Option<(egui::Rect, bool)> {
    let id = s.ctx.memory(|m| m.focused())?;
    let r = s.ctx.read_response(id)?;
    Some((r.rect, r.layer_id.order == egui::Order::Foreground))
}

/// Tab (egui's own focus order) until All Projects has the keyboard focus.
fn tab_to_home(s: &mut WebState) {
    let home = s.ui.chrome.home.expect("All Projects");
    let mut tabs = 0;
    while focused(s).map(|(r, _)| r) != Some(home) {
        assert!(tabs < 300, "Tab reaches All Projects");
        s.key_down("Tab", "Tab", false);
        s.key_up("Tab", "Tab");
        step(s);
        step(s);
        tabs += 1;
    }
}

#[test]
fn rev20_the_all_projects_menu_works_from_the_keyboard() {
    // Run147: Tab to All Projects; Enter opens its menu with the focus on its first item; the
    // arrow keys move the focus through the menu (never the selected drawing); Escape closes
    // it; Enter on Back to All Projects is the same guarded way home.
    // Panels collapsed (Collapse never reveals), so the Tab order is the floating bars' few
    // buttons and no inspector field is focused on the way.
    let mut s = fresh(W, H);
    s.run(Action::TogglePanels);
    for _ in 0..4 {
        step(&mut s);
    }
    let id = s.ed.doc.root[0];
    let before = page_to_json(&s.ed.page_data(0));
    let undo = s.ed.history.undo_len();
    for close in ["Escape", "Enter"] {
        tab_to_home(&mut s);
        // Something selected, so an arrow key the editor got would move it.
        s.ed.selection = vec![id];
        step(&mut s);
        press(&mut s, "Enter", "Enter");
        step(&mut s);
        step(&mut s);
        assert!(paints(&mut s, "Back to All Projects"), "{close}: open");
        let back = painted_at(&mut s, "Back to All Projects", |_| true);
        let back = egui::pos2(back.0 as f32, back.1 as f32);
        let (r, in_menu) = focused(&s).expect("the menu has the focus");
        assert!(in_menu && r.contains(back), "{close}: first item");
        press(&mut s, "ArrowDown", "ArrowDown");
        step(&mut s);
        step(&mut s);
        let (r, in_menu) = focused(&s).expect("still in the menu");
        assert!(in_menu && r.min.y > back.y, "{close}: the next item");
        press(&mut s, "ArrowUp", "ArrowUp");
        step(&mut s);
        step(&mut s);
        let (r, _) = focused(&s).expect("still in the menu");
        assert!(r.contains(back), "{close}: the first item again");
        assert_eq!(page_to_json(&s.ed.page_data(0)), before, "{close}");
        assert_eq!(s.ed.history.undo_len(), undo, "{close}");
        press(&mut s, close, close);
        step(&mut s);
        step(&mut s);
        assert!(!paints(&mut s, "Back to All Projects"), "{close}: closed");
        let home = (close == "Enter").then_some(HostRequest::Home);
        assert_eq!(s.take_request(), home, "{close}");
        assert_eq!(s.ed.selection, vec![id], "{close}: the selection stays");
    }
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
    assert!(!s.dirty());
}

#[test]
fn rev20_a_held_space_released_while_a_menu_has_the_focus_ends_the_pan() {
    // R61-F1 (the R61 review's reproduction): Space held on the canvas, All Projects opened with
    // the pointer, Tab until a widget of the menu has the focus, Space released there, Escape,
    // then a Select drag. Before, the release stayed with egui (a focused widget has the
    // keyboard), Space stayed held and the drag panned the camera by (-80, -40) instead of
    // moving the rectangle. With the menu unfocused, and with no menu at all, as controls.
    for case in ["no menu", "menu", "menu focused"] {
        let mut s = fresh(W, H);
        s.run(Action::TogglePanels);
        timed_frames(&mut s, 4);
        let id = s.ed.doc.root[0];
        let original = s.ed.doc.get(id).unwrap().rect;
        let undo = s.ed.history.undo_len();
        assert!(s.key_down(" ", "Space", false));
        timed_frames(&mut s, 1);
        assert!(s.ed.space_down, "{case}: Space holds the pan");
        if case != "no menu" {
            open_menu(&mut s, 1000.0);
            assert!(paints(&mut s, "Back to All Projects"), "{case}: open");
        }
        if case == "menu focused" {
            let mut tabs = 0;
            while !focused(&s).is_some_and(|(_, in_menu)| in_menu) {
                assert!(tabs < 100, "Tab reaches the menu");
                press(&mut s, "Tab", "Tab");
                timed_frames(&mut s, 3);
                tabs += 1;
            }
            assert!(s.field_has_keyboard(), "the menu has the keyboard");
        }
        assert!(s.ed.space_down, "{case}: still held");
        s.key_up(" ", "Space");
        timed_frames(&mut s, 2);
        assert!(!s.ed.space_down, "{case}: the release ends the pan");
        if case != "no menu" {
            press(&mut s, "Escape", "Escape");
            timed_frames(&mut s, 3);
            assert!(!paints(&mut s, "Back to All Projects"), "{case}: closed");
        }
        assert_eq!(s.ed.history.undo_len(), undo, "{case}: nothing edited");
        assert_eq!(s.ed.tool, Tool::Select, "{case}");
        let camera = s.ed.camera;
        let (from, to) = (at(&s, 150.0, 150.0), at(&s, 230.0, 190.0));
        let t = s.now_ms + 1000.0;
        drag(&mut s, from, to, t);
        timed_frames(&mut s, 3);
        let r = s.ed.doc.get(id).unwrap().rect;
        assert!(
            (r.x - original.x - 80.0).abs() < 8.0 && (r.y - original.y - 40.0).abs() < 8.0,
            "{case}: the Select drag moved the rectangle: {r:?}"
        );
        assert_eq!(s.ed.camera, camera, "{case}: no pan");
    }
}

#[test]
fn rev20_a_held_space_released_in_a_text_field_ends_the_pan_and_the_field_keeps_its_own() {
    // R61-F1, the field side: Space held on the canvas, then the palette's search takes the
    // keyboard; releasing Space there still ends the pan. Control: a Space typed into the field
    // is the field's text, and the editor never holds it.
    let mut s = fresh(W, H);
    assert!(s.key_down(" ", "Space", false));
    assert!(s.ed.space_down, "Space holds the pan");
    open_palette(&mut s);
    assert!(s.ed.space_down, "still held");
    s.key_up(" ", "Space");
    step(&mut s);
    assert!(!s.ed.space_down, "the release ends the pan");
    s.paste("abc");
    step(&mut s);
    press(&mut s, " ", "Space");
    step(&mut s);
    assert!(!s.ed.space_down, "the field's own Space never pans");
    s.key_up(" ", "Space");
    step(&mut s);
    assert_eq!(s.ui.palette.query, "abc ", "typed into the search");
    press(&mut s, "Escape", "Escape");
    step(&mut s);
    assert!(!s.ui.palette.open);
    assert!(!s.dirty(), "nothing edited");
}

#[test]
fn rev20_the_press_that_closes_a_menu_never_reaches_the_canvas() {
    // Run147 (same ownership, the pointer side): with the Rectangle tool active and the All
    // Projects menu open, a click on blank canvas closes the menu and draws nothing, as a press
    // that closes a flyout or the canvas's own menu does. Before, that press reached the
    // editor too. The next click draws.
    let mut s = fresh(W, H);
    s.run(Action::Tool(Tool::Rect));
    step(&mut s);
    let nodes = s.ed.doc.nodes.len();
    open_menu(&mut s, 1000.0);
    assert!(paints(&mut s, "Back to All Projects"), "the menu is open");
    click_at(&mut s, 600.0, 450.0, 2000.0);
    timed_frames(&mut s, 2);
    assert!(
        !paints(&mut s, "Back to All Projects"),
        "the press closed it"
    );
    assert_eq!(s.ed.doc.nodes.len(), nodes, "and drew nothing");
    assert_eq!(s.ed.tool, Tool::Rect, "the tool stays");
    assert!(!s.dirty());
    // Control: with the menu closed the same click is the canvas's.
    click_at(&mut s, 600.0, 450.0, 3000.0);
    assert_eq!(s.ed.doc.nodes.len(), nodes + 1, "drawn");
}

#[test]
fn rev20_the_all_projects_menu_scrolls_on_a_short_surface_and_reaches_its_lower_items() {
    // Run144 finding: at 320 × 240 the menu ran past the surface, so everything below Go to
    // Paper (the upstream menus, Appearance, Background, About) was out of reach. It now
    // scrolls within the surface; its submenus too.
    let mut s = fresh(320.0, 240.0);
    timed_frames(&mut s, 2);
    let before = page_to_json(&s.ed.page_data(0));
    let (tool, camera) = (s.ed.tool, s.ed.camera);
    let surface = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 240.0));
    open_menu(&mut s, 1000.0);
    timed_frames(&mut s, 2);
    assert!(
        painted_whole(&mut s, "Back to All Projects", surface),
        "Back to All Projects first, in view"
    );
    let back = painted_at(&mut s, "Back to All Projects", |_| true);
    let mut wheels = 0;
    while !painted_whole(&mut s, "About & notices", surface) {
        assert!(wheels < 30, "the menu scrolls to its last item");
        s.wheel(0.0, 120.0, back.0, back.1 + 60.0);
        timed_frames(&mut s, 2);
        wheels += 1;
    }
    assert!(wheels > 0, "it starts below the fold, as it clipped before");
    // A lower submenu opens and its choice works: Appearance ▸ Dark.
    let appearance = painted_at(&mut s, "Appearance", |r| surface.contains_rect(r));
    s.pointer_move(appearance.0, appearance.1, s.now_ms + 100.0);
    step(&mut s);
    let at = s.now_ms + 120.0;
    click(&mut s, appearance, at);
    timed_frames(&mut s, 2);
    let at = s.now_ms + 200.0;
    click_front_label(&mut s, "Dark", at);
    assert_eq!(s.appearance(), "dark");
    assert_eq!(s.take_request(), Some(HostRequest::Appearance));
    assert_eq!(s.take_request(), None, "no Home or other request");
    // Nothing reached the canvas.
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
    assert_eq!((s.ed.tool, s.ed.camera), (tool, camera));
    assert!(!s.dirty());
}

#[test]
fn rev20_theme_on_a_short_sheet_shows_the_paper_s_styles_first() {
    // Run144 finding: at 320 × 240 the font paragraph filled the Theme sheet. Now the Paper's
    // own colour row is in view at once (this fixture's rectangle has a fill).
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let mut p = page("p", "Page 1", rect);
    for n in p.doc.nodes.values_mut() {
        n.style = crate::model::Style::with_fill(Color::rgb(0xe5, 0xed, 0xff));
    }
    let mut s = WebState::open(&canvas(&[p], "p"), false, 320.0, 240.0).unwrap();
    settle(&mut s, 0.0);
    toggle(&mut s, "Layers", 100.0);
    click_label(&mut s, "Theme", 200.0);
    let surface = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 240.0));
    assert!(
        painted_whole(&mut s, "#e5edff", surface),
        "the colour row is in view"
    );
}

#[test]
fn rev20_a_reveal_waits_for_a_focused_paper_rename_draft_and_keeps_it() {
    // Papers open, Design closed by the person; a Paper's rename draft has the keyboard when
    // something gets selected (as an agent's change or a command can select): the reveal waits
    // for the field, the draft and its guard stay, Enter renames that Paper only, and then the
    // panel shows.
    let mut s = two_pages();
    toggle(&mut s, "Design", 50.0);
    assert!(!design_shown(&s));
    let p = page_row(&mut s, "Beta");
    double_click(&mut s, p, 100.0);
    assert!(s.ui.pages.renaming.is_some() && s.field_has_keyboard());
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    let canvas = s.canvas_rect();
    s.ed.selection = vec![s.ed.doc.root[0]];
    timed_frames(&mut s, 4);
    assert!(!design_shown(&s), "the reveal waits for the field");
    assert_eq!(s.canvas_rect(), canvas, "nothing moved under the field");
    assert!(s.field_has_keyboard() && s.unsaved_work() && !s.can_reload());
    let draft =
        s.ui.pages
            .renaming
            .as_ref()
            .map(|(_, n)| n.clone())
            .unwrap();
    assert!(draft.contains('q'), "{draft:?}");
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 4);
    assert_eq!(s.ed.pages[page_index(&s, "beta22")].name, draft);
    assert_eq!(s.ed.pages[page_index(&s, "alpha1")].name, "Alpha");
    assert!(design_shown(&s), "then revealed");
    assert_eq!(s.ui.panels.design_choice(), Some(false));
}

#[test]
fn rev20_the_theme_view_shows_what_is_supported_and_names_what_is_blocked() {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let mut p = page("p", "Page 1", rect);
    let t = p.doc.alloc_id();
    let kind = NodeKind::Text {
        text: "Label".into(),
        font_size: 24.0,
        auto_width: true,
        align: crate::model::TextAlign::Left,
    };
    let r = crate::geom::Rect::new(400.0, 300.0, 60.0, 20.0);
    p.doc.nodes.insert(t, Node::new(t, kind, r));
    p.doc.root.push(t);
    let mut s = WebState::open(&canvas(&[p], "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    open_layers(&mut s);
    let left = s.canvas_rect().min.x;
    assert_eq!(s.ui.left_tab, crate::ui::LeftTab::Design);
    assert!(paints(&mut s, "Papers"), "Design: the Papers list");
    click_label_in(&mut s, "Theme", |r| r.max.x <= left, 1000.0);
    assert_eq!(s.ui.left_tab, crate::ui::LeftTab::Theme);
    assert!(!paints(&mut s, "Papers"), "Design's rows are hidden");
    let text: Vec<String> = painted_text(&mut s)
        .into_iter()
        .filter(|(_, r)| r.max.x <= left)
        .map(|(t, _)| t.trim().to_string())
        .collect();
    // The useful rows first, then one short status row each (run144: the long font paragraph
    // that filled a 320 × 240 sheet became a row; the reasons are tooltips and Details).
    for wanted in [
        "Text styles",
        "24 · Regular",
        "Colours",
        "Font",
        "Inter · 4 faces",
        "Named tokens",
        "Not available",
        "DESIGN.md",
        "Not connected",
        "Details",
    ] {
        assert!(text.iter().any(|t| t == wanted), "{wanted}: {text:?}");
    }
    let style_y = painted_at(&mut s, "24 · Regular", |r| r.max.x <= left).1;
    let status_y = painted_at(&mut s, "Named tokens", |r| r.max.x <= left).1;
    assert!(style_y < status_y, "the Paper's own styles come first");
    // Choosing the text style selects its layer, for the Design panel to edit; nothing in the
    // document changes, and Theme isn't the appearance.
    let before = page_to_json(&s.ed.page_data(0));
    click_label_in(&mut s, "24 · Regular", |r| r.max.x <= left, 2000.0);
    assert_eq!(s.ed.selection, vec![t]);
    assert_eq!(size_shown(&mut s), Some(24.0), "its Text section");
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
    assert!(!s.dirty() && s.can_reload());
    assert_eq!(s.appearance(), "system");
    click_label_in(&mut s, "Design", |r| r.max.x <= left, 3000.0);
    assert!(paints(&mut s, "Papers"), "back to the Papers list");
}

#[test]
fn rev20_switching_to_theme_finishes_a_paper_rename_on_its_own_paper() {
    let mut s = two_pages();
    let p = page_row(&mut s, "Beta");
    double_click(&mut s, p, 100.0);
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    let left = s.canvas_rect().min.x;
    click_label_in(&mut s, "Theme", |r| r.max.x <= left, 3000.0);
    assert!(
        s.ui.pages.renaming.is_none(),
        "finished, not kept for a hidden row"
    );
    let name = s.ed.pages[page_index(&s, "beta22")].name.clone();
    assert!(name != "Beta" && name.contains('q'), "{name}");
    assert_eq!(s.ed.pages[page_index(&s, "alpha1")].name, "Alpha");
    // Rename Paper… from the menu brings the Design view back for its one field.
    menu_rename_paper(&mut s, 4000.0);
    timed_frames(&mut s, 3);
    assert_eq!(s.ui.left_tab, crate::ui::LeftTab::Design);
    assert!(s.field_has_keyboard(), "the Papers list's field");
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 2);
    assert_eq!(s.ed.pages[page_index(&s, "beta22")].name, name);
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

/// The rectangle of [`state`], a second one drawn at (600, 100), an arrow drawn from the
/// first to the second (bound to both) and labelled "sends", the three grouped: all through
/// the browser's input. Returns (arrow, first, second, group).
fn labelled_arrow(s: &mut WebState) -> [crate::model::NodeId; 4] {
    let a = s.ed.doc.root[0];
    s.run(Action::Tool(Tool::Rect));
    let (from, to) = (at(s, 600.0, 100.0), at(s, 760.0, 180.0));
    timed_drag(s, from, to);
    let b = s.ed.selection[0];
    s.run(Action::Tool(Tool::Arrow));
    let (from, to) = (at(s, 200.0, 150.0), at(s, 680.0, 140.0));
    timed_drag(s, from, to);
    let arrow = s.ed.selection[0];
    let NodeKind::Connector { start, end, .. } = &s.ed.doc.get(arrow).unwrap().kind else {
        panic!("an arrow")
    };
    let ends = (start.map(|e| e.node), end.map(|e| e.node));
    assert_eq!(ends, (Some(a), Some(b)), "bound to both");
    s.run(Action::EditText);
    s.ed.handle(crate::editor::InputEvent::Text("sends".into()));
    press(s, "Escape", "Escape");
    s.ed.selection = vec![a, b, arrow];
    s.run(Action::Group);
    let g = s.ed.selection[0];
    timed_frames(s, 2);
    [arrow, a, b, g]
}

/// R15: the page's `paste` event while an arrow's label is being edited edits that label:
/// its text replaces the selected characters or goes in at the caret, as plain text, whatever
/// it looks like (Unicode, several lines, a layer clipboard's JSON, SVG). Never layers, SVG or
/// a new text layer: the same bound arrow in its group on the same page, nothing added, one
/// undo step for the edit. Controls: a paste on the canvas with no label edited still makes a
/// text layer, and a focused inspector field still takes the paste itself.
#[test]
fn a_browser_paste_while_editing_an_arrow_label_edits_that_label_as_plain_text() {
    let mut s = state();
    let [arrow, ..] = labelled_arrow(&mut s);
    let label = |s: &WebState| s.ed.doc.get(arrow).unwrap().kind.label().map(String::from);
    assert_eq!(label(&s).as_deref(), Some("sends"));
    // Everything but the arrow's label: the same nodes, bindings, group and order.
    let unlabelled = |s: &WebState| {
        let mut nodes = s.ed.doc.nodes.clone();
        if let NodeKind::Connector { label, .. } = &mut nodes.get_mut(&arrow).unwrap().kind {
            *label = None;
        }
        nodes
    };
    let rest = unlabelled(&s);
    let before = s.ed.doc.clone();
    let steps = s.ed.history.undo_len();
    // Enter's command edits it; Home, then Shift+→ twice selects its first two characters.
    s.ed.selection = vec![arrow];
    s.run(Action::EditText);
    timed_frames(&mut s, 1);
    press(&mut s, "Home", "Home");
    s.modifiers(true, false, false, false);
    press(&mut s, "ArrowRight", "ArrowRight");
    press(&mut s, "ArrowRight", "ArrowRight");
    s.modifiers(false, false, false, false);
    assert_eq!(s.ed.text_selection(), Some((0, 2)));
    assert!(
        !s.field_has_keyboard(),
        "the label is the editor's text, not an egui field"
    );
    s.paste("Ω🙂\r\nline 2");
    timed_frames(&mut s, 1);
    assert_eq!(label(&s).as_deref(), Some("Ω🙂\nline 2nds"));
    let caret = s.ed.text_edit.as_ref().map(|te| te.caret);
    assert_eq!(
        caret,
        Some("Ω🙂\nline 2".chars().count()),
        "after the pasted text"
    );
    // Then at the caret: a layer clipboard's JSON and SVG are only text here.
    let json = "{\"type\":\"paper-rs/clipboard\",\"nodes\":[]}";
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"9\"/></svg>";
    s.paste(json);
    s.paste(svg);
    timed_frames(&mut s, 1);
    let editing = s.ed.text_edit.as_ref().map(|te| te.id);
    assert_eq!(editing, Some(arrow), "still editing it");
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 1);
    assert_eq!(label(&s), Some(format!("Ω🙂\nline 2{json}{svg}nds")));
    assert_eq!(unlabelled(&s), rest, "nothing else changed or was added");
    assert_eq!(s.ed.pages.len(), 1);
    assert_eq!(s.ed.page_data(0).id, "p", "the same page");
    assert_eq!(s.ed.history.undo_len(), steps + 1, "one undo step");
    let after = s.ed.doc.clone();
    s.run(Action::Undo);
    assert_eq!(s.ed.doc, before);
    s.run(Action::Redo);
    assert_eq!(s.ed.doc, after);
    // Control: on the canvas, with no label edited, text still pastes as a new text layer.
    s.ed.selection.clear();
    let nodes = s.ed.doc.nodes.len();
    s.paste("Ω🙂 note");
    assert_eq!(s.ed.doc.nodes.len(), nodes + 1);
    let [t] = s.ed.selection[..] else {
        panic!("one pasted text layer")
    };
    assert!(matches!(
        &s.ed.doc.get(t).unwrap().kind,
        NodeKind::Text { text, .. } if text == "Ω🙂 note"
    ));
    assert_eq!(label(&s), Some(format!("Ω🙂\nline 2{json}{svg}nds")));
    // Control: a focused inspector field takes the paste itself, as before.
    let mut s = state();
    let id = open_number_field(&mut s, "X  ");
    let nodes = s.ed.doc.nodes.clone();
    s.paste("-150");
    step(&mut s);
    assert_eq!(axis(&s, id, "X  "), -150.0, "into the field");
    assert_eq!(s.ed.doc.nodes.len(), nodes.len(), "no layer pasted");
}

/// R43-F1 in the browser: Enter edits the arrow's label, Ctrl+Shift+L (as the page delivers
/// the keys) locks the arrow while the label stays open, and the page's `paste` event then
/// changes nothing: the label, the document, the selection and the undo history stay as Lock
/// left them, and nothing is pasted elsewhere. Unlocked again (control), the same paste edits
/// the label. The arrow is selected directly first (fixture, as in the test above).
#[test]
fn a_browser_paste_never_changes_the_label_of_an_arrow_locked_while_it_is_edited() {
    let mut s = state();
    let [arrow, ..] = labelled_arrow(&mut s);
    let label = |s: &WebState| s.ed.doc.get(arrow).unwrap().kind.label().map(String::from);
    let lock_keys = |s: &mut WebState| {
        s.modifiers(true, true, false, false);
        press(s, "L", "KeyL");
        s.modifiers(false, false, false, false);
        timed_frames(s, 1);
    };
    s.ed.selection = vec![arrow];
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 1);
    assert_eq!(s.ed.text_edit.as_ref().map(|te| te.id), Some(arrow));
    lock_keys(&mut s);
    assert!(
        s.ed.doc.get(arrow).unwrap().locked,
        "Lock reached the arrow"
    );
    assert!(s.ed.selection.is_empty(), "Lock empties the selection");
    assert_eq!(s.ed.text_edit.as_ref().map(|te| te.id), Some(arrow));
    let (doc, steps) = (s.ed.doc.clone(), s.ed.history.undo_len());
    s.paste("PASTED INTO LOCKED LABEL");
    timed_frames(&mut s, 1);
    assert_eq!(label(&s).as_deref(), Some("sends"), "the label is kept");
    assert_eq!(s.ed.doc, doc, "nothing pasted instead");
    assert_eq!(s.ed.history.undo_len(), steps, "nothing recorded");
    assert!(s.ed.selection.is_empty());
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 1);
    assert_eq!(label(&s).as_deref(), Some("sends"));
    assert!(s.ed.doc.get(arrow).unwrap().locked, "still locked");
    // Control: unlocked with the same keys, the same paste edits the label.
    s.ed.selection = vec![arrow];
    lock_keys(&mut s);
    assert!(!s.ed.doc.get(arrow).unwrap().locked, "unlocked");
    s.ed.selection = vec![arrow];
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 1);
    s.paste("PASTED");
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 1);
    assert_eq!(label(&s).as_deref(), Some("PASTED"));
}

/// R18/R15 in the browser: with the arrow locked (Ctrl+Shift+L) while Enter's edit of its
/// label is open, typing (keys as the page delivers them, modifiers released), Enter,
/// Backspace, Delete and the page's `cut` event change nothing: the label, the document, the
/// undo history, the selection and the edit with its range stay as they were, and the cut
/// event gets no text. Unlocked with the same keys, typing into the still-open edit changes
/// the label; a fresh edit types Unicode in one undo step (controls). The arrow and another
/// layer are selected directly (fixture).
#[test]
fn browser_typing_never_changes_a_label_locked_while_it_is_edited() {
    let mut s = state();
    let [arrow, a, ..] = labelled_arrow(&mut s);
    let label = |s: &WebState| s.ed.doc.get(arrow).unwrap().kind.label().map(String::from);
    let edit = |s: &WebState| s.ed.text_edit.as_ref().map(|t| (t.id, t.caret, t.anchor));
    let lock_keys = |s: &mut WebState| {
        s.modifiers(true, true, false, false);
        press(s, "L", "KeyL");
        s.modifiers(false, false, false, false);
        timed_frames(s, 1);
    };
    let type_keys = |s: &mut WebState, keys: &[(&str, &str)]| {
        for (k, code) in keys {
            press(s, k, code);
            timed_frames(s, 1);
        }
    };
    s.ed.selection = vec![arrow];
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 1);
    assert_eq!(s.ed.text_selection(), Some((0, 5)), "Enter selects all");
    lock_keys(&mut s);
    assert!(
        s.ed.doc.get(arrow).unwrap().locked,
        "Lock reached the arrow"
    );
    assert!(s.ed.selection.is_empty(), "Lock empties the selection");
    s.ed.selection = vec![a];
    let open = edit(&s);
    assert_eq!(open.map(|o| o.0), Some(arrow), "the edit is open");
    let (doc, steps) = (s.ed.doc.clone(), s.ed.history.undo_len());
    assert!(!s.field_has_keyboard());
    type_keys(
        &mut s,
        &[
            ("é", "KeyE"),
            ("x", "KeyX"),
            ("Enter", "Enter"),
            ("Backspace", "Backspace"),
            ("Delete", "Delete"),
        ],
    );
    assert_eq!(s.copy(true), None, "the cut event gets no text");
    timed_frames(&mut s, 1);
    assert_eq!(label(&s).as_deref(), Some("sends"), "the label is kept");
    assert_eq!(s.ed.doc, doc, "nothing changed or deleted");
    assert_eq!(s.ed.history.undo_len(), steps, "nothing recorded");
    assert_eq!(s.ed.selection, vec![a]);
    assert_eq!(edit(&s), open, "the edit as it was");
    // Unlocked with the same keys, the still-open edit types again.
    s.ed.selection = vec![arrow];
    lock_keys(&mut s);
    assert!(!s.ed.doc.get(arrow).unwrap().locked, "unlocked");
    type_keys(&mut s, &[("é", "KeyE")]);
    assert_eq!(label(&s).as_deref(), Some("é"));
    assert_eq!(edit(&s), Some((arrow, 1, None)), "the caret after it");
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 1);
    // Control: a fresh edit types Unicode over the whole label, in one undo step.
    let before = s.ed.doc.clone();
    let steps = s.ed.history.undo_len();
    s.ed.selection = vec![arrow];
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 1);
    type_keys(&mut s, &[("Ω", "KeyO"), ("🙂", "Unidentified")]);
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 1);
    assert_eq!(label(&s).as_deref(), Some("Ω🙂"));
    assert_eq!(s.ed.history.undo_len(), steps + 1, "one undo step");
    s.run(Action::Undo);
    assert_eq!(s.ed.doc, before);
}

/// R15 through the page's own clipboard events: the `copy` and `cut` events of a group holding
/// a labelled, bound arrow get the labelled payload (never the unversioned `paper-rs/clipboard`),
/// and its `paste` event in another canvas puts the group back with its label and bindings, in
/// one undo step. A payload of another version is refused there with a reason, nothing pasted
/// (no text layer either). An edit left open on a locked layer still takes neither the `paste`
/// nor the `cut` event (R45), with this payload too. The arrow is selected directly (fixture).
#[test]
fn browser_copy_cut_and_paste_events_carry_labelled_arrows_with_their_labels() {
    use crate::io::clipboard::{LABELLED_TYPE, LABELLED_VERSION, TYPE};
    let head = format!("{{\"type\":\"{LABELLED_TYPE}\",\"version\":{LABELLED_VERSION},");
    let mut s = state();
    let [arrow, .., g] = labelled_arrow(&mut s);
    s.ed.selection = vec![g];
    let doc0 = s.ed.doc.clone();
    let text = s.copy(false).expect("the copy event gets the payload");
    assert!(text.starts_with(&head), "{text}");
    assert!(!text.contains(&format!("\"type\":\"{TYPE}\"")));
    assert_eq!(s.ed.doc, doc0, "copying changes nothing");
    // Cut: the same payload, the group gone in one step, Undo brings it back.
    let steps = s.ed.history.undo_len();
    let cut = s.copy(true).expect("the cut event gets the payload");
    assert!(cut.starts_with(&head));
    assert!(s.ed.doc.get(g).is_none() && s.ed.doc.get(arrow).is_none());
    assert_eq!(s.ed.history.undo_len(), steps + 1);
    s.run(Action::Undo);
    assert_eq!(s.ed.doc, doc0);
    // Pasted in another canvas: the group, its label and its bindings, in one step.
    let mut t = state();
    let (before, steps) = (t.ed.doc.clone(), t.ed.history.undo_len());
    t.paste(&text);
    assert_eq!(t.ed.history.undo_len(), steps + 1, "one undo step");
    let [pg] = t.ed.selection[..] else {
        panic!("the group, pasted")
    };
    let inside = t.ed.doc.subtree(pg);
    let copy = *inside
        .iter()
        .find(|id| t.ed.doc.get(**id).unwrap().kind.label().is_some())
        .expect("the labelled arrow");
    let label = |t: &WebState| t.ed.doc.get(copy).unwrap().kind.label().map(String::from);
    assert_eq!(label(&t).as_deref(), Some("sends"));
    let NodeKind::Connector { start, end, .. } = &t.ed.doc.get(copy).unwrap().kind else {
        panic!("an arrow")
    };
    let bound = |b: &Option<crate::model::Binding>| b.is_some_and(|b| inside.contains(&b.node));
    assert!(bound(start) && bound(end), "bound to the pasted shapes");
    let pasted = t.ed.doc.clone();
    t.run(Action::Undo);
    assert_eq!(t.ed.doc, before);
    t.run(Action::Redo);
    assert_eq!(t.ed.doc, pasted);
    // Another version: refused with a reason, nothing pasted, no text layer.
    let newer = text.replacen("\"version\":1,", "\"version\":2,", 1);
    let steps = t.ed.history.undo_len();
    t.paste(&newer);
    assert!(t.ed.status.contains("can't be pasted"), "{}", t.ed.status);
    assert_eq!(t.ed.doc, pasted, "nothing pasted");
    assert_eq!(t.ed.history.undo_len(), steps);
    // R48-F1: a member named twice (raw text, `nodes` again after the header): refused too.
    let twice = text.replacen(&head, &format!("{head}\"nodes\":[],"), 1);
    t.paste(&twice);
    assert!(t.ed.status.contains("named twice"), "{}", t.ed.status);
    assert_eq!(t.ed.doc, pasted, "nothing pasted");
    assert_eq!(t.ed.history.undo_len(), steps);
    // R47-F2: the next good paste says so; the refusal's note doesn't stay.
    t.paste(&text);
    assert_eq!(t.ed.status, "Pasted 1 layer(s)", "the paste is said");
    assert_eq!(t.ed.history.undo_len(), steps + 1);
    t.run(Action::Undo);
    assert_eq!(t.ed.doc, pasted);
    // An edit left open on the pasted arrow, locked by the keys: no paste, no cut.
    t.ed.selection = vec![copy];
    press(&mut t, "Enter", "Enter");
    timed_frames(&mut t, 1);
    lock_keys(&mut t);
    assert!(t.ed.doc.get(copy).unwrap().locked, "Lock reached the arrow");
    let editing = t.ed.text_edit.as_ref().map(|e| e.id);
    assert_eq!(editing, Some(copy), "still edited");
    let (doc, steps) = (t.ed.doc.clone(), t.ed.history.undo_len());
    t.paste(&text);
    assert_eq!(t.copy(true), None, "the cut event gets no text");
    timed_frames(&mut t, 1);
    assert_eq!(t.ed.doc, doc, "nothing pasted or cut");
    assert_eq!(t.ed.history.undo_len(), steps);
    assert_eq!(label(&t).as_deref(), Some("sends"));
}

/// R15 curve re-edit in the browser, through the page's own pointer events: a bound arrow
/// drawn and bent from its middle has, still selected, a bend handle on its bend point, and a
/// diagonal drag from it moves that point by the drag in both axes (before, it was a box
/// resize that ignored x), the ends still bound, in one undo step that Undo takes back.
#[test]
fn browser_a_bent_arrow_re_bends_from_its_bend_point_in_both_axes() {
    let mut s = state();
    let a = s.ed.doc.root[0];
    s.run(Action::Tool(Tool::Rect));
    let (from, to) = (at(&s, 600.0, 100.0), at(&s, 760.0, 180.0));
    timed_drag(&mut s, from, to);
    let b = s.ed.selection[0];
    s.run(Action::Tool(Tool::Arrow));
    let (from, to) = (at(&s, 200.0, 150.0), at(&s, 680.0, 140.0));
    timed_drag(&mut s, from, to);
    let arrow = s.ed.selection[0];
    let handle = |s: &WebState| {
        let handles = s.ed.connector_handles();
        let (_, p) = handles.into_iter().find(|(i, _)| *i == 2)?;
        Some(at(s, p.x, p.y))
    };
    let bend = |s: &WebState| s.ed.connector_points(arrow).unwrap();
    let ends = |s: &WebState| match &s.ed.doc.get(arrow).unwrap().kind {
        NodeKind::Connector { start, end, .. } => (start.map(|b| b.node), end.map(|b| b.node)),
        _ => unreachable!(),
    };
    assert_eq!(ends(&s), (Some(a), Some(b)), "bound to both");
    let mid = handle(&s).expect("the straight arrow's bend handle");
    timed_drag(&mut s, mid, (mid.0, mid.1 + 200.0));
    assert_eq!(bend(&s).len(), 3, "bent");
    let steps = s.ed.history.undo_len();
    let apex = handle(&s).expect("a bend handle on the curve");
    let before = bend(&s)[1];
    timed_drag(&mut s, apex, (apex.0 + 80.0, apex.1 + 60.0));
    let after = bend(&s)[1];
    let moved = after - before;
    assert!((moved.x - 80.0).abs() < 0.01, "x: {moved:?}");
    assert!((moved.y - 60.0).abs() < 0.01, "y: {moved:?}");
    assert_eq!(bend(&s).len(), 3, "still one bend");
    assert_eq!(ends(&s), (Some(a), Some(b)), "still bound");
    assert_eq!(s.ed.history.undo_len(), steps + 1, "one undo step");
    s.run(Action::Undo);
    assert!(bend(&s)[1].dist(before) < 1e-9, "Undo takes it back");
}

/// R15 multi-bend re-edit in the browser, through the page's own pointer events: a selected
/// curve with two bends (made directly here, as an agent's would be) has a bend handle on
/// each, and a drag from the second moves that bend alone by the drag in both axes; the
/// first bend, the ends, the point count and the label stay, in one undo step that Undo
/// takes back.
#[test]
fn browser_each_bend_of_a_two_bend_curve_moves_alone() {
    use crate::model::{Head, Route};
    let mut s = state();
    let pts = [
        crate::geom::v2(400.0, 300.0),
        crate::geom::v2(500.0, 220.0),
        crate::geom::v2(600.0, 380.0),
        crate::geom::v2(700.0, 300.0),
    ];
    let lo = crate::geom::v2(400.0, 220.0);
    let kind = NodeKind::Connector {
        points: pts.iter().map(|p| *p - lo).collect(),
        start: None,
        end: None,
        route: Route::Curved,
        start_head: Head::None,
        end_head: Head::Arrow,
        label: Some("flows".into()),
    };
    let r = crate::geom::Rect::new(400.0, 220.0, 300.0, 160.0);
    let arrow =
        s.ed.add_node(Node::new(crate::model::NodeId(0), kind, r), None);
    s.ed.selection = vec![arrow];
    timed_frames(&mut s, 1);
    let points = |s: &WebState| s.ed.connector_points(arrow).unwrap();
    let handles = s.ed.connector_handles();
    assert_eq!(handles.len(), 4, "two ends and two bends");
    let (_, second) = handles[3];
    assert!(second.dist(pts[2]) < 1e-9, "the second bend's handle");
    let steps = s.ed.history.undo_len();
    let from = at(&s, second.x, second.y);
    timed_drag(&mut s, from, (from.0 + 60.0, from.1 + 40.0));
    let after = points(&s);
    assert_eq!(after.len(), 4, "still two bends");
    let moved = after[2] - pts[2];
    assert!((moved.x - 60.0).abs() < 0.01, "x: {moved:?}");
    assert!((moved.y - 40.0).abs() < 0.01, "y: {moved:?}");
    assert!(
        after[1].dist(pts[1]) < 1e-6,
        "the first bend stays: {after:?}"
    );
    assert!(after[0].dist(pts[0]) < 1e-6 && after[3].dist(pts[3]) < 1e-6);
    let label = s.ed.doc.get(arrow).unwrap().kind.label().map(String::from);
    assert_eq!(label.as_deref(), Some("flows"));
    assert_eq!(s.ed.history.undo_len(), steps + 1, "one undo step");
    s.run(Action::Undo);
    assert_eq!(points(&s), pts.to_vec(), "Undo takes it back");
}

/// R51 in the browser: a bent arrow's bend held (pointer down on its bend handle, not yet
/// dragged) while Ctrl+Shift+L locks it: the later move and release, as the page delivers
/// them, change nothing and record nothing, the arrow stays locked and nothing is busy.
#[test]
fn browser_a_bend_held_while_its_arrow_is_locked_changes_nothing_more() {
    let mut s = state();
    s.run(Action::Tool(Tool::Arrow));
    let (from, to) = (at(&s, 400.0, 300.0), at(&s, 700.0, 300.0));
    timed_drag(&mut s, from, to);
    let arrow = s.ed.selection[0];
    let handle = |s: &WebState| {
        let handles = s.ed.connector_handles();
        let (_, p) = handles.into_iter().find(|(i, _)| *i == 2)?;
        Some(at(s, p.x, p.y))
    };
    let mid = handle(&s).expect("the straight arrow's bend handle");
    timed_drag(&mut s, mid, (mid.0, mid.1 + 150.0));
    let apex = handle(&s).expect("a bend handle on the curve");
    s.tick(s.now_ms + 1000.0);
    s.pointer_move(apex.0, apex.1, s.now_ms);
    s.pointer_down(apex.0, apex.1, Button::Primary, s.now_ms);
    timed_frames(&mut s, 1);
    assert!(s.ed.is_busy(), "the bend is held");
    lock_keys(&mut s);
    assert!(
        s.ed.doc.get(arrow).unwrap().locked,
        "Lock reached the arrow"
    );
    let (doc, steps) = (s.ed.doc.clone(), s.ed.history.undo_len());
    let late = (apex.0 + 80.0, apex.1 + 60.0);
    s.pointer_move(late.0, late.1, s.now_ms + 16.0);
    timed_frames(&mut s, 1);
    s.pointer_up(late.0, late.1, Button::Primary, s.now_ms);
    timed_frames(&mut s, 2);
    assert_eq!(s.ed.doc, doc, "nothing bent after the Lock");
    assert_eq!(s.ed.history.undo_len(), steps, "nothing recorded");
    assert!(!s.ed.is_busy(), "the gesture ended");
    assert!(s.ed.doc.get(arrow).unwrap().locked, "still locked");
}

/// Ctrl+Shift+L as the page delivers it, then the modifiers released.
fn lock_keys(s: &mut WebState) {
    s.modifiers(true, true, false, false);
    press(s, "L", "KeyL");
    s.modifiers(false, false, false, false);
    timed_frames(s, 1);
}

/// R18/R15 in the browser, a text layer: locked (Ctrl+Shift+L) while its edit is open, with
/// another layer then selected, the page's `cut` and `paste` events change nothing: no text
/// for the cut event, the other layer neither cut nor joined by a pasted one, the document,
/// history, selection and the edit as they were. Escape leaves the edit and keeps the text.
/// Unlocked with the same keys, a fresh edit types again (control). The page holds one text
/// layer and one rectangle; the text and then the rectangle are selected directly (fixture).
#[test]
fn browser_cut_and_paste_on_a_locked_text_edit_change_no_layer() {
    let mut doc = Document::default();
    let t = doc.alloc_id();
    let kind = NodeKind::Text {
        text: "Orders".into(),
        font_size: 24.0,
        auto_width: true,
        align: crate::model::TextAlign::Left,
    };
    let r = crate::geom::Rect::new(100.0, 100.0, 80.0, 30.0);
    doc.nodes.insert(t, Node::new(t, kind, r));
    doc.root.push(t);
    let other = doc.alloc_id();
    let r = crate::geom::Rect::new(400.0, 300.0, 100.0, 60.0);
    doc.nodes.insert(other, Node::new(other, NodeKind::Rect, r));
    doc.root.push(other);
    let page = PageData {
        id: "p".into(),
        name: "Page 1".into(),
        doc,
        camera: Some(Default::default()),
    };
    let mut s = WebState::open(&canvas(&[page], "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    let text = |s: &WebState| match &s.ed.doc.get(t).unwrap().kind {
        NodeKind::Text { text, .. } => text.clone(),
        k => panic!("not text: {k:?}"),
    };
    let edit = |s: &WebState| s.ed.text_edit.as_ref().map(|e| (e.id, e.caret, e.anchor));
    s.ed.selection = vec![t];
    press(&mut s, "Enter", "Enter");
    press(&mut s, "End", "End");
    timed_frames(&mut s, 1);
    assert_eq!(edit(&s), Some((t, 6, None)), "edited, caret at its end");
    lock_keys(&mut s);
    assert!(s.ed.doc.get(t).unwrap().locked, "Lock reached the text");
    assert!(s.ed.selection.is_empty(), "Lock empties the selection");
    s.ed.selection = vec![other];
    let (doc, steps, open) = (s.ed.doc.clone(), s.ed.history.undo_len(), edit(&s));
    assert_eq!(s.copy(true), None, "the cut event gets no text");
    s.paste("PASTED");
    timed_frames(&mut s, 1);
    assert!(s.ed.doc.get(other).is_some(), "the other layer isn't cut");
    assert_eq!(s.ed.doc, doc, "nothing cut, deleted or pasted");
    assert_eq!(s.ed.history.undo_len(), steps, "nothing recorded");
    assert_eq!(s.ed.selection, vec![other]);
    assert_eq!(edit(&s), open, "the edit as it was");
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 1);
    assert_eq!(edit(&s), None, "Escape leaves the edit");
    assert_eq!(s.ed.doc, doc, "and changes nothing");
    // Control: unlocked with the same keys, a fresh edit types at its caret.
    s.ed.selection = vec![t];
    lock_keys(&mut s);
    assert!(!s.ed.doc.get(t).unwrap().locked, "unlocked");
    s.ed.selection = vec![t];
    press(&mut s, "Enter", "Enter");
    press(&mut s, "End", "End");
    press(&mut s, "!", "Digit1");
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 1);
    assert_eq!(text(&s), "Orders!");
}

/// R18/R15 in the browser, an arrow's label: emptied and typed blank ("   ") through the page's
/// keys, then locked while its edit is open, Escape leaves the edit and keeps the blank label
/// (and the arrow's bindings): the document is as Lock left it. Unlocked, an edit ended on the
/// blank label clears it, as before (control). The arrow is selected directly (fixture).
#[test]
fn browser_escape_keeps_a_blank_label_on_an_arrow_locked_while_it_is_edited() {
    let mut s = state();
    let [arrow, ..] = labelled_arrow(&mut s);
    let label = |s: &WebState| s.ed.doc.get(arrow).unwrap().kind.label().map(String::from);
    s.ed.selection = vec![arrow];
    press(&mut s, "Enter", "Enter");
    press(&mut s, "Backspace", "Backspace");
    for _ in 0..3 {
        press(&mut s, " ", "Space");
    }
    timed_frames(&mut s, 1);
    assert_eq!(label(&s).as_deref(), Some("   "), "typed blank");
    lock_keys(&mut s);
    assert!(s.ed.doc.get(arrow).unwrap().locked, "locked");
    let editing = s.ed.text_edit.as_ref().map(|e| e.id);
    assert_eq!(editing, Some(arrow), "still edited");
    let doc = s.ed.doc.clone();
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 1);
    assert!(s.ed.text_edit.is_none(), "Escape leaves the edit");
    assert_eq!(s.ed.doc, doc, "the locked blank label stays as it was");
    assert_eq!(label(&s).as_deref(), Some("   "));
    // Control: unlocked, an edit ended on the blank label clears it.
    s.ed.selection = vec![arrow];
    lock_keys(&mut s);
    assert!(!s.ed.doc.get(arrow).unwrap().locked, "unlocked");
    press(&mut s, "Enter", "Enter");
    press(&mut s, "Escape", "Escape");
    timed_frames(&mut s, 1);
    assert_eq!(label(&s), None, "cleared, as before");
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

// ---- the Library tab's Add and its own refusal (R39-F1)

/// Every text the layout painted, with its rect (CSS px).
fn painted_texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for s in shapes {
                painted_texts(s, out);
            }
        }
        egui::Shape::Text(t) => out.push((
            t.galley.job.text.clone(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        _ => {}
    }
}

/// One frame at `t` (see [`step`]), returning what it painted as text.
fn painted_frame(s: &mut WebState, t: &mut f64) -> Vec<(String, egui::Rect)> {
    *t += 50.0;
    s.tick(*t);
    let mut l = s.layout();
    let mut out = Vec::new();
    for c in l.output.shapes {
        painted_texts(&c.shape, &mut out);
    }
    l.output.textures_delta.clear();
    s.drawn();
    out
}

/// A click on the Library tab's real "+ Add selection" button, as laid out.
fn library_add(s: &mut WebState, t: &mut f64) {
    let shown = painted_frame(s, t);
    let p = shown
        .iter()
        .find(|(text, _)| text == "+ Add selection")
        .expect("the Library tab's Add button is laid out")
        .1
        .center();
    s.pointer_down(p.x as f64, p.y as f64, Button::Primary, *t + 1.0);
    painted_frame(s, t);
    s.pointer_up(p.x as f64, p.y as f64, Button::Primary, *t + 1.0);
    for _ in 0..4 {
        painted_frame(s, t);
    }
}

/// R39-F1: a successful Add resolves its own earlier "Nothing was added" refusal every time,
/// not only the first time (when the browser's one-time session note replaces it). Refusals
/// still add nothing and say so; the note isn't repeated; a newer, unrelated note survives a
/// successful Add; the document and its Undo never change.
#[test]
fn a_successful_library_add_clears_its_own_earlier_refusal_on_every_retry() {
    use crate::geom::{Rect, v2};
    use crate::model::{Head, NodeId, Route};
    let refused = crate::library::LABELS_REFUSED;
    let mut doc = Document::default();
    let mut user = Node::new(
        NodeId(1),
        NodeKind::Rect,
        Rect::new(50.0, 50.0, 120.0, 60.0),
    );
    user.name = "User".into();
    let kind = NodeKind::Connector {
        points: vec![v2(0.0, 0.0), v2(78.0, 0.0)],
        start: None,
        end: None,
        route: Route::Straight,
        start_head: Head::None,
        end_head: Head::Arrow,
        label: Some("dispatches".into()),
    };
    let arrow = Node::new(NodeId(2), kind, Rect::new(346.0, 210.0, 78.0, 0.0));
    doc.nodes.insert(user.id, user);
    doc.nodes.insert(arrow.id, arrow);
    doc.root = vec![NodeId(1), NodeId(2)];
    doc.next_id = 3;
    let p = PageData {
        id: "r39page".into(),
        name: "R39".into(),
        doc,
        camera: Some(Default::default()),
    };
    let mut s = WebState::open(&canvas(&[p], "r39page"), false, W, H).unwrap();
    let mut t = settle(&mut s, 0.0);
    open_layers(&mut s);
    s.ui.show_library = true;
    for _ in 0..5 {
        painted_frame(&mut s, &mut t);
    }
    let before = s.ed.doc.clone();
    let undo = s.ed.history.undo_len();
    let attempt = |s: &mut WebState, t: &mut f64, id: u64| {
        s.ed.select(vec![NodeId(id)]);
        library_add(s, t);
        assert_eq!(s.ed.doc, before, "the document never changes");
        assert_eq!(s.ed.history.undo_len(), undo, "nor its Undo");
        assert!(!s.ui.library.dirty, "the change was taken by the host");
        let shown = painted_frame(s, t);
        let painted = shown.iter().any(|(text, _)| text == refused);
        (
            s.ui.library.library.items.len(),
            s.ed.status.clone(),
            painted,
        )
    };
    // 1. Labelled: refused, said, nothing added.
    let (items, status, painted) = attempt(&mut s, &mut t, 2);
    assert_eq!((items, status.as_str(), painted), (0, refused, true));
    // 2. Plain: added; the one-time session note replaces the refusal (the passing first case).
    let (items, status, painted) = attempt(&mut s, &mut t, 1);
    assert_eq!(
        (items, status.as_str(), painted),
        (1, LIBRARY_SESSION, false)
    );
    // 3. Labelled again: refused again, one card still.
    let (items, status, painted) = attempt(&mut s, &mut t, 2);
    assert_eq!((items, status.as_str(), painted), (1, refused, true));
    // 4. Plain again: added, and its own refusal is gone (it was R39's failure), without
    // repeating the session note.
    let (items, status, painted) = attempt(&mut s, &mut t, 1);
    assert_eq!(items, 2, "a second card");
    assert_ne!(status, refused, "no stale Nothing was added");
    assert!(!painted, "nor painted");
    assert!(
        status.is_empty(),
        "the session note was said once: {status:?}"
    );
    // 5. A newer, unrelated note (a real command's) survives a successful Add.
    let (items, _, _) = attempt(&mut s, &mut t, 2);
    assert_eq!(items, 2);
    s.run(Action::ToggleSnapping);
    let newer = s.ed.status.clone();
    assert!(newer.starts_with("Snapping"), "{newer:?}");
    let (items, status, painted) = attempt(&mut s, &mut t, 1);
    assert_eq!(items, 3);
    assert_eq!(status, newer, "someone else's note stays");
    assert!(!painted);
    s.run(Action::ToggleSnapping);
}

// ---- revision 21: Theme changes a colour in use, through upstream's Theme view and egui's own
// colour picker, laid out headless (egui layout only: no GPU, browser or native proof).

const RED: Color = Color::rgb(255, 0, 0);
const GREEN: Color = Color::rgb(0, 255, 0);
const BLUE: Color = Color::rgb(0, 0, 255);
const CYAN: Color = Color::rgb(0, 255, 255);
const YELLOW: Color = Color::rgb(255, 255, 0);

/// The layers of [`theme_pages`]' shown Paper.
struct ThemeIds {
    /// Red as the lower of two fills (blue above it).
    a: crate::model::NodeId,
    /// A green ellipse with a red 3 px stroke.
    b: crate::model::NodeId,
    /// A locked red rectangle.
    c: crate::model::NodeId,
    /// A red rectangle inside the locked group `g`.
    d: crate::model::NodeId,
    /// A gradient from red to white.
    e: crate::model::NodeId,
    g: crate::model::NodeId,
    /// Black text whose characters are a red run.
    t: crate::model::NodeId,
}

/// A layer of `kind` at `(x, y, w, h)` at the top of `doc`, then `set` on it.
fn theme_layer(
    doc: &mut Document,
    kind: NodeKind,
    (x, y, w, h): (f64, f64, f64, f64),
    set: impl FnOnce(&mut Node),
) -> crate::model::NodeId {
    let id = doc.alloc_id();
    let mut n = Node::new(id, kind, crate::geom::Rect::new(x, y, w, h));
    set(&mut n);
    doc.nodes.insert(id, n);
    doc.root.push(id);
    id
}

/// Two Papers: "Page 1" (`p`, shown) uses red in every way [`ThemeIds`] lists; "Other" (`q`)
/// has one red rectangle, whose id is `a`'s (ids are per Paper).
fn theme_pages() -> (Vec<PageData>, ThemeIds) {
    use crate::model::{Paint, TextSpan};
    let solid = |c: Color| vec![Paint::solid(c)];
    let mut doc = Document::default();
    let a = theme_layer(&mut doc, NodeKind::Rect, (100.0, 100.0, 80.0, 60.0), |n| {
        n.style.fills = vec![Paint::solid(RED), Paint::solid(BLUE)];
    });
    let b = theme_layer(
        &mut doc,
        NodeKind::Ellipse,
        (250.0, 100.0, 80.0, 60.0),
        |n| {
            n.style.fills = solid(GREEN);
            n.style.stroke = Some(RED);
            n.style.stroke_width = 3.0;
        },
    );
    let c = theme_layer(&mut doc, NodeKind::Rect, (400.0, 100.0, 40.0, 40.0), |n| {
        n.style.fills = solid(RED);
        n.locked = true;
    });
    let g = theme_layer(&mut doc, NodeKind::Group, (500.0, 100.0, 40.0, 40.0), |n| {
        n.locked = true;
    });
    let d = theme_layer(&mut doc, NodeKind::Rect, (0.0, 0.0, 40.0, 40.0), |n| {
        n.style.fills = solid(RED);
        n.parent = Some(g);
    });
    doc.root.retain(|id| *id != d);
    doc.nodes.get_mut(&g).unwrap().children.push(d);
    let e = theme_layer(&mut doc, NodeKind::Rect, (100.0, 250.0, 80.0, 60.0), |n| {
        n.style.fills = vec![Paint::linear(RED, Color::WHITE)];
    });
    let text = NodeKind::Text {
        text: "Label".into(),
        font_size: 24.0,
        auto_width: true,
        align: crate::model::TextAlign::Left,
    };
    let t = theme_layer(&mut doc, text, (250.0, 250.0, 60.0, 20.0), |n| {
        n.style.fills = solid(Color::BLACK);
        n.text_style.spans.push(TextSpan {
            start: 0,
            end: 5,
            color: Some(RED),
            ..Default::default()
        });
    });
    let shown = PageData {
        id: "p".into(),
        name: "Page 1".into(),
        doc,
        camera: Some(Default::default()),
    };
    let mut other = page(
        "q",
        "Other",
        crate::geom::Rect::new(100.0, 100.0, 80.0, 60.0),
    );
    for n in other.doc.nodes.values_mut() {
        n.style.fills = solid(RED);
    }
    assert_eq!(other.doc.root, vec![a], "the other Paper reuses a's id");
    let ids = ThemeIds {
        a,
        b,
        c,
        d,
        e,
        g,
        t,
    };
    (vec![shown, other], ids)
}

/// [`theme_pages`] opened on "Page 1", laid out, the left panel open; with the other Paper as
/// it was given.
fn theme_fixture() -> (WebState, ThemeIds, PageData) {
    let (pages, ids) = theme_pages();
    let mut s = WebState::open(&canvas(&pages, "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    open_layers(&mut s);
    (s, ids, pages[1].clone())
}

/// The left panel's Theme view.
fn open_theme(s: &mut WebState) {
    let left = s.canvas_rect().min.x;
    let t = s.now_ms + 1000.0;
    click_label_in(s, "Theme", |r| r.max.x <= left, t);
    assert_eq!(s.ui.left_tab, crate::ui::LeftTab::Theme);
}

/// The swatch of Theme's colour row `hex`: 14 px, centred 15 px before the row's painted label.
fn theme_swatch(s: &mut WebState, hex: &str) -> (f64, f64) {
    let left = s.canvas_rect().min.x;
    let text = painted_text(s);
    let (_, r) = text
        .iter()
        .find(|(t, r)| t.trim() == hex && r.max.x <= left)
        .unwrap_or_else(|| panic!("{hex} isn't a Theme row: {text:?}"));
    (r.min.x as f64 - 15.0, r.center().y as f64)
}

/// A click on row `hex`'s swatch, which opens its colour picker.
fn open_picker(s: &mut WebState, hex: &str) {
    let p = theme_swatch(s, hex);
    timed_click(s, p);
    assert!(ui::popup_open(&s.ctx), "{hex}: its picker opened");
}

/// `value` typed into the open picker's `channel` field ("R ", "G ", "B ") with
/// [`picker_field`]: the paste replaces its selected text (applied as typed) and Enter
/// finishes it. The picker stays open.
fn picker_channel(s: &mut WebState, channel: &str, value: &str) {
    picker_field(s, channel);
    s.paste(value);
    step(s);
    press(s, "Enter", "Enter");
    timed_frames(s, 3);
    assert!(ui::popup_open(&s.ctx), "{channel:?}: the picker stays open");
}

/// The open picker's or style editor's `channel` field ("R ", "G ", "B ", or "Size ") in its
/// text mode, its text selected: a click puts egui's drag value in its text mode and Ctrl+A
/// selects its text. egui paints the field's prefix and its number as two texts on one line,
/// the number right after the prefix (run158's observed layout: "R " then "255"; run162's:
/// "Size " then "24.0"); the field is the two together, clicked at their middle. A colour
/// channel's number is a whole 0–255; the size's is a finite decimal.
fn picker_field(s: &mut WebState, channel: &str) {
    let valid = |number: &str| match channel {
        "Size " => number.trim().parse::<f64>().is_ok_and(f64::is_finite),
        _ => number.trim().parse::<u8>().is_ok(),
    };
    let text = painted_text(s);
    let r = text
        .windows(2)
        .rev()
        .find(|w| {
            let ((prefix, p), (number, n)) = (&w[0], &w[1]);
            prefix.as_str() == channel
                && valid(number)
                && (n.center().y - p.center().y).abs() < 1.0
                && (n.min.x - p.max.x).abs() < 1.0
        })
        .map(|w| w[0].1.union(w[1].1))
        .unwrap_or_else(|| panic!("no {channel:?} field painted: {text:?}"));
    timed_click(s, centre(r));
    assert!(s.field_has_keyboard(), "{channel:?}: its text mode");
    s.modifiers(false, true, false, false);
    press(s, "a", "KeyA");
    s.modifiers(false, false, false, false);
    step(s);
}

/// A press on blank canvas: it closes the open picker and never reaches the editor.
fn close_picker(s: &mut WebState) {
    let selection = s.ed.selection.clone();
    let p = at(s, 700.0, 500.0);
    timed_click(s, p);
    assert!(!ui::popup_open(&s.ctx), "a press outside closes the picker");
    assert_eq!(s.ed.selection, selection, "the press stayed the picker's");
}

#[test]
fn rev21_a_theme_colour_changes_where_it_was_used_in_one_step_and_saves_exactly() {
    use crate::model::Paint;
    let (mut s, ids, other) = theme_fixture();
    let before = s.ed.doc.clone();
    open_theme(&mut s);
    open_picker(&mut s, "#ff0000");
    assert!(
        s.ed.selection.is_empty(),
        "the swatch isn't the row: nothing is selected"
    );
    // Keys belong to the open picker, never the editor.
    let tool = s.tool();
    for (key, code) in [("r", "KeyR"), ("Delete", "Delete")] {
        assert!(s.key_down(key, code, false));
        s.key_up(key, code);
    }
    timed_frames(&mut s, 2);
    assert_eq!(s.tool(), tool);
    assert!(ui::popup_open(&s.ctx));
    assert_eq!(s.ed.doc, before, "nothing changed yet");
    // One picker use: red to black (the text's fill), to green (the ellipse's own fill), to
    // cyan. Each step passes a colour other layers already use; the rows merge and move, and
    // the picker stays open on the places that were red when it began.
    let node = |s: &WebState, id| s.ed.doc.get(id).unwrap().clone();
    picker_channel(&mut s, "R ", "0");
    let a = node(&s, ids.a).style.fills;
    assert_eq!(a, [Paint::solid(Color::BLACK), Paint::solid(BLUE)]);
    assert_eq!(node(&s, ids.b).style.stroke, Some(Color::BLACK));
    picker_channel(&mut s, "G ", "255");
    assert_eq!(node(&s, ids.a).style.fills[0], Paint::solid(GREEN));
    assert_eq!(node(&s, ids.b).style.stroke, Some(GREEN));
    picker_channel(&mut s, "B ", "255");
    close_picker(&mut s);
    let after = s.ed.doc.clone();
    // Only those places changed: the blue fill above, the ellipse's own green fill, the
    // text's black fill and red run, the locked layers, the gradient and the other Paper stay.
    let mut a = before.get(ids.a).unwrap().clone();
    a.style.fills = vec![Paint::solid(CYAN), Paint::solid(BLUE)];
    assert_eq!(after.get(ids.a), Some(&a));
    let mut b = before.get(ids.b).unwrap().clone();
    b.style.stroke = Some(CYAN);
    assert_eq!(after.get(ids.b), Some(&b));
    assert_eq!(b.style.fills, [Paint::solid(GREEN)]);
    for id in [ids.c, ids.d, ids.e, ids.g, ids.t] {
        assert_eq!(after.get(id), before.get(id), "{id:?}");
    }
    assert_eq!(
        (&after.root, after.nodes.len()),
        (&before.root, before.nodes.len())
    );
    assert_eq!(s.ed.page_data(1).doc.nodes, other.doc.nodes);
    assert!(paints(&mut s, "#ff0000"), "the locked layers keep red");
    // The whole picker use is one undo step; Redo brings it back.
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes, before.nodes, "one step");
    s.run(Action::Redo);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes, after.nodes);
    // A second picker use is a step of its own.
    open_picker(&mut s, "#00ffff");
    picker_channel(&mut s, "R ", "255");
    close_picker(&mut s);
    assert_eq!(node(&s, ids.a).style.fills[0], Paint::solid(Color::WHITE));
    assert_eq!(node(&s, ids.b).style.stroke, Some(Color::WHITE));
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes, after.nodes, "only the second use undone");
    s.run(Action::Redo);
    step(&mut s);
    let last = s.ed.doc.clone();
    // A row only locked layers use shows its swatch, but no picker.
    let p = theme_swatch(&mut s, "#ff0000");
    timed_click(&mut s, p);
    assert!(!ui::popup_open(&s.ctx));
    assert_eq!(s.ed.doc, last);
    // Saved exactly as edited (the shown Paper only), and reopened as saved.
    assert!(s.dirty());
    let snap = s.save_snapshot().expect("the change is saved");
    let sent = sent_page(&snap);
    assert_eq!(page_to_json(&sent), page_to_json(&s.ed.page_data(0)));
    s.save_succeeded();
    assert!(!s.dirty());
    let reopened = WebState::open(&canvas(&pages_now(&s), "p"), false, W, H).unwrap();
    assert_eq!(
        (&reopened.ed.doc.nodes, &reopened.ed.doc.root),
        (&last.nodes, &last.root)
    );
    assert_eq!(reopened.ed.page_data(1).doc.nodes, other.doc.nodes);
}

#[test]
fn rev21_a_reload_under_an_open_picker_ends_it_before_it_can_change_the_new_document() {
    use crate::model::Paint;
    let (mut s, ids, _) = theme_fixture();
    open_theme(&mut s);
    open_picker(&mut s, "#ff0000");
    picker_channel(&mut s, "R ", "0");
    assert_eq!(
        s.ed.doc.get(ids.a).unwrap().style.fills[0],
        Paint::solid(Color::BLACK)
    );
    // The explicit discard (the conflict banner's reload) loads another document whose
    // layers reuse a's and b's ids and already have the picker's current colour there.
    let (mut pages, _) = theme_pages();
    let doc = &mut pages[0].doc;
    doc.nodes.get_mut(&ids.a).unwrap().style.fills = vec![Paint::solid(Color::BLACK)];
    doc.nodes.get_mut(&ids.b).unwrap().style.stroke = Some(Color::BLACK);
    let loaded = pages[0].doc.clone();
    s.reload(&canvas(&pages, "p")).unwrap();
    timed_frames(&mut s, 3);
    assert!(
        !ui::popup_open(&s.ctx),
        "the picker closed with the document it was for"
    );
    let text = painted_text(&mut s);
    assert!(!text.iter().any(|(t, _)| t.starts_with("R ")), "{text:?}");
    assert_eq!(
        s.ed.doc.nodes, loaded.nodes,
        "nothing of it reached the new one"
    );
    assert!(!s.dirty());
    // A new picker use there starts from what that document has: the ellipse's green fill
    // only, not its black stroke or the black rectangle.
    open_picker(&mut s, "#00ff00");
    picker_channel(&mut s, "R ", "255");
    close_picker(&mut s);
    let mut b = loaded.get(ids.b).unwrap().clone();
    b.style.fills = vec![Paint::solid(YELLOW)];
    assert_eq!(s.ed.doc.get(ids.b), Some(&b));
    assert_eq!(s.ed.doc.get(ids.a), loaded.get(ids.a));
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes, loaded.nodes);
}

#[test]
fn rev21_hiding_theme_or_showing_another_paper_keeps_each_picker_use_on_its_own_paper() {
    use crate::model::Paint;
    let (mut s, ids, other) = theme_fixture();
    let before = s.ed.doc.clone();
    open_theme(&mut s);
    open_picker(&mut s, "#ff0000");
    picker_channel(&mut s, "R ", "0");
    let changed = s.ed.doc.clone();
    // Design while the picker is open: the press closes it, and Theme isn't drawn any more.
    let left = s.canvas_rect().min.x;
    for _ in 0..2 {
        if s.ui.left_tab != crate::ui::LeftTab::Design {
            let t = s.now_ms + 1000.0;
            click_label_in(&mut s, "Design", |r| r.max.x <= left, t);
        }
    }
    assert_eq!(s.ui.left_tab, crate::ui::LeftTab::Design);
    assert!(!ui::popup_open(&s.ctx));
    assert_eq!(s.ed.doc, changed);
    // The other Paper, whose rectangle reuses a's id: its own red, its own step.
    let t = s.now_ms + 1000.0;
    click_page(&mut s, "Other", t);
    assert_eq!(s.ed.active_page_id(), "q");
    open_theme(&mut s);
    open_picker(&mut s, "#ff0000");
    picker_channel(&mut s, "G ", "255");
    close_picker(&mut s);
    let q = s.ed.doc.get(ids.a).unwrap();
    assert_eq!(q.style.fills, [Paint::solid(YELLOW)]);
    assert_eq!(
        s.ed.page_data(0).doc.nodes,
        changed.nodes,
        "Page 1 as its own picker left it"
    );
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes, other.doc.nodes);
    // Back on Page 1, its picker use, though Theme was hidden in the middle of it, is one step.
    let left = s.canvas_rect().min.x;
    let t = s.now_ms + 1000.0;
    click_label_in(&mut s, "Design", |r| r.max.x <= left, t);
    let t = s.now_ms + 1000.0;
    click_page(&mut s, "Page 1", t);
    assert_eq!(s.ed.active_page_id(), "p");
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes, before.nodes);
    s.run(Action::Redo);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes, changed.nodes);
}

#[test]
fn rev21_autosaves_while_typing_into_the_picker_keep_its_use_one_undo_step() {
    // Run159's rendered failure: R typed key by key (1, 5, 0) with the page's autosave
    // acknowledged between the keys; each acknowledgement sealed the history, so one Undo went
    // back to R 15, not the colour the picker began with.
    use crate::model::Paint;
    let (mut s, ids, _) = theme_fixture();
    let before = page_to_json(&s.ed.page_data(0));
    open_theme(&mut s);
    open_picker(&mut s, "#ff0000");
    picker_field(&mut s, "R ");
    // Each key is a change of its own (R 1, then 15, then 150), and after each one the page's
    // autosave gets its chance: a save it offers is taken and acknowledged at once.
    let keys = [
        ("1", "Digit1", 1),
        ("5", "Digit5", 15),
        ("0", "Digit0", 150),
    ];
    let mut offered = 0;
    for (key, code, r) in keys {
        press(&mut s, key, code);
        timed_frames(&mut s, 2);
        let a = s.ed.doc.get(ids.a).unwrap();
        assert_eq!(a.style.fills[0], Paint::solid(Color::rgb(r, 0, 0)), "{key}");
        if s.save_snapshot().is_some() {
            s.save_succeeded();
            offered += 1;
        }
        timed_frames(&mut s, 1);
    }
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 3);
    assert!(ui::popup_open(&s.ctx), "the picker is still open");
    assert!(
        s.dirty(),
        "{offered} saves taken while typing; the change is still to save"
    );
    close_picker(&mut s);
    // Saved once the picker closed; the saved page reopens exactly.
    let edited = page_to_json(&s.ed.page_data(0));
    assert_ne!(edited, before);
    let snap = s.save_snapshot().expect("saved once the picker closed");
    let sent = sent_page(&snap);
    assert_eq!(page_to_json(&sent), edited);
    s.save_succeeded();
    assert!(!s.dirty());
    let other = pages_now(&s)[1].clone();
    let reopened = WebState::open(&canvas(&[sent, other], "p"), false, W, H).unwrap();
    assert_eq!(page_to_json(&reopened.ed.page_data(0)), edited);
    // One Undo: the whole page as it was before the picker; one Redo: as edited.
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), before, "one step");
    s.run(Action::Redo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), edited);
    // A departure (the page's blur) never waits on an open picker: its change saves then.
    open_picker(&mut s, "#960000");
    picker_field(&mut s, "R ");
    press(&mut s, "9", "Digit9");
    timed_frames(&mut s, 2);
    let a = s.ed.doc.get(ids.a).unwrap();
    assert_eq!(a.style.fills[0], Paint::solid(Color::rgb(9, 0, 0)));
    assert!(s.save_snapshot().is_none(), "held while the picker is open");
    s.set_focused(false);
    let snap = s.save_snapshot().expect("a blur lets it save");
    let sent = sent_page(&snap);
    assert_eq!(page_to_json(&sent), page_to_json(&s.ed.page_data(0)));
    s.save_succeeded();
    assert!(!s.dirty());
    s.set_focused(true);
}

// ---- revision 21: Theme changes a text style in use, through the Theme view's style editor
// (headless egui layout, as above).

/// The layers of [`text_style_pages`]' shown Paper.
struct StyleIds {
    /// "Title", all of it 24 Regular (grows to fit its text).
    whole: crate::model::NodeId,
    /// Unicode text, 18 Regular, centred, fixed width, spaced, with 24 Regular in two ranges
    /// (the second also red and underlined), an 18 Bold range and a struck 18 range.
    mixed: crate::model::NodeId,
    /// A locked 24 Regular layer.
    locked: crate::model::NodeId,
    /// A 24 Regular layer inside the locked group `group`.
    grouped: crate::model::NodeId,
    group: crate::model::NodeId,
    /// 36 Bold: another style.
    bold: crate::model::NodeId,
}

/// A text layer's kind: `text` at `size`, left aligned.
fn text_kind(text: &str, size: f64, auto_width: bool) -> NodeKind {
    NodeKind::Text {
        text: text.into(),
        font_size: size,
        auto_width,
        align: crate::model::TextAlign::Left,
    }
}

/// Two Papers: "Page 1" (`p`, shown) uses 24 Regular in every way [`StyleIds`] lists; "Other"
/// (`q`) has a 24 Regular text whose id is `whole`'s (ids are per Paper).
fn text_style_pages() -> (Vec<PageData>, StyleIds) {
    use crate::model::TextSpan;
    let span = |start, end| TextSpan {
        start,
        end,
        ..Default::default()
    };
    let mut doc = Document::default();
    let whole = theme_layer(
        &mut doc,
        text_kind("Title", 24.0, true),
        (100.0, 100.0, 20.0, 10.0),
        |_| {},
    );
    // Æble — øre ✓ ok: chars 0–3 "Æble", 7–9 "øre", 11 "✓", 12–14 " ok".
    let mixed = theme_layer(
        &mut doc,
        text_kind("Æble — øre ✓ ok", 18.0, false),
        (100.0, 200.0, 300.0, 30.0),
        |n| {
            if let NodeKind::Text { align, .. } = &mut n.kind {
                *align = crate::model::TextAlign::Center;
            }
            n.text_style.letter_spacing = 1.5;
            n.text_style.line_height = Some(1.4);
            n.text_style.spans = vec![
                TextSpan {
                    size: Some(24.0),
                    ..span(0, 4)
                },
                TextSpan {
                    size: Some(24.0),
                    color: Some(RED),
                    underline: Some(true),
                    ..span(7, 10)
                },
                TextSpan {
                    bold: Some(true),
                    ..span(11, 12)
                },
                TextSpan {
                    strike: Some(true),
                    ..span(12, 15)
                },
            ];
        },
    );
    let locked = theme_layer(
        &mut doc,
        text_kind("Locked", 24.0, true),
        (100.0, 300.0, 80.0, 30.0),
        |n| n.locked = true,
    );
    let group = theme_layer(&mut doc, NodeKind::Group, (400.0, 300.0, 80.0, 30.0), |n| {
        n.locked = true
    });
    let grouped = theme_layer(
        &mut doc,
        text_kind("Grouped", 24.0, true),
        (0.0, 0.0, 80.0, 30.0),
        |n| n.parent = Some(group),
    );
    doc.root.retain(|id| *id != grouped);
    doc.nodes.get_mut(&group).unwrap().children.push(grouped);
    let bold = theme_layer(
        &mut doc,
        text_kind("Heading", 36.0, true),
        (100.0, 400.0, 160.0, 44.0),
        |n| n.text_style.bold = true,
    );
    let shown = PageData {
        id: "p".into(),
        name: "Page 1".into(),
        doc,
        camera: Some(Default::default()),
    };
    let mut doc = Document::default();
    let elsewhere = theme_layer(
        &mut doc,
        text_kind("Elsewhere", 24.0, true),
        (100.0, 100.0, 120.0, 30.0),
        |_| {},
    );
    assert_eq!(elsewhere, whole, "the other Paper reuses whole's id");
    let other = PageData {
        id: "q".into(),
        name: "Other".into(),
        doc,
        camera: Some(Default::default()),
    };
    let ids = StyleIds {
        whole,
        mixed,
        locked,
        grouped,
        group,
        bold,
    };
    (vec![shown, other], ids)
}

/// [`text_style_pages`] opened on "Page 1", laid out, the left panel open.
fn text_style_fixture() -> (WebState, StyleIds, Vec<PageData>) {
    let (pages, ids) = text_style_pages();
    let mut s = WebState::open(&canvas(&pages, "p"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    open_layers(&mut s);
    (s, ids, pages)
}

/// Every character's resolved style, as the layout draws it.
fn resolved(n: &Node) -> Vec<crate::model::CharStyle> {
    let size = n.kind.font_size().unwrap();
    let len = n.kind.text().unwrap().chars().count();
    (0..len)
        .map(|i| n.text_style.char_style(i, size, Color::INK))
        .collect()
}

/// Keys typed one by one into the field that has the keyboard, the page's autosave offered
/// after each (a save it gives is taken and acknowledged at once); how many it gave.
fn type_with_autosaves(s: &mut WebState, keys: &[(&str, &str)]) -> usize {
    let mut offered = 0;
    for (key, code) in keys {
        press(s, key, code);
        timed_frames(s, 2);
        if s.save_snapshot().is_some() {
            s.save_succeeded();
            offered += 1;
        }
        timed_frames(s, 1);
    }
    offered
}

#[test]
fn rev21_a_theme_text_style_changes_only_its_characters_in_one_step_and_saves_exactly() {
    use crate::model::CharStyle;
    let (mut s, ids, pages) = text_style_fixture();
    let before_page = page_to_json(&s.ed.page_data(0));
    let before = s.ed.doc.clone();
    open_theme(&mut s);
    // The row's Aa button (in the swatch's place) opens the style's editor; the row isn't
    // chosen, so nothing is selected.
    open_picker(&mut s, "24 · Regular");
    assert!(s.ed.selection.is_empty());
    // Its size typed key by key (3, then 36), the autosave offered between the keys.
    picker_field(&mut s, "Size ");
    let offered = type_with_autosaves(&mut s, &[("3", "Digit3"), ("6", "Digit6")]);
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 3);
    assert!(ui::popup_open(&s.ctx));
    assert!(s.dirty(), "{offered} saves taken while typing");
    let size = |s: &WebState, id| s.ed.doc.get(id).unwrap().kind.font_size();
    assert_eq!(size(&s, ids.whole), Some(36.0));
    // Bold: the style now meets 36 Bold (another layer's), the rows merge and move, and the
    // editor stays on the characters it began with: 40 reaches them, not the heading.
    let t = s.now_ms + 1000.0;
    click_front_label(&mut s, "Bold", t);
    assert!(ui::popup_open(&s.ctx), "a face keeps the editor open");
    picker_field(&mut s, "Size ");
    type_with_autosaves(&mut s, &[("4", "Digit4"), ("0", "Digit0")]);
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 3);
    assert!(s.dirty());
    close_picker(&mut s);
    let after = s.ed.doc.clone();
    let get = |d: &Document, id| d.get(id).unwrap().clone();
    // The whole layer: its own size and face, laid out again where it was.
    let (w0, w1) = (get(&before, ids.whole), get(&after, ids.whole));
    let mut want = w0.clone();
    if let NodeKind::Text { font_size, .. } = &mut want.kind {
        *font_size = 40.0;
    }
    want.text_style.bold = true;
    want.rect = w1.rect;
    assert_eq!(w1, want);
    assert_eq!((w1.rect.x, w1.rect.y), (w0.rect.x, w0.rect.y));
    assert!(w1.rect.w > w0.rect.w, "{:?} -> {:?}", w0.rect, w1.rect);
    // The mixed layer: exactly its two 24 Regular ranges, their colour and underline kept;
    // its other characters, base size, alignment, spacing and width as they were.
    let (m0, m1) = (get(&before, ids.mixed), get(&after, ids.mixed));
    let target = |i: usize| i < 4 || (7..10).contains(&i);
    let want: Vec<CharStyle> = resolved(&m0)
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            if target(i) {
                CharStyle {
                    size: 40.0,
                    bold: true,
                    ..c
                }
            } else {
                c
            }
        })
        .collect();
    assert_eq!(resolved(&m1), want);
    let mut same = m0.clone();
    same.text_style.spans = m1.text_style.spans.clone();
    same.rect.h = m1.rect.h;
    assert_eq!(m1, same, "only its ranges' styles and its laid-out height");
    // Locked layers (themselves or through a group), the other style and the other Paper.
    for id in [ids.locked, ids.grouped, ids.group, ids.bold] {
        assert_eq!(after.get(id), before.get(id), "{id:?}");
    }
    assert_eq!(after.root, before.root);
    assert_eq!(s.ed.page_data(1).doc.nodes, pages[1].doc.nodes);
    // Saved once the editor closed, exactly as edited; the saved page reopens exactly.
    let edited = page_to_json(&s.ed.page_data(0));
    let snap = s.save_snapshot().expect("saved once the editor closed");
    let sent = sent_page(&snap);
    assert_eq!(page_to_json(&sent), edited);
    s.save_succeeded();
    assert!(!s.dirty());
    let reopened = WebState::open(&canvas(&[sent, pages[1].clone()], "p"), false, W, H).unwrap();
    assert_eq!(page_to_json(&reopened.ed.page_data(0)), edited);
    // One Undo: the whole page as it was; one Redo: as edited.
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), before_page, "one step");
    s.run(Action::Redo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), edited);
    // A second use is its own step: 40 Bold to Italic.
    open_picker(&mut s, "40 · Bold");
    let t = s.now_ms + 1000.0;
    click_front_label(&mut s, "Italic", t);
    close_picker(&mut s);
    let w = get(&s.ed.doc, ids.whole);
    assert_eq!((w.text_style.bold, w.text_style.italic), (false, true));
    assert!(resolved(&get(&s.ed.doc, ids.mixed))[7].italic);
    assert_eq!(get(&s.ed.doc, ids.bold), get(&before, ids.bold));
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(
        page_to_json(&s.ed.page_data(0)),
        edited,
        "only the second use undone"
    );
    // A style only locked layers use has no editor.
    let p = theme_swatch(&mut s, "24 · Regular");
    timed_click(&mut s, p);
    assert!(!ui::popup_open(&s.ctx));
    assert_eq!(page_to_json(&s.ed.page_data(0)), edited);
}

#[test]
fn rev21_a_text_style_editor_never_outlives_its_document_or_paper() {
    let (mut s, ids, pages) = text_style_fixture();
    let before = s.ed.doc.clone();
    open_theme(&mut s);
    open_picker(&mut s, "24 · Regular");
    picker_field(&mut s, "Size ");
    type_with_autosaves(&mut s, &[("3", "Digit3")]);
    let size = |s: &WebState, id| s.ed.doc.get(id).unwrap().kind.font_size();
    assert_eq!(size(&s, ids.whole), Some(3.0));
    // The explicit discard loads another document whose text reuses `whole`'s id and already
    // has the editor's current style: the editor closes with the document it was for.
    let (mut fresh, _) = text_style_pages();
    if let NodeKind::Text { font_size, .. } =
        &mut fresh[0].doc.nodes.get_mut(&ids.whole).unwrap().kind
    {
        *font_size = 3.0;
    }
    let loaded = fresh[0].doc.clone();
    s.reload(&canvas(&fresh, "p")).unwrap();
    timed_frames(&mut s, 3);
    assert!(
        !ui::popup_open(&s.ctx),
        "the editor closed with its document"
    );
    let text = painted_text(&mut s);
    assert!(!text.iter().any(|(t, _)| t == "Size "), "{text:?}");
    assert_eq!(
        s.ed.doc.nodes, loaded.nodes,
        "nothing of it reached the new one"
    );
    assert!(!s.dirty());
    // Back to the fixture as saved; a face on Page 1, then Design while it is open: the press
    // closes it and Theme isn't drawn any more.
    s.reload(&canvas(&pages, "p")).unwrap();
    timed_frames(&mut s, 2);
    open_picker(&mut s, "24 · Regular");
    let t = s.now_ms + 1000.0;
    click_front_label(&mut s, "Italic", t);
    let changed = s.ed.doc.clone();
    assert!(get_italic(&changed, ids.whole));
    let left = s.canvas_rect().min.x;
    for _ in 0..2 {
        if s.ui.left_tab != crate::ui::LeftTab::Design {
            let t = s.now_ms + 1000.0;
            click_label_in(&mut s, "Design", |r| r.max.x <= left, t);
        }
    }
    assert!(!ui::popup_open(&s.ctx));
    // The other Paper, whose text reuses `whole`'s id: its own 24 Regular, its own step.
    let t = s.now_ms + 1000.0;
    click_page(&mut s, "Other", t);
    assert_eq!(s.ed.active_page_id(), "q");
    open_theme(&mut s);
    open_picker(&mut s, "24 · Regular");
    let t = s.now_ms + 1000.0;
    click_front_label(&mut s, "Bold", t);
    close_picker(&mut s);
    let q = s.ed.doc.get(ids.whole).unwrap();
    assert_eq!((q.text_style.bold, q.text_style.italic), (true, false));
    assert_eq!(
        s.ed.page_data(0).doc.nodes,
        changed.nodes,
        "Page 1 as its use left it"
    );
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes, pages[1].doc.nodes);
    // Back on Page 1, its use, though Theme was hidden in the middle of it, is one step.
    let t = s.now_ms + 1000.0;
    click_label_in(&mut s, "Design", |r| r.max.x <= left, t);
    let t = s.now_ms + 1000.0;
    click_page(&mut s, "Page 1", t);
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(s.ed.doc.nodes, before.nodes);
}

/// Whether text layer `id` of `doc` is italic as a whole.
fn get_italic(doc: &Document, id: crate::model::NodeId) -> bool {
    doc.get(id).unwrap().text_style.italic
}

// ---- R67 corrections: Theme beside component instances, and the native host's autosave.
// The independent R67 review's probes are kept here; where one was adapted, its comment says
// how and why.

/// The native app's own project session (upstream `src/session.rs`), for its autosave decision.
#[allow(dead_code)]
#[path = "../../../src/session.rs"]
mod native_session;

#[test]
fn r67_late_colour_ack_keeps_newer_edit_dirty_and_one_step() {
    // The reviewer's probe, unchanged but for formatting.
    let (mut s, ids, _) = theme_fixture();
    s.ed.edit("Earlier unrelated move", |tx| {
        tx.node_mut(ids.e).unwrap().rect.x += 10.0;
    });
    let before = page_to_json(&s.ed.page_data(0));
    let earlier = s
        .save_snapshot()
        .expect("earlier save starts before picker");
    assert_eq!(page_to_json(&sent_page(&earlier)), before);
    open_theme(&mut s);
    open_picker(&mut s, "#ff0000");
    picker_field(&mut s, "R ");
    press(&mut s, "1", "Digit1");
    timed_frames(&mut s, 2);
    s.save_succeeded(); // Real acknowledgement after the first newer picker change.
    assert!(
        s.dirty(),
        "older acknowledgement cannot mark the newer edit saved"
    );
    assert!(
        s.save_snapshot().is_none(),
        "newer picker edit remains held"
    );
    type_with_autosaves(&mut s, &[("5", "Digit5"), ("0", "Digit0")]);
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 3);
    close_picker(&mut s);
    let edited = page_to_json(&s.ed.page_data(0));
    let save = s.save_snapshot().expect("final value offered on close");
    assert_eq!(page_to_json(&sent_page(&save)), edited);
    s.save_succeeded();
    assert!(!s.dirty());
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(
        page_to_json(&s.ed.page_data(0)),
        before,
        "late ack did not split picker"
    );
    s.run(Action::Redo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), edited);
}

#[test]
fn r67_late_style_ack_keeps_newer_edit_dirty_and_one_step() {
    // The reviewer's probe, unchanged but for formatting.
    let (mut s, ids, _) = text_style_fixture();
    s.ed.edit("Earlier unrelated move", |tx| {
        tx.node_mut(ids.bold).unwrap().rect.x += 10.0;
    });
    let before = page_to_json(&s.ed.page_data(0));
    s.save_snapshot().expect("earlier save starts");
    open_theme(&mut s);
    open_picker(&mut s, "24 · Regular");
    picker_field(&mut s, "Size ");
    press(&mut s, "3", "Digit3");
    timed_frames(&mut s, 2);
    s.save_succeeded();
    assert!(s.dirty());
    assert!(s.save_snapshot().is_none());
    type_with_autosaves(&mut s, &[("2", "Digit2")]);
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 3);
    close_picker(&mut s);
    s.save_snapshot().expect("final style save");
    s.save_succeeded();
    assert!(!s.dirty());
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), before);
}

#[test]
fn r67_failed_save_releases_inflight_and_discard_ends_hold_immediately() {
    // The reviewer's probe, unchanged but for formatting.
    let (mut s, _, _) = theme_fixture();
    let original = pages_now(&s);
    open_theme(&mut s);
    open_picker(&mut s, "#ff0000");
    picker_channel(&mut s, "R ", "150");
    assert!(s.save_snapshot().is_none());
    s.set_focused(false);
    s.save_snapshot()
        .expect("departure can send while popup is open");
    s.save_failed(); // Same WebState callback used on conflict/refusal.
    assert!(s.dirty());
    assert!(!s.can_reload(), "failed save never makes it clean");
    s.set_focused(true);
    assert!(
        s.save_snapshot().is_none(),
        "focus holds current edit again"
    );
    s.reload(&canvas(&original, "p")).expect("explicit discard");
    assert!(
        !ui::design_system::editing(&s.ctx, &s.ed),
        "stale stamp ends hold before next draw"
    );
    timed_frames(&mut s, 3);
    assert!(!ui::popup_open(&s.ctx));
    assert!(!s.dirty());
    assert!(s.can_reload());
    assert_eq!(page_to_json(&s.ed.page_data(0)), page_to_json(&original[0]));
}

#[test]
fn r67_the_native_session_autosave_keeps_one_theme_use_one_undo_step() {
    // R67 F2: the reviewer's `r67_native_session_autosave_retains_one_undo_per_colour_use`
    // failed with three `Session` saves while R was typed 1, 5, 0 into one open picker, and
    // one Undo at R 15, not 255. This drives the native app's own decision,
    // `Session::autosave_due_ui` (what `App::tick_autosave` calls with its egui context), with
    // the actual `Session` save. Adapted from the probe: a fresh temporary store instead of
    // the reviewer's own fixture path, the clock past the autosave delay at every key (the
    // save is due by time, and held), and the normal autosave after the picker closes, the
    // one save the corrected hold leaves (the probe expected the disk to have R 150 without
    // any further `Session` call). Its one-Undo-to-255 and full-page assertions are kept.
    use crate::model::Paint;
    use std::time::Instant;
    let root = std::env::temp_dir().join(format!("opd-r67-session-{:016x}", fastrand::u64(..)));
    let store = crate::project::Store::open(root.clone()).unwrap();
    let mut sess = native_session::Session::new(store);
    let mut empty = Editor::default();
    sess.create(&mut empty, "R67 Session").unwrap();
    let (mut s, ids, _) = theme_fixture();
    s.ed.meta_dirty = true;
    sess.save(&mut s.ed).unwrap(); // Seed this fresh project with the exact fixture.
    let before = page_to_json(&s.ed.page_data(0));
    open_theme(&mut s);
    open_picker(&mut s, "#ff0000");
    picker_field(&mut s, "R ");
    let delay = native_session::AUTOSAVE_DELAY;
    let mut now = Instant::now();
    let mut saves = 0;
    let keys = [
        ("1", "Digit1", 1),
        ("5", "Digit5", 15),
        ("0", "Digit0", 150),
    ];
    for (key, code, value) in keys {
        press(&mut s, key, code);
        timed_frames(&mut s, 2);
        let a = s.ed.doc.get(ids.a).unwrap();
        assert_eq!(a.style.fills[0], Paint::solid(Color::rgb(value, 0, 0)));
        assert!(ui::design_system::editing(&s.ctx, &s.ed));
        now += delay * 3;
        if sess.autosave_due_ui(&s.ed, &s.ctx, now) {
            sess.save(&mut s.ed).unwrap();
            saves += 1;
        }
    }
    assert_eq!(saves, 0, "no save while the picker is open");
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 3);
    close_picker(&mut s);
    assert!(!ui::design_system::editing(&s.ctx, &s.ed));
    // The normal autosave once the picker closed: due an autosave delay later.
    now += delay;
    assert!(
        sess.autosave_due_ui(&s.ed, &s.ctx, now),
        "due once it closed"
    );
    sess.save(&mut s.ed).unwrap();
    let edited = page_to_json(&s.ed.page_data(0));
    let saved = sess.project.as_ref().unwrap().load_pages().unwrap();
    assert_eq!(
        page_to_json(&saved[0]),
        edited,
        "the Session saved the page"
    );
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(
        s.ed.doc.get(ids.a).unwrap().style.fills[0],
        Paint::solid(Color::rgb(255, 0, 0)),
        "one Undo restores the starting red"
    );
    assert_eq!(
        page_to_json(&s.ed.page_data(0)),
        before,
        "one full-page Undo"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// Layer `main_child` made a main component (wrapped in a component frame) and one instance of
/// it placed beside, locked if `locked`: the instance and its child inheriting from
/// `main_child`, laid out with nothing selected.
fn instance_of(
    s: &mut WebState,
    main_child: crate::model::NodeId,
    locked: bool,
) -> (crate::model::NodeId, crate::model::NodeId) {
    s.ed.selection = vec![main_child];
    s.ed.create_component();
    let main = s.ed.selection[0];
    let instance =
        s.ed.create_instance(main, crate::geom::v2(700.0, 100.0))
            .unwrap();
    let child = s.ed.doc.get(instance).unwrap().children[0];
    let link = s.ed.doc.get(child).unwrap().instance.clone().unwrap();
    assert_eq!(link.main, main_child);
    if locked {
        s.ed.edit("Lock instance", |tx| {
            tx.node_mut(instance).unwrap().locked = true
        });
        assert!(s.ed.doc.is_effectively_locked(child));
    }
    s.ed.selection.clear();
    timed_frames(s, 4);
    (instance, child)
}

/// A Theme row's change refused because a locked instance inherits it: choosing its swatch or
/// "Aa" opens nothing, says `reason` in the status line, and changes nothing at all (the page,
/// the history, the document stamp, the unsaved state, the selection).
fn assert_refused(s: &mut WebState, row: &str, reason: &str) {
    let page = page_to_json(&s.ed.page_data(0));
    let kept = (s.ed.history.undo_len(), s.ed.doc_stamp, s.dirty());
    let p = theme_swatch(s, row);
    timed_click(s, p);
    assert!(!ui::popup_open(&s.ctx), "{row}: nothing opens");
    assert_eq!(s.ed.status, reason);
    assert!(paints(s, reason), "{row}: the reason is shown");
    assert_eq!(
        page_to_json(&s.ed.page_data(0)),
        page,
        "{row}: nothing changed"
    );
    assert_eq!((s.ed.history.undo_len(), s.ed.doc_stamp, s.dirty()), kept);
    assert!(s.ed.selection.is_empty());
}

#[test]
fn r67_a_colour_a_locked_instance_inherits_is_refused_whole_and_said() {
    // R67 F1: the reviewer's `r67_theme_colour_keeps_locked_component_instance_exact` failed
    // with the locked instance's child turned yellow (component sync reached it behind
    // Theme's lock filter). Corrected by refusal, before anything changes, with the reason:
    // that probe's "main turns yellow" expectation isn't this contract, so its fixture is kept
    // and its locked-child-exact assertion is strengthened to the whole Paper and history.
    use crate::model::Paint;
    let (mut s, ids, _) = theme_fixture();
    let (_, child) = instance_of(&mut s, ids.a, true);
    let child_before = s.ed.doc.get(child).unwrap().clone();
    assert_eq!(child_before.style.fills[0], Paint::solid(RED));
    open_theme(&mut s);
    assert_refused(&mut s, "#ff0000", ui::design_system::INHERITED_COLOUR);
    assert_eq!(s.ed.doc.get(child), Some(&child_before));
    // Blue, `a`'s upper fill, is inherited too and refused the same way; green, the ellipse's
    // own fill, which no locked instance inherits, still changes.
    assert_refused(&mut s, "#0000ff", ui::design_system::INHERITED_COLOUR);
    open_picker(&mut s, "#00ff00");
    picker_channel(&mut s, "R ", "255");
    close_picker(&mut s);
    let b = s.ed.doc.get(ids.b).unwrap();
    assert_eq!(b.style.fills[0], Paint::solid(YELLOW));
    assert_eq!(s.ed.doc.get(child), Some(&child_before));
}

#[test]
fn r67_an_unlocked_instance_takes_a_theme_colour_with_its_main_in_one_step() {
    // The successful control for R67 F1: the same component, its instance unlocked.
    use crate::model::Paint;
    let (mut s, ids, _) = theme_fixture();
    let (_, child) = instance_of(&mut s, ids.a, false);
    let before_doc = s.ed.doc.clone();
    let before = page_to_json(&s.ed.page_data(0));
    open_theme(&mut s);
    open_picker(&mut s, "#ff0000");
    picker_channel(&mut s, "G ", "255");
    close_picker(&mut s);
    assert_eq!(
        s.ed.doc.get(ids.a).unwrap().style.fills[0],
        Paint::solid(YELLOW)
    );
    assert_eq!(
        s.ed.doc.get(child).unwrap().style.fills[0],
        Paint::solid(YELLOW)
    );
    for id in [ids.c, ids.d, ids.g] {
        assert_eq!(s.ed.doc.get(id), before_doc.get(id), "{id:?}");
    }
    let edited = page_to_json(&s.ed.page_data(0));
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), before, "one step");
    s.run(Action::Redo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), edited);
}

#[test]
fn r67_a_text_style_a_locked_instance_inherits_is_refused_whole_and_said() {
    // R67 F1, typography: the reviewer's `r67_theme_style_keeps_locked_component_instance_exact`
    // failed with the locked instance's child at 32. Refused as for colours (see there).
    let (mut s, ids, _) = text_style_fixture();
    let (_, child) = instance_of(&mut s, ids.whole, true);
    let child_before = s.ed.doc.get(child).unwrap().clone();
    assert_eq!(child_before.kind.font_size(), Some(24.0));
    open_theme(&mut s);
    assert_refused(&mut s, "24 · Regular", ui::design_system::INHERITED_STYLE);
    assert_eq!(s.ed.doc.get(child), Some(&child_before));
}

#[test]
fn r67_an_unlocked_instance_takes_a_theme_text_style_with_its_main_in_one_step() {
    // The successful control for R67 F1, typography.
    let (mut s, ids, _) = text_style_fixture();
    let (_, child) = instance_of(&mut s, ids.whole, false);
    let before_doc = s.ed.doc.clone();
    let before = page_to_json(&s.ed.page_data(0));
    open_theme(&mut s);
    open_picker(&mut s, "24 · Regular");
    picker_field(&mut s, "Size ");
    type_with_autosaves(&mut s, &[("3", "Digit3"), ("2", "Digit2")]);
    press(&mut s, "Enter", "Enter");
    timed_frames(&mut s, 3);
    close_picker(&mut s);
    let size = |s: &WebState, id| s.ed.doc.get(id).unwrap().kind.font_size();
    assert_eq!(size(&s, ids.whole), Some(32.0));
    assert_eq!(size(&s, child), Some(32.0));
    for id in [ids.locked, ids.grouped, ids.group, ids.bold] {
        assert_eq!(s.ed.doc.get(id), before_doc.get(id), "{id:?}");
    }
    let edited = page_to_json(&s.ed.page_data(0));
    s.run(Action::Undo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), before, "one step");
    s.run(Action::Redo);
    step(&mut s);
    assert_eq!(page_to_json(&s.ed.page_data(0)), edited);
}
