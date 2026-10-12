//! Three-way merge of one page at node granularity.
//!
//! `base` is the revision the browser started from, `theirs` is what is on disk now (e.g.
//! after an agent's edits), `mine` is the browser's page. The result keeps both sides' changes
//! when they touch different nodes, and reports a conflict when they don't:
//!
//! - A node's content (everything except its child list) changed on one side: that side wins.
//!   Changed on both sides to the same value: fine. Changed differently: conflict.
//! - Deleted on one side and unchanged on the other: deleted. Deleted on one side and changed
//!   on the other: conflict.
//! - Added on one side: kept. Both sides allocated the same new id for different nodes: mine
//!   is renumbered (with every reference to it) so both are kept.
//! - Child order (z-order) of each container: one side's reordering of the children both sides
//!   kept wins; both reordering them differently is a conflict on the container. Children one
//!   side inserted are placed before their nearest following sibling (or on top).
//! - A node left without its parent, or a cycle, is a conflict.

use paper_rs::model::{Document, Node, NodeId, NodeKind};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Conflicting node ids (a container id for a z-order conflict; `None` = the page root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflicts(pub BTreeSet<Option<u64>>);

impl Conflicts {
    pub fn ids(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|c| c.map_or_else(|| "root".to_string(), |id| id.to_string()))
            .collect()
    }
}

/// The node without its child list (children are merged separately as an ordering).
fn content(n: &Node) -> Node {
    let mut n = n.clone();
    n.children.clear();
    n
}

/// Every node reference inside a node.
fn remap_node(n: &mut Node, map: &HashMap<NodeId, NodeId>) {
    let m = |id: &mut NodeId| {
        if let Some(to) = map.get(id) {
            *id = *to;
        }
    };
    m(&mut n.id);
    if let Some(p) = n.parent.as_mut() {
        m(p);
    }
    n.children.iter_mut().for_each(m);
    if let Some(i) = n.instance.as_mut() {
        m(&mut i.main);
    }
    if let NodeKind::Connector { start, end, .. } = &mut n.kind {
        for b in [start, end].into_iter().flatten() {
            m(&mut b.node);
        }
    }
}

fn remap_doc(doc: &mut Document, map: &HashMap<NodeId, NodeId>) {
    let nodes = std::mem::take(&mut doc.nodes);
    for (_, mut n) in nodes {
        remap_node(&mut n, map);
        doc.nodes.insert(n.id, n);
    }
    for id in doc.root.iter_mut() {
        if let Some(to) = map.get(id) {
            *id = *to;
        }
    }
}

fn children(doc: &Document, parent: Option<NodeId>) -> Vec<NodeId> {
    match parent {
        None => doc.root.clone(),
        Some(p) => doc.get(p).map(|n| n.children.clone()).unwrap_or_default(),
    }
}

/// Merge one container's child order. `keep` = the merged child set of this container.
fn merge_order(
    base: &[NodeId],
    theirs: &[NodeId],
    mine: &[NodeId],
    keep: &HashSet<NodeId>,
) -> Option<Vec<NodeId>> {
    let in_all: HashSet<NodeId> = base
        .iter()
        .filter(|id| theirs.contains(id) && mine.contains(id))
        .copied()
        .collect();
    let rel = |l: &[NodeId]| -> Vec<NodeId> {
        l.iter().filter(|id| in_all.contains(id)).copied().collect()
    };
    let (b, t, m) = (rel(base), rel(theirs), rel(mine));
    if t != b && m != b && t != m {
        return None;
    }
    let (primary, secondary) = if m != b {
        (mine, theirs)
    } else {
        (theirs, mine)
    };
    let mut out: Vec<NodeId> = Vec::new();
    for id in primary {
        if keep.contains(id) && !out.contains(id) {
            out.push(*id);
        }
    }
    // The other side's insertions, before their nearest following sibling (new layers on
    // top stay on top).
    let mut pending: Vec<(usize, NodeId)> = Vec::new();
    for (i, id) in secondary.iter().enumerate() {
        if keep.contains(id) && !out.contains(id) {
            pending.push((i, *id));
        }
    }
    for (i, id) in pending.into_iter().rev() {
        let at = secondary[i + 1..]
            .iter()
            .find_map(|f| out.iter().position(|o| o == f))
            .unwrap_or(out.len());
        out.insert(at, id);
    }
    // Anything else that belongs here (e.g. moved in by a side whose list we didn't use).
    let mut rest: Vec<NodeId> = keep
        .iter()
        .filter(|id| !out.contains(id))
        .copied()
        .collect();
    rest.sort_by_key(|id| id.0);
    out.extend(rest);
    Some(out)
}

fn reachable(doc: &Document) -> usize {
    let mut seen = HashSet::new();
    let mut stack: Vec<NodeId> = doc.root.clone();
    while let Some(id) = stack.pop() {
        if seen.insert(id)
            && let Some(n) = doc.get(id)
        {
            stack.extend(n.children.iter().copied());
        }
    }
    seen.len()
}

/// Merge `mine` into `theirs` relative to `base`.
pub fn merge(base: &Document, theirs: &Document, mine: &Document) -> Result<Document, Conflicts> {
    let mut conflicts = BTreeSet::new();
    let mut mine = mine.clone();

    // Ids both sides allocated independently for different nodes: renumber mine.
    let mut next = theirs.next_id.max(mine.next_id);
    let mut map = HashMap::new();
    let mut clashing: Vec<NodeId> = mine
        .nodes
        .keys()
        .filter(|id| !base.nodes.contains_key(id))
        .filter(|id| {
            theirs.nodes.get(id).is_some_and(|t| {
                content(t) != content(&mine.nodes[id]) || t.children != mine.nodes[id].children
            })
        })
        .copied()
        .collect();
    clashing.sort_by_key(|id| id.0);
    for id in clashing {
        map.insert(id, NodeId(next));
        next += 1;
    }
    if !map.is_empty() {
        remap_doc(&mut mine, &map);
    }

    // Node contents.
    let ids: BTreeSet<NodeId> = base
        .nodes
        .keys()
        .chain(theirs.nodes.keys())
        .chain(mine.nodes.keys())
        .copied()
        .collect();
    let mut merged: HashMap<NodeId, Node> = HashMap::new();
    for id in &ids {
        let (b, t, m) = (base.nodes.get(id), theirs.nodes.get(id), mine.nodes.get(id));
        let pick = match (b, t, m) {
            (None, Some(t), None) => Some(t),
            (None, None, Some(m)) => Some(m),
            (None, Some(t), Some(_)) => Some(t), // same new node on both sides
            (None, None, None) => None,
            (Some(_), None, None) => None,
            (Some(b), None, Some(m)) => {
                if content(m) != content(b) {
                    conflicts.insert(Some(id.0));
                }
                None
            }
            (Some(b), Some(t), None) => {
                if content(t) != content(b) {
                    conflicts.insert(Some(id.0));
                }
                None
            }
            (Some(b), Some(t), Some(m)) => {
                let (cb, ct, cm) = (content(b), content(t), content(m));
                if ct == cb {
                    Some(m)
                } else if cm == cb || cm == ct {
                    Some(t)
                } else {
                    conflicts.insert(Some(id.0));
                    Some(t)
                }
            }
        };
        if let Some(n) = pick {
            let mut n = content(n);
            n.children.clear();
            merged.insert(*id, n);
        }
    }

    // Parents must exist.
    for n in merged.values() {
        if let Some(p) = n.parent
            && !merged.contains_key(&p)
        {
            conflicts.insert(Some(n.id.0));
        }
    }
    if !conflicts.is_empty() {
        return Err(Conflicts(conflicts));
    }

    // Child orders, per container.
    let mut members: HashMap<Option<NodeId>, HashSet<NodeId>> = HashMap::new();
    for n in merged.values() {
        members.entry(n.parent).or_default().insert(n.id);
    }
    let containers: Vec<Option<NodeId>> = std::iter::once(None)
        .chain(merged.keys().map(|id| Some(*id)))
        .collect();
    let mut root = Vec::new();
    let mut lists: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for c in containers {
        let keep = members.remove(&c).unwrap_or_default();
        match merge_order(
            &children(base, c),
            &children(theirs, c),
            &children(&mine, c),
            &keep,
        ) {
            Some(order) => match c {
                None => root = order,
                Some(p) => {
                    lists.insert(p, order);
                }
            },
            None => {
                conflicts.insert(c.map(|p| p.0));
            }
        }
    }
    if !conflicts.is_empty() {
        return Err(Conflicts(conflicts));
    }
    for (p, l) in lists {
        merged.get_mut(&p).unwrap().children = l;
    }
    let max = merged.keys().map(|id| id.0 + 1).max().unwrap_or(0);
    let doc = Document {
        name: theirs.name.clone(),
        nodes: merged,
        root,
        next_id: next.max(max),
    };
    // Cycles (each side reparenting under the other) leave nodes unreachable from the root.
    // (`Document::validate` checks parent/child consistency but not reachability.)
    if doc.validate().is_err() || reachable(&doc) != doc.nodes.len() {
        let moved: BTreeSet<Option<u64>> = doc
            .nodes
            .values()
            .filter(|n| {
                let bp = base.nodes.get(&n.id).map(|b| b.parent);
                bp != Some(n.parent)
            })
            .map(|n| Some(n.id.0))
            .collect();
        return Err(Conflicts(if moved.is_empty() {
            [None].into()
        } else {
            moved
        }));
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use paper_rs::geom::Rect;

    fn node(id: u64, parent: Option<u64>, x: f64) -> Node {
        let mut n = Node::new(NodeId(id), NodeKind::Rect, Rect::new(x, 0.0, 10.0, 10.0));
        n.parent = parent.map(NodeId);
        n
    }

    /// A document from (id, parent, x) triples; children in the order listed.
    fn doc(nodes: &[(u64, Option<u64>, f64)]) -> Document {
        let mut d = Document::default();
        for &(id, parent, x) in nodes {
            d.nodes.insert(NodeId(id), node(id, parent, x));
            match parent {
                None => d.root.push(NodeId(id)),
                Some(p) => d
                    .nodes
                    .get_mut(&NodeId(p))
                    .unwrap()
                    .children
                    .push(NodeId(id)),
            }
        }
        d.next_id = nodes.iter().map(|n| n.0 + 1).max().unwrap_or(1);
        d.validate().unwrap();
        d
    }

    fn x(d: &Document, id: u64) -> f64 {
        d.nodes[&NodeId(id)].rect.x
    }

    fn ids(v: &[NodeId]) -> Vec<u64> {
        v.iter().map(|i| i.0).collect()
    }

    fn conflicts(r: Result<Document, Conflicts>) -> Vec<String> {
        r.expect_err("conflict").ids()
    }

    #[test]
    fn different_nodes_changed_on_each_side_merge() {
        let b = doc(&[(1, None, 0.0), (2, None, 0.0)]);
        let t = doc(&[(1, None, 50.0), (2, None, 0.0)]);
        let m = doc(&[(1, None, 0.0), (2, None, 70.0)]);
        let r = merge(&b, &t, &m).unwrap();
        assert_eq!((x(&r, 1), x(&r, 2)), (50.0, 70.0));
        assert_eq!(ids(&r.root), [1, 2]);
    }

    #[test]
    fn same_node_changed_differently_conflicts() {
        let b = doc(&[(1, None, 0.0)]);
        let t = doc(&[(1, None, 50.0)]);
        let m = doc(&[(1, None, 70.0)]);
        assert_eq!(conflicts(merge(&b, &t, &m)), ["1"]);
        // The same change on both sides is fine.
        assert_eq!(x(&merge(&b, &t, &t).unwrap(), 1), 50.0);
    }

    #[test]
    fn adds_on_both_sides_are_kept_and_clashing_ids_renumbered() {
        let b = doc(&[(1, None, 0.0)]);
        // Both sides allocated id 2 for different new nodes; mine also added 3 inside its 2.
        let t = doc(&[(1, None, 0.0), (2, None, 100.0)]);
        let m = doc(&[(1, None, 0.0), (2, None, 200.0), (3, Some(2), 5.0)]);
        let r = merge(&b, &t, &m).unwrap();
        assert_eq!(r.nodes.len(), 4);
        assert_eq!(x(&r, 2), 100.0, "theirs keeps its id");
        let renumbered = r
            .nodes
            .values()
            .find(|n| n.rect.x == 200.0)
            .expect("mine kept")
            .id;
        assert_ne!(renumbered, NodeId(2));
        assert_eq!(
            r.nodes[&NodeId(3)].parent,
            Some(renumbered),
            "child follows"
        );
        assert_eq!(ids(&r.root), [1, 2, renumbered.0]);
        assert!(r.next_id > renumbered.0);
        r.validate().unwrap();
    }

    #[test]
    fn deletes_merge_unless_the_other_side_changed_the_node() {
        let b = doc(&[(1, None, 0.0), (2, None, 0.0)]);
        // Theirs deletes 1, mine moves 2: both apply.
        let t = doc(&[(2, None, 0.0)]);
        let m = doc(&[(1, None, 0.0), (2, None, 9.0)]);
        let r = merge(&b, &t, &m).unwrap();
        assert_eq!(ids(&r.root), [2]);
        assert_eq!(x(&r, 2), 9.0);
        // Delete vs edit, either way round, conflicts.
        let edited = doc(&[(1, None, 5.0), (2, None, 0.0)]);
        assert_eq!(conflicts(merge(&b, &t, &edited)), ["1"]);
        assert_eq!(conflicts(merge(&b, &edited, &t)), ["1"]);
    }

    #[test]
    fn deleting_a_parent_while_the_other_side_adds_or_edits_inside_conflicts() {
        let b = doc(&[(1, None, 0.0), (2, Some(1), 0.0)]);
        // Theirs deletes the frame and its child; mine adds a new child to it.
        let t = doc(&[]);
        let m = doc(&[(1, None, 0.0), (2, Some(1), 0.0), (3, Some(1), 0.0)]);
        let c = conflicts(merge(&b, &t, &m));
        assert!(c.contains(&"3".to_string()), "{c:?}");
    }

    #[test]
    fn z_order_changes_merge_or_conflict() {
        let b = doc(&[(1, None, 0.0), (2, None, 0.0), (3, None, 0.0)]);
        // Mine reorders, theirs adds a node at the end: both apply.
        let t = doc(&[
            (1, None, 0.0),
            (2, None, 0.0),
            (3, None, 0.0),
            (4, None, 0.0),
        ]);
        let m = doc(&[(3, None, 0.0), (1, None, 0.0), (2, None, 0.0)]);
        let r = merge(&b, &t, &m).unwrap();
        assert_eq!(ids(&r.root), [3, 1, 2, 4]);
        // Both reorder differently: conflict on the page root.
        let t2 = doc(&[(2, None, 0.0), (1, None, 0.0), (3, None, 0.0)]);
        assert_eq!(conflicts(merge(&b, &t2, &m)), ["root"]);
    }

    #[test]
    fn reparenting_merges_and_conflicting_parents_conflict() {
        let b = doc(&[(1, None, 0.0), (2, None, 0.0), (3, None, 0.0)]);
        // Mine moves 3 into frame 1; theirs edits 2.
        let t = doc(&[(1, None, 0.0), (2, None, 7.0), (3, None, 0.0)]);
        let m = doc(&[(1, None, 0.0), (3, Some(1), 0.0), (2, None, 0.0)]);
        let r = merge(&b, &t, &m).unwrap();
        assert_eq!(ids(&r.root), [1, 2]);
        assert_eq!(ids(&r.nodes[&NodeId(1)].children), [3]);
        assert_eq!(x(&r, 2), 7.0);
        r.validate().unwrap();
        // Both move 3, into different parents: conflict on 3.
        let t2 = doc(&[(1, None, 0.0), (2, None, 0.0), (3, Some(2), 0.0)]);
        assert_eq!(conflicts(merge(&b, &t2, &m)), ["3"]);
        // A cycle: theirs puts 2 under 1, mine puts 1 under 2.
        let b2 = doc(&[(1, None, 0.0), (2, None, 0.0)]);
        let t3 = doc(&[(1, None, 0.0), (2, Some(1), 0.0)]);
        let m3 = doc(&[(2, None, 0.0), (1, Some(2), 0.0)]);
        assert!(merge(&b2, &t3, &m3).is_err());
    }
}
