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
//!
//! Layers holding a font family other than Inter are written as [`LABELLED_TYPE`] version
//! [`FAMILY_VERSION`] (labels or not): a reader of version 1 only refuses it, as it refuses any
//! version it doesn't know, rather than pasting the layers in Inter. A version 1 or
//! `paper-rs/clipboard` payload naming another family, and any payload naming a family this
//! build doesn't have, is refused the same way, before anything is registered.

use crate::geom::{Rect, V2};
use crate::model::{FontFamily, Node, NodeId, Paint};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The layer payload without arrow labels (unversioned, shared with upstream paper-rs).
pub const TYPE: &str = "paper-rs/clipboard";
/// The layer payload with arrow labels. Not `TYPE`, so no reader of `TYPE` alone takes it.
pub const LABELLED_TYPE: &str = "openpaperdesign/clipboard";
/// The version of [`LABELLED_TYPE`] with arrow labels and Inter text only. A change of what
/// it holds gets a new version, which earlier readers refuse.
pub const LABELLED_VERSION: u64 = 1;
/// The version of [`LABELLED_TYPE`] whose layers may have a font family other than Inter
/// (`text_style.family`); otherwise the same fields as version 1. Written only when a layer
/// has one, so every other payload keeps its type, version and bytes.
pub const FAMILY_VERSION: u64 = 2;
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

/// The payload for these layers: [`LABELLED_TYPE`] version [`FAMILY_VERSION`] when one of them
/// has a font family other than Inter, else version [`LABELLED_VERSION`] when one is an arrow
/// with a label, otherwise `TYPE`, byte for byte as before.
pub fn encode(nodes: &[Node], roots: &[NodeId], world: &[Rect], page: Option<&str>) -> String {
    let family = nodes.iter().any(|n| !n.text_style.family.is_default());
    let labelled = nodes.iter().any(|n| n.kind.label().is_some());
    let (kind, version) = if family {
        (LABELLED_TYPE, Some(FAMILY_VERSION))
    } else if labelled {
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
        if let Some(why) = plain_family(t) {
            return Content::Refused(format!(
                "The clipboard's layers can't be pasted: {why}. Nothing was pasted."
            ));
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
    /// A top-level `type` is [`TYPE`].
    plain: bool,
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
                    if v.as_str() == Some(TYPE) {
                        self.0.borrow_mut().plain = true;
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

/// One JSON value in which no object names a member twice, at any depth (names compared as
/// decoded, so two spellings of one name are one name). serde_json's `Value` keeps only the
/// last of repeated members, so a labelled payload is read through this first: what is
/// checked (its images included) is then exactly what would be pasted, whatever the order.
#[derive(Clone, Copy)]
struct Unambiguous;

impl<'de> serde::de::DeserializeSeed<'de> for Unambiguous {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for Unambiguous {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq.next_element_seed(self)?.is_some() {}
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut names = std::collections::HashSet::new();
        while let Some(name) = map.next_key::<String>()? {
            if !names.insert(name.clone()) {
                return Err(serde::de::Error::custom(format!("{name:?} named twice")));
            }
            map.next_value_seed(self)?;
        }
        Ok(())
    }
}

/// `t` is one whole JSON value with no member named twice (see [`Unambiguous`]); else why not.
fn check_unambiguous(t: &str) -> Result<(), String> {
    use serde::de::DeserializeSeed;
    let mut de = serde_json::Deserializer::from_str(t);
    let read = Unambiguous.deserialize(&mut de).and_then(|()| de.end());
    match read {
        Ok(()) => Ok(()),
        Err(e) if e.is_data() => Err(format!("they hold a member twice ({e})")),
        Err(_) => Err("they aren't complete".into()),
    }
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
    // Before `Value` drops repeated members: a repeated `assets`, image id or any other
    // member is ambiguous, whatever its order or values.
    if let Err(why) = check_unambiguous(t) {
        return refused(why);
    }
    let Ok(serde_json::Value::Object(fields)) = serde_json::from_str::<serde_json::Value>(t) else {
        return refused("they aren't complete".into());
    };
    let version = match fields.get("version").and_then(|v| v.as_u64()) {
        Some(v @ (LABELLED_VERSION | FAMILY_VERSION)) => v,
        Some(v) => {
            return refused(format!(
                "they are in format version {v}, which this OpenPaperDesign doesn't read"
            ));
        }
        None => return refused("they have no valid format version".into()),
    };
    // Every family named is one this build has, and only version 2 holds one besides Inter.
    match families(fields.get("nodes")) {
        Err(why) => return refused(why),
        Ok(Some(f)) if version < FAMILY_VERSION => {
            return refused(format!(
                "they use the font family {}, which their format version {version} can't hold",
                f.name()
            ));
        }
        Ok(_) => {}
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
    let images = match check_images(&p) {
        Ok(images) => images,
        Err(why) => return refused(why),
    };
    // Every image checked: only now is any registered (all of them, or none above).
    for bytes in &images {
        let _ = crate::assets::register(bytes);
    }
    Some(Content::Layers(p))
}

/// The font families a payload's layers (`nodes`, as JSON) name: an error when one isn't the
/// exact name of a family this build has, else the first one other than Inter, if any.
fn families(nodes: Option<&serde_json::Value>) -> Result<Option<FontFamily>, String> {
    let mut other = None;
    for n in nodes.and_then(|n| n.as_array()).into_iter().flatten() {
        let Some(f) = n.get("text_style").and_then(|t| t.get("family")) else {
            continue;
        };
        match f.as_str().and_then(FontFamily::from_name) {
            None => {
                return Err(format!(
                    "they name the font family {f}, which this app doesn't have"
                ));
            }
            Some(known) if !known.is_default() => {
                other.get_or_insert(known);
            }
            Some(_) => {}
        }
    }
    Ok(other)
}

/// Where [`FamilyProbe`] is in a layer payload: its top level, its `nodes` list, one node, or
/// one node's `text_style`.
#[derive(Clone, Copy)]
enum Level {
    Top,
    Nodes,
    Node,
    Style,
}

/// Every `nodes[*].text_style.family` member of a layer payload, each value as written (`None`
/// when the text ends inside it), read
/// with serde member by member as far as the text is JSON (as [`Header`] reads the top level):
/// a cut-off text still shows a family read before its end, and every repeated `text_style` or
/// `family` member is seen, not only the last (which is all `Value` keeps). Names are compared
/// as decoded, so an escaped spelling of `family` or `text_style` is the same member. Only
/// members count: a layer's text, a name or any other string that merely contains those words
/// is never taken for one.
#[derive(Clone, Copy)]
struct FamilyProbe<'a> {
    found: &'a std::cell::RefCell<Vec<Option<serde_json::Value>>>,
    level: Level,
}

impl<'de> serde::de::DeserializeSeed<'de> for FamilyProbe<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for FamilyProbe<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        if let Level::Nodes = self.level {
            let node = FamilyProbe {
                level: Level::Node,
                ..self
            };
            while seq.next_element_seed(node)?.is_some() {}
        } else {
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
        }
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            let inner = match (self.level, key.as_str()) {
                (Level::Top, "nodes") => Some(Level::Nodes),
                (Level::Node, "text_style") => Some(Level::Style),
                _ => None,
            };
            match inner {
                Some(level) => map.next_value_seed(FamilyProbe { level, ..self })?,
                None if matches!(self.level, Level::Style) && key == "family" => {
                    // Counted before its value is read: a text cut off inside it still names
                    // a family (one that can't be read).
                    self.found.borrow_mut().push(None);
                    let v = map.next_value::<serde_json::Value>()?;
                    if let Some(last) = self.found.borrow_mut().last_mut() {
                        *last = Some(v);
                    }
                }
                None => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

/// The `family` members of `t`'s layers ([`FamilyProbe`]): as much of it as is JSON (its errors
/// only end the reading).
fn family_members(t: &str) -> Vec<Option<serde_json::Value>> {
    use serde::de::DeserializeSeed;
    let found = std::cell::RefCell::new(Vec::new());
    let mut de = serde_json::Deserializer::from_str(t);
    let probe = FamilyProbe {
        found: &found,
        level: Level::Top,
    };
    let _ = probe.deserialize(&mut de);
    found.into_inner()
}

/// Why a `paper-rs/clipboard` payload (unversioned: this writer never puts a font family in
/// one) that names a family isn't pasted; `None` for any other text, decoded as before. Each
/// `family` member counts, read member by member before anything else ([`family_members`]), so
/// neither a cut-off text (R177: the same payload without its final brace became a text layer)
/// nor a repeated `text_style` hiding the first one (likewise) slips by: one besides Inter, or
/// one this build doesn't have, is refused whatever follows. A payload naming only Inter is
/// taken only whole and with no member named twice.
fn plain_family(t: &str) -> Option<String> {
    if !header(t).plain {
        return None;
    }
    let named = family_members(t);
    if named.is_empty() {
        return None;
    }
    for f in &named {
        let Some(f) = f else {
            return Some("they name a font family that isn't complete".into());
        };
        match f.as_str().and_then(FontFamily::from_name) {
            None => {
                return Some(format!(
                    "they name the font family {f}, which this app doesn't have"
                ));
            }
            Some(known) if !known.is_default() => {
                return Some(format!(
                    "they use the font family {}, which their format ({TYPE}) can't hold",
                    known.name()
                ));
            }
            Some(_) => {}
        }
    }
    check_unambiguous(t).err()
}

/// The payload's images: every one its layers use is in it, and every one it holds is whole
/// (its base64 exactly as `assets::base64` writes it, its bytes an image that decodes, their
/// `assets::hash` the id it names). Checked for all of them before any is registered, so a
/// payload is taken with all its images or refused with none, whatever their order; an image
/// already registered under a claimed id never stands in for a bad or missing body. Returns
/// the checked bytes.
fn check_images(p: &Payload) -> Result<Vec<Vec<u8>>, String> {
    for n in &p.nodes {
        for f in &n.style.fills {
            if let Paint::Image { asset, .. } = f
                && !p.assets.contains_key(asset)
            {
                return Err(format!("an image they use isn't with them ({asset})"));
            }
        }
    }
    let mut images = Vec::with_capacity(p.assets.len());
    for (id, b64) in &p.assets {
        let bytes = crate::assets::base64_decode(b64)
            .filter(|b| crate::assets::base64(b) == *b64)
            .ok_or_else(|| format!("an image isn't valid base64 ({id})"))?;
        if crate::assets::hash(&bytes) != *id {
            return Err(format!("an image isn't the one it names ({id})"));
        }
        if image::load_from_memory(&bytes).is_err() {
            return Err(format!("an image can't be read ({id})"));
        }
        images.push(bytes);
    }
    Ok(images)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeKind, TextAlign};
    use serde_json::{Value, json};

    /// One text layer in `family` (with an image fill when `image` names one).
    fn text_layer(family: FontFamily, image: Option<&str>) -> Node {
        let mut n = Node::new(
            NodeId(1),
            NodeKind::Text {
                text: "Æøå✓ ok".into(),
                font_size: 18.0,
                auto_width: true,
                align: TextAlign::Left,
            },
            Rect::new(10.0, 10.0, 80.0, 24.0),
        );
        n.text_style.family = family;
        if let Some(asset) = image {
            n.style.fills.push(Paint::Image {
                asset: asset.into(),
                mode: Default::default(),
                opacity: 1.0,
            });
        }
        n
    }

    fn payload(n: &Node) -> String {
        encode(std::slice::from_ref(n), &[n.id], &[n.rect], None)
    }

    fn refused(c: Content) -> String {
        match c {
            Content::Refused(why) => why,
            Content::Layers(_) => panic!("pasted as layers"),
            Content::Foreign(..) => panic!("pasted as foreign layers"),
            Content::Text(_) => panic!("pasted as text"),
        }
    }

    #[test]
    fn inter_payloads_keep_their_format_and_another_family_gets_version_2() {
        // Inter: `paper-rs/clipboard`, unversioned, no family: as before families existed.
        let inter = payload(&text_layer(FontFamily::Inter, None));
        assert!(
            inter.starts_with(&format!("{{\"type\":\"{TYPE}\",\"nodes\"")),
            "{inter}"
        );
        assert!(!inter.contains("family") && !inter.contains("\"version\""));
        // Geist Mono: the OpenPaperDesign type, version 2, and it pastes as itself.
        let mono = payload(&text_layer(FontFamily::GeistMono, None));
        let head = format!("{{\"type\":\"{LABELLED_TYPE}\",\"version\":{FAMILY_VERSION},");
        assert!(mono.starts_with(&head), "{mono}");
        assert!(mono.contains("\"family\":\"Geist Mono\""));
        let Content::Layers(p) = decode(&mono) else {
            panic!("pastes")
        };
        assert_eq!(p.nodes, vec![text_layer(FontFamily::GeistMono, None)]);
        // A labelled arrow alongside: still version 2 (version 1 has no families).
        let mut arrow = Node::new(
            NodeId(2),
            NodeKind::Connector {
                points: vec![V2::ZERO, crate::geom::v2(40.0, 0.0)],
                start: None,
                end: None,
                route: Default::default(),
                start_head: Default::default(),
                end_head: Default::default(),
                label: Some("writes".into()),
            },
            Rect::new(0.0, 0.0, 40.0, 0.0),
        );
        let both = encode(
            &[text_layer(FontFamily::GeistMono, None), arrow.clone()],
            &[NodeId(1), NodeId(2)],
            &[Rect::new(10.0, 10.0, 80.0, 24.0), arrow.rect],
            None,
        );
        assert!(both.starts_with(&head), "{both}");
        // A labelled arrow alone: version 1, exactly as before.
        arrow.id = NodeId(3);
        let labelled = encode(
            std::slice::from_ref(&arrow),
            &[arrow.id],
            &[arrow.rect],
            None,
        );
        let v1 = format!("{{\"type\":\"{LABELLED_TYPE}\",\"version\":{LABELLED_VERSION},");
        assert!(labelled.starts_with(&v1) && !labelled.contains("family"));
    }

    #[test]
    fn an_underdeclared_or_unknown_family_is_refused_before_anything_is_registered() {
        // A unique image so its registration can be told apart.
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([17, 174, 91, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let bytes = bytes.into_inner();
        let id = crate::assets::hash(&bytes);
        let b64 = crate::assets::base64(&bytes);
        let mono = text_layer(FontFamily::GeistMono, Some(&id));
        let with_image = |v: &mut Value| v["assets"] = json!({ id.clone(): b64.clone() });
        let base: Value = serde_json::from_str(&payload(&mono)).unwrap();
        let edit = |f: &dyn Fn(&mut Value)| {
            let mut v = base.clone();
            with_image(&mut v);
            f(&mut v);
            v.to_string()
        };
        let family = |f: Value| edit(&move |v| v["nodes"][0]["text_style"]["family"] = f.clone());
        let cases: Vec<(&str, String)> = vec![
            // An older writer's version 1 can't hold a family: it never wrote one.
            (
                "version 1",
                edit(&|v| v["version"] = json!(LABELLED_VERSION)),
            ),
            // Unknown or misspelt, at the family version.
            ("unknown", family(json!("Helvetica"))),
            ("spelling", family(json!("geist mono"))),
            ("not text", family(json!(4))),
            // Unversioned `paper-rs/clipboard` with a family (known or not).
            (
                "plain known",
                edit(&|v| {
                    v["type"] = json!(TYPE);
                    v.as_object_mut().unwrap().remove("version");
                }),
            ),
            (
                "plain unknown",
                edit(&|v| {
                    v["type"] = json!(TYPE);
                    v.as_object_mut().unwrap().remove("version");
                    v["nodes"][0]["text_style"]["family"] = json!("Comic Sans");
                }),
            ),
            // A version this reader doesn't know.
            ("version 3", edit(&|v| v["version"] = json!(3))),
        ];
        for (what, text) in cases {
            let why = refused(decode(&text));
            assert!(why.contains("can't be pasted"), "{what}: {why}");
            assert!(
                crate::assets::get(&id).is_none(),
                "{what}: nothing registered"
            );
        }
        // A family named twice can't hide one behind the other.
        let plain = edit(&|v| {
            v["type"] = json!(TYPE);
            v.as_object_mut().unwrap().remove("version");
            v["nodes"][0]["text_style"]["family"] = json!("Inter");
        });
        let repeated = "\"family\":\"Geist Mono\",\"family\":\"Inter\"";
        let twice = plain.replacen("\"family\":\"Inter\"", repeated, 1);
        assert_ne!(twice, plain);
        refused(decode(&twice));
        assert!(crate::assets::get(&id).is_none());
        // The family payload itself, with its image: pasted, the image registered.
        let Content::Layers(p) = decode(&edit(&|_| {})) else {
            panic!("the version 2 payload pastes")
        };
        assert_eq!(p.nodes[0].text_style.family, FontFamily::GeistMono);
        assert!(crate::assets::get(&id).is_some());
    }

    /// R177's probe inputs, byte for byte: an underdeclared `paper-rs/clipboard` payload naming
    /// Geist Mono (refused before), the same cut off by its final brace, and one whose real
    /// `text_style` is followed by a second, empty one. The last two became a text layer.
    const UNDERDECLARED: &str = r##"{"type":"paper-rs/clipboard","nodes":[{"id":1,"name":"Text","type":"text","text":"Æøå✓","font_size":20.0,"auto_width":true,"align":"left","rect":{"x":0.0,"y":0.0,"w":80.0,"h":24.0},"style":{"fills":[{"type":"solid","color":"#1c1c1e"}],"stroke":null,"stroke_width":1.0,"corner_radius":0.0,"opacity":1.0},"visible":true,"locked":false,"text_style":{"bold":false,"italic":false,"family":"Geist Mono","letter_spacing":0.0}}],"roots":[1],"world":[{"x":0.0,"y":0.0,"w":80.0,"h":24.0}],"page":null,"assets":{}}"##;
    const CUT_AFTER_FAMILY: &str = r##"{"type":"paper-rs/clipboard","nodes":[{"id":1,"name":"Text","type":"text","text":"Æøå✓","font_size":20.0,"auto_width":true,"align":"left","rect":{"x":0.0,"y":0.0,"w":80.0,"h":24.0},"style":{"fills":[{"type":"solid","color":"#1c1c1e"}],"stroke":null,"stroke_width":1.0,"corner_radius":0.0,"opacity":1.0},"visible":true,"locked":false,"text_style":{"bold":false,"italic":false,"family":"Geist Mono","letter_spacing":0.0}}],"roots":[1],"world":[{"x":0.0,"y":0.0,"w":80.0,"h":24.0}],"page":null,"assets":{}"##;
    const HIDDEN_BY_REPEAT: &str = r##"{"type":"paper-rs/clipboard","nodes":[{"id":1,"name":"Text","type":"text","text":"Æøå✓","font_size":20.0,"auto_width":true,"align":"left","rect":{"x":0.0,"y":0.0,"w":80.0,"h":24.0},"style":{"fills":[{"type":"solid","color":"#1c1c1e"}],"stroke":null,"stroke_width":1.0,"corner_radius":0.0,"opacity":1.0},"visible":true,"locked":false,"text_style":{"family":"Geist Mono"},"text_style":{}}],"roots":[1],"world":[{"x":0.0,"y":0.0,"w":80.0,"h":24.0}],"page":null,"assets":{}}"##;

    #[test]
    fn a_cut_off_or_repeated_underdeclared_family_is_refused_never_pasted_as_text() {
        assert_eq!(CUT_AFTER_FAMILY, &UNDERDECLARED[..UNDERDECLARED.len() - 1]);
        // A unique image in the payload: refused payloads register nothing.
        let img = image::RgbaImage::from_pixel(2, 3, image::Rgba([91, 17, 174, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let bytes = bytes.into_inner();
        let id = crate::assets::hash(&bytes);
        let assets = format!(
            "\"assets\":{{\"{id}\":\"{}\"}}",
            crate::assets::base64(&bytes)
        );
        let with_image = |t: &str| t.replacen("\"assets\":{}", &assets, 1);
        let escaped_key = |t: &str| t.replacen("\"family\":", r#""fam\u0069ly":"#, 1);
        let escaped_family = escaped_key(CUT_AFTER_FAMILY);
        assert_ne!(
            escaped_family, CUT_AFTER_FAMILY,
            "the family key really is escaped"
        );
        let escaped_style =
            HIDDEN_BY_REPEAT.replacen(r#","text_style":{}"#, r#","text_st\u0079le":{}"#, 1);
        assert_ne!(
            escaped_style, HIDDEN_BY_REPEAT,
            "the text_style key really is escaped"
        );
        let cases: Vec<(&str, String)> = vec![
            ("complete (the paired control)", UNDERDECLARED.into()),
            ("cut after its family", CUT_AFTER_FAMILY.into()),
            ("a repeated text_style hides it", HIDDEN_BY_REPEAT.into()),
            ("cut, with its image", with_image(CUT_AFTER_FAMILY)),
            ("repeated, with its image", with_image(HIDDEN_BY_REPEAT)),
            // The same members spelled with escapes are the same members.
            ("cut, escaped family", escaped_family),
            ("repeated, escaped text_style", escaped_style),
            (
                "repeated family member",
                UNDERDECLARED.replacen(
                    r#""family":"Geist Mono""#,
                    r#""family":"Geist Mono","family":"Inter""#,
                    1,
                ),
            ),
            // Cut off inside the family's own value: still a family, one that isn't complete.
            (
                "cut inside its family",
                UNDERDECLARED[..UNDERDECLARED.find("Geist Mo").unwrap() + 8].to_string(),
            ),
        ];
        for (what, text) in cases {
            assert!(header(&text).plain, "{what}: its type is read");
            let why = match decode(&text) {
                Content::Refused(why) => why,
                Content::Text(_) => panic!("{what}: pasted as a text layer"),
                Content::Layers(_) => panic!("{what}: pasted as layers"),
                Content::Foreign(..) => panic!("{what}: pasted as foreign layers"),
            };
            assert!(why.contains("can't be pasted"), "{what}: {why}");
            assert!(why.contains("font family"), "{what}: {why}");
            assert!(
                crate::assets::get(&id).is_none(),
                "{what}: nothing registered"
            );
        }
        // Controls, decoded as before: the same historical payload without a family pastes; cut
        // off, it is the text it always was; prose naming a family is text; a layer whose own
        // text spells out a family member is a layer; and the version 2 payload pastes.
        let historical = UNDERDECLARED.replacen(r#""family":"Geist Mono","#, "", 1);
        assert!(!historical.contains("family"));
        let Content::Layers(p) = decode(&historical) else {
            panic!("the historical Inter payload pastes")
        };
        assert_eq!(p.nodes[0].text_style.family, FontFamily::Inter);
        let cut = &historical[..historical.len() - 1];
        assert!(matches!(decode(cut), Content::Text(t) if t == cut));
        let prose = r#"Set "text_style":{"family":"Geist Mono"} in the payload"#;
        assert!(matches!(decode(prose), Content::Text(t) if t == prose));
        let mut spelled = text_layer(FontFamily::Inter, None);
        if let NodeKind::Text { text, .. } = &mut spelled.kind {
            *text = r#""text_style":{"family":"Geist Mono"}"#.into();
        }
        let p = payload(&spelled);
        assert!(p.starts_with(&format!("{{\"type\":\"{TYPE}\"")));
        let Content::Layers(back) = decode(&p) else {
            panic!("a layer's own text is never taken for a member")
        };
        assert_eq!(back.nodes, vec![spelled]);
        let Content::Layers(v2) = decode(&payload(&text_layer(FontFamily::GeistMono, None))) else {
            panic!("the version 2 payload pastes")
        };
        assert_eq!(v2.nodes[0].text_style.family, FontFamily::GeistMono);
        assert!(crate::assets::get(&id).is_none());
    }
}
