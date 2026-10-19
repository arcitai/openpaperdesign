//! System clipboard interop.
//!
//! Copying writes a JSON payload (layers, their world rects and the images they use) as text,
//! so layers paste between windows, projects and app restarts. Pasting accepts that payload,
//! Excalidraw clipboard/scene JSON, SVG markup, images and plain text (which becomes a text
//! layer).
//!
//! Two layer payloads, the same fields: layers without arrow labels are written exactly as
//! before, as `paper-rs/clipboard` (unversioned; upstream paper-rs and every earlier
//! OpenPaperDesign read it). Layers holding an arrow label are written as [`LABELLED_TYPE`]
//! with `version` [`LABELLED_VERSION`]: a reader that only knows `paper-rs/clipboard` (no
//! labels) doesn't take it for layers, so it can't paste the arrows without their labels. Such
//! a reader decodes it as plain text, the whole JSON (see `decode` there). A labelled payload
//! of another version, or one that isn't a whole, consistent layer tree, is refused before
//! anything happens ([`Content::Refused`]). It is recognized by its top-level `type` member,
//! read with serde as far as the text is JSON, so a cut-off or trailing-text payload in any
//! formatting or key order is refused too, never pasted as a text layer instead.

use crate::geom::{Rect, V2};
use crate::model::{Node, NodeId, Paint};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The layer payload without arrow labels (unversioned, shared with upstream paper-rs).
pub const TYPE: &str = "paper-rs/clipboard";
/// The layer payload with arrow labels. Not `TYPE`, so no reader of `TYPE` alone takes it.
pub const LABELLED_TYPE: &str = "openpaperdesign/clipboard";
/// The only version of [`LABELLED_TYPE`] this reader understands. A change of what it holds
/// gets a new version, which earlier readers refuse.
pub const LABELLED_VERSION: u64 = 1;
/// The fields of a labelled payload: anything else is from a writer this reader doesn't know.
const LABELLED_FIELDS: [&str; 7] = [
    "type", "version", "nodes", "roots", "world", "page", "assets",
];

/// Platform clipboard access, injected into the editor (tests use an in-memory one).
pub trait Provider {
    fn read_text(&mut self) -> Option<String>;
    fn write_text(&mut self, s: &str);
    fn read_image(&mut self) -> Option<image::RgbaImage> {
        None
    }
    fn write_image(&mut self, _img: &image::RgbaImage) {}
}

/// In-memory clipboard (tests, headless).
#[derive(Default)]
pub struct Memory {
    pub text: Option<String>,
    pub image: Option<image::RgbaImage>,
}

impl Provider for Memory {
    fn read_text(&mut self) -> Option<String> {
        self.text.clone()
    }
    fn write_text(&mut self, s: &str) {
        self.text = Some(s.into());
        self.image = None;
    }
    fn read_image(&mut self) -> Option<image::RgbaImage> {
        self.image.clone()
    }
    fn write_image(&mut self, img: &image::RgbaImage) {
        self.image = Some(img.clone());
    }
}

/// The OS clipboard via arboard (None when no clipboard is available, e.g. headless).
pub struct System(arboard::Clipboard);

impl System {
    pub fn new() -> Option<System> {
        arboard::Clipboard::new().ok().map(System)
    }
}

impl Provider for System {
    fn read_text(&mut self) -> Option<String> {
        self.0.get_text().ok()
    }
    fn write_text(&mut self, s: &str) {
        let _ = self.0.set_text(s.to_string());
    }
    fn read_image(&mut self) -> Option<image::RgbaImage> {
        let img = self.0.get_image().ok()?;
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
    }
    fn write_image(&mut self, img: &image::RgbaImage) {
        let _ = self.0.set_image(arboard::ImageData {
            width: img.width() as usize,
            height: img.height() as usize,
            bytes: std::borrow::Cow::Borrowed(img.as_raw()),
        });
    }
}

#[derive(Serialize, Deserialize)]
pub struct Payload {
    #[serde(rename = "type")]
    pub kind: String,
    /// [`LABELLED_TYPE`]'s version; absent from `TYPE` payloads (written exactly as before).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    pub nodes: Vec<Node>,
    pub roots: Vec<NodeId>,
    pub world: Vec<Rect>,
    /// Page the layers came from (same page: offset paste).
    #[serde(default)]
    pub page: Option<String>,
    /// Images used by the layers: asset id → base64 bytes.
    #[serde(default)]
    pub assets: HashMap<String, String>,
}

/// The payload for these layers: [`LABELLED_TYPE`] (version [`LABELLED_VERSION`]) when one of
/// them is an arrow with a label, otherwise `TYPE`, byte for byte as before.
pub fn encode(nodes: &[Node], roots: &[NodeId], world: &[Rect], page: Option<&str>) -> String {
    let labelled = nodes.iter().any(|n| n.kind.label().is_some());
    let (kind, version) = if labelled {
        (LABELLED_TYPE, Some(LABELLED_VERSION))
    } else {
        (TYPE, None)
    };
    let mut assets = HashMap::new();
    for n in nodes {
        for p in &n.style.fills {
            if let Paint::Image { asset, .. } = p
                && let Some(a) = crate::assets::get(asset)
            {
                assets.insert(asset.clone(), crate::assets::base64(&a.bytes));
            }
        }
    }
    serde_json::to_string(&Payload {
        kind: kind.into(),
        version,
        nodes: nodes.to_vec(),
        roots: roots.to_vec(),
        world: world.to_vec(),
        page: page.map(String::from),
        assets,
    })
    .expect("clipboard payload serializes")
}

/// What a clipboard text holds.
pub enum Content {
    Layers(Payload),
    /// Layers from another app (Excalidraw JSON or SVG), in their own coordinates.
    Foreign(Vec<Node>, &'static str),
    Text(String),
    /// OpenPaperDesign layers this reader won't paste (another version of
    /// [`LABELLED_TYPE`], or not a whole layer tree), with the reason to show. Nothing was
    /// registered for it: no image, no layer.
    Refused(String),
}

/// Interpret clipboard text.
pub fn decode(text: &str) -> Content {
    let t = text.trim_start();
    if t.starts_with('{') {
        if let Some(c) = decode_labelled(t) {
            return c;
        }
        if let Ok(p) = serde_json::from_str::<Payload>(t)
            && p.kind == TYPE
        {
            register_assets(&p);
            return Content::Layers(p);
        }
        if let Ok(doc) = crate::io::excalidraw::import_scene(t) {
            // Flatten the imported page into root subtrees.
            let nodes: Vec<Node> = doc
                .paint_order()
                .into_iter()
                .filter_map(|id| doc.get(id).cloned())
                .collect();
            if !nodes.is_empty() {
                return Content::Foreign(nodes, "Excalidraw");
            }
        }
    }
    if (t.starts_with("<svg") || (t.starts_with("<?xml") && t.contains("<svg")))
        && let Ok(nodes) = crate::io::svg_import::parse(t)
    {
        return Content::Foreign(nodes, "SVG");
    }
    Content::Text(text.to_string())
}

fn register_assets(p: &Payload) {
    for (id, b64) in &p.assets {
        if crate::assets::get(id).is_none()
            && let Some(bytes) = crate::assets::base64_decode(b64)
        {
            let _ = crate::assets::register(&bytes);
        }
    }
}

/// What the top level of a JSON object says about its format, read key by key with serde as
/// far as the text is JSON: a cut-off text, or one followed by more text, still says what it
/// began as, whatever its formatting or key order. Only a top-level `type` member counts;
/// text that merely mentions the type (in another member, or not as JSON) doesn't.
#[derive(Default)]
struct Header {
    /// A top-level `type` is [`LABELLED_TYPE`].
    labelled: bool,
    /// How many top-level `type` and `version` members it has (more than one is ambiguous).
    types: usize,
    versions: usize,
}

#[derive(Clone, Copy)]
struct HeaderProbe<'a>(&'a std::cell::RefCell<Header>);

impl<'de> serde::de::DeserializeSeed<'de> for HeaderProbe<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_map(self)
    }
}

impl<'de> serde::de::Visitor<'de> for HeaderProbe<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON object")
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        use serde::de::IgnoredAny;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "type" => {
                    self.0.borrow_mut().types += 1;
                    let v = map.next_value::<serde_json::Value>()?;
                    if v.as_str() == Some(LABELLED_TYPE) {
                        self.0.borrow_mut().labelled = true;
                    }
                }
                "version" => {
                    self.0.borrow_mut().versions += 1;
                    map.next_value::<IgnoredAny>()?;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

/// [`Header`] of `t`: as much of its top-level object as is there (its errors only end it).
fn header(t: &str) -> Header {
    use serde::de::DeserializeSeed;
    let read = std::cell::RefCell::new(Header::default());
    let mut de = serde_json::Deserializer::from_str(t);
    let _ = HeaderProbe(&read).deserialize(&mut de);
    read.into_inner()
}

/// A [`LABELLED_TYPE`] payload, checked whole before anything is registered: `None` when the
/// JSON doesn't name that type at its top level (another kind of text, decoded as before).
fn decode_labelled(t: &str) -> Option<Content> {
    let head = header(t);
    if !head.labelled {
        return None;
    }
    let refused = |why: String| {
        Some(Content::Refused(format!(
            "The clipboard's OpenPaperDesign layers can't be pasted: {why}. Nothing was pasted."
        )))
    };
    if head.types > 1 || head.versions > 1 {
        return refused("they name their format twice".into());
    }
    let Ok(serde_json::Value::Object(fields)) = serde_json::from_str::<serde_json::Value>(t) else {
        return refused("they aren't complete".into());
    };
    match fields.get("version").and_then(|v| v.as_u64()) {
        Some(LABELLED_VERSION) => {}
        Some(v) => {
            return refused(format!(
                "they are in format version {v}, which this OpenPaperDesign doesn't read"
            ));
        }
        None => return refused("they have no valid format version".into()),
    }
    if let Some(k) = fields
        .keys()
        .find(|k| !LABELLED_FIELDS.contains(&k.as_str()))
    {
        return refused(format!(
            "they hold a field this version doesn't know ({k:?})"
        ));
    }
    let p = match serde_json::from_value::<Payload>(serde_json::Value::Object(fields)) {
        Ok(p) => p,
        Err(e) => return refused(format!("they aren't readable ({e})")),
    };
    if let Err(why) = check_tree(&p) {
        return refused(why);
    }
    register_assets(&p);
    Some(Content::Layers(p))
}

/// The payload's layers are one whole forest: every id once, each root present (with its
/// position) and not inside another copied layer, each child present and naming its parent,
/// and nothing left over. Walked iteratively from the roots, so a cycle is found, not followed.
fn check_tree(p: &Payload) -> Result<(), String> {
    use std::collections::HashSet;
    let mut by_id: HashMap<NodeId, &Node> = HashMap::with_capacity(p.nodes.len());
    for n in &p.nodes {
        if by_id.insert(n.id, n).is_some() {
            return Err(format!("layer {} appears twice", n.id));
        }
    }
    if p.roots.is_empty() {
        return Err("they hold no layers".into());
    }
    if p.world.len() != p.roots.len() {
        return Err("their positions don't match their layers".into());
    }
    let mut todo: Vec<(NodeId, Option<NodeId>)> = Vec::new();
    for r in &p.roots {
        let n = by_id
            .get(r)
            .ok_or_else(|| format!("layer {r} is missing"))?;
        if n.parent.is_some_and(|q| by_id.contains_key(&q)) {
            return Err(format!(
                "layer {r} is listed twice, inside another copied layer"
            ));
        }
        todo.push((*r, None));
    }
    let mut seen = HashSet::with_capacity(p.nodes.len());
    while let Some((id, parent)) = todo.pop() {
        if !seen.insert(id) {
            return Err(format!("layer {id} appears twice"));
        }
        let n = by_id
            .get(&id)
            .ok_or_else(|| format!("layer {id} is missing"))?;
        if parent.is_some_and(|q| n.parent != Some(q)) {
            return Err(format!("layer {id} doesn't name its group"));
        }
        todo.extend(n.children.iter().map(|c| (*c, Some(id))));
    }
    if seen.len() != by_id.len() {
        return Err("some layers aren't inside the copied ones".into());
    }
    Ok(())
}

/// Root ids and world rects of a node list whose roots have no parent in the list.
pub fn roots_of(nodes: &[Node]) -> (Vec<NodeId>, Vec<Rect>) {
    let ids: std::collections::HashSet<NodeId> = nodes.iter().map(|n| n.id).collect();
    let mut roots = Vec::new();
    let mut world = Vec::new();
    for n in nodes {
        if n.parent.is_none_or(|p| !ids.contains(&p)) {
            roots.push(n.id);
            world.push(n.rect);
        }
    }
    (roots, world)
}

/// Bounds of root rects.
pub fn bounds(world: &[Rect]) -> Option<Rect> {
    Rect::union_all(world)
}

pub fn center_offset(world: &[Rect], at: V2) -> V2 {
    bounds(world).map_or(V2::ZERO, |b| (at - b.center()).round())
}
