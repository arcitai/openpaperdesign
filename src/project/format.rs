//! On-disk formats for projects and pages, with versioned migrations.
//!
//! Every file carries `format` and `version`. Loading parses into `serde_json::Value` (a page
//! after a raw check that no member it reads is named twice, see `check_unambiguous`), runs the
//! migration chain from the file's version up to the current one, then deserializes. Each step
//! is a pure function `Value -> Value`, so old files keep loading forever and every step is
//! unit-testable on its own.

use crate::geom::Camera;
use crate::model::{Document, Node, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const PROJECT_FORMAT: &str = "paper-rs-project";
pub const PAGE_FORMAT: &str = "paper-rs-page";
/// The single-file format of paper-rs 0.1 (`.paper-rs`), version 1. It migrates to a page.
pub const LEGACY_FORMAT: &str = "paper-rs";

pub const PROJECT_VERSION: u32 = 1;
/// v1: legacy single file. v2: page file inside a project. v3: fill layers (`fills`). A page is
/// written as v3 unless it needs v4 ([`PAGE_VERSION_LABELS`]) or v5
/// ([`PAGE_VERSION_FAMILIES`]).
pub const PAGE_VERSION: u32 = 3;
/// v4: arrow labels (a connector's `label`). Only a page with at least one label is written as
/// v4, so every other page keeps its v3 bytes and stays readable by older readers, and an
/// older reader refuses a labelled page ("newer than this app supports") instead of reading it
/// and dropping its labels on the next save, as it would an unknown field in a v3 page. That
/// refusal is a deliberate downgrade limit: a labelled page doesn't open in a reader without
/// labels.
pub const PAGE_VERSION_LABELS: u32 = 4;
/// v5: a text layer's font family other than Inter (`text_style.family`, see
/// [`crate::model::FontFamily`]). Only a page with such a layer is written as v5, so every
/// Inter page keeps its v3 or v4 bytes; an older reader (one that knows labels but not
/// families) refuses a v5 page as newer instead of drawing its layers in Inter and saving them
/// without their family. As for labels, that refusal is the deliberate downgrade limit.
pub const PAGE_VERSION_FAMILIES: u32 = 5;
/// The newest page version this build reads.
pub const PAGE_VERSION_MAX: u32 = PAGE_VERSION_FAMILIES;

#[derive(Debug)]
pub enum FormatError {
    Io(std::io::Error),
    Json(serde_json::Error),
    WrongFormat(String),
    TooNew { version: u32, supported: u32 },
    Invalid(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Io(e) => write!(f, "I/O error: {e}"),
            FormatError::Json(e) => write!(f, "malformed file: {e}"),
            FormatError::WrongFormat(s) => write!(f, "not a paper-rs file (format {s:?})"),
            FormatError::TooNew { version, supported } => write!(
                f,
                "file version {version} is newer than this app supports ({supported})"
            ),
            FormatError::Invalid(s) => write!(f, "invalid document: {s}"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<std::io::Error> for FormatError {
    fn from(e: std::io::Error) -> Self {
        FormatError::Io(e)
    }
}

// ---------------------------------------------------------------------------------- project

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub name: String,
    /// Unix milliseconds.
    pub created: u64,
    pub modified: u64,
    pub pages: Vec<PageRef>,
    #[serde(default)]
    pub active_page: Option<String>,
}

impl Manifest {
    pub fn new(name: &str) -> Manifest {
        let now = now_ms();
        let page = PageRef {
            id: new_id(),
            name: "Page 1".into(),
        };
        Manifest {
            format: PROJECT_FORMAT.into(),
            version: PROJECT_VERSION,
            id: new_id(),
            name: name.into(),
            created: now,
            modified: now,
            active_page: Some(page.id.clone()),
            pages: vec![page],
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("manifest serializes")
    }

    pub fn from_json(s: &str) -> Result<Manifest, FormatError> {
        let v: Value = serde_json::from_str(s).map_err(FormatError::Json)?;
        let v = migrate(v, PROJECT_FORMAT, PROJECT_VERSION, &[], &[])?;
        serde_json::from_value(v).map_err(FormatError::Json)
    }
}

// ---------------------------------------------------------------------------------- page

/// One page: an infinite canvas with its own nodes and camera.
#[derive(Debug, Clone, PartialEq)]
pub struct PageData {
    pub id: String,
    pub name: String,
    pub doc: Document,
    pub camera: Option<Camera>,
}

#[derive(Serialize, Deserialize)]
struct PageFile {
    format: String,
    version: u32,
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera: Option<Camera>,
    next_id: u64,
    root: Vec<NodeId>,
    nodes: Vec<Node>,
}

/// Page migrations, indexed by source version: `PAGE_STEPS[0]` turns v1 into v2.
const PAGE_STEPS: &[fn(Value) -> Value] =
    &[page_v1_to_v2, page_v2_to_v3, page_v3_to_v4, page_v4_to_v5];
/// Formats accepted as older versions of a page.
const PAGE_ALIASES: &[&str] = &[LEGACY_FORMAT];

/// v1 (`.paper-rs` single file) → v2 page: new format tag and a page id.
fn page_v1_to_v2(mut v: Value) -> Value {
    v["format"] = json!(PAGE_FORMAT);
    if v.get("id").is_none() {
        v["id"] = json!(new_id());
    }
    if v.get("name").is_none() {
        v["name"] = json!("Page 1");
    }
    v
}

/// v2 → v3: `style.fill: "#rrggbb"` becomes `style.fills: [{type: solid, color}]`.
fn page_v2_to_v3(mut v: Value) -> Value {
    if let Some(nodes) = v.get_mut("nodes").and_then(|n| n.as_array_mut()) {
        for n in nodes {
            let Some(style) = n.get_mut("style").and_then(|s| s.as_object_mut()) else {
                continue;
            };
            if let Some(fill) = style.remove("fill")
                && let Some(c) = fill.as_str()
            {
                style.insert("fills".into(), json!([{ "type": "solid", "color": c }]));
            }
        }
    }
    v
}

/// Whether `doc` has a layer whose font family isn't Inter (what needs page version 5).
pub fn has_font_family(doc: &Document) -> bool {
    doc.nodes
        .values()
        .any(|n| !n.text_style.family.is_default())
}

/// The page version `doc` needs: v5 when a layer has a font family other than Inter, else v4
/// when an arrow has a label, else v3.
pub fn page_version_for(doc: &Document) -> u32 {
    if has_font_family(doc) {
        PAGE_VERSION_FAMILIES
    } else if doc.nodes.values().any(|n| n.kind.label().is_some()) {
        PAGE_VERSION_LABELS
    } else {
        PAGE_VERSION
    }
}

/// v3 → v4: v4 adds the optional arrow label only, so a v3 page is already a v4 page.
fn page_v3_to_v4(v: Value) -> Value {
    v
}

/// v4 → v5: v5 adds the optional font family only, so a v4 page is already a v5 page.
fn page_v4_to_v5(v: Value) -> Value {
    v
}

pub fn page_to_json(p: &PageData) -> String {
    let mut nodes: Vec<Node> = p.doc.nodes.values().cloned().collect();
    nodes.sort_by_key(|n| n.id);
    let f = PageFile {
        format: PAGE_FORMAT.into(),
        version: page_version_for(&p.doc),
        id: p.id.clone(),
        name: p.name.clone(),
        camera: p.camera,
        next_id: p.doc.next_id,
        root: p.doc.root.clone(),
        nodes,
    };
    serde_json::to_string_pretty(&f).expect("page serializes")
}

/// Parse a page file (or a legacy `.paper-rs` document) of any supported version.
pub fn page_from_json(s: &str) -> Result<PageData, FormatError> {
    // Before `Value`, which keeps only the last of a member named twice (R73-F2).
    check_unambiguous(s)?;
    let v: Value = serde_json::from_str(s).map_err(FormatError::Json)?;
    check_labels(&v)?;
    check_families(&v)?;
    let v = migrate(v, PAGE_FORMAT, PAGE_VERSION_MAX, PAGE_ALIASES, PAGE_STEPS)?;
    let f: PageFile = serde_json::from_value(v).map_err(FormatError::Json)?;
    let max_id = f.nodes.iter().map(|n| n.id.0).max().unwrap_or(0);
    let nodes: HashMap<NodeId, Node> = f.nodes.into_iter().map(|n| (n.id, n)).collect();
    let doc = Document {
        name: f.name.clone(),
        nodes,
        root: f.root,
        next_id: f.next_id.max(max_id + 1),
    };
    doc.validate().map_err(FormatError::Invalid)?;
    Ok(PageData {
        id: f.id,
        name: f.name,
        doc,
        camera: f.camera,
    })
}

/// Where [`Unambiguous`] is in a page: its top level, its `nodes` list, one node, or one node's
/// `text_style`.
#[derive(Clone, Copy)]
enum Level {
    Top,
    Nodes,
    Node,
    Style,
}

/// A page read member by member with serde, before it becomes a `Value` (which keeps only the
/// last of a member named twice, so a checked value could differ from the one written): in its
/// top level (`format`, `version`, `nodes`…), each node and each node's `text_style`, a member
/// named twice is an error, names compared as decoded (an escaped spelling of `family` is
/// `family`). Those are the objects whose members the page checks read (`check_labels`,
/// `check_families`, `migrate`): a second `family`, `text_style`, `nodes` or `version` could
/// otherwise hide an unknown family, or one an older version can't hold, behind an accepted one
/// and drop it on the next save. The rest of the page is skipped.
#[derive(Clone, Copy)]
struct Unambiguous(Level);

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
        if let Level::Nodes = self.0 {
            while seq.next_element_seed(Unambiguous(Level::Node))?.is_some() {}
        } else {
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
        }
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let checked = !matches!(self.0, Level::Nodes);
        let mut names = std::collections::HashSet::new();
        while let Some(name) = map.next_key::<String>()? {
            if checked && !names.insert(name.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "{name:?} is named twice in one object, so what it holds is ambiguous"
                )));
            }
            let inner = match (self.0, name.as_str()) {
                (Level::Top, "nodes") => Some(Level::Nodes),
                (Level::Node, "text_style") => Some(Level::Style),
                _ => None,
            };
            match inner {
                Some(level) => map.next_value_seed(Unambiguous(level))?,
                None => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

/// A page with a member named twice where the page checks read ([`Unambiguous`]) is refused
/// whole, before anything is read from it. Text that isn't JSON is left to the parse that
/// follows (its error is the one reported, as before).
fn check_unambiguous(s: &str) -> Result<(), FormatError> {
    use serde::de::DeserializeSeed;
    let mut de = serde_json::Deserializer::from_str(s);
    match Unambiguous(Level::Top).deserialize(&mut de) {
        Err(e) if e.is_data() => Err(FormatError::Invalid(format!("the page's {e}"))),
        _ => Ok(()),
    }
}

/// A `label` is a connector field of page v4: a page that declares an older version (or none)
/// and has one, or that has one on another kind of node, is refused rather than read, since a
/// reader that doesn't know labels would have lost it (and this one can't tell what it was
/// meant to be). Its value must be a string.
fn check_labels(v: &Value) -> Result<(), FormatError> {
    let version = v.get("version").and_then(Value::as_u64).unwrap_or(1);
    let Some(nodes) = v.get("nodes").and_then(Value::as_array) else {
        return Ok(());
    };
    for n in nodes {
        let Some(label) = n.get("label") else {
            continue;
        };
        let id = n.get("id").map_or_else(String::new, |i| format!(" {i}"));
        if n.get("type").and_then(Value::as_str) != Some("connector") {
            return Err(FormatError::Invalid(format!(
                "node{id} has a label, which only an arrow can have"
            )));
        }
        if version < u64::from(PAGE_VERSION_LABELS) {
            return Err(FormatError::Invalid(format!(
                "arrow{id} has a label, which needs page version {PAGE_VERSION_LABELS}; this \
                 page says version {version}"
            )));
        }
        if !label.is_string() {
            return Err(FormatError::Invalid(format!(
                "arrow{id}'s label isn't text"
            )));
        }
    }
    Ok(())
}

/// A font family is a text-style field of page v5: each one stored must be the exact name of a
/// family this build has (an unknown or misspelt one is refused, never read as Inter or any
/// other family and saved over), and one other than Inter in a page that declares an older
/// version (or none) is refused, since a reader without families would have drawn and saved
/// it as Inter. An explicit "Inter" means what its absence means, at any version.
fn check_families(v: &Value) -> Result<(), FormatError> {
    let version = v.get("version").and_then(Value::as_u64).unwrap_or(1);
    let Some(nodes) = v.get("nodes").and_then(Value::as_array) else {
        return Ok(());
    };
    for n in nodes {
        let Some(family) = n.get("text_style").and_then(|t| t.get("family")) else {
            continue;
        };
        let id = n.get("id").map_or_else(String::new, |i| format!(" {i}"));
        let known = family
            .as_str()
            .and_then(crate::model::FontFamily::from_name);
        let Some(known) = known else {
            return Err(FormatError::Invalid(format!(
                "layer{id} names the font family {family}, which this app doesn't have \
                 (it has {})",
                family_names()
            )));
        };
        if !known.is_default() && version < u64::from(PAGE_VERSION_FAMILIES) {
            return Err(FormatError::Invalid(format!(
                "layer{id} uses the font family {}, which needs page version \
                 {PAGE_VERSION_FAMILIES}; this page says version {version}",
                known.name()
            )));
        }
    }
    Ok(())
}

/// The bundled families' names, for messages ("Inter, Geist Mono").
pub fn family_names() -> String {
    crate::model::FontFamily::ALL.map(|f| f.name()).join(", ")
}

/// Check the format tag and run the migration chain up to `current`.
fn migrate(
    mut v: Value,
    format: &str,
    current: u32,
    aliases: &[&str],
    steps: &[fn(Value) -> Value],
) -> Result<Value, FormatError> {
    let tag = v
        .get("format")
        .and_then(|f| f.as_str())
        .unwrap_or("")
        .to_string();
    if tag != format && !aliases.contains(&tag.as_str()) {
        return Err(FormatError::WrongFormat(tag));
    }
    // The raw version is checked before it is narrowed or migrated: a missing one is v1 (the
    // first files had none), anything else must be a whole number from 1 to `current`. A
    // larger one would otherwise be cut to 32 bits (2^32 + 3 read as 3) and rewritten as
    // supported data.
    let raw = match v.get("version") {
        None => 1,
        Some(x) => x.as_u64().filter(|n| *n >= 1).ok_or_else(|| {
            FormatError::Invalid(format!(
                "the file's version {x} isn't a whole number from 1"
            ))
        })?,
    };
    if raw > u64::from(current) {
        return Err(match u32::try_from(raw) {
            Ok(version) => FormatError::TooNew {
                version,
                supported: current,
            },
            Err(_) => FormatError::Invalid(format!(
                "file version {raw} is newer than this app supports ({current})"
            )),
        });
    }
    let mut version = raw as u32;
    while version < current {
        let step = steps
            .get(version as usize - 1)
            .ok_or_else(|| FormatError::Invalid(format!("no migration from v{version}")))?;
        v = step(v);
        version += 1;
        v["version"] = json!(version);
    }
    Ok(v)
}

// ---------------------------------------------------------------------------------- helpers

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Random 16-hex-digit identifier.
pub fn new_id() -> String {
    format!("{:016x}", fastrand::u64(..))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn page_roundtrip_is_deterministic() {
        let ed = demo::showcase();
        let p = PageData {
            id: "abc".into(),
            name: "Plan".into(),
            doc: Document {
                name: "Plan".into(),
                ..ed.doc.clone()
            },
            camera: Some(Camera::default()),
        };
        let s = page_to_json(&p);
        let back = page_from_json(&s).unwrap();
        assert_eq!(back, p);
        assert_eq!(page_to_json(&back), s);
    }

    #[test]
    fn legacy_v1_file_migrates_to_page() {
        let ed = demo::showcase();
        let legacy = crate::io::file::to_string(&ed.doc, None);
        assert!(legacy.contains("\"version\": 1"));
        let p = page_from_json(&legacy).unwrap();
        assert_eq!(p.doc.nodes, ed.doc.nodes);
        assert_eq!(p.name, ed.doc.name);
        assert_eq!(p.id.len(), 16);
    }

    #[test]
    fn rejects_foreign_and_future_files() {
        assert!(matches!(
            page_from_json(r#"{"format":"x","version":1}"#),
            Err(FormatError::WrongFormat(_))
        ));
        assert!(matches!(
            page_from_json(r#"{"format":"paper-rs-page","version":99}"#),
            Err(FormatError::TooNew { version: 99, .. })
        ));
        assert!(matches!(
            Manifest::from_json(r#"{"format":"paper-rs-page","version":1}"#),
            Err(FormatError::WrongFormat(_))
        ));
        let broken = r#"{"format":"paper-rs-page","version":2,"id":"a","name":"x","next_id":1,"root":[5],"nodes":[]}"#;
        assert!(matches!(
            page_from_json(broken),
            Err(FormatError::Invalid(_))
        ));
    }

    #[test]
    fn v2_fill_migrates_to_fill_layers() {
        let v2 = r##"{"format":"paper-rs-page","version":2,"id":"a","name":"P","next_id":3,
            "root":[1,2],"nodes":[
            {"id":1,"name":"R","type":"rect","rect":{"x":0,"y":0,"w":5,"h":5},
             "style":{"fill":"#ff0000","stroke":null,"stroke_width":1,"corner_radius":2,"opacity":1}},
            {"id":2,"name":"E","type":"ellipse","rect":{"x":0,"y":0,"w":5,"h":5},
             "style":{"fill":null,"stroke":"#000000","stroke_width":1,"corner_radius":0,"opacity":1}}]}"##;
        let p = page_from_json(v2).unwrap();
        let r = p.doc.get(NodeId(1)).unwrap();
        assert_eq!(r.style.fill(), Some(crate::model::Color::rgb(255, 0, 0)));
        assert_eq!(r.style.corner_radius, 2.0);
        assert!(p.doc.get(NodeId(2)).unwrap().style.fills.is_empty());
        assert!(page_to_json(&p).contains("\"version\": 3"));
    }

    /// One page with one arrow, labelled `label`.
    fn arrow_page(label: Option<&str>) -> PageData {
        use crate::geom::{Rect, V2, v2};
        use crate::model::{Head, NodeKind, Route};
        let id = NodeId(1);
        let n = Node::new(
            id,
            NodeKind::Connector {
                points: vec![V2::ZERO, v2(100.0, 0.0)],
                start: None,
                end: None,
                route: Route::Straight,
                start_head: Head::None,
                end_head: Head::Arrow,
                label: label.map(String::from),
            },
            Rect::new(0.0, 0.0, 100.0, 0.0),
        );
        // A page's document is named after the page, as every reader rebuilds it.
        let mut doc = Document {
            name: "P".into(),
            ..Document::default()
        };
        doc.nodes.insert(id, n);
        doc.root.push(id);
        doc.next_id = 2;
        PageData {
            id: "abc".into(),
            name: "P".into(),
            doc,
            camera: None,
        }
    }

    #[test]
    fn only_a_labelled_page_is_written_as_v4_and_reads_back_exactly() {
        // Unlabelled: v3, no `label` key, the same bytes as before labels existed.
        let plain = page_to_json(&arrow_page(None));
        assert!(
            plain.contains("\"version\": 3") && !plain.contains("\"label\""),
            "{plain}"
        );
        assert_eq!(page_to_json(&page_from_json(&plain).unwrap()), plain);
        // Labelled: v4, and the label (inner spaces and a line break kept) reads back.
        let p = arrow_page(Some("writes  to\nqueue"));
        let s = page_to_json(&p);
        assert!(s.contains("\"version\": 4"), "{s}");
        let back = page_from_json(&s).unwrap();
        assert_eq!(back, p);
        assert_eq!(page_to_json(&back), s);
        // A v4 page whose labels are gone is written back as v3 (nothing needs v4).
        let mut cleared = back.clone();
        for n in cleared.doc.nodes.values_mut() {
            if let crate::model::NodeKind::Connector { label, .. } = &mut n.kind {
                *label = None;
            }
        }
        assert_eq!(page_to_json(&cleared), plain);
    }

    #[test]
    fn a_label_an_older_reader_would_drop_is_refused_not_read() {
        let s = page_to_json(&arrow_page(Some("writes to")));
        let mut v: Value = serde_json::from_str(&s).unwrap();
        // Declared v3 (as an older writer would never write it, and an older reader would lose).
        v["version"] = json!(3);
        let e = page_from_json(&v.to_string()).unwrap_err();
        assert!(
            matches!(e, FormatError::Invalid(ref m) if m.contains("version 4")),
            "{e}"
        );
        // A label on something that isn't an arrow.
        let mut w: Value = serde_json::from_str(&s).unwrap();
        w["nodes"][0]["type"] = json!("rect");
        assert!(matches!(
            page_from_json(&w.to_string()),
            Err(FormatError::Invalid(_))
        ));
        // A label that isn't text.
        let mut x: Value = serde_json::from_str(&s).unwrap();
        x["nodes"][0]["label"] = json!(7);
        assert!(matches!(
            page_from_json(&x.to_string()),
            Err(FormatError::Invalid(_))
        ));
        // Newer than this build's newest (v5 since font families) is refused as before; a
        // labelled page that says v5 reads (v5 has labels too) and is written back as v4.
        let mut y: Value = serde_json::from_str(&s).unwrap();
        y["version"] = json!(6);
        assert!(matches!(
            page_from_json(&y.to_string()),
            Err(FormatError::TooNew {
                version: 6,
                supported: 5
            })
        ));
        y["version"] = json!(5);
        let p = page_from_json(&y.to_string()).unwrap();
        assert_eq!(page_to_json(&p), s);
        // An unlabelled page that says v4 reads as before (and is written back as v3).
        let mut z: Value = serde_json::from_str(&page_to_json(&arrow_page(None))).unwrap();
        z["version"] = json!(4);
        let p = page_from_json(&z.to_string()).unwrap();
        assert!(page_to_json(&p).contains("\"version\": 3"));
    }

    /// R38-F3: the raw version is checked before it is narrowed. A labelled v4 page whose version
    /// is 2^32 + 3 was read as 3, migrated to 4 and accepted.
    #[test]
    fn a_raw_page_version_is_checked_before_it_is_narrowed_or_migrated() {
        let s = page_to_json(&arrow_page(Some("writes")));
        let with = |version: Value| {
            let mut v: Value = serde_json::from_str(&s).unwrap();
            v["version"] = version;
            page_from_json(&v.to_string())
        };
        assert!(with(json!(4)).is_ok(), "control: the labelled page itself");
        for bad in [
            json!(4_294_967_299_u64),
            json!(4_294_967_300_u64),
            json!(8_589_934_596_u64),
            json!(4_294_967_301_u64),
            json!(u64::MAX),
            json!(6),
            json!(0),
            json!(-1),
            json!(3.0),
            json!("4"),
            Value::Null,
        ] {
            assert!(with(bad.clone()).is_err(), "{bad} must be refused");
        }
        // Newer than v5 (the newest since font families; it was v4 before).
        assert!(matches!(
            with(json!(6)),
            Err(FormatError::TooNew {
                version: 6,
                supported: 5
            })
        ));
        // Legitimate older pages still migrate: an unlabelled v3, and v1/v2 through their steps.
        let plain = page_to_json(&arrow_page(None));
        for old in [1, 2, 3] {
            let mut v: Value = serde_json::from_str(&plain).unwrap();
            v["version"] = json!(old);
            let p = page_from_json(&v.to_string()).unwrap();
            assert_eq!(p.doc.nodes, arrow_page(None).doc.nodes, "v{old}");
            assert_eq!(page_to_json(&p), plain, "v{old} is written back as v3");
        }
        // A manifest goes through the same check.
        let mut m: Value = serde_json::from_str(&Manifest::new("M").to_json()).unwrap();
        m["version"] = json!(4_294_967_297_u64);
        assert!(Manifest::from_json(&m.to_string()).is_err());
    }

    /// One page with one Unicode text layer in `family` (and, with `label`, a labelled arrow).
    fn text_page(family: crate::model::FontFamily, label: bool) -> PageData {
        use crate::geom::Rect;
        use crate::model::{Color, NodeKind, TextAlign, TextSpan};
        let mut p = arrow_page(label.then_some("writes"));
        if !label {
            p.doc.nodes.clear();
            p.doc.root.clear();
        }
        let id = NodeId(2);
        let mut n = Node::new(
            id,
            NodeKind::Text {
                text: "Æble — øre ✓".into(),
                font_size: 20.0,
                auto_width: false,
                align: TextAlign::Left,
            },
            Rect::new(10.0, 20.0, 160.0, 24.0),
        );
        n.text_style.family = family;
        n.text_style.line_height = Some(1.4);
        n.text_style.spans = vec![TextSpan {
            start: 0,
            end: 4,
            color: Some(Color::rgb(255, 0, 0)),
            underline: Some(true),
            ..Default::default()
        }];
        p.doc.nodes.insert(id, n);
        p.doc.root.push(id);
        p.doc.next_id = 3;
        p
    }

    #[test]
    fn only_a_page_with_another_font_family_is_written_as_v5_and_reads_back_exactly() {
        use crate::model::FontFamily;
        // Inter pages keep their versions and bytes: no `family` key, v3 (or v4 with a label).
        let inter = page_to_json(&text_page(FontFamily::Inter, false));
        assert!(
            inter.contains("\"version\": 3") && !inter.contains("family"),
            "{inter}"
        );
        let labelled = page_to_json(&text_page(FontFamily::Inter, true));
        assert!(labelled.contains("\"version\": 4") && !labelled.contains("family"));
        // Geist Mono: v5 with the family's name, everything else (text, range, metric, fixed
        // box) read back exactly, and written back byte for byte.
        for label in [false, true] {
            let p = text_page(FontFamily::GeistMono, label);
            let s = page_to_json(&p);
            assert!(s.contains("\"version\": 5"), "{s}");
            assert!(s.contains("\"family\": \"Geist Mono\""), "{s}");
            let back = page_from_json(&s).unwrap();
            assert_eq!(back, p);
            assert_eq!(page_to_json(&back), s);
            // Back to Inter: written back as the Inter page it now is.
            let mut cleared = back.clone();
            for n in cleared.doc.nodes.values_mut() {
                n.text_style.family = FontFamily::Inter;
            }
            let want = if label { &labelled } else { &inter };
            assert_eq!(&page_to_json(&cleared), want);
        }
        // A v3/v4 page holding an explicit "Inter" reads as Inter (and is written without it).
        let mut v: Value = serde_json::from_str(&inter).unwrap();
        v["nodes"][0]["text_style"]["family"] = json!("Inter");
        assert_eq!(
            page_to_json(&page_from_json(&v.to_string()).unwrap()),
            inter
        );
    }

    #[test]
    fn a_font_family_an_older_reader_would_drop_or_this_one_lacks_is_refused_not_read() {
        use crate::model::FontFamily;
        let s = page_to_json(&text_page(FontFamily::GeistMono, false));
        let with = |f: &dyn Fn(&mut Value)| {
            let mut v: Value = serde_json::from_str(&s).unwrap();
            f(&mut v);
            page_from_json(&v.to_string())
        };
        assert!(with(&|_| {}).is_ok(), "control: the v5 page itself");
        // Declared older than v5 (an older writer never writes it; an older reader loses it).
        for old in [json!(3), json!(4), Value::Null] {
            let e = with(&|v| {
                if old.is_null() {
                    v.as_object_mut().unwrap().remove("version");
                } else {
                    v["version"] = old.clone();
                }
            })
            .unwrap_err();
            assert!(
                matches!(e, FormatError::Invalid(ref m) if m.contains("version 5")),
                "{old}: {e}"
            );
        }
        // A family this build doesn't have, or not exactly its name, or not a name at all: refused
        // (never read as Inter or a near match, so never saved over).
        for bad in [
            json!("Helvetica"),
            json!("geist mono"),
            json!("GeistMono"),
            json!(""),
            json!(5),
            json!(null),
            json!({"name": "Geist Mono"}),
        ] {
            let e = with(&|v| v["nodes"][0]["text_style"]["family"] = bad.clone()).unwrap_err();
            assert!(
                matches!(e, FormatError::Invalid(ref m) if m.contains("font family")),
                "{e}"
            );
        }
        // Newer than v5 still refused; an unsupported raw version still checked before the rest.
        assert!(matches!(
            with(&|v| v["version"] = json!(6)),
            Err(FormatError::TooNew {
                version: 6,
                supported: 5
            })
        ));
        assert!(with(&|v| v["version"] = json!(4_294_967_301_u64)).is_err());
    }

    /// `name` with its character `at` written as a JSON Unicode escape (a backslash, `u` and
    /// four hex digits), built at run time so the input really holds the escape, not the plain
    /// name.
    fn escaped(name: &str, at: usize) -> String {
        name.chars()
            .enumerate()
            .map(|(i, c)| {
                if i == at {
                    format!("{}u{:04x}", '\\', c as u32)
                } else {
                    c.to_string()
                }
            })
            .collect()
    }

    #[test]
    fn a_member_named_twice_cannot_hide_a_font_family_and_is_refused_whole() {
        use crate::model::FontFamily;
        // R73-F2: these raw pages (a `Value` can't hold a member twice, so they are text) were
        // read with the last member only: an unknown family behind Geist Mono, and a Geist Mono
        // in a v3 page behind Inter, were accepted and dropped on the next save.
        let mono = page_to_json(&text_page(FontFamily::GeistMono, false));
        let inter = page_to_json(&text_page(FontFamily::Inter, false));
        let marker = "\"family\": \"Geist Mono\"";
        assert!(mono.contains(marker) && mono.contains("\"version\": 5"));
        let under = mono.replacen("\"version\": 5", "\"version\": 3", 1);
        let family = escaped("family", 3);
        let text_style = escaped("text_style", 8);
        assert_eq!(family, format!("fam{}u0069ly", '\\'));
        // The nodes list of the Geist Mono page, to put before the Inter page's own.
        let at = mono.find("\"nodes\": ").unwrap();
        let mono_nodes = mono[at..mono.rfind('}').unwrap()].trim_end();
        let at = inter.find("\"nodes\": [").unwrap();
        let node = at + inter[at..].find("\"id\": ").unwrap();
        let mut shadowed_style = inter.clone();
        shadowed_style.insert_str(
            node,
            &format!(r#""{text_style}": {{"family": "Geist Mono"}}, "text_style": {{}}, "#),
        );
        let with = |page: &str, members: &str| page.replacen(marker, members, 1);
        let escaped_second = format!(r#""family": "Helvetica", "{family}": "Geist Mono""#);
        let cases = [
            // The reviewer's three inputs.
            (
                "unknown, then Geist Mono",
                with(&mono, r#""family": "Helvetica", "family": "Geist Mono""#),
            ),
            (
                "unknown, then an escaped Geist Mono",
                with(&mono, &escaped_second),
            ),
            (
                "v3: Geist Mono, then Inter",
                with(&under, r#""family": "Geist Mono", "family": "Inter""#),
            ),
            // The other order of each.
            (
                "Geist Mono, then unknown",
                with(&mono, r#""family": "Geist Mono", "family": "Helvetica""#),
            ),
            (
                "v3: Inter, then Geist Mono",
                with(&under, r#""family": "Inter", "family": "Geist Mono""#),
            ),
            // The containers a family is read through.
            ("text_style twice (escaped first)", shadowed_style),
            (
                "nodes twice",
                inter.replacen("\"nodes\": ", &format!("{mono_nodes}, \"nodes\": "), 1),
            ),
            (
                "version twice",
                under.replacen("\"version\": 3", "\"version\": 3, \"version\": 5", 1),
            ),
        ];
        for (what, raw) in cases {
            let e = page_from_json(&raw).unwrap_err();
            assert!(
                matches!(e, FormatError::Invalid(ref m) if m.contains("named twice")),
                "{what}: {e}"
            );
        }
        // Controls: the single-member pages, as before. Geist Mono at v5 reads back exactly,
        // also with its key escaped (the same member); an explicit Inter reads as Inter; an Inter
        // page keeps its v3 bytes; an unknown family and a v3 Geist Mono are refused as before.
        let back = page_from_json(&mono).unwrap();
        assert_eq!(page_to_json(&back), mono);
        let escaped_key = mono.replacen(marker, &format!("\"{family}\": \"Geist Mono\""), 1);
        assert_ne!(escaped_key, mono);
        assert_eq!(page_from_json(&escaped_key).unwrap(), back);
        let explicit = mono.replacen(marker, "\"family\": \"Inter\"", 1);
        let explicit = explicit.replacen("\"version\": 5", "\"version\": 3", 1);
        assert_eq!(page_to_json(&page_from_json(&explicit).unwrap()), inter);
        assert_eq!(page_to_json(&page_from_json(&inter).unwrap()), inter);
        let unknown = mono.replacen(marker, "\"family\": \"Helvetica\"", 1);
        assert!(matches!(
            page_from_json(&unknown),
            Err(FormatError::Invalid(ref m)) if m.contains("font family")
        ));
        assert!(matches!(
            page_from_json(&under),
            Err(FormatError::Invalid(ref m)) if m.contains("version 5")
        ));
        // Text that isn't JSON is still reported as such.
        assert!(matches!(
            page_from_json(&mono[..mono.len() - 1]),
            Err(FormatError::Json(_))
        ));
    }

    #[test]
    fn a_project_refuses_an_ambiguous_page_without_writing_it() {
        use crate::model::FontFamily;
        // R73-F2 through the normal project reader, on a test-owned temporary project: the
        // refused page file is left byte for byte, so no save can drop the hidden family.
        let root = std::env::temp_dir().join(format!("opd-r180-{:016x}", fastrand::u64(..)));
        let store = super::super::Store::open(root.clone()).unwrap();
        let page = text_page(FontFamily::GeistMono, false);
        let p = store
            .create_with_pages("R180 family", vec![page.clone()])
            .unwrap();
        let p = super::super::Project::open(&p.dir).unwrap();
        let loaded = p.load_page(&page.id).unwrap();
        assert_eq!(loaded.doc.nodes, page.doc.nodes, "control");
        p.save_page(&loaded).unwrap();
        let mono = std::fs::read_to_string(p.page_path(&page.id)).unwrap();
        assert!(mono.contains("\"family\": \"Geist Mono\""));
        let marker = "\"family\": \"Geist Mono\"";
        let under = mono.replacen("\"version\": 5", "\"version\": 3", 1);
        let with = |page: &str, members: &str| page.replacen(marker, members, 1);
        for raw in [
            with(&under, r#""family": "Geist Mono", "family": "Inter""#),
            with(&mono, r#""family": "Helvetica", "family": "Geist Mono""#),
            with(&mono, r#""family": "Helvetica""#),
        ] {
            std::fs::write(p.page_path(&page.id), &raw).unwrap();
            assert!(p.load_page(&page.id).is_err());
            assert!(p.load_pages().is_err(), "the project load too");
            assert_eq!(
                std::fs::read_to_string(p.page_path(&page.id)).unwrap(),
                raw,
                "the refused file is unchanged"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_roundtrip() {
        let m = Manifest::new("Hello");
        assert_eq!(Manifest::from_json(&m.to_json()).unwrap(), m);
    }
}
