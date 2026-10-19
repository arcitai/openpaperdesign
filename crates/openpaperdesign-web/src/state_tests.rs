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

/// A click on the header's Layers toggle, then the arrangement settles.
fn toggle_layers(s: &mut WebState) {
    let r =
        s.ui.chrome
            .layers
            .expect("the header's Layers toggle is laid out");
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

// ---- one app: the file's identity and the way back live in upstream's own top bar

#[test]
fn the_file_name_is_upstream_s_top_bar_title_above_the_page_name_and_follows_a_rename() {
    let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
    let pages = [page("p", "Page 1", rect)];
    let mut s = WebState::open(&canvas_named(&pages, "p", "Landing"), false, W, H).unwrap();
    settle(&mut s, 0.0);
    let top = s.canvas_rect().min.y;
    let in_bar = |s: &mut WebState, wanted: &str| {
        painted_text(s)
            .iter()
            .any(|(t, r)| t.trim() == wanted && r.max.y <= top)
    };
    assert!(in_bar(&mut s, "Landing"), "the file's name is the title");
    assert!(
        in_bar(&mut s, "Page 1"),
        "the page's name is shown beneath it"
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
    let top = s.canvas_rect().min.y;
    // The header spans the whole surface above both panels: its first control, at the left
    // edge, is All projects; the compact menu ("···") and the Layers toggle follow it.
    let home = s.ui.chrome.home.expect("All projects is laid out");
    let menu = s.ui.chrome.menu.expect("the compact menu is laid out");
    let layers = s.ui.chrome.layers.expect("the Layers toggle is laid out");
    assert!(home.min.x < 12.0 && home.max.y <= top, "{home:?}");
    assert!(home.max.x <= menu.min.x && menu.max.x <= layers.min.x);
    let (menu_x, _) = painted_at(&mut s, "···", |r| r.max.y <= top);
    assert!((menu_x as f32 - menu.center().x).abs() < 8.0);
    assert!(
        s.canvas_rect().min.x >= 180.0,
        "the layers panel below the header"
    );
    click(&mut s, centre(home), 1000.0);
    step(&mut s);
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

/// Click the header's toggle `label` ("Layers", "Design") where it is laid out and let the
/// arrangement settle (it changes after one frame without a focused text field).
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
        assert!(
            chevron.min.x >= main.max.x - 0.5 && chevron.width() >= 24.0 - 0.5,
            "the chevron is its own target, beside the main button: {chevron:?}"
        );
        for (k, tool) in tools.iter().enumerate() {
            click(s, centre(chevron), t);
            step(s);
            step(s);
            t += 500.0;
            assert_eq!(s.ui.rail.flyout, Some(fi), "the chevron opens the flyout");
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
        c.min.y > 0.0 && c.height() > 600.0,
        "under the top bar: {c:?}"
    );
    assert_eq!(
        (s.ed.viewport.x, s.ed.viewport.y),
        (c.width() as f64, c.height() as f64)
    );
    // The header carries the file's name, the menu and the Design toggle (text) and the
    // All projects and Layers toggles (icons), inside the surface and above the canvas.
    for wanted in ["Landing", "Design", "···"] {
        let painted = painted_text(&mut s);
        let (_, r) = painted
            .iter()
            .find(|(t, _)| t.trim() == wanted)
            .unwrap_or_else(|| panic!("{wanted} not painted: {painted:?}"));
        assert!(
            r.max.x <= NARROW as f32 && r.max.y <= c.min.y,
            "{wanted}: {r:?}"
        );
    }
    for r in [s.ui.chrome.home, s.ui.chrome.layers, s.ui.chrome.design] {
        let r = r.expect("laid out");
        assert!(r.max.x <= NARROW as f32 && r.max.y <= c.min.y, "{r:?}");
    }
    // All projects asks the host to go Home.
    let home = centre(s.ui.chrome.home.unwrap());
    click(&mut s, home, 1000.0);
    step(&mut s);
    assert_eq!(
        s.take_request(),
        Some(HostRequest::Home),
        "All projects is reachable"
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
        paints(&mut s, "Pages") && paints(&mut s, "Page 1"),
        "upstream's pages and layers"
    );
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
    assert!(!paints(&mut s, "Pages"), "one panel at a time");
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
    assert!(paints(&mut s, "Pages") && paints(&mut s, "Export"));
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
    assert!(
        s.ui.renaming.is_some(),
        "the rename stays open while they appear"
    );
    assert!(s.field_has_keyboard(), "its field has the keyboard");
    press(&mut s, "q", "KeyQ");
    step(&mut s);
    assert_eq!(s.ed.doc.get(id).unwrap().name, old, "still a draft");
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
    // The rectangle selected, then Design docked on the right.
    click_at(&mut s, 150.0, 150.0, 0.0);
    step(&mut s);
    let id = s.ed.selection[0];
    toggle(&mut s, "Design", 500.0);
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
fn the_top_bar_keeps_the_file_name_and_its_controls_apart_at_every_width() {
    // The bar's text never overlaps: the file's name (truncated if need be) ends before the
    // first control painted right of it, from the narrow sidebar to a wide window.
    for w in [320.0, 480.0, 640.0, 800.0, 1100.0, 1600.0] {
        let rect = crate::geom::Rect::new(100.0, 100.0, 200.0, 100.0);
        let pages = [page("p", "Page 1", rect)];
        let name = "A rather long file name for the top bar";
        let mut s = WebState::open(&canvas_named(&pages, "p", name), false, w, TALL).unwrap();
        settle(&mut s, 0.0);
        let top = s.canvas_rect().min.y;
        let bar: Vec<(String, egui::Rect)> = painted_text(&mut s)
            .into_iter()
            .filter(|(_, r)| r.max.y <= top)
            .collect();
        let (_, title) = bar
            .iter()
            .find(|(t, _)| t.trim() == name)
            .unwrap_or_else(|| panic!("{w}: the file name is painted: {bar:?}"));
        assert!(
            title.max.x <= w as f32,
            "{w}: inside the surface: {title:?}"
        );
        for (t, r) in bar
            .iter()
            .filter(|(t, r)| t.trim() != name && r.min.x > title.min.x)
        {
            assert!(
                r.min.x >= title.max.x - 0.5 || r.max.y <= title.min.y || r.min.y >= title.max.y,
                "{w}: {t:?} {r:?} overlaps the name {title:?}"
            );
        }
    }
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
    click_label(&mut s, "···", 100.0);
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
    click_label(&mut s, "···", 500.0);
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
    open_layers(&mut s);
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
        open_layers(&mut s);
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
    double_click(&mut s, p, 100.0);
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
    double_click(&mut s, p, 100.0);
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

#[test]
fn rev2_a_fresh_view_keeps_layers_closed_and_lays_the_header_out_across_the_whole_surface() {
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
        assert!(c.min.x < 1.0, "{w}: no left panel: {c:?}");
        if design {
            assert!(
                c.max.x <= w as f32 - 220.0,
                "{w}: the quiet inspector: {c:?}"
            );
        } else {
            assert!(c.width() >= w as f32 - 1.0, "{w}: canvas only: {c:?}");
        }
        // The header comes first: its controls start at the surface's left edge and the
        // centred identity sits on the surface's centre (or between the groups when narrow).
        let home = s.ui.chrome.home.unwrap();
        let design_toggle = s.ui.chrome.design.unwrap();
        let id = s.ui.chrome.identity.unwrap();
        assert!(home.min.x < 12.0 && home.max.y <= c.min.y, "{w}: {home:?}");
        assert!(
            design_toggle.max.x > w as f32 - 12.0,
            "{w}: {design_toggle:?}"
        );
        if w >= 600.0 {
            assert!(
                (id.center().x - w as f32 / 2.0).abs() < 1.0,
                "{w}: centred: {id:?}"
            );
        }
        // Opening Layers puts the panel under the header, which keeps its place.
        toggle(&mut s, "Layers", 1000.0);
        let after = s.ui.chrome.home.unwrap();
        assert_eq!(after, home, "{w}: the header doesn't move for a panel");
        assert!(s.ui.narrow.shown().unwrap().layers, "{w}");
        assert_eq!(
            s.canvas_rect().min.y,
            c.min.y,
            "{w}: panels start below the header"
        );
        assert!(paints(&mut s, "Pages"), "{w}: the pages and layers panel");
        assert!(!s.dirty(), "{w}: arranging changes nothing");
    }
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
    // Collapse through the compact menu, as a person finds it.
    click_label(&mut s, "···", 100.0);
    click_label(&mut s, "View", 200.0);
    click_label(&mut s, "Collapse / Restore Panels", 300.0);
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
        "the header, the rail and the zoom stay"
    );
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
fn rev2_zoom_sits_at_the_canvas_s_lower_left_clear_of_the_rail_and_offers_fit() {
    let mut s = state();
    let c = s.canvas_rect();
    let zoom = s.ui.chrome.zoom.unwrap();
    let rail = s.ui.rail.rects.rail.unwrap();
    assert!(c.contains_rect(zoom), "{zoom:?} in {c:?}");
    assert!(
        zoom.min.x - c.min.x <= 9.0 && c.max.y - zoom.max.y <= 9.0,
        "{zoom:?}"
    );
    assert!(!overlap(zoom, rail), "{zoom:?} {rail:?}");
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
    assert!(r.min.x >= zoom.max.x, "{r:?} {zoom:?}");
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
        assert!(c.contains_rect(zoom), "{w}×{h}: {zoom:?} in {c:?}");
        if c.height() < crate::ui::rail::SHORT_H {
            assert!(
                (rail.min.y - c.min.y - crate::ui::rail::INSET).abs() < 0.5,
                "{w}×{h}: a short canvas anchors the rail at the top"
            );
            assert!(zoom.min.x >= rail.max.x, "{w}×{h}: the zoom beside it");
        }
        // The largest family's flyout fits inside the canvas, beside the rail where it can.
        let (_, chevron) = s.ui.rail.rects.rows[2];
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

// ---- revision 2, slice 2: one rail, six families, all sixteen tools

#[test]
fn rev2_the_rail_reaches_all_sixteen_tools_and_each_family_remembers_its_member() {
    let mut s = fresh(W, H);
    assert_eq!(s.ui.rail.rects.rows.len(), 6, "six families");
    let reached = reach_every_tool(&mut s, 1000.0);
    assert_eq!(reached.len(), 16, "{reached:?}");
    assert_eq!(s.ed.doc.nodes.len(), 1, "probing the tools drew nothing");
    // A shortcut makes its tool the family's member: S is Sticky note, in Text.
    press(&mut s, "Escape", "Escape");
    assert!(s.key_down("s", "KeyS", false));
    step(&mut s);
    assert_eq!(s.ed.tool, Tool::Sticky);
    assert_eq!(s.ui.rail.members[4], Tool::Sticky);
    // Escape outside a flyout is back to Select; the Text family keeps Sticky note.
    press(&mut s, "Escape", "Escape");
    step(&mut s);
    assert_eq!(s.ed.tool, Tool::Select);
    assert_eq!(s.ui.rail.members[4], Tool::Sticky);
    let (text_main, _) = s.ui.rail.rects.rows[4];
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
    let (_, chevron) = s.ui.rail.rects.rows[2];
    click(&mut s, centre(chevron.unwrap()), 100.0);
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ui.rail.flyout, Some(2));
    let mut tabs = 0;
    while s.ui.rail.rects.focus.map(|(_, t)| t) != Some(Some(Tool::Polygon)) {
        assert!(tabs < 300, "Tab reaches the Polygon row");
        s.key_down("Tab", "Tab", false);
        s.key_up("Tab", "Tab");
        step(&mut s);
        step(&mut s);
        tabs += 1;
    }
    assert_eq!(s.ui.rail.flyout, Some(2), "Tab keeps the flyout open");
    assert_eq!(s.ed.selection, vec![label], "Tab cycled the one layer");
    assert!(s.ed.text_edit.is_none());
    // Enter chooses the focused row; the editor never sees it.
    assert!(s.key_down("Enter", "Enter", false), "the row's key");
    s.key_up("Enter", "Enter");
    step(&mut s);
    step(&mut s);
    assert_eq!(s.ed.tool, Tool::Polygon, "Enter chose the focused row");
    assert_eq!(s.ui.rail.flyout, None, "and closed the flyout");
    assert_eq!(s.ui.rail.members[2], Tool::Polygon, "its member now");
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
        let (_, chevron) = s.ui.rail.rects.rows[2];
        click(s, centre(chevron.unwrap()), t);
        step(s);
        step(s);
        assert_eq!(s.ui.rail.flyout, Some(2));
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
    assert!(text.iter().any(|t| t == "Page"), "{text:?}");
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
    for label in [
        "Background",
        "None",
        "Dots",
        "Lines",
        "Zoom to Fit",
        "Collapse Panels",
    ] {
        assert!(paints(&mut s, label), "{label}");
    }
    click_label(&mut s, "Lines", 200.0);
    assert_eq!(s.ui.background, scene::Background::Lines);
    assert!(s.ui.canvas_menu.is_none(), "a choice closes it");
    assert_eq!(requests(&mut s), [HostRequest::Background]);
    assert_eq!(s.background(), "lines");
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

// ---- revision 2, slice 6: the file's real name, renamed from the header and saved with the
// canvas (the same canvas save as the pages), and the shown page's own actions

/// Where text starting with `wanted` is painted in the header (its centre, CSS px).
fn in_header(s: &mut WebState, wanted: &str) -> (f64, f64) {
    let top = s.canvas_rect().min.y;
    let text = painted_text(s);
    let (_, r) = text
        .iter()
        .find(|(t, r)| t.trim().starts_with(wanted) && r.max.y <= top)
        .unwrap_or_else(|| panic!("{wanted:?} isn't in the header: {text:?}"));
    centre(*r)
}

/// Click the header's title (`name`) to open its rename field, select its text and type
/// `text`, laid out: a draft until it is finished.
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
fn rev2_the_page_line_offers_the_shown_page_s_own_actions() {
    let mut s = state();
    let page = in_header(&mut s, "Page 1");
    click(&mut s, page, 100.0);
    step(&mut s);
    step(&mut s);
    click_label(&mut s, "Duplicate page", 200.0);
    let names: Vec<&str> = s.ed.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Page 1", "Page 1 copy"]);
    assert_eq!(s.ed.page_name(), "Page 1 copy");
    // Rename page… opens the shown page's own rename in the pages list: the same field, the
    // same Enter, and the same page-list save.
    let page = in_header(&mut s, "Page 1 copy");
    click(&mut s, page, 300.0);
    step(&mut s);
    step(&mut s);
    click_label(&mut s, "Rename page…", 400.0);
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
    // canvas's left side and the zoom sits beside it at the top.
    let mut s = fresh(320.0, 270.0);
    let before = page_to_json(&s.ed.page_data(0));
    s.run(Action::CopyAsPng);
    timed_frames(&mut s, 3);
    let canvas = s.canvas_rect();
    assert!(canvas.height() < crate::ui::rail::SHORT_H, "{canvas:?}");
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
    // R33-F5: at 1024 × 270 both panels dragged to their widest (420).
    let mut s = fresh(1024.0, 270.0);
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
    let inside = |s: &mut WebState, why: &str| {
        timed_frames(s, 3);
        let canvas = s.canvas_rect();
        let rail = s.ui.rail.rects.rail.unwrap();
        let zoom = s.ui.chrome.zoom.expect("a zoom control");
        assert!(canvas.contains_rect(zoom), "{why}: {zoom:?} in {canvas:?}");
        assert!(!overlap(zoom, rail), "{why}: clear of the rail");
    };
    inside(&mut s, "100%");
    // Beside the rail there is no room for the whole group (R33 measured it 108 px wide):
    // only the level is shown, and its own menu zooms in and out.
    assert_eq!((s.ui.chrome.zoom_out, s.ui.chrome.zoom_in), (None, None));
    let level = s.ui.chrome.zoom_level.unwrap();
    timed_click(&mut s, centre(level));
    let (zoom, t) = (s.ed.camera.zoom, s.now_ms);
    click_label(&mut s, "Zoom In", t);
    timed_frames(&mut s, 2);
    assert!(s.ed.camera.zoom > zoom, "the level's Zoom In");
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
    // The rail's own targets still work beside it.
    let chevron = s.ui.rail.rects.rows[2].1.unwrap();
    timed_click(&mut s, centre(chevron));
    assert_eq!(s.ui.rail.flyout, Some(2));
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
