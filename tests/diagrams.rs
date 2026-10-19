//! Revision 2 slice 7 (OPD-UX-PLAN section 7, R15): the editable diagram workflow with the
//! objects that already exist (a group of one rectangle and its text label, bound arrows with
//! straight, curved and elbow routes, duplicate, copy, delete, undo, save and export), through
//! the editor's own input path and commands. These are proofs of current behaviour before any
//! repair: each test can fail on a wrong binding, a lost or corrupted label, an arrow that no
//! longer follows its shape, or an edit that takes more than one undo step.

mod common;

use common::*;
use paper_rs::editor::shortcuts::Action;
use paper_rs::editor::{Button, Editor, InputEvent, Key, Modifiers, Tool};
use paper_rs::geom::{Rect, V2, v2};
use paper_rs::model::{Color, Node, NodeId, NodeKind, Route, TextAlign};

const BLUE: Color = Color::rgb(0xa5, 0xd8, 0xff);
const PINK: Color = Color::rgb(0xff, 0xc9, 0xc9);

/// A filled, stroked shape (as `tests/excalidraw.rs` makes them).
fn shape(e: &mut Editor, kind: NodeKind, r: Rect, fill: Color) -> NodeId {
    let mut n = Node::new(NodeId(0), kind, r);
    n.style.set_fill(Some(fill));
    n.style.stroke = Some(Color::INK);
    n.style.stroke_width = 2.0;
    e.add_node(n, None)
}

fn rect(e: &mut Editor, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    shape(e, NodeKind::Rect, Rect::new(x, y, w, h), PINK)
}

/// A one-line text layer at 20 px, its box fitted to the text, centred on `c` (world).
fn text_at(e: &mut Editor, text: &str, c: V2) -> NodeId {
    let kind = NodeKind::Text {
        text: text.into(),
        font_size: 20.0,
        auto_width: true,
        align: TextAlign::Left,
    };
    let n = Node::new(NodeId(0), kind, Rect::new(0.0, 0.0, 1.0, 1.0));
    let t = e.add_node(n, None);
    let tr = e.doc.get(t).unwrap().rect;
    paper_rs::ui::inspector::apply(e, "x", &[t], |n| {
        n.rect.x = (c.x - tr.w * 0.5).round();
        n.rect.y = (c.y - tr.h * 0.5).round();
    });
    t
}

/// Group(Rect + Text): a rectangle at `r` and its label centred on it, grouped with the
/// editor's own Group command. Returns (group, rectangle, label).
fn label_group(e: &mut Editor, r: Rect, label: &str) -> (NodeId, NodeId, NodeId) {
    let rect = shape(e, NodeKind::Rect, r, BLUE);
    let text = text_at(e, label, r.center());
    e.selection = vec![rect, text];
    e.run(Action::Group);
    let g = e.selection[0];
    assert!(e.doc.get(g).unwrap().is_group(), "grouped");
    assert_eq!(e.doc.get(g).unwrap().children, vec![rect, text]);
    (g, rect, text)
}

/// Draw an arrow with the Arrow tool from `a` to `b` (screen = world: zoom 1, no pan).
fn draw_arrow(e: &mut Editor, a: V2, b: V2, t: f64) -> NodeId {
    e.run(Action::Tool(Tool::Arrow));
    ev_drag(e, a, b, t, true);
    let id = e.selection[0];
    assert!(is_arrow(e, id));
    id
}

fn is_arrow(e: &Editor, id: NodeId) -> bool {
    matches!(e.doc.get(id).unwrap().kind, NodeKind::Connector { .. })
}

/// (world points, start node, end node, route) of a connector.
fn conn(e: &Editor, id: NodeId) -> (Vec<V2>, Option<NodeId>, Option<NodeId>, Route) {
    let NodeKind::Connector {
        start, end, route, ..
    } = &e.doc.get(id).unwrap().kind
    else {
        panic!("not a connector")
    };
    (
        e.connector_points(id).unwrap(),
        start.map(|b| b.node),
        end.map(|b| b.node),
        *route,
    )
}

/// The nodes a connector's two ends are bound to.
fn ends(e: &Editor, id: NodeId) -> (Option<NodeId>, Option<NodeId>) {
    let (_, s, en, _) = conn(e, id);
    (s, en)
}

fn world(e: &Editor, id: NodeId) -> Rect {
    e.doc.world_rect(id).unwrap()
}

fn last(pts: &[V2]) -> V2 {
    *pts.last().unwrap()
}

/// An orbit-bound end: just outside its shape's box, within the binding gap (5 + stroke/2,
/// with room for the elbow router's own gap).
fn on_outline(p: V2, r: Rect) -> bool {
    !r.contains(p) && r.inflate(12.0).contains(p)
}

fn orthogonal(pts: &[V2]) -> bool {
    pts.windows(2)
        .all(|w| (w[0].x - w[1].x).abs() < 1e-6 || (w[0].y - w[1].y).abs() < 1e-6)
}

fn text_of(e: &Editor, id: NodeId) -> (String, f64) {
    match &e.doc.get(id).unwrap().kind {
        NodeKind::Text {
            text, font_size, ..
        } => (text.clone(), *font_size),
        k => panic!("not text: {k:?}"),
    }
}

fn connectors(e: &Editor) -> Vec<NodeId> {
    let mut v: Vec<NodeId> = e
        .doc
        .nodes
        .values()
        .filter(|n| matches!(n.kind, NodeKind::Connector { .. }))
        .map(|n| n.id)
        .collect();
    v.sort();
    v
}

/// The arrow among `ids` (a paste or duplicate's new roots).
fn arrow_in(e: &Editor, ids: &[NodeId]) -> NodeId {
    *ids.iter().find(|id| is_arrow(e, **id)).unwrap()
}

/// The label is laid out whole and inside its own box: every non-space character has a glyph,
/// and every glyph ends within the box's width (it wraps there). Returns (layout height, box).
fn label_laid_out(e: &Editor, t: NodeId) -> (f64, Rect) {
    let n = e.doc.get(t).unwrap();
    let text = n.kind.text().unwrap().clone();
    let l = paper_rs::text::layout_node(n).expect("a text layout");
    let shown: std::collections::HashSet<usize> = l.glyphs.iter().map(|g| g.char_index).collect();
    for (i, c) in text.chars().enumerate() {
        if !c.is_whitespace() {
            assert!(shown.contains(&i), "char {i} of {text:?} is lost");
        }
    }
    for g in &l.glyphs {
        let right = g.x + g.advance;
        assert!(right <= n.rect.w + 1.0, "{text:?}: {right} > {}", n.rect.w);
    }
    (l.height, n.rect)
}

#[test]
fn a_label_on_a_grouped_rectangle_binds_the_rectangle_not_the_text_or_the_group() {
    let mut e = editor(1200.0, 900.0);
    let r = Rect::new(100.0, 100.0, 200.0, 100.0);
    let (ga, ra, ta) = label_group(&mut e, r, "API service");
    let b = rect(&mut e, 700.0, 120.0, 160.0, 80.0);
    let label = world(&e, ta).center();
    assert!(world(&e, ra).contains(label), "on its rectangle");
    assert!(world(&e, ta).contains(label));
    // From the middle of the label (inside the text and the rectangle) to a plain rectangle.
    let target = world(&e, b).center();
    let arrow = draw_arrow(&mut e, label, target, 0.0);
    let (pts, s, en, _) = conn(&e, arrow);
    assert_eq!(s, Some(ra), "the rectangle under the label");
    assert_ne!(s, Some(ta), "not the text");
    assert_ne!(s, Some(ga), "a group is never a target");
    assert_eq!(en, Some(b), "control: a plain rectangle");
    assert!(on_outline(pts[0], world(&e, ra)), "{pts:?}");
    assert!(on_outline(last(&pts), world(&e, b)), "{pts:?}");
    // Control: a label over no shape is itself the target (the rule is about labels on shapes).
    let free = text_at(&mut e, "Note", v2(200.0, 420.0));
    let (from, to) = (world(&e, free).center(), world(&e, b).center());
    let arrow2 = draw_arrow(&mut e, from, to, 1000.0);
    assert_eq!(ends(&e, arrow2).0, Some(free));
    // Deliberately ambiguous: one label over two overlapping rectangles in one group. The
    // shape under the point that is painted on top (D) wins, as the binding rule says; neither
    // the label nor the group. On C alone, C.
    let c = rect(&mut e, 100.0, 600.0, 200.0, 100.0);
    let d = rect(&mut e, 220.0, 620.0, 200.0, 100.0);
    let tl = text_at(&mut e, "DB", v2(260.0, 660.0));
    e.selection = vec![c, d, tl];
    e.run(Action::Group);
    let gcd = e.selection[0];
    let over_both = world(&e, tl).center();
    assert!(world(&e, c).contains(over_both));
    assert!(world(&e, d).contains(over_both));
    let arrow3 = draw_arrow(&mut e, over_both, to, 2000.0);
    assert_eq!(ends(&e, arrow3).0, Some(d), "the topmost shape under it");
    let arrow4 = draw_arrow(&mut e, v2(130.0, 680.0), to, 3000.0);
    assert_eq!(ends(&e, arrow4).0, Some(c));
    for a in [arrow, arrow2, arrow3, arrow4] {
        assert_ne!(ends(&e, a).0, Some(gcd));
    }
    e.doc.validate().unwrap();
}

#[test]
fn moving_an_ancestor_group_reroutes_straight_curved_and_elbow_arrows_in_one_undo_step() {
    let mut e = editor(1200.0, 900.0);
    e.snapping = false;
    let r = Rect::new(100.0, 100.0, 200.0, 100.0);
    let (ga, ra, ta) = label_group(&mut e, r, "Gateway");
    let b = rect(&mut e, 700.0, 140.0, 160.0, 80.0);
    let label = world(&e, ta).center();
    let straight = draw_arrow(&mut e, label, v2(760.0, 180.0), 0.0);
    e.arrow_route = Route::Elbow;
    let elbow = draw_arrow(&mut e, v2(130.0, 180.0), v2(780.0, 200.0), 1000.0);
    e.arrow_route = Route::Straight;
    let curved = draw_arrow(&mut e, v2(270.0, 120.0), v2(840.0, 150.0), 2000.0);
    // Bend the third one from its midpoint handle: a curved route through three points.
    e.selection = vec![curved];
    let mid = e
        .connector_handles()
        .into_iter()
        .find(|(i, _)| *i == 2)
        .unwrap()
        .1;
    ev_drag(&mut e, mid, mid + v2(0.0, -80.0), 3000.0, true);
    let all = [straight, elbow, curved];
    for a in all {
        assert_eq!(ends(&e, a), (Some(ra), Some(b)), "bound");
    }
    assert_eq!(conn(&e, elbow).3, Route::Elbow);
    assert!(orthogonal(&conn(&e, elbow).0));
    let (bent, _, _, route) = conn(&e, curved);
    assert_eq!((route, bent.len()), (Route::Curved, 3));
    let before: Vec<Vec<V2>> = all.map(|a| conn(&e, a).0).to_vec();
    // Move the group (pressed on its rectangle, clear of the label and the arrows).
    e.selection = vec![ga];
    let steps = e.history.undo_len();
    ev_drag(&mut e, v2(130.0, 130.0), v2(130.0, 330.0), 4000.0, true);
    let moved = Rect::new(100.0, 300.0, 200.0, 100.0);
    assert_eq!(world(&e, ra), moved, "the group moved");
    assert_eq!(e.history.undo_len(), steps + 1, "one undo step");
    let after: Vec<Vec<V2>> = all.map(|a| conn(&e, a).0).to_vec();
    for (k, a) in all.into_iter().enumerate() {
        let pts = &after[k];
        assert_eq!(ends(&e, a), (Some(ra), Some(b)), "{k}: bound");
        assert!(on_outline(pts[0], world(&e, ra)), "{k}: {pts:?}");
        assert!(on_outline(last(pts), world(&e, b)), "{k}: {pts:?}");
        assert_ne!(*pts, before[k], "{k}: rerouted");
    }
    let down = after[0][0].y - before[0][0].y;
    assert!(down > 100.0, "the straight start followed: {down}");
    assert!(orthogonal(&after[1]), "{:?}", after[1]);
    let (bent, _, _, route) = conn(&e, curved);
    assert_eq!((route, bent.len()), (Route::Curved, 3), "still curved");
    // Undo and redo restore exactly the routed geometry of that one move.
    e.run(Action::Undo);
    assert_eq!(world(&e, ra), r);
    for (k, a) in all.into_iter().enumerate() {
        assert_eq!(conn(&e, a).0, before[k], "{k}: undo");
    }
    e.run(Action::Redo);
    for (k, a) in all.into_iter().enumerate() {
        assert_eq!(conn(&e, a).0, after[k], "{k}: redo");
    }
    e.doc.validate().unwrap();
}

#[test]
fn resizing_a_label_group_keeps_its_text_bytes_and_layout_and_the_arrow_follows() {
    let mut e = editor(1200.0, 900.0);
    e.snapping = false;
    let r0 = Rect::new(100.0, 100.0, 240.0, 120.0);
    let (g, r, t) = label_group(&mut e, r0, "Payments API");
    let b = rect(&mut e, 700.0, 120.0, 160.0, 80.0);
    let (from, to) = (world(&e, t).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    assert_eq!(ends(&e, arrow).0, Some(r));
    let label0 = text_of(&e, t);
    let doc0 = e.doc.clone();
    // Enlarge the group 1.5× from its bottom-right handle.
    e.selection = vec![g];
    let gb = world(&e, g);
    assert_eq!(gb, r0, "the group is its rectangle");
    let corner = v2(gb.right(), gb.bottom());
    let steps = e.history.undo_len();
    ev_drag(&mut e, corner, corner + v2(120.0, 60.0), 1000.0, true);
    assert_eq!(e.history.undo_len(), steps + 1, "one undo step");
    let big = Rect::new(100.0, 100.0, 360.0, 180.0);
    assert_eq!(world(&e, r), big);
    assert_eq!(text_of(&e, t), label0, "the same text and font size");
    let (h, tr) = label_laid_out(&e, t);
    assert!(h <= tr.h + 0.5, "the label fits its box: {h} > {}", tr.h);
    assert!(big.contains_rect(&world(&e, t)), "still on its rectangle");
    // A label that grew to fit its text keeps one line where it fits (its box is the text's).
    let natural = paper_rs::text::layout(&label0.0, label0.1, None, TextAlign::Left).width;
    let lines = |e: &Editor| {
        paper_rs::text::layout_node(e.doc.get(t).unwrap())
            .unwrap()
            .lines
    };
    assert_eq!(lines(&e).len(), 1);
    assert!((tr.w - natural).abs() < 0.5, "{} vs {natural}", tr.w);
    let start = conn(&e, arrow).0[0];
    assert!(on_outline(start, big), "the arrow follows: {start:?}");
    e.run(Action::Undo);
    assert_eq!(e.doc, doc0, "undo restores all of it");
    // Shrink it to half: the text is never cut or stretched, and its font isn't scaled. It
    // wraps at its space within the narrower rectangle and stays visibly on it, centred.
    ev_drag(&mut e, corner, v2(220.0, 160.0), 2000.0, true);
    let small = Rect::new(100.0, 100.0, 120.0, 60.0);
    assert_eq!(world(&e, r), small);
    assert_eq!(text_of(&e, t), label0, "the same text and font size");
    assert!(
        natural > small.w,
        "the fixture's label is wider than the half-size box"
    );
    let (h, tr) = label_laid_out(&e, t);
    assert!(h <= tr.h + 0.5, "its box holds every line: {h} > {}", tr.h);
    let wrapped = lines(&e);
    assert_eq!(wrapped.len(), 2, "two lines");
    assert_eq!(wrapped[1].start, 9, "broken at the space, before \"API\"");
    let label = world(&e, t);
    assert!(
        small.contains_rect(&label),
        "on its rectangle: {label:?} in {small:?}"
    );
    assert!(
        label.center().dist(small.center()) < 0.5,
        "centred: {label:?}"
    );
    assert_eq!(world(&e, g), small, "the group is still its rectangle");
    assert!(on_outline(conn(&e, arrow).0[0], small));
    e.run(Action::Undo);
    assert_eq!(e.doc, doc0, "one undo step");
    // Control: a group of two shapes and a text isn't a label group, and is scaled as before
    // (its text box scales with it; nothing is laid out again).
    let r2 = shape(
        &mut e,
        NodeKind::Rect,
        Rect::new(100.0, 400.0, 240.0, 120.0),
        BLUE,
    );
    let extra = rect(&mut e, 360.0, 400.0, 60.0, 60.0);
    let t2 = text_at(&mut e, "Two parts", v2(220.0, 460.0));
    e.selection = vec![r2, extra, t2];
    e.run(Action::Group);
    let g2 = e.selection[0];
    assert!(
        paper_rs::layout::label_group(&e.doc, g2).is_none(),
        "not a label group"
    );
    let t2_0 = world(&e, t2);
    let b2 = world(&e, g2);
    let c2 = v2(b2.right(), b2.bottom());
    e.selection = vec![g2];
    ev_drag(&mut e, c2, c2 - v2(b2.w * 0.5, b2.h * 0.5), 3000.0, true);
    let t2_1 = world(&e, t2);
    assert!((t2_1.w - t2_0.w * 0.5).abs() < 0.5, "{t2_0:?} -> {t2_1:?}");
    e.doc.validate().unwrap();
}

#[test]
fn duplicate_copy_paste_delete_undo_redo_keep_or_clear_the_right_bindings() {
    let mut e = editor(1400.0, 900.0);
    e.snapping = false;
    let ra0 = Rect::new(100.0, 100.0, 200.0, 100.0);
    let rb0 = Rect::new(600.0, 100.0, 200.0, 100.0);
    let (ga, ra, ta) = label_group(&mut e, ra0, "Client");
    let (gb, rb, _) = label_group(&mut e, rb0, "Server");
    let (from, to) = (world(&e, ta).center(), world(&e, rb).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    assert_eq!(ends(&e, arrow), (Some(ra), Some(rb)));
    let base = e.doc.clone();

    // 1. Duplicate one group: a copy of its rectangle and label; the arrow stays on the
    // original.
    e.selection = vec![ga];
    let steps = e.history.undo_len();
    e.run(Action::Duplicate);
    assert_eq!(e.history.undo_len(), steps + 1, "one undo step");
    let ga2 = e.selection[0];
    assert_ne!(ga2, ga);
    let kids = e.doc.get(ga2).unwrap().children.clone();
    assert_eq!(kids.len(), 2);
    assert!(matches!(e.doc.get(kids[0]).unwrap().kind, NodeKind::Rect));
    assert_eq!(text_of(&e, kids[1]).0, "Client", "the label's bytes");
    assert_eq!(connectors(&e), vec![arrow], "no arrow was added");
    assert_eq!(ends(&e, arrow).0, Some(ra), "still on the original");
    // The copy is independent: moving it leaves the arrow as it was.
    let pts = conn(&e, arrow).0;
    let at = world(&e, kids[0]).min() + v2(20.0, 20.0);
    ev_drag(&mut e, at, at + v2(0.0, 300.0), 1000.0, true);
    assert_eq!(conn(&e, arrow).0, pts);
    e.run(Action::Undo);
    e.run(Action::Undo);
    assert_eq!(e.doc, base, "both steps undone");

    // 2. Duplicate the whole diagram: the copied arrow binds the copied rectangles.
    e.selection = vec![ga, gb, arrow];
    let steps = e.history.undo_len();
    e.run(Action::Duplicate);
    assert_eq!(e.history.undo_len(), steps + 1);
    let copies = e.selection.clone();
    let copy_arrow = arrow_in(&e, &copies);
    let (s, en) = ends(&e, copy_arrow);
    let in_copies = |x: Option<NodeId>| {
        let inside = |x: NodeId| copies.iter().any(|c| *c == x || e.doc.is_ancestor(*c, x));
        x.is_some_and(inside)
    };
    assert!(in_copies(s) && in_copies(en), "remapped: {s:?} {en:?}");
    assert!(s != Some(ra) && en != Some(rb));
    assert_eq!(ends(&e, arrow), (Some(ra), Some(rb)));
    // Moving the original reroutes only the original arrow. The copies sit 16 px down-right
    // and are painted on top, so press where only the original is.
    let copy_pts = conn(&e, copy_arrow).0;
    e.selection = vec![ga];
    let press = v2(250.0, 110.0);
    assert!(world(&e, ra).contains(press));
    for c in copies.iter().filter(|c| e.doc.get(**c).unwrap().is_group()) {
        assert!(!world(&e, *c).contains(press), "only the original");
    }
    ev_drag(&mut e, press, press + v2(0.0, 300.0), 2000.0, true);
    assert_eq!(world(&e, ra).y, 400.0, "the original moved");
    assert!(on_outline(conn(&e, arrow).0[0], world(&e, ra)));
    assert_eq!(conn(&e, copy_arrow).0, copy_pts, "the copy's own");
    e.run(Action::Undo);
    e.run(Action::Undo);
    assert_eq!(e.doc, base);

    // 3. Copy a group with its arrow and paste on the same page: the start goes to the pasted
    // rectangle; the end, outside the copied set, keeps the original target.
    e.selection = vec![ga, arrow];
    e.run(Action::Copy);
    let steps = e.history.undo_len();
    e.run(Action::Paste);
    assert_eq!(e.history.undo_len(), steps + 1, "one undo step");
    let pasted = e.selection.clone();
    let (s, en) = ends(&e, arrow_in(&e, &pasted));
    assert!(s.is_some() && s != Some(ra), "start on the copy");
    assert_eq!(en, Some(rb), "same page: the end is kept");
    e.run(Action::Undo);
    assert_eq!(e.doc, base);

    // ...and on another page: a target that isn't there is let go, never left dangling.
    e.selection = vec![ga, arrow];
    e.run(Action::Copy);
    let first_page = e.active_page;
    e.add_page();
    e.run(Action::Paste);
    let pasted = e.selection.clone();
    let (s, en) = ends(&e, arrow_in(&e, &pasted));
    assert!(s.is_some_and(|x| e.doc.get(x).is_some()), "on the copy");
    assert_eq!(en, None, "not carried over to another page");
    e.doc.validate().unwrap();
    e.switch_page(first_page);
    assert_eq!(e.doc, base, "the first page is untouched");

    // 4. Delete the target group: the arrow stays and lets go; undo and redo are exact.
    e.selection = vec![gb];
    let steps = e.history.undo_len();
    e.run(Action::Delete);
    assert_eq!(e.history.undo_len(), steps + 1, "one undo step");
    assert!(e.doc.get(rb).is_none() && e.doc.get(arrow).is_some());
    assert_eq!(ends(&e, arrow).1, None, "the binding is dropped");
    let deleted = e.doc.clone();
    e.run(Action::Undo);
    assert_eq!(e.doc, base, "undo restores the binding too");
    assert_eq!(ends(&e, arrow).1, Some(rb));
    e.run(Action::Redo);
    assert_eq!(e.doc, deleted);
    e.doc.validate().unwrap();

    // 5. Sticky stays a Note: a duplicate is a sticky with the same text, and an arrow binds
    // the note itself.
    let kind = NodeKind::Sticky {
        text: "Decide later".into(),
        font_size: 24.0,
    };
    let n = Node::new(NodeId(0), kind, Rect::new(100.0, 500.0, 200.0, 200.0));
    let note = e.add_node(n, None);
    e.selection = vec![note];
    e.run(Action::Duplicate);
    let note2 = e.selection[0];
    assert!(matches!(
        &e.doc.get(note2).unwrap().kind,
        NodeKind::Sticky { text, .. } if text == "Decide later"
    ));
    // Aimed where only the original note is (its copy sits 16 px down-right, on top).
    let on_note = v2(108.0, 508.0);
    assert!(world(&e, note).contains(on_note));
    assert!(!world(&e, note2).contains(on_note));
    let from = world(&e, ra).center();
    let to_note = draw_arrow(&mut e, from, on_note, 5000.0);
    assert_eq!(ends(&e, to_note).1, Some(note));
    let kind = &e.doc.get(note).unwrap().kind;
    assert!(matches!(kind, NodeKind::Sticky { .. }), "still a note");
}

#[test]
fn a_grouped_diagram_saves_reopens_and_exports_with_its_labels_and_bindings() {
    use paper_rs::project::format::{page_from_json, page_to_json};
    let mut e = editor(1400.0, 900.0);
    e.snapping = false;
    let (ga, ra, ta) = label_group(&mut e, Rect::new(100.0, 100.0, 200.0, 100.0), "User");
    let rb0 = Rect::new(500.0, 100.0, 200.0, 100.0);
    let (_, rb, tb) = label_group(&mut e, rb0, "Orders service");
    let rc0 = Rect::new(900.0, 100.0, 200.0, 100.0);
    let (_, rc, tc) = label_group(&mut e, rc0, "Postgres");
    let (pa, pb, pc) = (
        world(&e, ta).center(),
        world(&e, tb).center(),
        world(&e, tc).center(),
    );
    let one = draw_arrow(&mut e, pa, pb, 0.0);
    e.arrow_route = Route::Elbow;
    let two = draw_arrow(&mut e, pb, pc, 1000.0);
    e.arrow_route = Route::Straight;
    assert_eq!(ends(&e, one), (Some(ra), Some(rb)));
    assert_eq!(ends(&e, two), (Some(rb), Some(rc)));

    // Save (the upstream page file every save path writes) and reopen it in a new editor.
    let saved = page_to_json(&e.page_data(0));
    let back = page_from_json(&saved).unwrap();
    assert_eq!(back.doc.nodes, e.doc.nodes, "every node, exactly");
    assert_eq!(page_to_json(&back), saved, "the same bytes again");
    let mut r = editor(1400.0, 900.0);
    r.open_pages(vec![back], None);
    r.camera = e.camera;
    r.snapping = false;
    assert_eq!(text_of(&r, tb).0, "Orders service");
    // The reopened diagram still works: moving the first group reroutes its arrow.
    let pts = conn(&r, one).0;
    r.selection = vec![ga];
    ev_drag(&mut r, v2(130.0, 130.0), v2(130.0, 330.0), 0.0, true);
    assert_eq!(ends(&r, one).0, Some(ra));
    assert!(on_outline(conn(&r, one).0[0], world(&r, ra)));
    assert_ne!(conn(&r, one).0, pts);

    // Vector export of the same document: labels by their text, arrows as stroked paths.
    let svg = paper_rs::io::svg::export(&e.doc, None, None).unwrap();
    for label in ["User", "Orders service", "Postgres"] {
        let attr = format!("aria-label=\"{label}\"");
        assert!(svg.contains(&attr), "{label}");
    }
    let element = |id: NodeId| {
        let at = svg.find(&format!("<g id=\"n{}\"", id.0)).unwrap();
        let end = at + svg[at..].find("</g>").unwrap();
        &svg[at..end]
    };
    for a in [one, two] {
        let g = element(a);
        assert!(g.contains("fill=\"none\"") && g.contains("stroke="), "{g}");
    }
    // The label is written inside its group's element.
    let g_at = svg.find(&format!("<g id=\"n{}\"", ga.0)).unwrap();
    let t_at = svg.find(&format!("<g id=\"n{}\"", ta.0)).unwrap();
    assert!(g_at < t_at);
    // A relabel shows in both exports (text is outlined, so its paths change too).
    let pages = paper_rs::io::pdf::pages_for(&e.doc, None, false, None);
    let pdf = paper_rs::io::pdf::export(&pages);
    assert!(pdf.starts_with(b"%PDF"));
    let mut relabelled = e.doc.clone();
    let node = relabelled.nodes.get_mut(&tc).unwrap();
    *node.kind.text_mut().unwrap() = "Postgres 17".into();
    let svg2 = paper_rs::io::svg::export(&relabelled, None, None).unwrap();
    assert!(svg2.contains("aria-label=\"Postgres 17\"") && svg2 != svg);
    let pages2 = paper_rs::io::pdf::pages_for(&relabelled, None, false, None);
    assert_ne!(paper_rs::io::pdf::export(&pages2), pdf);
}

/// Arrow labels and the page format (changed deliberately with the labels: until then this test
/// pinned that a reader without labels accepts a `label` on a v3 connector and drops it on its
/// next save, as the retained old603 production reader does, and refuses a newer version).
/// Now: an unlabelled page keeps exactly its v3 bytes; a labelled one is written as v4, which
/// that old reader refuses before editing instead of losing the label; and this reader refuses
/// a label in a page that says v3, rather than reading what an older reader would have lost.
#[test]
fn connector_label_format_an_unlabelled_page_stays_v3_and_a_label_needs_v4() {
    use paper_rs::project::format::{
        FormatError, PAGE_VERSION, PAGE_VERSION_LABELS, page_from_json, page_to_json,
    };
    let mut e = editor(1000.0, 700.0);
    let a = rect(&mut e, 100.0, 100.0, 160.0, 80.0);
    let b = rect(&mut e, 500.0, 100.0, 160.0, 80.0);
    let (from, to) = (world(&e, a).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    let saved = page_to_json(&e.page_data(0));
    let mut v: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(v["version"], serde_json::json!(PAGE_VERSION));
    assert!(!saved.contains("\"label\""), "no label key without a label");
    let nodes = v["nodes"].as_array_mut().unwrap();
    let c = nodes
        .iter_mut()
        .find(|n| n["id"] == serde_json::json!(arrow.0))
        .unwrap();
    assert_eq!(c["type"], "connector");
    c["label"] = serde_json::json!("writes to");
    // A label in a page that says v3: refused, never read and never dropped.
    let v3 = serde_json::to_string(&v).unwrap();
    assert!(matches!(page_from_json(&v3), Err(FormatError::Invalid(_))));
    // At v4 it is read, and written back exactly.
    v["version"] = serde_json::json!(PAGE_VERSION_LABELS);
    let v4 = serde_json::to_string(&v).unwrap();
    let read = page_from_json(&v4).unwrap();
    assert_eq!(read.doc.get(arrow).unwrap().kind.label(), Some("writes to"));
    let again = page_to_json(&read);
    assert!(again.contains("\"version\": 4") && again.contains("\"label\": \"writes to\""));
    assert_eq!(page_to_json(&page_from_json(&again).unwrap()), again);
    // Everything else reads as the unlabelled page did.
    let mut unlabelled = read.doc.nodes.clone();
    if let NodeKind::Connector { label, .. } = &mut unlabelled.get_mut(&arrow).unwrap().kind {
        *label = None;
    }
    assert_eq!(unlabelled, e.doc.nodes);
    // A newer page version is still refused, not misread.
    v["version"] = serde_json::json!(PAGE_VERSION_LABELS + 1);
    let newer = serde_json::to_string(&v).unwrap();
    assert!(matches!(
        page_from_json(&newer),
        Err(FormatError::TooNew { .. })
    ));
}

/// A click at `p` (screen = world: zoom 1, no pan).
fn click(e: &mut Editor, p: V2, t: f64) {
    e.handle(InputEvent::PointerDown {
        pos: p,
        button: Button::Primary,
        pressure: None,
        time_ms: t,
    });
    e.handle(InputEvent::PointerUp {
        pos: p,
        button: Button::Primary,
        time_ms: t + 40.0,
    });
}

/// Quick connect on a label group (revision 2 slice 7): its "+" knobs copy the whole group
/// beside it, through the duplicate transaction, and one arrow joins the two shapes. The label
/// stays the group's own editable text. Only a strict label group (one shape and its text)
/// has knobs; its label alone, free text, a group of several shapes and a group around a
/// label group don't, and a plain shape keeps its own.
#[test]
fn a_label_group_quick_connects_to_a_copy_of_itself_shape_to_shape_in_one_undo_step() {
    let mut e = editor(1400.0, 900.0);
    e.snapping = false;
    let r0 = Rect::new(100.0, 100.0, 200.0, 100.0);
    let (g, r, t) = label_group(&mut e, r0, "Orders");
    assert_eq!(paper_rs::layout::label_group(&e.doc, g), Some((r, t)));
    // A click on the label selects its group; a double-click edits the label itself.
    let on_label = world(&e, t).center();
    e.selection.clear();
    click(&mut e, on_label, 0.0);
    assert_eq!(e.selection, vec![g], "the group, not yet entered");
    click(&mut e, on_label, 100.0);
    assert_eq!(
        e.text_edit.as_ref().map(|te| te.id),
        Some(t),
        "the label is edited"
    );
    e.handle(InputEvent::Text("!".into()));
    e.end_text_edit();
    assert_eq!(text_of(&e, t).0.chars().count(), "Orders".len() + 1);
    e.run(Action::Undo);
    assert_eq!(text_of(&e, t).0, "Orders");

    // Knobs: a label group has them; the controls don't (a plain shape keeps its own).
    e.selection = vec![g];
    assert_eq!(e.quick_knobs().len(), 4, "a label group");
    e.selection = vec![t];
    assert!(e.quick_knobs().is_empty(), "its label alone");
    let free = text_at(&mut e, "Free", v2(200.0, 700.0));
    e.selection = vec![free];
    assert!(e.quick_knobs().is_empty(), "free text");
    let c = rect(&mut e, 600.0, 600.0, 120.0, 80.0);
    let d = rect(&mut e, 760.0, 600.0, 120.0, 80.0);
    let td = text_at(&mut e, "Two", v2(660.0, 640.0));
    e.selection = vec![c, d, td];
    e.run(Action::Group);
    assert!(e.quick_knobs().is_empty(), "two shapes and a text");
    let (inner, ..) = label_group(&mut e, Rect::new(1000.0, 600.0, 160.0, 80.0), "Inner");
    e.selection = vec![inner];
    e.run(Action::Group);
    assert_ne!(e.selection, vec![inner]);
    assert!(e.quick_knobs().is_empty(), "a group around a label group");
    let plain = rect(&mut e, 600.0, 300.0, 120.0, 80.0);
    e.selection = vec![plain];
    assert_eq!(e.quick_knobs().len(), 4, "control: a plain shape");

    // The right knob: a copy of the group to the right, an arrow shape to shape, one step.
    e.selection = vec![g];
    let (dir, right) = e.quick_knobs()[0];
    assert_eq!(dir, 0, "right");
    let before = e.doc.clone();
    let steps = e.history.undo_len();
    click(&mut e, right, 1000.0);
    assert_eq!(e.history.undo_len(), steps + 1, "one undo step");
    let [copy] = e.selection[..] else {
        panic!("one new selection: {:?}", e.selection)
    };
    assert_ne!(copy, g);
    let (r2, t2) = paper_rs::layout::label_group(&e.doc, copy).expect("a label group copy");
    assert_eq!(
        text_of(&e, t2),
        text_of(&e, t),
        "the label is copied as it is"
    );
    let step = v2(r0.w + 80.0, 0.0);
    assert_eq!(world(&e, r2), r0.translate(step));
    assert_eq!(world(&e, t2), world(&e, t).translate(step));
    let added: Vec<NodeId> = connectors(&e)
        .into_iter()
        .filter(|a| before.get(*a).is_none())
        .collect();
    let [arrow] = added[..] else {
        panic!("one new arrow: {added:?}")
    };
    assert_eq!(
        ends(&e, arrow),
        (Some(r), Some(r2)),
        "shapes, never labels or groups"
    );
    let pts = conn(&e, arrow).0;
    assert!(on_outline(pts[0], world(&e, r)), "{pts:?}");
    assert!(on_outline(last(&pts), world(&e, r2)), "{pts:?}");
    let after = e.doc.clone();

    // Moving either group reroutes the arrow, one step each.
    e.selection = vec![g];
    ev_drag(&mut e, v2(120.0, 120.0), v2(120.0, 320.0), 2000.0, true);
    assert_eq!(
        world(&e, r),
        r0.translate(v2(0.0, 200.0)),
        "the original moved"
    );
    let moved = conn(&e, arrow).0;
    assert!(on_outline(moved[0], world(&e, r)), "{moved:?}");
    assert_ne!(moved, pts);
    e.selection = vec![copy];
    let p = world(&e, r2).min() + v2(20.0, 20.0);
    ev_drag(&mut e, p, p + v2(0.0, 300.0), 3000.0, true);
    assert_eq!(
        world(&e, r2),
        r0.translate(step + v2(0.0, 300.0)),
        "the copy moved"
    );
    assert_eq!(ends(&e, arrow), (Some(r), Some(r2)));
    assert!(on_outline(last(&conn(&e, arrow).0), world(&e, r2)));
    assert_eq!(e.history.undo_len(), steps + 3);
    e.run(Action::Undo);
    e.run(Action::Undo);
    assert_eq!(e.doc, after, "the moves undo");
    e.run(Action::Undo);
    assert_eq!(e.doc, before, "quick connect undoes in one step");
    e.run(Action::Redo);
    assert_eq!(e.doc, after, "and redoes the same copy and arrow");

    // Deleting the target group: the arrow stays and lets go; undo binds it again.
    e.selection = vec![copy];
    e.run(Action::Delete);
    assert!(e.doc.get(r2).is_none() && e.doc.get(arrow).is_some());
    assert_eq!(ends(&e, arrow), (Some(r), None), "the end is cleared");
    e.run(Action::Undo);
    assert_eq!(e.doc, after);
    e.doc.validate().unwrap();
}

/// An agent reads back what an arrow is bound to through the existing upstream reads
/// (`get_node_info`, `get_tree_summary`), named as `create_shape` takes them, and the reads
/// follow edits, a deleted target and undo. A plain shape reads as before. Headless: the
/// upstream MCP executor on a session host, no window or project files.
#[test]
fn an_agent_reads_the_layers_an_arrow_is_bound_to_through_edits_delete_and_undo() {
    use paper_rs::mcp::{Out, exec};
    use serde_json::{Value, json};
    let root = std::env::temp_dir().join("opd-diagrams-mcp-unused");
    let mut host = exec::SessionHost::new(paper_rs::project::Store { root });
    let mut call = |tool: &str, args: Value| -> String {
        match exec::call(&mut host, tool, &args) {
            Ok(out) => match &out[..] {
                [Out::Text(t), ..] => t.clone(),
                o => panic!("{tool}: {o:?}"),
            },
            Err(e) => panic!("{tool}: {e}"),
        }
    };
    let id = |out: String| -> String {
        let v: Value = serde_json::from_str(&out).unwrap();
        v["id"].as_str().unwrap().to_string()
    };
    let box_at =
        |x: f64| json!({"type": "rectangle", "x": x, "y": 100, "width": 160, "height": 80});
    let a = id(call("create_shape", box_at(100.0)));
    let b = id(call("create_shape", box_at(500.0)));
    let arrow = call(
        "create_shape",
        json!({"type": "arrow", "startNodeId": a, "endNodeId": b, "route": "elbow"}),
    );
    let arrow = id(arrow);
    let mut info = |n: &str| -> Value {
        serde_json::from_str(&call("get_node_info", json!({"nodeId": n}))).unwrap()
    };
    let v = info(&arrow);
    assert_eq!(v["type"], "Arrow");
    assert_eq!(v["startNodeId"], json!(a));
    assert_eq!(v["endNodeId"], json!(b));
    let anchor = json!({"x": 0.5, "y": 0.5, "inside": false});
    assert_eq!(v["startAnchor"], anchor);
    assert_eq!(v["endAnchor"], anchor);
    assert_eq!(v["route"], "elbow");
    let points = v["points"].as_array().unwrap().clone();
    assert!(points.len() >= 2, "{points:?}");
    // The first routed point is at Service's outline: outside its box, within the gap.
    let p0 = v2(
        points[0][0].as_f64().unwrap(),
        points[0][1].as_f64().unwrap(),
    );
    assert!(
        on_outline(p0, Rect::new(100.0, 100.0, 160.0, 80.0)),
        "{p0:?}"
    );
    // A plain shape reads as before (control).
    let plain = info(&a);
    for k in ["startNodeId", "endNodeId", "startAnchor", "route", "points"] {
        assert!(plain.get(k).is_none(), "{k} on a rectangle");
    }
    drop(info);
    let tree = call("get_tree_summary", json!({"nodeId": "root"}));
    assert!(tree.contains(&format!("from [{a}] to [{b}]")), "{tree}");
    // An edit of the target: still bound to it, rerouted.
    call(
        "update_styles",
        json!({"updates": [{"nodeIds": [b], "styles": {"height": 300}}]}),
    );
    let read = |call: &mut dyn FnMut(&str, Value) -> String, n: &str| -> Value {
        serde_json::from_str(&call("get_node_info", json!({"nodeId": n}))).unwrap()
    };
    let v = read(&mut call, &arrow);
    assert_eq!(v["endNodeId"], json!(b));
    assert_ne!(v["points"], json!(points), "rerouted");
    let edited = v["points"].clone();
    // The target deleted: that end is free, and read so.
    call("delete_nodes", json!({"nodeIds": [b]}));
    let v = read(&mut call, &arrow);
    assert_eq!(v["startNodeId"], json!(a));
    assert_eq!(v["endNodeId"], Value::Null);
    assert_eq!(v["endAnchor"], Value::Null);
    let tree = call("get_tree_summary", json!({"nodeId": "root"}));
    assert!(tree.contains(&format!("from [{a}] to free")), "{tree}");
    // Undo: bound again, exactly as it was.
    call("undo", json!({}));
    let v = read(&mut call, &arrow);
    assert_eq!(v["endNodeId"], json!(b));
    assert_eq!(v["endAnchor"], anchor);
    assert_eq!(v["points"], edited);
}

// ---- arrow labels: text attached to the arrow itself (the Connector's `label`) ----

fn label_of(e: &Editor, id: NodeId) -> Option<String> {
    e.doc.get(id)?.kind.label().map(String::from)
}

/// The centre of an arrow's label box (world), if it has a label.
fn label_centre(e: &Editor, id: NodeId) -> Option<V2> {
    let n = e.doc.get(id)?;
    paper_rs::render::draw::label_rect(n, world(e, id)).map(|r| r.center())
}

/// The middle of an arrow's line as drawn (world).
fn line_middle(e: &Editor, id: NodeId) -> V2 {
    let (pts, _, _, route) = conn(e, id);
    paper_rs::connector::midpoint(&pts, route).unwrap()
}

/// Type `text` as an arrow's label through the editor's own text editing (Enter's command,
/// typing, Escape).
fn type_label(e: &mut Editor, arrow: NodeId, text: &str) {
    e.selection = vec![arrow];
    e.run(Action::EditText);
    assert_eq!(e.text_edit.as_ref().map(|te| te.id), Some(arrow));
    e.handle(InputEvent::Text(text.into()));
    key(e, Key::Escape, Modifiers::NONE);
}

/// A label is typed on the canvas into the arrow itself: a double-click on the arrow, typing,
/// Escape; one undo step, the same bound arrow. It sits on the middle of the line, is part of
/// the arrow for a press, and follows the line when its target moves. Enter edits it; clearing
/// it removes the label (the arrow stays) in one undo step.
#[test]
fn an_arrow_label_is_typed_on_the_canvas_follows_its_line_and_clears_in_one_step() {
    use paper_rs::editor::hit::hit_test;
    let mut e = editor(1200.0, 900.0);
    e.snapping = false;
    let a = rect(&mut e, 100.0, 100.0, 160.0, 80.0);
    let b = rect(&mut e, 600.0, 100.0, 160.0, 80.0);
    let (from, to) = (world(&e, a).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    assert_eq!(ends(&e, arrow), (Some(a), Some(b)));
    let mid = line_middle(&e, arrow);
    let above = mid + v2(0.0, -9.0);
    assert_eq!(
        hit_test(&e.doc, above, 1.0),
        None,
        "control: nothing there yet"
    );
    e.selection = vec![arrow];
    let bend = |e: &Editor| e.connector_handles().iter().any(|(i, _)| *i == 2);
    assert!(bend(&e), "control: an unlabelled arrow's bend handle");
    // A double-click on the arrow, typing and Escape.
    e.selection.clear();
    let steps = e.history.undo_len();
    click(&mut e, mid, 1000.0);
    assert_eq!(e.selection, vec![arrow]);
    click(&mut e, mid, 1100.0);
    assert_eq!(
        e.text_edit.as_ref().map(|te| te.id),
        Some(arrow),
        "its label is edited"
    );
    e.handle(InputEvent::Text("writes to".into()));
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert!(e.text_edit.is_none());
    assert_eq!(label_of(&e, arrow).as_deref(), Some("writes to"));
    assert_eq!(e.history.undo_len(), steps + 1, "one undo step");
    assert_eq!(ends(&e, arrow), (Some(a), Some(b)), "the same arrow, bound");
    assert!(
        label_centre(&e, arrow).unwrap().dist(mid) < 1e-6,
        "on the middle"
    );
    assert_eq!(
        hit_test(&e.doc, above, 1.0),
        Some(arrow),
        "part of the arrow"
    );
    e.selection = vec![arrow];
    assert!(!bend(&e), "no bend handle under the label");
    // The target moves: the arrow reroutes and the label follows the new middle.
    e.selection = vec![b];
    let p = world(&e, b).min() + v2(20.0, 20.0);
    ev_drag(&mut e, p, p + v2(0.0, 240.0), 2000.0, true);
    let moved = line_middle(&e, arrow);
    assert!(moved.dist(mid) > 50.0, "{mid:?} -> {moved:?}");
    assert!(label_centre(&e, arrow).unwrap().dist(moved) < 1e-6);
    e.run(Action::Undo);
    assert!(
        label_centre(&e, arrow).unwrap().dist(mid) < 1e-6,
        "undo: back with its line"
    );
    // Enter edits it with all of it selected, so typing replaces it.
    type_label(&mut e, arrow, "reads");
    assert_eq!(label_of(&e, arrow).as_deref(), Some("reads"));
    // Cleared: the label goes, the arrow stays, one undo step; Undo and Redo are exact.
    let labelled = e.doc.clone();
    let steps = e.history.undo_len();
    e.selection = vec![arrow];
    e.run(Action::EditText);
    key(&mut e, Key::Backspace, Modifiers::NONE);
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(label_of(&e, arrow), None);
    assert_eq!(ends(&e, arrow), (Some(a), Some(b)), "the arrow stays");
    assert_eq!(e.history.undo_len(), steps + 1);
    let cleared = e.doc.clone();
    e.run(Action::Undo);
    assert_eq!(e.doc, labelled);
    e.run(Action::Redo);
    assert_eq!(e.doc, cleared);
    // An edit begun and left empty leaves the arrow as it was.
    e.selection = vec![arrow];
    e.run(Action::EditText);
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(e.doc, cleared);
    e.doc.validate().unwrap();
}

/// A labelled arrow is an ordinary node for every copy: Duplicate copies it with its label and
/// rebinds it to the copied shapes; a paste on another page keeps the label. Saved, the page
/// is v4 and reopens with the label, which still follows a move; SVG and PDF draw it.
#[test]
fn a_labelled_arrow_is_copied_saved_reopened_and_exported_with_its_label() {
    use paper_rs::project::format::{page_from_json, page_to_json};
    let mut e = editor(1400.0, 900.0);
    e.snapping = false;
    let (ga, ra, _) = label_group(&mut e, Rect::new(100.0, 100.0, 200.0, 100.0), "Service");
    let (gb, rb, _) = label_group(&mut e, Rect::new(600.0, 100.0, 200.0, 100.0), "Queue");
    let (from, to) = (world(&e, ra).center(), world(&e, rb).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    type_label(&mut e, arrow, "publishes");
    assert_eq!(label_of(&e, arrow).as_deref(), Some("publishes"));
    // Duplicate the whole diagram: the copied arrow has the label and binds the copies.
    e.selection = vec![ga, gb, arrow];
    e.run(Action::Duplicate);
    let copy = arrow_in(&e, &e.selection.clone());
    assert_ne!(copy, arrow);
    assert_eq!(label_of(&e, copy).as_deref(), Some("publishes"));
    let (s, en) = ends(&e, copy);
    assert!(
        s.is_some_and(|x| x != ra) && en.is_some_and(|x| x != rb),
        "{s:?} {en:?}"
    );
    assert!(label_centre(&e, copy).unwrap().dist(line_middle(&e, copy)) < 1e-6);
    e.run(Action::Undo);
    // Copy is refused for labelled layers (R38-F1: the clipboard's layer format has no version,
    // so an older app would drop the label); this used to paste the labelled arrow on another
    // page. Nothing is copied, and the other page gets nothing.
    e.selection = vec![ga, arrow];
    e.run(Action::Copy);
    assert_eq!(e.status, paper_rs::io::clipboard::LABELS_REFUSED);
    let first = e.active_page;
    e.add_page();
    e.run(Action::Paste);
    assert!(e.doc.nodes.is_empty(), "nothing to paste");
    e.switch_page(first);

    // Saved as v4; reopened, the label is there and follows a move.
    let saved = page_to_json(&e.page_data(first));
    assert!(saved.contains("\"version\": 4"), "a labelled page");
    let back = page_from_json(&saved).unwrap();
    assert_eq!(back.doc.nodes, e.doc.nodes, "every node, exactly");
    let mut r = editor(1400.0, 900.0);
    r.open_pages(vec![back], None);
    r.camera = e.camera;
    r.snapping = false;
    assert_eq!(label_of(&r, arrow).as_deref(), Some("publishes"));
    let before = label_centre(&r, arrow).unwrap();
    r.selection = vec![gb];
    ev_drag(&mut r, v2(620.0, 120.0), v2(620.0, 420.0), 0.0, true);
    let after = label_centre(&r, arrow).unwrap();
    assert!(after.dist(before) > 50.0);
    assert!(after.dist(line_middle(&r, arrow)) < 1e-6);

    // SVG: the arrow's own element names the label and draws it (a backing and glyph outlines
    // besides its line and head); without the label it doesn't.
    let svg = paper_rs::io::svg::export(&e.doc, None, None).unwrap();
    assert!(svg.contains("aria-label=\"publishes\""));
    let element = |svg: &str| {
        let at = svg.find(&format!("<g id=\"n{}\"", arrow.0)).unwrap();
        let end = at + svg[at..].find("</g>").unwrap();
        svg[at..end].to_string()
    };
    let mut plain = e.doc.clone();
    if let NodeKind::Connector { label, .. } = &mut plain.nodes.get_mut(&arrow).unwrap().kind {
        *label = None;
    }
    let svg_plain = paper_rs::io::svg::export(&plain, None, None).unwrap();
    let (with, without) = (element(&svg), element(&svg_plain));
    assert!(with.contains("fill=\"#ffffff\""), "the backing: {with}");
    assert!(
        with.matches("<path").count() >= without.matches("<path").count() + 2,
        "the backing and the glyphs: {with}\n{without}"
    );
    assert!(!without.contains("aria-label"));
    // PDF draws the same ops: the label changes the file.
    let pdf = |doc: &paper_rs::model::Document| {
        let pages = paper_rs::io::pdf::pages_for(doc, None, false, None);
        paper_rs::io::pdf::export(&pages)
    };
    assert_ne!(pdf(&e.doc), pdf(&plain));
    e.doc.validate().unwrap();
}

/// An agent sets and reads an arrow's label with the existing upstream tools: create_shape's
/// `text`, get_node_info's `label`, get_tree_summary, set_text_content (exact text; empty
/// removes it) and undo. A rectangle still isn't a text layer (control).
#[test]
fn an_agent_creates_reads_sets_and_clears_an_arrow_label() {
    use paper_rs::mcp::{Out, exec};
    use serde_json::{Value, json};
    let root = std::env::temp_dir().join("opd-diagrams-mcp-labels-unused");
    let mut host = exec::SessionHost::new(paper_rs::project::Store { root });
    let mut call = |tool: &str, args: Value| -> String {
        match exec::call(&mut host, tool, &args) {
            Ok(out) => match &out[..] {
                [Out::Text(t), ..] => t.clone(),
                o => panic!("{tool}: {o:?}"),
            },
            Err(e) => panic!("{tool}: {e}"),
        }
    };
    let id = |out: String| -> String {
        let v: Value = serde_json::from_str(&out).unwrap();
        v["id"].as_str().unwrap().to_string()
    };
    let box_at =
        |x: f64| json!({"type": "rectangle", "x": x, "y": 100, "width": 160, "height": 80});
    let a = id(call("create_shape", box_at(100.0)));
    let b = id(call("create_shape", box_at(500.0)));
    let arrow = id(call(
        "create_shape",
        json!({"type": "arrow", "startNodeId": a, "endNodeId": b, "text": "writes to"}),
    ));
    let info = |call: &mut dyn FnMut(&str, Value) -> String, n: &str| -> Value {
        serde_json::from_str(&call("get_node_info", json!({"nodeId": n}))).unwrap()
    };
    let v = info(&mut call, &arrow);
    assert_eq!(v["label"], "writes to");
    assert_eq!(v["startNodeId"], json!(a));
    assert_eq!(v["endNodeId"], json!(b));
    let tree = call("get_tree_summary", json!({"nodeId": "root"}));
    assert!(
        tree.contains(&format!("\"writes to\" from [{a}] to [{b}]")),
        "{tree}"
    );
    let set = |call: &mut dyn FnMut(&str, Value) -> String, t: &str| {
        call(
            "set_text_content",
            json!({"updates": [{"nodeId": arrow, "textContent": t}]}),
        );
    };
    set(&mut call, "  reads  ");
    assert_eq!(info(&mut call, &arrow)["label"], "  reads  ", "exactly");
    set(&mut call, "");
    let v = info(&mut call, &arrow);
    assert_eq!(v["label"], Value::Null, "removed");
    assert_eq!(v["type"], "Arrow", "the arrow stays");
    assert_eq!(v["endNodeId"], json!(b));
    call("undo", json!({}));
    assert_eq!(info(&mut call, &arrow)["label"], "  reads  ");
    drop(call);
    let e = exec::call(
        &mut host,
        "set_text_content",
        &json!({"updates": [{"nodeId": a, "textContent": "x"}]}),
    )
    .unwrap_err();
    assert!(e.contains("not a text layer"), "{e}");
}

/// R37-F1: shrinking a label group and growing it back, with no Undo in between, gives the
/// label its one line again every time: a label that grew to fit its text keeps that policy
/// while it is wrapped. One undo step per resize; the text, font and style never change. A
/// width the person chose stays chosen through the same cycles (control).
#[test]
fn a_label_shrunk_and_grown_again_without_undo_returns_to_one_line_every_cycle() {
    let mut e = editor(1400.0, 1000.0);
    e.snapping = false;
    let r0 = Rect::new(100.0, 100.0, 240.0, 120.0);
    let small = Rect::new(100.0, 100.0, 120.0, 60.0);
    let (g, r, t) = label_group(&mut e, r0, "Payments API");
    let fresh = e.doc.get(t).unwrap().clone();
    let text0 = text_of(&e, t);
    let lines = |e: &Editor, t: NodeId| {
        paper_rs::text::layout_node(e.doc.get(t).unwrap())
            .unwrap()
            .lines
            .len()
    };
    let resize = |e: &mut Editor, g: NodeId, size: V2, at: f64| {
        e.selection = vec![g];
        let b = world(e, g);
        ev_drag(e, v2(b.right(), b.bottom()), b.min() + size, at, true);
    };
    for cycle in 0..3 {
        let at = 10_000.0 * cycle as f64;
        let steps = e.history.undo_len();
        resize(&mut e, g, v2(120.0, 60.0), at);
        assert_eq!(world(&e, r), small, "{cycle}");
        assert_eq!(lines(&e, t), 2, "{cycle}: wrapped at its space");
        assert!(small.contains_rect(&world(&e, t)), "{cycle}: on its shape");
        resize(&mut e, g, v2(240.0, 120.0), at + 1000.0);
        assert_eq!(world(&e, r), r0, "{cycle}");
        assert_eq!(
            e.history.undo_len(),
            steps + 2,
            "{cycle}: one step per resize"
        );
        assert_eq!(lines(&e, t), 1, "{cycle}: one line again");
        let n = e.doc.get(t).unwrap();
        assert_eq!(text_of(&e, t), text0, "{cycle}");
        assert_eq!(n.text_style, fresh.text_style, "{cycle}");
        assert!(
            matches!(
                n.kind,
                NodeKind::Text {
                    auto_width: true,
                    ..
                }
            ),
            "{cycle}: it grows to fit its text again"
        );
        assert!(
            (n.rect.w - fresh.rect.w).abs() < 0.5,
            "{cycle}: {:?}",
            n.rect
        );
        assert!(
            world(&e, t).center().dist(r0.center()) < 0.5,
            "{cycle}: centred"
        );
    }
    // Undo and Redo of the last grow are exact (the wrapped state, then one line).
    let big = e.doc.clone();
    e.run(Action::Undo);
    assert_eq!(lines(&e, t), 2);
    e.run(Action::Redo);
    assert_eq!(e.doc, big);
    // Control: a width the person chose (here 100) is kept through shrink and grow alike.
    let (g2, r2, t2) = label_group(
        &mut e,
        Rect::new(500.0, 100.0, 240.0, 120.0),
        "Payments API",
    );
    paper_rs::ui::inspector::apply(&mut e, "fixed width", &[t2], |n| {
        n.rect.w = 100.0;
        if let NodeKind::Text { auto_width, .. } = &mut n.kind {
            *auto_width = false;
        }
    });
    let chosen = e.doc.get(t2).unwrap().clone();
    for cycle in 0..2 {
        let at = 100_000.0 + 10_000.0 * cycle as f64;
        resize(&mut e, g2, v2(120.0, 60.0), at);
        assert!(
            (world(&e, t2).w - 100.0).abs() < 0.01,
            "{cycle}: kept when shrunk"
        );
        resize(&mut e, g2, v2(360.0, 180.0), at + 1000.0);
        assert_eq!(world(&e, r2).w, 360.0);
        assert!(
            (world(&e, t2).w - 100.0).abs() < 0.01,
            "{cycle}: kept when grown"
        );
        let n = e.doc.get(t2).unwrap();
        assert!(matches!(
            n.kind,
            NodeKind::Text {
                auto_width: false,
                ..
            }
        ));
        assert_eq!(n.text_style, chosen.text_style);
    }
    e.doc.validate().unwrap();
}

/// A label's width policy is its explicit sizing, never its measured width. A width the
/// person fixed is kept even at exactly its natural width or its widest wrapped line; an
/// automatic wrap is recorded as the existing `Sizing::Hug` (it hugs its lines), survives save
/// and reopen, and grows back to one line; explicit Fixed and Hug choices after a wrap, and a
/// numeric width over MCP, take effect.
#[test]
fn a_label_s_width_policy_is_its_explicit_sizing_through_wraps_saves_and_choices() {
    use paper_rs::model::Sizing;
    use paper_rs::project::format::{page_from_json, page_to_json};
    use paper_rs::ui::inspector::apply;
    let resize = |e: &mut Editor, g: NodeId, size: V2, at: f64| {
        e.selection = vec![g];
        let b = world(e, g);
        ev_drag(e, v2(b.right(), b.bottom()), b.min() + size, at, true);
    };
    let lines = |e: &Editor, t: NodeId| {
        paper_rs::text::layout_node(e.doc.get(t).unwrap())
            .unwrap()
            .lines
            .len()
    };
    let auto = |e: &Editor, t: NodeId| {
        matches!(
            e.doc.get(t).unwrap().kind,
            NodeKind::Text {
                auto_width: true,
                ..
            }
        )
    };
    let sizing = |e: &Editor, t: NodeId| e.doc.get(t).unwrap().layout_child.sizing[0];
    let fixed = |n: &mut Node| {
        n.layout_child.sizing[0] = Sizing::Fixed;
        if let NodeKind::Text { auto_width, .. } = &mut n.kind {
            *auto_width = false;
        }
    };
    let mut e = editor(1400.0, 1000.0);
    e.snapping = false;

    // Fixed at its natural width (the inspector's Fixed, nothing else changed): kept.
    let (g, r, t) = label_group(
        &mut e,
        Rect::new(100.0, 100.0, 240.0, 120.0),
        "Payments API",
    );
    let natural = world(&e, t).w;
    apply(&mut e, "Fixed", &[t], fixed);
    resize(&mut e, g, v2(480.0, 240.0), 0.0);
    assert_eq!(world(&e, r).w, 480.0);
    assert!(
        (world(&e, t).w - natural).abs() < 0.01,
        "kept at the chosen width"
    );
    assert!(!auto(&e, t) && sizing(&e, t) == Sizing::Fixed);

    // An automatic wrap is recorded as Hug, and is saved and reopened as such.
    let (g2, _, t2) = label_group(
        &mut e,
        Rect::new(100.0, 500.0, 240.0, 120.0),
        "Payments API",
    );
    resize(&mut e, g2, v2(120.0, 60.0), 1000.0);
    assert_eq!(lines(&e, t2), 2);
    assert!(
        !auto(&e, t2) && sizing(&e, t2) == Sizing::Hug,
        "wrapped, still fitting"
    );
    let saved = page_to_json(&e.page_data(0));
    assert!(
        saved.contains("\"version\": 3"),
        "an existing field: no new version"
    );
    let back = page_from_json(&saved).unwrap();
    assert_eq!(back.doc.nodes, e.doc.nodes, "exactly, sizing included");
    let mut re = editor(1400.0, 1000.0);
    re.open_pages(vec![back], None);
    re.camera = e.camera;
    re.snapping = false;
    resize(&mut re, g2, v2(240.0, 120.0), 0.0);
    assert_eq!(lines(&re, t2), 1, "reopened, it grows back to one line");
    assert!(auto(&re, t2));

    // After a wrap, an explicit Fixed at the wrapped width (its widest line) is kept.
    let widest = world(&e, t2).w;
    apply(&mut e, "Fixed", &[t2], fixed);
    resize(&mut e, g2, v2(240.0, 120.0), 2000.0);
    assert!(
        (world(&e, t2).w - widest).abs() < 0.01,
        "kept at its widest line"
    );
    assert_eq!(lines(&e, t2), 2);
    assert!(!auto(&e, t2) && sizing(&e, t2) == Sizing::Fixed);
    // ...and an explicit Hug (the inspector's: hug, auto width) makes it fit its text again.
    apply(&mut e, "Hug", &[t2], |n| {
        n.layout_child.sizing[0] = Sizing::Hug;
        if let NodeKind::Text { auto_width, .. } = &mut n.kind {
            *auto_width = true;
        }
    });
    resize(&mut e, g2, v2(260.0, 130.0), 3000.0);
    assert_eq!(lines(&e, t2), 1);
    assert!((world(&e, t2).w - natural).abs() < 0.5);
    e.doc.validate().unwrap();

    // A numeric width over MCP is an explicit Fixed too, after a wrap.
    use paper_rs::mcp::exec;
    use serde_json::json;
    let root = std::env::temp_dir().join("opd-diagrams-label-policy-unused");
    let mut h = exec::SessionHost::new(paper_rs::project::Store { root });
    h.ed = editor(1400.0, 1000.0);
    h.ed.snapping = false;
    let (g3, _, t3) = label_group(
        &mut h.ed,
        Rect::new(100.0, 100.0, 240.0, 120.0),
        "Payments API",
    );
    resize(&mut h.ed, g3, v2(120.0, 60.0), 0.0);
    assert_eq!(sizing(&h.ed, t3), Sizing::Hug);
    exec::call(
        &mut h,
        "update_styles",
        &json!({"updates": [{"nodeIds": [t3.0.to_string()], "styles": {"width": 100}}]}),
    )
    .unwrap();
    assert!(!auto(&h.ed, t3) && sizing(&h.ed, t3) == Sizing::Fixed);
    resize(&mut h.ed, g3, v2(240.0, 120.0), 1000.0);
    assert!(
        (world(&h.ed, t3).w - 100.0).abs() < 0.01,
        "the agent's width is kept"
    );
}

/// R38-F1: layers holding an arrow label are refused, visibly, by Copy and Cut (nested in a
/// group too) and by the native Library's save, before anything changes: the clipboard keeps
/// what it had, nothing is cut, no undo step, the library file stays. Duplicate (not a
/// transfer), unlabelled layers and a label's own selected characters (plain text) are
/// controls. The refusal is a transfer limit, not label transfer support.
#[test]
fn labelled_arrows_are_refused_by_copy_cut_and_the_native_library_before_any_effect() {
    use paper_rs::io::{clipboard, file};
    use paper_rs::library::{LibItem, Library};
    use std::{cell::RefCell, rc::Rc};
    struct Capture(Rc<RefCell<Option<String>>>);
    impl clipboard::Provider for Capture {
        fn read_text(&mut self) -> Option<String> {
            self.0.borrow().clone()
        }
        fn write_text(&mut self, s: &str) {
            *self.0.borrow_mut() = Some(s.into());
        }
    }
    let mut e = editor(1200.0, 900.0);
    e.snapping = false;
    let a = rect(&mut e, 100.0, 100.0, 160.0, 80.0);
    let b = rect(&mut e, 600.0, 100.0, 160.0, 80.0);
    let (from, to) = (world(&e, a).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    type_label(&mut e, arrow, "sends");
    let plain = rect(&mut e, 100.0, 500.0, 120.0, 80.0);
    let cap = Rc::new(RefCell::new(None));
    e.clipboard_io = Some(Box::new(Capture(cap.clone())));
    // Something already on the clipboard: an unlabelled copy (control).
    e.selection = vec![plain];
    e.run(Action::Copy);
    let earlier = cap.borrow().clone();
    assert!(
        earlier
            .as_deref()
            .is_some_and(|t| t.contains(clipboard::TYPE))
    );
    // The arrow alone, and the whole diagram: refused, unchanged, said.
    let doc0 = e.doc.clone();
    let steps = e.history.undo_len();
    for sel in [vec![arrow], vec![a, b, arrow]] {
        for action in [Action::Copy, Action::Cut] {
            e.selection = sel.clone();
            e.status.clear();
            e.run(action);
            assert_eq!(e.status, clipboard::LABELS_REFUSED, "{action:?} {sel:?}");
            assert_eq!(*cap.borrow(), earlier, "the clipboard keeps what it had");
            assert_eq!(e.doc, doc0, "nothing cut");
            assert_eq!(e.history.undo_len(), steps, "no undo step");
        }
    }
    // Nested: a group holding the labelled arrow is refused the same way.
    e.selection = vec![a, b, arrow];
    e.run(Action::Group);
    let g = e.selection[0];
    let grouped = e.doc.clone();
    let steps = e.history.undo_len();
    e.run(Action::Cut);
    assert_eq!(e.status, clipboard::LABELS_REFUSED);
    assert_eq!(
        (e.doc.clone(), e.history.undo_len()),
        (grouped.clone(), steps)
    );
    assert_eq!(*cap.borrow(), earlier);
    // The internal clipboard kept its earlier layers too: Paste gives the rectangle back.
    e.run(Action::Paste);
    let [pasted] = e.selection[..] else {
        panic!("one pasted layer")
    };
    assert!(matches!(e.doc.get(pasted).unwrap().kind, NodeKind::Rect));
    e.run(Action::Undo);
    assert_eq!(e.doc, grouped);
    // Control: Duplicate copies the labelled arrow exactly, inside the document.
    e.selection = vec![g];
    e.run(Action::Duplicate);
    let copy = e.selection[0];
    let labels: Vec<String> = e
        .doc
        .subtree(copy)
        .into_iter()
        .filter_map(|id| label_of(&e, id))
        .collect();
    assert_eq!(labels, vec!["sends".to_string()]);
    e.run(Action::Undo);
    // Control: unlabelled layers cut as before.
    e.selection = vec![plain];
    e.run(Action::Cut);
    assert!(e.doc.get(plain).is_none());
    e.run(Action::Undo);
    // A label's own selected characters are copied as plain text; Cut takes only them, in the
    // label's edit (one undo step).
    e.selection = vec![arrow];
    e.run(Action::EditText);
    key(&mut e, Key::Home, Modifiers::NONE);
    key(&mut e, Key::Right, Modifiers::SHIFT);
    key(&mut e, Key::Right, Modifiers::SHIFT);
    e.run(Action::Copy);
    assert_eq!(cap.borrow().as_deref(), Some("se"), "plain text, no layers");
    let steps = e.history.undo_len();
    e.run(Action::Cut);
    assert_eq!(cap.borrow().as_deref(), Some("se"));
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(label_of(&e, arrow).as_deref(), Some("nds"));
    assert!(e.doc.get(arrow).is_some(), "the arrow stays");
    assert_eq!(e.history.undo_len(), steps + 1);
    e.run(Action::Undo);
    assert_eq!(label_of(&e, arrow).as_deref(), Some("sends"));

    // Native Library: a labelled item is refused by save (whoever built it); the file stays.
    let item = LibItem::from_selection(&e.doc, &[g], "Flow").unwrap();
    assert!(item.has_labels());
    let path = std::env::temp_dir().join(format!("opd-label-library-{}.json", std::process::id()));
    std::fs::write(&path, b"existing library").unwrap();
    let lib = Library {
        items: vec![item],
        path: Some(path.clone()),
        ..Library::default()
    };
    let err = lib.save().unwrap_err();
    assert_eq!(err.to_string(), paper_rs::library::LABELS_REFUSED);
    assert_eq!(std::fs::read(&path).unwrap(), b"existing library");
    // Control: an unlabelled item saves and loads.
    let boxed = LibItem::from_selection(&e.doc, &[plain], "Box").unwrap();
    let lib = Library {
        items: vec![boxed],
        path: Some(path.clone()),
        ..Library::default()
    };
    lib.save().unwrap();
    assert_eq!(Library::load(&path).items, lib.items);
    std::fs::remove_file(&path).ok();
    // The single-file format's checked encoder refuses labels as `save` does.
    assert!(file::to_string_checked(&e.doc, None).is_err());
    e.doc.validate().unwrap();
}

/// R15: Paste while an arrow's label is edited (a bound, grouped arrow) edits that label: the
/// clipboard's text replaces the selected characters or goes in at the caret, as plain text,
/// whatever it looks like (Unicode, several lines, a layer clipboard's JSON, SVG). It is never
/// read as layers, SVG or a new text layer; the arrow, its bindings, its group and every other
/// layer stay exactly as they were, and each edit is one undo step. Controls: Paste with no
/// label edited still pastes text as a new text layer and layers as layers.
#[test]
fn paste_while_editing_an_arrow_label_edits_that_label_as_plain_text_in_one_step() {
    use paper_rs::io::clipboard;
    use std::{cell::RefCell, rc::Rc};
    struct Board(Rc<RefCell<Option<String>>>);
    impl clipboard::Provider for Board {
        fn read_text(&mut self) -> Option<String> {
            self.0.borrow().clone()
        }
        fn write_text(&mut self, s: &str) {
            *self.0.borrow_mut() = Some(s.into());
        }
    }
    let mut e = editor(1200.0, 900.0);
    e.snapping = false;
    let a = rect(&mut e, 100.0, 100.0, 160.0, 80.0);
    let b = rect(&mut e, 600.0, 100.0, 160.0, 80.0);
    let (from, to) = (world(&e, a).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    type_label(&mut e, arrow, "sends");
    e.selection = vec![a, b, arrow];
    e.run(Action::Group);
    let g = e.selection[0];
    let board = Rc::new(RefCell::new(None));
    e.clipboard_io = Some(Box::new(Board(board.clone())));
    // Everything but the arrow's label: the same nodes, bindings, group and order.
    let unlabelled = |e: &Editor| {
        let mut nodes = e.doc.nodes.clone();
        if let NodeKind::Connector { label, .. } = &mut nodes.get_mut(&arrow).unwrap().kind {
            *label = None;
        }
        nodes
    };
    let rest = unlabelled(&e);
    let edit = |e: &mut Editor, text: &str, select: bool| {
        *board.borrow_mut() = Some(text.into());
        let steps = e.history.undo_len();
        let before = e.doc.clone();
        e.selection = vec![arrow];
        e.run(Action::EditText);
        if select {
            // The first two characters.
            key(e, Key::Home, Modifiers::NONE);
            key(e, Key::Right, Modifiers::SHIFT);
            key(e, Key::Right, Modifiers::SHIFT);
        } else {
            key(e, Key::End, Modifiers::NONE);
        }
        e.run(Action::Paste);
        let editing = e.text_edit.as_ref().map(|te| te.id);
        assert_eq!(editing, Some(arrow), "still editing it");
        assert!(
            e.text_selection().is_none(),
            "the caret after the pasted text"
        );
        key(e, Key::Escape, Modifiers::NONE);
        assert_eq!(e.history.undo_len(), steps + 1, "one undo step: {text:?}");
        assert_eq!(unlabelled(e), rest, "nothing else changed: {text:?}");
        assert_eq!(ends(e, arrow), (Some(a), Some(b)), "the same bound arrow");
        assert_eq!(e.doc.get(arrow).unwrap().parent, Some(g), "in its group");
        assert_eq!(e.selection, vec![arrow]);
        let after = e.doc.clone();
        e.run(Action::Undo);
        assert_eq!(e.doc, before, "undo: {text:?}");
        e.run(Action::Redo);
        assert_eq!(e.doc, after, "redo: {text:?}");
        label_of(e, arrow).unwrap()
    };
    // The selected characters replaced: Unicode over several lines (CRLF as typed Enter).
    assert_eq!(edit(&mut e, "Ω🙂\r\nline 2", true), "Ω🙂\nline 2nds");
    // At the caret: a layer clipboard's JSON and SVG are only text here.
    let json = format!("{{\"type\":\"{}\",\"nodes\":[]}}", clipboard::TYPE);
    let label = edit(&mut e, &json, false);
    assert_eq!(label, format!("Ω🙂\nline 2nds{json}"));
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"9\"/></svg>";
    assert_eq!(edit(&mut e, svg, false), format!("{label}{svg}"));
    // A tab is a space and other control characters are dropped, as typed.
    let label = label_of(&e, arrow).unwrap();
    assert_eq!(edit(&mut e, "a\tb\u{7}c", false), format!("{label}a bc"));
    // Nothing to paste: no change and no undo step.
    *board.borrow_mut() = None;
    let doc0 = e.doc.clone();
    let steps = e.history.undo_len();
    e.selection = vec![arrow];
    e.run(Action::EditText);
    e.run(Action::Paste);
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!((e.doc.clone(), e.history.undo_len()), (doc0.clone(), steps));
    // Controls, with no label edited: text pastes as a new text layer, in one step...
    *board.borrow_mut() = Some("Ω🙂 note".into());
    e.selection.clear();
    e.run(Action::Paste);
    let [t] = e.selection[..] else {
        panic!("one pasted text layer")
    };
    assert_eq!(text_of(&e, t).0, "Ω🙂 note");
    assert_eq!(e.history.undo_len(), steps + 1);
    e.run(Action::Undo);
    assert_eq!(e.doc, doc0);
    // ...and an unlabelled layer copied from the canvas pastes as a layer.
    let plain = rect(&mut e, 100.0, 500.0, 120.0, 80.0);
    e.selection = vec![plain];
    e.run(Action::Copy);
    e.run(Action::Paste);
    let [p] = e.selection[..] else {
        panic!("one pasted layer")
    };
    assert_ne!(p, plain);
    assert!(matches!(e.doc.get(p).unwrap().kind, NodeKind::Rect));
    e.doc.validate().unwrap();
}

/// R43-F1: Paste never changes the label of a locked arrow, even while that label is still
/// being edited. Enter edits the label, Lock (⌘⇧L) locks the arrow and empties the selection
/// while the edit stays open, and ⌘V then changes nothing: the label, the document, the
/// selection and the undo history are as Lock left them, nothing is pasted anywhere else, and
/// the clipboard isn't even read. The same holds when the lock is the arrow's group's. Unlocked
/// again (control), the same keys paste into the label as before.
#[test]
fn paste_never_changes_the_label_of_an_arrow_locked_while_its_label_is_edited() {
    use paper_rs::io::clipboard;
    use std::{cell::RefCell, rc::Rc};
    /// The clipboard's text and how many times it was read.
    struct Board(Rc<RefCell<(Option<String>, usize)>>);
    impl clipboard::Provider for Board {
        fn read_text(&mut self) -> Option<String> {
            let mut b = self.0.borrow_mut();
            b.1 += 1;
            b.0.clone()
        }
        fn write_text(&mut self, s: &str) {
            self.0.borrow_mut().0 = Some(s.into());
        }
    }
    const PASTED: &str = "PASTED INTO LOCKED LABEL";
    let mut e = editor(1200.0, 900.0);
    e.snapping = false;
    let a = rect(&mut e, 100.0, 100.0, 160.0, 80.0);
    let b = rect(&mut e, 600.0, 100.0, 160.0, 80.0);
    let (from, to) = (world(&e, a).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    type_label(&mut e, arrow, "protected");
    let board = Rc::new(RefCell::new((Some(PASTED.to_string()), 0)));
    e.clipboard_io = Some(Box::new(Board(board.clone())));
    let reads = |board: &Rc<RefCell<(Option<String>, usize)>>| board.borrow().1;
    // Paste with the label open and the arrow locked: nothing at all happens.
    let refused = |e: &mut Editor, why: &str| {
        assert!(e.doc.is_effectively_locked(arrow), "{why}: locked");
        assert_eq!(e.text_edit.as_ref().map(|te| te.id), Some(arrow), "{why}");
        let (doc, steps) = (e.doc.clone(), e.history.undo_len());
        let selection = e.selection.clone();
        let edit = e.text_edit.as_ref().map(|t| (t.id, t.caret, t.anchor));
        let read = reads(&board);
        key(e, Key::Char('v'), Modifiers::cmd());
        assert_eq!(label_of(e, arrow).as_deref(), Some("protected"), "{why}");
        assert_eq!(e.doc, doc, "{why}: nothing pasted instead");
        assert_eq!(e.history.undo_len(), steps, "{why}: nothing recorded");
        assert_eq!(e.selection, selection, "{why}");
        let after = e.text_edit.as_ref().map(|t| (t.id, t.caret, t.anchor));
        assert_eq!(after, edit, "{why}: the edit as it was");
        assert_eq!(reads(&board), read, "{why}: the clipboard isn't read");
        key(e, Key::Escape, Modifiers::NONE);
        assert_eq!(label_of(e, arrow).as_deref(), Some("protected"), "{why}");
        assert!(e.doc.is_effectively_locked(arrow), "{why}: still locked");
        assert_eq!(e.doc.nodes.len(), doc.nodes.len(), "{why}: no extra node");
    };
    // The arrow itself, locked by the shortcut while its label is open.
    e.selection = vec![arrow];
    key(&mut e, Key::Enter, Modifiers::NONE);
    let editing = e.text_edit.as_ref().map(|te| te.id);
    assert_eq!(editing, Some(arrow), "Enter edits it");
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(e.doc.get(arrow).unwrap().locked, "Lock reached the arrow");
    assert!(e.selection.is_empty(), "Lock empties the selection");
    refused(&mut e, "the arrow");
    // Control: unlocked again, the same keys paste into its label (Enter selects all of it).
    e.selection = vec![arrow];
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(!e.doc.is_effectively_locked(arrow), "unlocked");
    e.selection = vec![arrow];
    key(&mut e, Key::Enter, Modifiers::NONE);
    let steps = e.history.undo_len();
    key(&mut e, Key::Char('v'), Modifiers::cmd());
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(label_of(&e, arrow).as_deref(), Some(PASTED), "pasted");
    assert_eq!(e.history.undo_len(), steps + 1, "one undo step");
    e.run(Action::Undo);
    assert_eq!(label_of(&e, arrow).as_deref(), Some("protected"));
    // Its group locked (fixture: the group and then the arrow are selected directly, as a
    // locked group's arrow can't be picked on the canvas): Paste is refused the same way.
    e.selection = vec![a, b, arrow];
    e.run(Action::Group);
    let g = e.selection[0];
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(e.doc.get(g).unwrap().locked, "the group is locked");
    assert!(!e.doc.get(arrow).unwrap().locked, "the arrow itself isn't");
    e.selection = vec![arrow];
    key(&mut e, Key::Enter, Modifiers::NONE);
    assert_eq!(e.text_edit.as_ref().map(|te| te.id), Some(arrow));
    refused(&mut e, "its group");
    // Control: the group unlocked, Paste edits the label again.
    e.selection = vec![g];
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(!e.doc.is_effectively_locked(arrow));
    e.selection = vec![arrow];
    key(&mut e, Key::Enter, Modifiers::NONE);
    key(&mut e, Key::Char('v'), Modifiers::cmd());
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(label_of(&e, arrow).as_deref(), Some(PASTED));
    assert_eq!(ends(&e, arrow), (Some(a), Some(b)), "the same bound arrow");
    assert_eq!(e.doc.get(arrow).unwrap().parent, Some(g), "in its group");
    e.doc.validate().unwrap();
}

/// A clipboard that counts how often it is read and written.
#[derive(Default)]
struct Clip {
    text: Option<String>,
    reads: usize,
    writes: usize,
}

type SharedClip = std::rc::Rc<std::cell::RefCell<Clip>>;

struct CountingBoard(SharedClip);

impl paper_rs::io::clipboard::Provider for CountingBoard {
    fn read_text(&mut self) -> Option<String> {
        let mut c = self.0.borrow_mut();
        c.reads += 1;
        c.text.clone()
    }
    fn write_text(&mut self, s: &str) {
        let mut c = self.0.borrow_mut();
        c.writes += 1;
        c.text = Some(s.into());
    }
}

/// Every way an open edit could change its locked target's text, or fall back to the selected
/// layers: typed text (with no modifier held, so a held command key can't make it pass for the
/// wrong reason), Enter, Backspace, Delete, Bold (⌘B), Cut (⌘X) and Paste (⌘V). Each is
/// refused: the document (so every selected layer too), the history, the document stamp, the
/// selection, the edit with its caret and selected characters, and the clipboard (neither read
/// nor written) are exactly as they were.
fn refuses_every_change(e: &mut Editor, target: NodeId, clip: &SharedClip, why: &str) {
    assert!(e.doc.is_effectively_locked(target), "{why}: locked");
    let edit = |e: &Editor| e.text_edit.as_ref().map(|t| (t.id, t.caret, t.anchor));
    let open = edit(e);
    assert_eq!(open.map(|o| o.0), Some(target), "{why}: the edit is open");
    let doc = e.doc.clone();
    let state = (e.history.undo_len(), e.doc_stamp);
    let selection = e.selection.clone();
    let counts = (clip.borrow().reads, clip.borrow().writes);
    e.handle(InputEvent::Modifiers(Modifiers::NONE));
    e.handle(InputEvent::Text("Ω🙂x".into()));
    for k in [Key::Enter, Key::Backspace, Key::Delete] {
        key(e, k, Modifiers::NONE);
    }
    for c in ['b', 'x', 'v'] {
        key(e, Key::Char(c), Modifiers::cmd());
    }
    assert_eq!(e.doc, doc, "{why}: the document is unchanged");
    let now = (e.history.undo_len(), e.doc_stamp);
    assert_eq!(now, state, "{why}: no history step, nothing to save");
    assert_eq!(e.selection, selection, "{why}");
    assert_eq!(edit(e), open, "{why}: the edit as it was");
    let after = (clip.borrow().reads, clip.borrow().writes);
    assert_eq!(after, counts, "{why}: the clipboard isn't touched");
}

/// R18/R15: an edit left open on a layer that gets locked (Lock while editing empties the
/// selection but not the edit) changes nothing more. Typing, Enter, Backspace, Delete, Bold, Cut
/// and Paste are refused for an arrow locked itself (by the real shortcut) and for a text layer
/// whose group is locked, with nothing or another layer selected; nothing is copied, deleted or
/// pasted instead, and the edit stays open with its caret. Copy still copies. Unlocked with the
/// edit still open, typing changes the text again; a fresh edit replaces selected characters
/// with Unicode in one undo step (controls).
#[test]
fn a_text_edit_left_open_on_a_locked_layer_changes_nothing_until_it_is_unlocked() {
    let mut e = editor(1200.0, 900.0);
    e.snapping = false;
    let a = rect(&mut e, 100.0, 100.0, 160.0, 80.0);
    let b = rect(&mut e, 600.0, 100.0, 160.0, 80.0);
    let (from, to) = (world(&e, a).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    type_label(&mut e, arrow, "sends");
    let other = rect(&mut e, 100.0, 600.0, 120.0, 80.0);
    let clip = SharedClip::default();
    e.clipboard_io = Some(Box::new(CountingBoard(clip.clone())));

    // The arrow itself, locked by ⌘⇧L while Enter's edit of its label is open.
    e.selection = vec![arrow];
    key(&mut e, Key::Enter, Modifiers::NONE);
    assert_eq!(e.text_selection(), Some((0, 5)), "Enter selects all");
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(e.doc.get(arrow).unwrap().locked, "Lock reached the arrow");
    assert!(e.selection.is_empty(), "Lock empties the selection");
    refuses_every_change(&mut e, arrow, &clip, "arrow, nothing selected");
    // Fixture: another layer selected directly while the edit stays open.
    e.selection = vec![other];
    refuses_every_change(&mut e, arrow, &clip, "arrow, another layer selected");
    assert_eq!(label_of(&e, arrow).as_deref(), Some("sends"));
    assert_eq!(ends(&e, arrow), (Some(a), Some(b)), "still bound");
    // Copy changes nothing and still copies the selected characters.
    key(&mut e, Key::Char('c'), Modifiers::cmd());
    assert_eq!(clip.borrow().text.as_deref(), Some("sends"));
    // Unlocked (fixture: the arrow selected again for ⌘⇧L), the open edit types again.
    e.selection = vec![arrow];
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(!e.doc.is_effectively_locked(arrow), "unlocked");
    assert_eq!(e.text_selection(), Some((0, 5)), "its range kept");
    e.handle(InputEvent::Text("Ω🙂".into()));
    assert_eq!(label_of(&e, arrow).as_deref(), Some("Ω🙂"));
    let caret = e.text_edit.as_ref().map(|t| (t.caret, t.anchor));
    assert_eq!(caret, Some((2, None)), "the caret after the new text");
    key(&mut e, Key::Escape, Modifiers::NONE);
    // Control: a fresh edit replaces a selected character, in one undo step.
    let before = e.doc.clone();
    let steps = e.history.undo_len();
    e.selection = vec![arrow];
    key(&mut e, Key::Enter, Modifiers::NONE);
    key(&mut e, Key::Home, Modifiers::NONE);
    key(&mut e, Key::Right, Modifiers::SHIFT);
    e.handle(InputEvent::Text("é".into()));
    assert_eq!(e.text_edit.as_ref().map(|t| t.caret), Some(1));
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(label_of(&e, arrow).as_deref(), Some("é🙂"));
    assert_eq!(e.history.undo_len(), steps + 1, "one undo step");
    let after = e.doc.clone();
    e.run(Action::Undo);
    assert_eq!(e.doc, before);
    e.run(Action::Redo);
    assert_eq!(e.doc, after);

    // A text layer whose group is locked (fixture: the text, then its group, selected
    // directly; a locked group's text isn't picked on the canvas).
    let (g, _, t) = label_group(&mut e, Rect::new(600.0, 500.0, 200.0, 100.0), "Orders");
    e.selection = vec![t];
    key(&mut e, Key::Enter, Modifiers::NONE);
    key(&mut e, Key::End, Modifiers::NONE);
    assert_eq!(e.text_edit.as_ref().map(|te| te.id), Some(t));
    e.selection = vec![g];
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(e.doc.get(g).unwrap().locked, "the group is locked");
    assert!(!e.doc.get(t).unwrap().locked, "the text itself isn't");
    refuses_every_change(&mut e, t, &clip, "text, nothing selected");
    // Fixture: another layer selected directly. Cut and Paste don't fall back to it: it is
    // neither cut (copied and deleted) nor pasted beside.
    e.selection = vec![other];
    refuses_every_change(&mut e, t, &clip, "text, another layer selected");
    assert!(e.doc.get(other).is_some(), "the other layer stays");
    assert_eq!(text_of(&e, t).0, "Orders");
    // Unlocked, the open edit types again, at its caret.
    e.selection = vec![g];
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(!e.doc.is_effectively_locked(t));
    e.handle(InputEvent::Text("!".into()));
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(text_of(&e, t).0, "Orders!");
    e.doc.validate().unwrap();
}

/// R18/R15: leaving (Escape) an edit whose layer was locked while it was open changes no
/// locked data. An emptied text layer in a locked group stays, empty, and a blank label on a
/// locked arrow stays blank, with its bindings: Escape only ends the edit. Unlocked, Escape
/// still removes an empty text layer and clears a blank label, as before (controls). Group
/// and text are selected directly (fixture; a locked group's text isn't picked on the canvas).
#[test]
fn leaving_an_edit_on_a_locked_layer_keeps_its_empty_text_and_blank_label() {
    let mut e = editor(1200.0, 900.0);
    e.snapping = false;
    let empty_out = |e: &mut Editor, t: NodeId| {
        e.selection = vec![t];
        key(e, Key::Enter, Modifiers::NONE);
        key(e, Key::Home, Modifiers::NONE);
        key(e, Key::End, Modifiers::SHIFT);
        key(e, Key::Backspace, Modifiers::NONE);
        assert_eq!(text_of(e, t).0, "", "emptied, still a layer");
    };
    // An emptied text, its group locked while the edit is open.
    let (g, _, t) = label_group(&mut e, Rect::new(100.0, 100.0, 200.0, 100.0), "Orders");
    empty_out(&mut e, t);
    e.selection = vec![g];
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(e.doc.is_effectively_locked(t), "locked through its group");
    let editing = e.text_edit.as_ref().map(|te| te.id);
    assert_eq!(editing, Some(t), "still edited");
    let doc = e.doc.clone();
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert!(e.text_edit.is_none(), "Escape leaves the edit");
    assert_eq!(e.doc, doc, "the locked empty text stays as it was");
    assert_eq!(text_of(&e, t).0, "");
    // Control: an unlocked emptied text is removed when the edit ends.
    let (_, _, t2) = label_group(&mut e, Rect::new(100.0, 300.0, 200.0, 100.0), "Notes");
    empty_out(&mut e, t2);
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert!(e.doc.get(t2).is_none(), "removed, as before");

    // A blank label, its arrow locked while the edit is open.
    let a = rect(&mut e, 500.0, 500.0, 120.0, 80.0);
    let b = rect(&mut e, 900.0, 500.0, 120.0, 80.0);
    let (from, to) = (world(&e, a).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 20_000.0);
    e.selection = vec![arrow];
    key(&mut e, Key::Enter, Modifiers::NONE);
    e.handle(InputEvent::Modifiers(Modifiers::NONE));
    e.handle(InputEvent::Text("   ".into()));
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(e.doc.is_effectively_locked(arrow), "locked");
    let editing = e.text_edit.as_ref().map(|te| te.id);
    assert_eq!(editing, Some(arrow), "still edited");
    assert_eq!(label_of(&e, arrow).as_deref(), Some("   "));
    let doc = e.doc.clone();
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert!(e.text_edit.is_none(), "Escape leaves the edit");
    assert_eq!(e.doc, doc, "the locked blank label stays as it was");
    assert_eq!(label_of(&e, arrow).as_deref(), Some("   "));
    assert_eq!(ends(&e, arrow), (Some(a), Some(b)), "still bound");
    // Control: unlocked, an edit ended on the blank label clears it (the arrow stays).
    e.selection = vec![arrow];
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(!e.doc.is_effectively_locked(arrow), "unlocked");
    key(&mut e, Key::Enter, Modifiers::NONE);
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(label_of(&e, arrow), None, "cleared, as before");
    assert_eq!(ends(&e, arrow), (Some(a), Some(b)));
    e.doc.validate().unwrap();
}

fn inside_rounded(p: V2, r: Rect, radius: f64) -> bool {
    if !r.contains(p) {
        return false;
    }
    let x = p.x.clamp(r.x + radius, r.right() - radius);
    let y = p.y.clamp(r.y + radius, r.bottom() - radius);
    p.dist(v2(x, y)) <= radius + 1e-9
}

/// Points sampled along polylines, and around them at a stroke's half width.
fn stroke_samples(lines: &[Vec<V2>], width: f64) -> Vec<V2> {
    let mut out = Vec::new();
    for line in lines {
        for seg in line.windows(2) {
            for i in 0..=100 {
                let at = seg[0].lerp(seg[1], i as f64 / 100.0);
                for j in 0..36 {
                    let a = j as f64 / 36.0 * std::f64::consts::TAU;
                    out.push(at + v2(a.cos(), a.sin()) * (width / 2.0));
                }
            }
        }
    }
    out
}

/// A free arrow through world points `pts`, 2 px, an Arrow end and a label.
fn free_arrow(e: &mut Editor, pts: &[V2], route: Route, label: &str) -> NodeId {
    use paper_rs::model::Head;
    let lo = pts
        .iter()
        .fold(pts[0], |m, p| v2(m.x.min(p.x), m.y.min(p.y)));
    let hi = pts
        .iter()
        .fold(pts[0], |m, p| v2(m.x.max(p.x), m.y.max(p.y)));
    let kind = NodeKind::Connector {
        points: pts.iter().map(|p| *p - lo).collect(),
        start: None,
        end: None,
        route,
        start_head: Head::None,
        end_head: Head::Arrow,
        label: Some(label.into()),
    };
    let mut n = Node::new(
        NodeId(0),
        kind,
        Rect::new(lo.x, lo.y, hi.x - lo.x, hi.y - lo.y),
    );
    n.style.stroke = Some(Color::INK);
    n.style.stroke_width = 2.0;
    e.add_node(n, None)
}

/// R38-F2: a short arrow with a longer label keeps both its direction and its readable label.
/// The R38 browser fixture's arrow (78 px, (346,210)→(424,210), 2 px Arrow end, "dispatches")
/// and other orientations: no sampled point of the head's stroke (nor, for straight arrows, of
/// the whole line) is under the label's backing, in the very ops every renderer draws. One
/// shared placement serves drawing, hit testing and text editing; a long arrow keeps its
/// label centred on the line. The short arrow's bend handle is free again, and a double-click
/// on its label still edits it.
#[test]
fn a_short_arrow_keeps_its_direction_visible_beside_a_longer_label() {
    use paper_rs::model::Head;
    use paper_rs::render::draw;
    let s = |x: f64, y: f64| v2(x, y);
    let cases: Vec<(Vec<V2>, Route)> = vec![
        (vec![s(346.0, 210.0), s(424.0, 210.0)], Route::Straight),
        (vec![s(424.0, 210.0), s(346.0, 210.0)], Route::Straight),
        (vec![s(400.0, 150.0), s(400.0, 228.0)], Route::Straight),
        (vec![s(400.0, 228.0), s(400.0, 150.0)], Route::Straight),
        (vec![s(350.0, 150.0), s(405.0, 205.0)], Route::Straight),
        (
            vec![s(346.0, 210.0), s(385.0, 185.0), s(424.0, 210.0)],
            Route::Curved,
        ),
        (
            vec![
                s(346.0, 210.0),
                s(385.0, 210.0),
                s(385.0, 240.0),
                s(424.0, 240.0),
            ],
            Route::Elbow,
        ),
    ];
    for (k, (pts, route)) in cases.into_iter().enumerate() {
        let mut e = editor(1200.0, 900.0);
        let id = free_arrow(&mut e, &pts, route, "dispatches");
        let n = e.doc.get(id).unwrap().clone();
        let wr = world(&e, id);
        let lr = draw::label_rect(&n, wr).unwrap();
        let world_pts = conn(&e, id).0;
        let (tip, from) = (last(&world_pts), world_pts[world_pts.len() - 2]);
        let (heads, _) =
            paper_rs::connector::head_shapes(Head::Arrow, tip, from, 2.0, tip.dist(from));
        let covered = stroke_samples(&heads, 2.0)
            .into_iter()
            .filter(|p| inside_rounded(*p, lr, 4.0))
            .count();
        assert_eq!(covered, 0, "{k}: the head is visible beside {lr:?}");
        if route == Route::Straight {
            let line = stroke_samples(&[world_pts.clone()], 2.0);
            assert!(
                line.iter().all(|p| !inside_rounded(*p, lr, 4.0)),
                "{k}: the whole line is clear of the label"
            );
        }
        // The ops every renderer draws (GPU scene, SVG, PDF): the backing is that rect, the
        // glyphs are inside it, and nothing of the arrow is drawn after them.
        let ops = draw::node_draw(&n, wr, 1.0).ops;
        let white = |op: &draw::DrawOp| matches!(op, draw::DrawOp::Fill { paint: draw::PaintSpec::Solid(c), .. } if *c == [1.0; 4]);
        let backing = ops.iter().position(white).expect("a backing");
        let draw::DrawOp::Fill { path, .. } = &ops[backing] else {
            unreachable!()
        };
        let bb = path.bounds().unwrap();
        assert!(
            (bb.x - lr.x).abs() < 1e-6 && (bb.w - lr.w).abs() < 1e-6,
            "{k}"
        );
        for op in &ops[backing + 1..] {
            let draw::DrawOp::Glyphs { glyphs, .. } = op else {
                panic!("{k}: only glyphs after the backing: {op:?}")
            };
            assert!(glyphs.iter().all(|(_, p)| lr.contains(*p)), "{k}");
        }
        assert!(
            draw::draw_bounds(&n, wr).contains_rect(&lr),
            "{k}: bounds and culling"
        );
        // Hit testing and text editing use the same place.
        assert_eq!(
            paper_rs::editor::hit::hit_test(&e.doc, lr.center(), 1.0),
            Some(id)
        );
        e.selection.clear();
        click(&mut e, lr.center(), 1000.0);
        click(&mut e, lr.center(), 1100.0);
        assert_eq!(
            e.text_edit.as_ref().map(|te| te.id),
            Some(id),
            "{k}: edited"
        );
        key(&mut e, Key::Escape, Modifiers::NONE);
        assert_eq!(label_of(&e, id).as_deref(), Some("dispatches"));
        assert_eq!(e.doc.get(id).unwrap(), &n, "{k}: nothing else changed");
    }
    // The fixture arrow's bend handle is free beside the label, and a drag still bends it.
    let mut e = editor(1200.0, 900.0);
    let id = free_arrow(
        &mut e,
        &[s(346.0, 210.0), s(424.0, 210.0)],
        Route::Straight,
        "dispatches",
    );
    e.selection = vec![id];
    let mid = e
        .connector_handles()
        .into_iter()
        .find(|(i, _)| *i == 2)
        .expect("a bend handle beside the label")
        .1;
    ev_drag(&mut e, mid, mid + v2(0.0, 50.0), 2000.0, true);
    let (bent, _, _, route) = conn(&e, id);
    assert_eq!((route, bent.len()), (Route::Curved, 3));
    assert_eq!(label_of(&e, id).as_deref(), Some("dispatches"));
    // Control: a long arrow keeps its label centred on the line, under which there is no
    // bend handle.
    let mut e = editor(1200.0, 900.0);
    let id = free_arrow(
        &mut e,
        &[s(100.0, 210.0), s(700.0, 210.0)],
        Route::Straight,
        "dispatches",
    );
    assert!(label_centre(&e, id).unwrap().dist(line_middle(&e, id)) < 1e-6);
    e.selection = vec![id];
    assert!(!e.connector_handles().iter().any(|(i, _)| *i == 2));
}

/// A double-click on the middle of an arrow that is already selected edits its label: the
/// first click lands on the bend handle, and a press there bends nothing until it becomes a
/// drag. Only the label changes, in one undo step. A real drag from the handle still bends the
/// arrow (control), and a press that moves less than the drag threshold doesn't.
#[test]
fn a_double_click_on_a_selected_arrow_edits_its_label_without_bending_it_and_a_drag_still_bends() {
    let mut e = editor(1200.0, 900.0);
    e.snapping = false;
    let a = rect(&mut e, 100.0, 100.0, 160.0, 80.0);
    let b = rect(&mut e, 600.0, 100.0, 160.0, 80.0);
    let (from, to) = (world(&e, a).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    assert_eq!(e.selection, vec![arrow], "drawing leaves it selected");
    let before = conn(&e, arrow);
    let steps = e.history.undo_len();
    let mid = line_middle(&e, arrow);
    let handle = |e: &Editor| {
        e.connector_handles()
            .into_iter()
            .find(|(i, _)| *i == 2)
            .map(|(_, p)| p)
    };
    assert!(
        handle(&e).unwrap().dist(mid) < 1e-6,
        "the bend handle is the middle"
    );
    click(&mut e, mid, 1000.0);
    assert_eq!(
        conn(&e, arrow),
        before,
        "a click on the middle bends nothing"
    );
    assert_eq!(e.history.undo_len(), steps, "and records nothing");
    click(&mut e, mid, 1100.0);
    assert_eq!(e.text_edit.as_ref().map(|te| te.id), Some(arrow));
    e.handle(InputEvent::Text("writes".into()));
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(label_of(&e, arrow).as_deref(), Some("writes"));
    assert_eq!(conn(&e, arrow), before, "only the label changed");
    assert_eq!(e.history.undo_len(), steps + 1, "one label edit");
    // Control: a drag from an unlabelled arrow's bend handle still bends it, in one step.
    let c = rect(&mut e, 100.0, 500.0, 160.0, 80.0);
    let d = rect(&mut e, 600.0, 500.0, 160.0, 80.0);
    let (from, to) = (world(&e, c).center(), world(&e, d).center());
    let other = draw_arrow(&mut e, from, to, 2000.0);
    let straight = conn(&e, other);
    let h = handle(&e).unwrap();
    let steps = e.history.undo_len();
    ev_drag(&mut e, h, h + v2(0.0, -80.0), 3000.0, true);
    let (pts, s, en, route) = conn(&e, other);
    assert_eq!((route, pts.len()), (Route::Curved, 3), "bent");
    assert!(
        pts[1].dist(h + v2(0.0, -80.0)) < 1e-6,
        "through the drag's end"
    );
    assert_eq!((s, en), (Some(c), Some(d)));
    assert_eq!(e.history.undo_len(), steps + 1, "one bend step");
    e.run(Action::Undo);
    assert_eq!(conn(&e, other), straight);
    // A press that stays within the drag threshold is a click: nothing bends.
    e.selection = vec![other];
    let steps = e.history.undo_len();
    ev_drag(&mut e, h, h + v2(1.5, 1.5), 4000.0, true);
    assert_eq!(conn(&e, other), straight);
    assert_eq!(e.history.undo_len(), steps);
    e.doc.validate().unwrap();
}
