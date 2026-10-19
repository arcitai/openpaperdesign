//! On-disk formats for projects and pages, with versioned migrations.
//!
//! Every file carries `format` and `version`. Loading parses into `serde_json::Value`, runs the
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
/// written as v3 unless it needs v4 ([`PAGE_VERSION_LABELS`]).
pub const PAGE_VERSION: u32 = 3;
/// v4: arrow labels (a connector's `label`). Only a page with at least one label is written as
/// v4, so every other page keeps its v3 bytes and stays readable by older readers, and an
/// older reader refuses a labelled page ("newer than this app supports") instead of reading it
/// and dropping its labels on the next save, as it would an unknown field in a v3 page. That
/// refusal is a deliberate downgrade limit: a labelled page doesn't open in a reader without
/// labels.
pub const PAGE_VERSION_LABELS: u32 = 4;
/// The newest page version this build reads.
pub const PAGE_VERSION_MAX: u32 = PAGE_VERSION_LABELS;

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
const PAGE_STEPS: &[fn(Value) -> Value] = &[page_v1_to_v2, page_v2_to_v3, page_v3_to_v4];
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

/// The page version `doc` needs: v4 when an arrow has a label, else v3.
pub fn page_version_for(doc: &Document) -> u32 {
    if doc.nodes.values().any(|n| n.kind.label().is_some()) {
        PAGE_VERSION_LABELS
    } else {
        PAGE_VERSION
    }
}

/// v3 → v4: v4 adds the optional arrow label only, so a v3 page is already a v4 page.
fn page_v3_to_v4(v: Value) -> Value {
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
    let v: Value = serde_json::from_str(s).map_err(FormatError::Json)?;
    check_labels(&v)?;
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
    let mut version = v.get("version").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
    if version > current {
        return Err(FormatError::TooNew {
            version,
            supported: current,
        });
    }
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
        // Newer than v4 is refused as before.
        let mut y: Value = serde_json::from_str(&s).unwrap();
        y["version"] = json!(5);
        assert!(matches!(
            page_from_json(&y.to_string()),
            Err(FormatError::TooNew {
                version: 5,
                supported: 4
            })
        ));
        // An unlabelled page that says v4 reads as before (and is written back as v3).
        let mut z: Value = serde_json::from_str(&page_to_json(&arrow_page(None))).unwrap();
        z["version"] = json!(4);
        let p = page_from_json(&z.to_string()).unwrap();
        assert!(page_to_json(&p).contains("\"version\": 3"));
    }

    #[test]
    fn manifest_roundtrip() {
        let m = Manifest::new("Hello");
        assert_eq!(Manifest::from_json(&m.to_json()).unwrap(), m);
    }
}
