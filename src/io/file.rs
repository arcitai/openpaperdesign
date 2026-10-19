//! The `.paper-rs` document format: versioned, pretty-printed JSON.

use crate::geom::Camera;
use crate::model::{Document, Node, NodeId};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FORMAT: &str = "paper-rs";
pub const VERSION: u32 = 1;
pub const EXTENSION: &str = "paper-rs";

#[derive(Serialize, Deserialize)]
struct FileV1 {
    format: String,
    version: u32,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera: Option<Camera>,
    next_id: u64,
    root: Vec<NodeId>,
    nodes: Vec<Node>,
}

#[derive(Debug)]
pub enum FileError {
    Io(std::io::Error),
    Json(serde_json::Error),
    NotPaperRs,
    UnsupportedVersion(u32),
    Invalid(String),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::Io(e) => write!(f, "I/O error: {e}"),
            FileError::Json(e) => write!(f, "malformed file: {e}"),
            FileError::NotPaperRs => write!(f, "not a .paper-rs document"),
            FileError::UnsupportedVersion(v) => {
                write!(
                    f,
                    "file version {v} is newer than this app supports ({VERSION})"
                )
            }
            FileError::Invalid(s) => write!(f, "invalid document: {s}"),
        }
    }
}

impl std::error::Error for FileError {}

/// The single-file text of `doc`, unchecked. Version 1 has no arrow labels: for a labelled
/// document this gives a file this reader refuses (and an older one would read without its
/// labels). Use [`to_string_checked`], which [`save`] does, for anything that leaves the app.
pub fn to_string(doc: &Document, camera: Option<&Camera>) -> String {
    let mut nodes: Vec<Node> = doc.nodes.values().cloned().collect();
    nodes.sort_by_key(|n| n.id);
    let f = FileV1 {
        format: FORMAT.into(),
        version: VERSION,
        name: doc.name.clone(),
        camera: camera.copied(),
        next_id: doc.next_id,
        root: doc.root.clone(),
        nodes,
    };
    serde_json::to_string_pretty(&f).expect("document serializes")
}

pub fn from_str(s: &str) -> Result<(Document, Option<Camera>), FileError> {
    use crate::project::format::{FormatError, page_from_json};
    match page_from_json(s) {
        Ok(p) => Ok((p.doc, p.camera)),
        Err(FormatError::WrongFormat(_)) => Err(FileError::NotPaperRs),
        Err(FormatError::TooNew { version, .. }) => Err(FileError::UnsupportedVersion(version)),
        Err(FormatError::Json(e)) => Err(FileError::Json(e)),
        Err(FormatError::Invalid(e)) => Err(FileError::Invalid(e)),
        Err(FormatError::Io(e)) => Err(FileError::Io(e)),
    }
}

/// [`to_string`], refusing a document with an arrow label: version 1 has no labels, so the
/// file would lose them in an older reader and be refused by this one.
pub fn to_string_checked(doc: &Document, camera: Option<&Camera>) -> Result<String, FileError> {
    if doc.nodes.values().any(|n| n.kind.label().is_some()) {
        return Err(FileError::Invalid(
            "arrow labels can't be saved in a single .paper-rs file; keep the canvas in a \
             project instead. Nothing was written."
                .into(),
        ));
    }
    Ok(to_string(doc, camera))
}

/// Write atomically (temp file + rename) so a crash never leaves a truncated document.
///
/// This single-file format is version 1, which has no arrow labels: a document with a label is
/// refused (nothing is written) rather than saved where a reader would read past its labels or
/// refuse them. Projects keep labels (page version 4).
pub fn save(path: &Path, doc: &Document, camera: Option<&Camera>) -> Result<(), FileError> {
    let text = to_string_checked(doc, camera)?;
    let tmp = path.with_extension("paper-rs.tmp");
    std::fs::write(&tmp, text).map_err(FileError::Io)?;
    std::fs::rename(&tmp, path).map_err(FileError::Io)
}

pub fn load(path: &Path) -> Result<(Document, Option<Camera>), FileError> {
    let s = std::fs::read_to_string(path).map_err(FileError::Io)?;
    from_str(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn roundtrip_demo_document() {
        let ed = demo::showcase();
        let cam = Camera {
            pan: crate::geom::v2(3.0, 4.0),
            zoom: 0.5,
        };
        let s = to_string(&ed.doc, Some(&cam));
        let (doc, cam2) = from_str(&s).unwrap();
        assert_eq!(doc, ed.doc);
        assert_eq!(cam2, Some(cam));
        // Serialization is deterministic.
        assert_eq!(to_string(&doc, Some(&cam)), s);
    }

    #[test]
    fn rejects_foreign_and_future_files() {
        assert!(matches!(
            from_str("{\"hello\":1}"),
            Err(FileError::NotPaperRs)
        ));
        assert!(matches!(
            from_str("{\"format\":\"x\",\"version\":1}"),
            Err(FileError::NotPaperRs)
        ));
        assert!(matches!(
            from_str("{\"format\":\"paper-rs\",\"version\":99}"),
            Err(FileError::UnsupportedVersion(99))
        ));
        let broken =
            r#"{"format":"paper-rs","version":1,"name":"x","next_id":1,"root":[5],"nodes":[]}"#;
        assert!(matches!(from_str(broken), Err(FileError::Invalid(_))));
    }

    #[test]
    fn save_and_load_file() {
        let dir = std::env::temp_dir().join(format!("paper-rs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.paper-rs");
        let ed = demo::showcase();
        save(&path, &ed.doc, None).unwrap();
        let (doc, cam) = load(&path).unwrap();
        assert_eq!(doc, ed.doc);
        assert!(cam.is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
