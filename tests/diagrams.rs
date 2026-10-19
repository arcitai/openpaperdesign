//! Revision 2 slice 7 (OPD-UX-PLAN section 7, R15): the editable diagram workflow with the
//! objects that already exist (a group of one rectangle and its text label, bound arrows with
//! straight, curved and elbow routes, duplicate, copy, delete, undo, save and export), through
//! the editor's own input path and commands. These are proofs of current behaviour before any
//! repair: each test can fail on a wrong binding, a lost or corrupted label, an arrow that no
//! longer follows its shape, or an edit that takes more than one undo step.

mod common;

use common::*;
use paper_rs::editor::shortcuts::Action;
use paper_rs::editor::{Editor, Tool};
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
    let start = conn(&e, arrow).0[0];
    assert!(on_outline(start, big), "the arrow follows: {start:?}");
    e.run(Action::Undo);
    assert_eq!(e.doc, doc0, "undo restores all of it");
    // Shrink it to half: the text is never cut or stretched; it wraps in its narrower box.
    ev_drag(&mut e, corner, v2(220.0, 160.0), 2000.0, true);
    let small = Rect::new(100.0, 100.0, 120.0, 60.0);
    assert_eq!(world(&e, r), small);
    assert_eq!(text_of(&e, t), label0, "the same text and font size");
    let (h, tr) = label_laid_out(&e, t);
    // Measured, not asserted: whether a wrapped label taller than its scaled box is the
    // intended behaviour is the open question in the writer100 handoff.
    eprintln!("half-size label: layout height {h}, box {tr:?}");
    assert!(on_outline(conn(&e, arrow).0[0], small));
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

/// Connector labels don't exist yet (see the writer100 handoff). This pins, on a disposable
/// page, the two facts a future label must be designed around: today's reader accepts an
/// unknown connector field and silently drops it on its next save (optional-field acceptance
/// is no safety), and it refuses a newer page version outright (the existing admission a
/// labelled page can use). A label implementation must change this test deliberately.
#[test]
fn connector_label_baseline_an_unknown_field_is_dropped_and_a_newer_page_is_refused() {
    use paper_rs::project::format::{FormatError, PAGE_VERSION, page_from_json, page_to_json};
    let mut e = editor(1000.0, 700.0);
    let a = rect(&mut e, 100.0, 100.0, 160.0, 80.0);
    let b = rect(&mut e, 500.0, 100.0, 160.0, 80.0);
    let (from, to) = (world(&e, a).center(), world(&e, b).center());
    let arrow = draw_arrow(&mut e, from, to, 0.0);
    let saved = page_to_json(&e.page_data(0));
    let mut v: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(v["version"], serde_json::json!(PAGE_VERSION));
    let nodes = v["nodes"].as_array_mut().unwrap();
    let c = nodes
        .iter_mut()
        .find(|n| n["id"] == serde_json::json!(arrow.0))
        .unwrap();
    assert_eq!(c["type"], "connector");
    c["label"] = serde_json::json!("writes to");
    let with_label = serde_json::to_string(&v).unwrap();
    let read = page_from_json(&with_label).unwrap();
    assert_eq!(
        read.doc.nodes, e.doc.nodes,
        "everything else reads as before"
    );
    let again = page_to_json(&read);
    assert!(!again.contains("writes to"), "the unknown label is dropped");
    // A newer page version is refused, not misread.
    v["version"] = serde_json::json!(PAGE_VERSION + 1);
    let newer = serde_json::to_string(&v).unwrap();
    let refused = page_from_json(&newer);
    assert!(matches!(refused, Err(FormatError::TooNew { .. })));
}
