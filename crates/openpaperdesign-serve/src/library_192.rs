//! The app library (`openpaperdesign-mcp --app-library <dir>`): canvases OpenPaperDesign itself
//! owns, kept in a data folder the host configures for the plugin (Codex's `${PLUGIN_DATA}`).
//!
//! Layout, created by this module only and never named by tool input:
//!
//! ```text
//! <root>/library/v1/canvases/<id>/    one standard paper-rs project per canvas: the live copy
//! <root>/library/v1/locks/<id>.lock   its lease file: permanent, never removed or truncated
//! <root>/library/v1/locks/<id>.editor the binding (and view) that has its editing, if any
//! <root>/library/v1/locks/<id>.handover   the views' handover: offers waiting, views waited for
//! <root>/library/v1/locks/<id>.offer-<o>  one offer's canvas (named by the handover record)
//! <root>/library/v1/appearance        the views' appearance preference (system/light/dark)
//! ```
//!
//! `<id>` is 16 lowercase hex digits made by the server. A canvas's name is data in its
//! `project.json`, never part of a path. Nothing here reads a workspace, a folder named by a tool,
//! the process working directory or any Codex state. `$HOME` and `$PLUGIN_ROOT` are read only to
//! refuse them (or anything inside the package) as the root.
//!
//! Ownership across processes: every Codex chat or panel may run its own `openpaperdesign-mcp`
//! on the same root, and since issue #23 several of them may have the same canvas open at once
//! (one view per process; the canvas stays one set of files, never copied). The exclusive OS
//! lock on its lease file (`File::try_lock`) is held only around what must not interleave with
//! another process: creating or opening it (taken before upstream `Project::open`, whose
//! stale-temp cleanup deletes every matching temporary file, and kept until this binding has
//! claimed its editing), each write (the revision check against the files on disk, the commit
//! and its rollback: `Editing::admit`), a canvas snapshot read (shared), a rename or export of
//! a canvas that isn't open here, and a binding's claim and release of the editing. Each wait
//! for it is bounded (`LEASE_TRIES` × `LEASE_PAUSE`, one second): longer means another process
//! is in a long
//! write, keeps it after a write it couldn't undo (as this one does, see `Broken`), or runs an
//! older OpenPaperDesign that holds it for as long as it has the canvas open; that is refused
//! as busy, with nothing changed. The lock is advisory: it coordinates OpenPaperDesign
//! processes and nothing else (not a native paper-rs app, not other programs). Lease files stay
//! forever, so every process locks the same file.
//!
//! Editing (#23, the owner's "the newest view edits"): exactly one view at a time may write a
//! canvas, the one named in its `<id>.editor` record: `<token>:<view>`, a binding's random
//! editing identity (binding ids are only unique per process) and one of its views (a panel
//! mount `m<n>`, a browser tab `t<n>`), or `<token>` alone (a binding opened, none of its views
//! yet). Opening, creating or importing a canvas claims its editing for the new binding, and
//! every new view (a mount, a tab) claims it for itself, whoever had it, without asking or
//! finding the view that had it: a view hidden in another chat or tab, closed, disconnected or
//! ended never blocks a new one. Every write re-reads the record under the lease, so a view that
//! lost the editing (an older mount or tab of the same binding too, a delayed save or a
//! reconnected tab through it) is refused before anything is read or written. Only the chat's
//! agent (its MCP binding, never a browser route) acts for its binding without a view: it may
//! write while any view of that binding has the editing, and is refused once another binding
//! has it. Every browser request that changes a library canvas names an issued, live view
//! (`Server::tab_of`: a missing, malformed, never issued or released one is refused). A view
//! that lost the editing keeps its unsaved work on screen (`crates/openpaperdesign-web/static/
//! session.js`) and answers the handover (`Editing::answer`): the view that took the editing
//! records the one it took it from as pending, until that one answers (with what it has: its
//! whole canvas, whether it is a change, and its unfinished edits as continuations; or with
//! nothing) or ends. The editing view is told and acts on each offer by its id, under the same
//! admission as a save: apply it (a save on the revision it was made on, so never over anything
//! saved since), keep it as a new canvas (a conflicting one is recovered whole, never merged),
//! take up its unfinished edits, dismiss it, or stop waiting for a view. Offers stay if the
//! view that made them goes away. Every record change (`replace_record`) is a checked
//! transaction: a failure puts the record back, and one that can't be put back fences the
//! server (`Broken`), which keeps the lease's exclusive lock with the lease for good
//! (`Lease::poison`: no later read, claim or cleanup lets it go). A view can also take the
//! editing back explicitly ("Edit here", or opening the canvas again); its next save is then
//! revision-checked against whatever was saved meanwhile, so nothing is overwritten. Reads never
//! need the editing: every open view keeps showing what is saved. A binding's close or end clears
//! the record only if it still names that binding (a view that isn't editing never takes the
//! editing from the one that is), and marks its views still waited for as ended. A process that
//! ends without closing leaves its record, which names no live binding: the canvas then has no
//! editor until a view takes it, and nothing waits for that process (its pending entry says it
//! hasn't answered; the editing view may stop waiting). What only lived in an ended process's
//! memory and was never handed over (an unsaved edit in a view whose host went away first) is
//! not on disk and can't be recovered from here.
//!
//! Identity: the root, the `library/v1/{canvases,locks}` folders and the held lease file are
//! pinned (device, inode) when first seen and checked again on every library action and, through
//! the server's guard, on every browser and MCP access to the open canvas. A changed identity
//! fences that canvas for good: every later read and write through it is refused. The check and
//! the file operation after it can still race, as for project roots (see `lib.rs`).

use crate::{Kind, dev_ino, plain};
use paper_rs::project::{ASSETS_DIR, MANIFEST, Manifest, PAGES_DIR, PageData};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[path = "library_payload.rs"]
pub(crate) mod payload;

const LIBRARY_DIR: &str = "library";
const VERSION_DIR: &str = "v1";
const CANVASES_DIR: &str = "canvases";
const LOCKS_DIR: &str = "locks";
/// Directory entries one listing looks at, and canvases it returns.
const MAX_SCAN: usize = 1000;
const MAX_LIST: usize = 200;
/// Bounds checked before a canvas is listed, opened or exported.
pub(crate) const MAX_MANIFEST_BYTES: u64 = 1 << 20;
pub(crate) const MAX_PAGES: usize = 200;
pub(crate) const MAX_ASSETS: usize = 256;
pub(crate) const MAX_ASSET_BYTES: u64 = 16 << 20;
/// All of one canvas's files together.
pub(crate) const MAX_DOCUMENT_BYTES: u64 = 256 << 20;
/// A new canvas's name, in characters.
pub(crate) const MAX_NAME_CHARS: usize = 60;
/// The longest editing record read (an editing identity is 64 hex digits, and a view).
const MAX_EDITOR_BYTES: u64 = 256;
/// The largest record in the locks folder (an editing or handover record, see
/// `Editing::answer`: offers' metadata and continuations, never their canvases).
const MAX_DRAFT_BYTES: u64 = 16 << 20;
/// The largest offer's canvas (a canvas save, as a save is bounded).
const MAX_OFFER_BYTES: u64 = crate::MAX_PAGE_BYTES as u64;
/// How long an open, a write, a claim or a release waits for another process's hold of a
/// canvas's lease (see the module docs): `LEASE_TRIES` × `LEASE_PAUSE`, one second.
const LEASE_TRIES: u32 = 100;
const LEASE_PAUSE: std::time::Duration = std::time::Duration::from_millis(10);
/// The views' kept appearance preference (`<root>/library/v1/appearance`, see
/// [`Library::appearance`]): one of [`APPEARANCES`], nothing else.
const APPEARANCE_FILE: &str = "appearance";
pub(crate) const APPEARANCES: [&str; 3] = ["system", "light", "dark"];
/// The views' other kept view preferences (`<root>/library/v1/view`, see [`Library::view`]):
/// one small JSON record with two keys, the home's file layout and the editor's canvas
/// background. Its own file, so the appearance's stays exactly as it is.
const VIEW_FILE: &str = "view";
const MAX_VIEW_BYTES: u64 = 256;
/// The lock every OpenPaperDesign process holds while it keeps a view preference (see
/// [`Library::set_view`]): an OS file lock on this file beside the record, created once and
/// never removed, truncated or replaced, so every process locks the same file. No canvas,
/// lease or binding is involved.
const VIEW_LOCK_FILE: &str = "view.lock";
/// A keep waits at most `VIEW_LOCK_TRIES` × `VIEW_LOCK_PAUSE` (one second) for another
/// process's keep to finish; after that it is refused with nothing written.
const VIEW_LOCK_TRIES: u32 = 100;
const VIEW_LOCK_PAUSE: std::time::Duration = std::time::Duration::from_millis(10);
const VIEW_BUSY: &str = "Another OpenPaperDesign window was keeping its view preferences at the \
    same moment, so this one wasn't kept. Nothing was changed; choose it again to keep it.";
pub(crate) const HOME_LAYOUTS: [&str; 2] = ["grid", "list"];
pub(crate) const BACKGROUNDS: [&str; 3] = ["none", "dots", "lines"];

/// The kept view preferences (DESIGN.md): the home lists files as a grid (the fresh default)
/// or a list, and the editor's canvas has no background pattern (the fresh default), dots or
/// lines. View preferences only: never part of a canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewPrefs {
    pub home_layout: &'static str,
    pub background: &'static str,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        ViewPrefs {
            home_layout: HOME_LAYOUTS[0],
            background: BACKGROUNDS[0],
        }
    }
}

impl ViewPrefs {
    pub fn to_json(self) -> Value {
        json!({"home_layout": self.home_layout, "background": self.background})
    }
}

/// A canvas id: exactly 16 lowercase hex digits (made by [`new_id`]).
pub(crate) fn valid_id(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A fresh random canvas id.
pub(crate) fn new_id() -> Result<String, String> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| format!("no random id available: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Sync a directory, so the entries created or removed in it are durable. Unlike upstream
/// `atomic::write`, a failure is an error.
pub(crate) fn sync_dir(dir: &Path) -> Result<(), String> {
    #[cfg(test)]
    if crate::test_support::fault(dir, "sync_dir") {
        return Err(format!("{}: injected sync failure", dir.display()));
    }
    File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("syncing {}: {e}", dir.display()))
}

fn real_dir(path: &Path) -> Result<std::fs::Metadata, String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if m.file_type().is_symlink() || !m.is_dir() {
        return Err(format!("{} is not a real folder", path.display()));
    }
    Ok(m)
}

fn gone(p: &Path) -> bool {
    matches!(std::fs::symlink_metadata(p), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// The configured library root, checked once at launch and pinned.
pub struct Library {
    root: PathBuf,
    root_id: (u64, u64),
    /// (device, inode) of each folder of the hierarchy once it was seen (it may not exist yet).
    seen: Mutex<HashMap<PathBuf, (u64, u64)>>,
    /// Writes this process makes to a canvas that isn't open in a binding (Rename), fenced as
    /// every server's writes are (`crate::Broken`, `crate::commit_fenced`): once such a write
    /// couldn't be undone, the canvas's entry names the files and every later rename, open or
    /// export of it here is refused. In memory, for this process, like a server's.
    broken: Mutex<HashMap<String, crate::Broken>>,
    /// The leases of those canvases, kept until this process ends, so no other OpenPaperDesign
    /// process (or a later binding here) opens and writes them meanwhile.
    kept: Mutex<Vec<Lease>>,
}

impl Library {
    /// Check the configured root: an existing real folder given as a plain absolute path,
    /// never the filesystem root, the home folder, the installed plugin package (`$PLUGIN_ROOT`)
    /// or a package cache or fallback data folder (none of those is durable app storage).
    /// Nothing is created.
    pub fn open(raw: &str) -> Result<Library, String> {
        let refuse = |why: &str| Err(format!("app library folder refused: {why}"));
        if raw.contains("${") {
            return refuse("the host didn't expand its placeholder (the value is still ${...})");
        }
        let trimmed = raw.trim_end_matches('/');
        if raw.starts_with('/') && trimmed.is_empty() {
            return refuse("it is the filesystem root");
        }
        if !trimmed.starts_with('/')
            || trimmed.starts_with("//")
            || trimmed.contains('\\')
            || trimmed.chars().any(char::is_control)
            || trimmed
                .split('/')
                .skip(1)
                .any(|p| p.is_empty() || p == "." || p == "..")
        {
            return refuse("it must be a plain absolute path (no ., .. or empty parts)");
        }
        let given = PathBuf::from(trimmed);
        if given.parent().is_none() {
            return refuse("it is the filesystem root");
        }
        let m = std::fs::symlink_metadata(&given)
            .map_err(|e| format!("app library folder {}: {e}", given.display()))?;
        if m.file_type().is_symlink() || !m.is_dir() {
            return refuse("it must be an existing real folder, not a link or a file");
        }
        let root = given
            .canonicalize()
            .map_err(|e| format!("app library folder {}: {e}", given.display()))?;
        if root.parent().is_none() {
            return refuse("it is the filesystem root");
        }
        let canonical =
            |v: Option<std::ffi::OsString>| v.and_then(|p| PathBuf::from(p).canonicalize().ok());
        if canonical(std::env::var_os("HOME")).as_deref() == Some(root.as_path()) {
            return refuse("it is your home folder");
        }
        if let Some(package) = canonical(std::env::var_os("PLUGIN_ROOT"))
            && root.starts_with(&package)
        {
            return refuse(
                "it is inside the installed plugin package (PLUGIN_ROOT), which is replaced on \
                 updates and is not durable storage",
            );
        }
        let names: Vec<String> = root
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if names.iter().any(|n| n == ".plugin-data") {
            return refuse(
                "it is a plugin package's fallback data folder (.plugin-data), which is not durable storage",
            );
        }
        if names
            .windows(2)
            .any(|w| w[0] == "plugins" && w[1] == "cache")
        {
            return refuse(
                "it is inside the plugin package cache (plugins/cache), which is not durable storage",
            );
        }
        let root_id = dev_ino(&real_dir(&root)?);
        Ok(Library {
            root,
            root_id,
            seen: Mutex::default(),
            broken: Mutex::default(),
            kept: Mutex::default(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn version_dir(&self) -> PathBuf {
        self.root.join(LIBRARY_DIR).join(VERSION_DIR)
    }

    pub(crate) fn canvases(&self) -> PathBuf {
        self.version_dir().join(CANVASES_DIR)
    }

    fn locks(&self) -> PathBuf {
        self.version_dir().join(LOCKS_DIR)
    }

    /// The folders of the hierarchy, shallowest first.
    fn hierarchy(&self) -> [PathBuf; 4] {
        [
            self.root.join(LIBRARY_DIR),
            self.version_dir(),
            self.canvases(),
            self.locks(),
        ]
    }

    /// A canvas's folder. `id` must be [`valid_id`].
    pub(crate) fn document_dir(&self, id: &str) -> PathBuf {
        self.canvases().join(id)
    }

    fn lock_path(&self, id: &str) -> PathBuf {
        self.locks().join(format!("{id}.lock"))
    }

    /// A canvas's editing record (see the module docs, "Editing").
    fn editor_path(&self, id: &str) -> PathBuf {
        self.locks().join(format!("{id}.editor"))
    }

    /// The editing identity a canvas's record names, if any. Read under its lease, or without
    /// it as a hint (the record is replaced whole, by rename, so it is never read half-written).
    /// A record that isn't a plain file is an error; one with unexpected content names an editor
    /// that is no binding's (nobody writes until one claims it).
    pub(crate) fn editor_of(&self, id: &str) -> Result<Option<String>, String> {
        let path = self.editor_path(id);
        if !plain(&path, Kind::File).map_err(|e| format!("this canvas's editing record: {e}"))? {
            return Ok(None);
        }
        let mut text = String::new();
        let read = File::open(&path).and_then(|f| {
            use std::io::Read;
            f.take(MAX_EDITOR_BYTES).read_to_string(&mut text)
        });
        match read {
            Ok(_) => Ok(Some(text.trim().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("this canvas's editing record: {e}")),
        }
    }

    /// Name `editor` in a canvas's editing record, or clear it (`None`). Only under its lease's
    /// exclusive lock. The record is written to a file beside it, made durable and renamed over
    /// it, so a reader sees the old record or the new one.
    fn set_editor(&self, id: &str, editor: Option<&str>) -> Result<(), RecordError> {
        self.replace_record(&self.editor_path(id), editor.map(str::as_bytes))
            .map_err(|e| e.context("this canvas's editing record couldn't be written"))
    }

    /// A canvas's handover record (see `Editing::answer`).
    fn handover_path(&self, id: &str) -> PathBuf {
        self.locks().join(format!("{id}.handover"))
    }

    /// A canvas's handover record (bounded; replaced whole by rename, so never read
    /// half-written): empty when there is none. One that isn't a plain file or doesn't parse
    /// is an error, so it is never overwritten unread.
    fn handover_of(&self, id: &str) -> Result<Handover, String> {
        let path = self.handover_path(id);
        match self.record_bytes(&path) {
            Ok(None) => Ok(Handover::default()),
            Ok(Some(b)) => std::str::from_utf8(&b)
                .ok()
                .and_then(Handover::from_json)
                .ok_or_else(|| "this canvas's handover record is unreadable".to_string()),
            Err(e) => Err(format!("this canvas's handover record: {e}")),
        }
    }

    /// Replace a canvas's handover record (removed when empty), only under its lease's
    /// exclusive lock, as a checked record change.
    fn set_handover(&self, id: &str, h: &Handover) -> Result<(), RecordError> {
        let text = (!h.offers.is_empty() || !h.pending.is_empty()).then(|| h.to_json());
        if text
            .as_ref()
            .is_some_and(|t| t.len() as u64 > MAX_DRAFT_BYTES)
        {
            return Err(RecordError::Failed(
                "this change is too large to hand over; it stays in its view".into(),
            ));
        }
        self.replace_record(&self.handover_path(id), text.as_deref().map(str::as_bytes))
            .map_err(|e| e.context("this canvas's handover record couldn't be written"))
    }

    /// Offer `offer`'s canvas file beside the handover record.
    fn offer_path(&self, id: &str, offer: &str) -> PathBuf {
        self.locks().join(format!("{id}.offer-{offer}"))
    }

    /// Write a new offer's canvas file (a new name each time, made durable with its folder
    /// entry) before the record names it.
    fn write_offer(&self, id: &str, offer: &str, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() as u64 > MAX_OFFER_BYTES {
            return Err("this change is too large to hand over; it stays in its view".into());
        }
        let path = self.offer_path(id, offer);
        plain(&path, Kind::File)?;
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            f.write_all(bytes)?;
            f.sync_all()
        };
        write()
            .map_err(|e| {
                let _ = std::fs::remove_file(&path);
                format!("the change couldn't be handed over ({e}); it stays in its view")
            })
            .and_then(|()| sync_dir(&self.locks()))
    }

    /// An offer's canvas, bounded, never through a link.
    fn read_offer(&self, id: &str, offer: &str) -> Result<String, String> {
        let path = self.offer_path(id, offer);
        if !plain(&path, Kind::File)? {
            return Err("that handed-over change's content is missing".into());
        }
        let mut text = String::new();
        File::open(&path)
            .and_then(|f| {
                use std::io::Read;
                f.take(MAX_OFFER_BYTES + 1).read_to_string(&mut text)
            })
            .map_err(|e| format!("that handed-over change couldn't be read ({e})"))?;
        if text.len() as u64 > MAX_OFFER_BYTES {
            return Err("that handed-over change is larger than a change can be".into());
        }
        Ok(text)
    }

    /// Remove an offer's canvas file once the record no longer names it (best effort: one left
    /// behind is never read).
    fn remove_offer(&self, id: &str, offer: &str) {
        let path = self.offer_path(id, offer);
        if matches!(plain(&path, Kind::File), Ok(true)) && std::fs::remove_file(&path).is_ok() {
            let _ = sync_dir(&self.locks());
        }
    }

    /// Replace `path` (a record in the locks folder) whole with `bytes`, or remove it (`None`),
    /// as one checked transaction (R78-06): its bytes before are read first (never through a
    /// link, bounded); the new ones are written to `<path>.next`, made durable and renamed over
    /// it (or it is removed), and the folder is synced. If that sync (or the rename) fails, the
    /// bytes before are put back the same way and the folder synced again: the error then
    /// means nothing changed (`RecordError::Failed`). If putting them back fails too, the
    /// record may be either: `RecordError::Uncertain`, naming it, which the caller turns into
    /// its server's `Broken` fence.
    fn replace_record(&self, path: &Path, bytes: Option<&[u8]>) -> Result<(), RecordError> {
        let failed = |e: String| RecordError::Failed(e);
        let before = self.record_bytes(path).map_err(failed)?;
        if before.as_deref() == bytes {
            return Ok(());
        }
        let done = self.put_record(path, bytes).and_then(|()| {
            #[cfg(test)]
            if crate::test_support::fault(&self.locks(), "record_sync") {
                return Err(format!("{}: injected sync failure", self.locks().display()));
            }
            sync_dir(&self.locks())
        });
        let Err(e) = done else {
            return Ok(());
        };
        // Put the record back as it was.
        let restored = (|| {
            #[cfg(test)]
            if crate::test_support::fault(&self.locks(), "record_restore") {
                return Err("injected restore failure".to_string());
            }
            self.put_record(path, before.as_deref())?;
            sync_dir(&self.locks())
        })();
        match restored {
            Ok(()) => Err(RecordError::Failed(format!(
                "{e}; it was put back as it was"
            ))),
            Err(r) => Err(RecordError::Uncertain {
                error: format!("{e}; putting it back failed too ({r})"),
                file: self.label(path),
            }),
        }
    }

    /// A record's bytes now (`None`: no record), bounded, never through a link.
    fn record_bytes(&self, path: &Path) -> Result<Option<Vec<u8>>, String> {
        if !plain(path, Kind::File)? {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        let read = File::open(path).and_then(|f| {
            use std::io::Read;
            f.take(MAX_DRAFT_BYTES + 1).read_to_end(&mut bytes)
        });
        match read {
            Ok(_) if bytes.len() as u64 > MAX_DRAFT_BYTES => Err(format!(
                "{} is larger than a record can be",
                self.label(path)
            )),
            Ok(_) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Write `bytes` over `path` (by `<path>.next`, synced and renamed) or remove it, without
    /// the folder sync. Never through a link.
    fn put_record(&self, path: &Path, bytes: Option<&[u8]>) -> Result<(), String> {
        plain(path, Kind::File)?;
        match bytes {
            None => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            },
            Some(bytes) => {
                let mut next = path.as_os_str().to_owned();
                next.push(".next");
                let next = PathBuf::from(next);
                plain(&next, Kind::File)?;
                let write = || -> std::io::Result<()> {
                    use std::io::Write;
                    let mut f = std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&next)?;
                    f.write_all(bytes)?;
                    f.sync_all()?;
                    std::fs::rename(&next, path)
                };
                write().map_err(|e| {
                    let _ = std::fs::remove_file(&next);
                    e.to_string()
                })
            }
        }
    }

    fn label(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .map(|r| r.display().to_string())
            .unwrap_or_else(|_| p.display().to_string())
    }

    /// The root is still the folder pinned at launch, and every hierarchy folder that exists
    /// is a real folder on its file system with the identity it had when first seen.
    pub fn check(&self) -> Result<(), String> {
        let m = std::fs::symlink_metadata(&self.root)
            .map_err(|e| format!("the app library folder: {e}"))?;
        if m.file_type().is_symlink()
            || !m.is_dir()
            || dev_ino(&m) != self.root_id
            || self.root.canonicalize().ok().as_deref() != Some(self.root.as_path())
        {
            return Err(
                "the app library folder was moved or replaced since this server started".into(),
            );
        }
        let mut seen = self.seen.lock().unwrap();
        for d in self.hierarchy() {
            match std::fs::symlink_metadata(&d) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if seen.contains_key(&d) {
                        return Err(format!("{} was removed or replaced", self.label(&d)));
                    }
                }
                Err(e) => return Err(format!("{}: {e}", self.label(&d))),
                Ok(m) => {
                    if m.file_type().is_symlink() || !m.is_dir() || dev_ino(&m).0 != self.root_id.0
                    {
                        return Err(format!(
                            "{} is not a real folder on the library's file system",
                            self.label(&d)
                        ));
                    }
                    let id = dev_ino(&m);
                    match seen.get(&d) {
                        Some(old) if *old != id => {
                            return Err(format!("{} was replaced", self.label(&d)));
                        }
                        Some(_) => {}
                        None => {
                            seen.insert(d.clone(), id);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Create the missing hierarchy folders (never through a link) and make every folder entry
    /// of the hierarchy durable. Shared folders are never removed again.
    fn ensure_hierarchy(&self) -> Result<(), String> {
        self.check()?;
        for d in self.hierarchy() {
            // `create_dir` never follows a final link; whatever is there must be a real folder.
            let made = std::fs::create_dir(&d);
            if real_dir(&d).is_err() {
                return Err(match made {
                    Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
                        format!("{}: {e}", self.label(&d))
                    }
                    _ => format!("{} is not a real folder", self.label(&d)),
                });
            }
        }
        // Each parent is synced every time, so an entry whose earlier sync failed is made
        // durable now.
        for parent in [
            self.root.clone(),
            self.root.join(LIBRARY_DIR),
            self.version_dir(),
        ] {
            sync_dir(&parent).map_err(|e| {
                format!(
                    "the library folder couldn't be made durable ({e}); nothing else was written"
                )
            })?;
        }
        self.check()
    }

    /// The canvases in the library, read-only and bounded: nothing is created, opened, locked
    /// or written. `open` is the canvas open in this process, if any.
    pub fn list(&self, open: Option<&str>) -> Result<Value, String> {
        self.check()?;
        let canvases = self.canvases();
        let (mut out, mut skipped, mut complete) = (Vec::new(), Vec::new(), true);
        if plain(&canvases, Kind::Dir)? {
            let rd = std::fs::read_dir(&canvases).map_err(|e| format!("library canvases: {e}"))?;
            for (n, entry) in rd.enumerate() {
                if n >= MAX_SCAN || out.len() >= MAX_LIST {
                    complete = false;
                    break;
                }
                let entry = entry.map_err(|e| format!("library canvases: {e}"))?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    skipped.push(json!({"id": null, "problem": "an entry that isn't a canvas id"}));
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                if !valid_id(name) {
                    skipped.push(json!({"id": null, "problem": "an entry that isn't a canvas id"}));
                    continue;
                }
                match self.read_document(name) {
                    Ok(m) => out.push(json!({
                        "id": name,
                        "name": m.name,
                        "pages": m.pages.len(),
                        "created": m.created,
                        "modified": m.modified,
                        "open": open == Some(name),
                    })),
                    Err(e) => skipped.push(json!({"id": name, "problem": e})),
                }
            }
        }
        out.sort_by(|a, b| {
            b["modified"]
                .as_u64()
                .cmp(&a["modified"].as_u64())
                .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
        });
        Ok(json!({
            "storage": "app-library",
            "library": self.root,
            "location": canvases,
            "complete": complete,
            "canvases": out,
            "skipped": skipped,
            "note": "Read-only: nothing was created, opened or locked. These canvases belong to \
                OpenPaperDesign's own library, not to a project folder.",
        }))
    }

    /// The appearance preference the library keeps for its views: `system`, `light` or `dark`
    /// (DESIGN.md). A view preference, never part of a canvas. Read-only and bounded: a missing,
    /// unreadable, linked, oversized or unknown value is `system`, and nothing is created.
    pub fn appearance(&self) -> &'static str {
        if self.check().is_err() {
            return "system";
        }
        let path = self.version_dir().join(APPEARANCE_FILE);
        // `symlink_metadata`: a link is not a plain file, and is never followed.
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() && m.len() <= 16 => {}
            _ => return "system",
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            return "system";
        };
        APPEARANCES
            .into_iter()
            .find(|a| *a == text.trim())
            .unwrap_or("system")
    }

    /// Keep `value` (`system`, `light` or `dark`) as the appearance preference: written to a new
    /// file beside it, made durable and renamed over the old one, so a reader sees the old or the
    /// new value, never part of one. Touches no canvas, lease or binding.
    pub fn set_appearance(&self, value: &str) -> Result<&'static str, String> {
        let v = APPEARANCES
            .into_iter()
            .find(|a| *a == value)
            .ok_or("An appearance of system, light or dark is required. Nothing was changed.")?;
        self.ensure_hierarchy()?;
        let dir = self.version_dir();
        let path = dir.join(APPEARANCE_FILE);
        if let Ok(m) = std::fs::symlink_metadata(&path)
            && !m.is_file()
        {
            return Err(
                "The kept appearance isn't a plain file, so it wasn't replaced. Nothing \
                was changed."
                    .into(),
            );
        }
        let tmp = dir.join(format!(".{APPEARANCE_FILE}.{}.tmp", new_id()?));
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            f.write_all(v.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, &path)
        };
        if let Err(e) = write() {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!(
                "The appearance couldn't be kept ({e}). Nothing else was changed."
            ));
        }
        sync_dir(&dir)?;
        Ok(v)
    }

    /// The view preferences the library keeps for its views (see [`ViewPrefs`]). Read-only and
    /// bounded: a missing, unreadable, linked, oversized or malformed record is the defaults,
    /// and a missing or unknown key is that key's default. Nothing is created.
    pub fn view(&self) -> ViewPrefs {
        self.read_view().unwrap_or_default()
    }

    /// [`Library::view`], except that a library whose pinned root or hierarchy no longer checks
    /// out is that error rather than the defaults: a keep must not take those defaults for the
    /// kept record. A missing, unreadable, linked, oversized or malformed record is still the
    /// defaults.
    fn read_view(&self) -> Result<ViewPrefs, String> {
        let mut out = ViewPrefs::default();
        self.check()?;
        let path = self.version_dir().join(VIEW_FILE);
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() && m.len() <= MAX_VIEW_BYTES => {}
            _ => return Ok(out),
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(out);
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return Ok(out);
        };
        let pick = |key: &str, from: &[&'static str]| {
            let got = v.get(key).and_then(Value::as_str)?;
            from.iter().copied().find(|c| *c == got)
        };
        if let Some(l) = pick("home_layout", &HOME_LAYOUTS) {
            out.home_layout = l;
        }
        if let Some(b) = pick("background", &BACKGROUNDS) {
            out.background = b;
        }
        Ok(out)
    }

    /// Keep `home_layout` and/or `background` (each one of its known values; at least one),
    /// with the other key as it is kept now: the whole record is written to a new file beside
    /// it, made durable and renamed over the old one, so a reader sees the old or the new
    /// record, never part of one. Touches no canvas, lease or binding.
    ///
    /// Keeps from different processes are serialized by the view lock (`VIEW_LOCK_FILE`): the
    /// record is read and replaced while it is held, so a keep of one key never undoes another
    /// process's acknowledged keep of the other. A keep that can't take the lock within its
    /// bound is refused with nothing written (never acknowledged), as is one whose library root
    /// or hierarchy no longer checks out once it holds the lock. Readers take no lock: they
    /// see a whole record. Blocking (the bounded wait): call it off the async executor.
    pub fn set_view(
        &self,
        home_layout: Option<&str>,
        background: Option<&str>,
    ) -> Result<ViewPrefs, String> {
        let known = |v: Option<&str>, from: &[&'static str], what: &str| match v {
            None => Ok(None),
            Some(v) => from
                .iter()
                .copied()
                .find(|c| *c == v)
                .map(Some)
                .ok_or_else(|| {
                    format!(
                        "A {what} of {} is required. Nothing was changed.",
                        from.join(", ")
                    )
                }),
        };
        let layout = known(home_layout, &HOME_LAYOUTS, "home layout")?;
        let pattern = known(background, &BACKGROUNDS, "background")?;
        if layout.is_none() && pattern.is_none() {
            return Err(
                "A home_layout or a background to keep is required. Nothing was changed.".into(),
            );
        }
        self.ensure_hierarchy()?;
        // Held until this keep returns (dropping the file releases the lock).
        let _held = self.lock_view()?;
        // Read under the lock: the record as the last finished keep, of any process, left it.
        // The library is checked again first, since it may have been moved or replaced while
        // this keep waited: then nothing is kept.
        let mut next = self.read_view().map_err(|e| {
            format!("The view preferences weren't kept ({e}). Nothing was changed.")
        })?;
        if let Some(l) = layout {
            next.home_layout = l;
        }
        if let Some(b) = pattern {
            next.background = b;
        }
        let dir = self.version_dir();
        let path = dir.join(VIEW_FILE);
        if let Ok(m) = std::fs::symlink_metadata(&path)
            && !m.is_file()
        {
            return Err(
                "The kept view preferences aren't a plain file, so they weren't replaced. \
                 Nothing was changed."
                    .into(),
            );
        }
        let tmp = dir.join(format!(".{VIEW_FILE}.{}.tmp", new_id()?));
        let bytes = next.to_json().to_string();
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            f.write_all(bytes.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, &path)
        };
        if let Err(e) = write() {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!(
                "The view preferences couldn't be kept ({e}). Nothing else was changed."
            ));
        }
        sync_dir(&dir)?;
        Ok(next)
    }

    /// Take the view lock (see `VIEW_LOCK_FILE`), waiting a bounded time for another process's
    /// keep. As for a canvas's lease file, the opened file must be the path's own regular,
    /// single-link file, before and after locking.
    fn lock_view(&self) -> Result<File, String> {
        let dir = self.version_dir();
        let path = dir.join(VIEW_LOCK_FILE);
        let unusable = |e: String| {
            format!("The view preferences' lock file is unusable ({e}). Nothing was changed.")
        };
        let existed = plain(&path, Kind::File).map_err(unusable)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| unusable(e.to_string()))?;
        if !existed {
            sync_dir(&dir)?;
        }
        let mut tries = 0;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if tries < VIEW_LOCK_TRIES => {
                    tries += 1;
                    std::thread::sleep(VIEW_LOCK_PAUSE);
                }
                Err(std::fs::TryLockError::WouldBlock) => return Err(VIEW_BUSY.into()),
                Err(std::fs::TryLockError::Error(e)) => return Err(unusable(e.to_string())),
            }
        }
        let handle = file.metadata().map_err(|e| unusable(e.to_string()))?;
        let m = std::fs::symlink_metadata(&path).map_err(|e| unusable(e.to_string()))?;
        if m.file_type().is_symlink()
            || !m.is_file()
            || !handle.is_file()
            || crate::links(&handle) != 1
            || dev_ino(&m) != dev_ino(&handle)
        {
            return Err(unusable("it was replaced or linked".into()));
        }
        Ok(file)
    }

    /// A canvas's manifest, read only from a real folder on the library's file system with a
    /// real, bounded `project.json` that passes the binding checks, lists at least one page and
    /// whose active page (if any) is one of them. Takes no lease: nothing is written.
    pub(crate) fn read_document(&self, id: &str) -> Result<Manifest, String> {
        if !valid_id(id) {
            return Err("not a canvas id".into());
        }
        let dir = self.document_dir(id);
        let m = std::fs::symlink_metadata(&dir).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "there is no such canvas in the library".to_string(),
            _ => format!("canvas folder: {e}"),
        })?;
        if m.file_type().is_symlink() || !m.is_dir() {
            return Err("its folder is not a real folder (links are not followed)".into());
        }
        if dev_ino(&m).0 != self.root_id.0 {
            return Err("its folder is on another file system (not followed)".into());
        }
        crate::check_project_dir(&dir)?;
        let len = std::fs::symlink_metadata(dir.join(MANIFEST))
            .map_err(|e| format!("{MANIFEST}: {e}"))?
            .len();
        if len > MAX_MANIFEST_BYTES {
            return Err(format!(
                "its {MANIFEST} is larger than {MAX_MANIFEST_BYTES} bytes"
            ));
        }
        let manifest = crate::read_manifest_checked(&dir)?;
        if manifest.pages.is_empty() {
            return Err(format!("its {MANIFEST} lists no pages"));
        }
        if manifest.pages.len() > MAX_PAGES {
            return Err(format!("it has more than {MAX_PAGES} pages"));
        }
        if let Some(a) = &manifest.active_page
            && !manifest.pages.iter().any(|p| &p.id == a)
        {
            return Err("its active page isn't one of its pages".into());
        }
        Ok(manifest)
    }

    /// [`Library::read_document`] plus the size bounds of every file the server will load,
    /// checked before it is prepared.
    pub(crate) fn check_document(&self, id: &str) -> Result<Manifest, String> {
        self.refuse_if_broken(id)?;
        let manifest = self.read_document(id)?;
        let dir = self.document_dir(id);
        let size = |p: &Path| {
            std::fs::symlink_metadata(p)
                .map(|m| m.len())
                .map_err(|e| format!("{}: {e}", self.label(p)))
        };
        let mut total = size(&dir.join(MANIFEST))?;
        for p in &manifest.pages {
            let path = dir.join(PAGES_DIR).join(format!("{}.json", p.id));
            if plain(&path, Kind::File)? {
                let n = size(&path)?;
                if n > crate::MAX_PAGE_BYTES as u64 {
                    return Err(format!(
                        "page {} is larger than {} bytes",
                        p.id,
                        crate::MAX_PAGE_BYTES
                    ));
                }
                total += n;
            }
        }
        let assets = dir.join(ASSETS_DIR);
        if plain(&assets, Kind::Dir)? {
            let rd = std::fs::read_dir(&assets).map_err(|e| format!("assets: {e}"))?;
            for (n, e) in rd.enumerate() {
                if n >= MAX_ASSETS {
                    return Err(format!(
                        "its assets folder has more than {MAX_ASSETS} entries"
                    ));
                }
                let e = e.map_err(|e| format!("assets: {e}"))?;
                let m = std::fs::symlink_metadata(e.path()).map_err(|e| format!("assets: {e}"))?;
                if m.is_file() {
                    if m.len() > MAX_ASSET_BYTES {
                        return Err(format!(
                            "an image in it is larger than {MAX_ASSET_BYTES} bytes"
                        ));
                    }
                    total += m.len();
                }
            }
        }
        if total > MAX_DOCUMENT_BYTES {
            return Err(format!(
                "its files are larger than {MAX_DOCUMENT_BYTES} bytes together"
            ));
        }
        Ok(manifest)
    }

    /// Write a new canvas for `lease` (held by this process): its folders and files, each
    /// folder entry synced, files through the checked commit. Nothing counts as created before
    /// every write and sync succeeded. On a failure, only what this call made and what is still
    /// exactly that is removed, and anything left or not durably removed is named.
    pub(crate) fn create_document(
        &self,
        lease: &Lease,
        doc: &NewDocument,
    ) -> Result<(PathBuf, Created), String> {
        lease.check(self)?;
        let dir = self.document_dir(&lease.doc);
        let mut made = Created::default();
        match std::fs::create_dir(&dir) {
            Ok(()) => made.dir(&dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err("a canvas with this new id already exists; nothing was created".into());
            }
            Err(e) => return Err(format!("library canvases: {e}; nothing was created")),
        }
        sync_dir(&self.canvases()).map_err(|e| made.fail(&e))?;
        let pages = dir.join(PAGES_DIR);
        std::fs::create_dir(&pages).map_err(|e| made.fail(&format!("pages: {e}")))?;
        made.dir(&pages);
        let assets = dir.join(ASSETS_DIR);
        if !doc.assets.is_empty() {
            std::fs::create_dir(&assets).map_err(|e| made.fail(&format!("assets: {e}")))?;
            made.dir(&assets);
        }
        sync_dir(&dir).map_err(|e| made.fail(&e))?;
        // Verify what was made before writing into it.
        for (d, id) in &made.dirs {
            if real_dir(d).ok().map(|m| dev_ino(&m)) != Some(*id) {
                return Err(made.fail("a new canvas folder was replaced while it was made"));
            }
        }
        if dir.canonicalize().ok().as_deref() != Some(dir.as_path()) {
            return Err(made.fail("the new canvas folder isn't where it was made"));
        }
        let mut writes = Vec::new();
        for (name, bytes) in &doc.assets {
            writes.push(crate::commit::Write {
                path: assets.join(name),
                data: bytes.clone(),
            });
        }
        for p in &doc.pages {
            writes.push(crate::commit::Write {
                path: pages.join(format!("{}.json", p.id)),
                data: paper_rs::project::format::page_to_json(p).into_bytes(),
            });
        }
        writes.push(crate::commit::Write {
            path: dir.join(MANIFEST),
            data: doc.manifest.to_json().into_bytes(),
        });
        if let Err(e) = crate::commit::commit(&writes) {
            // The commit put back (removed) what it wrote, or names what it couldn't.
            return Err(made.fail(&format!("the canvas was {}", e.describe())));
        }
        for w in &writes {
            made.file(&w.path);
        }
        lease.check(self).map_err(|e| made.fail(&e))?;
        Ok((dir, made))
    }
}

impl Library {
    /// Canvas `id`'s write fence in this process (see `broken`).
    fn fence(&self, id: &str) -> crate::Broken {
        self.broken
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_default()
            .clone()
    }

    /// Refused once a write of canvas `id` that isn't open in a binding couldn't be undone:
    /// names the files that may be inconsistent. Every other canvas is unaffected.
    pub(crate) fn refuse_if_broken(&self, id: &str) -> Result<(), String> {
        match self.broken.lock().unwrap().get(id) {
            Some(b) => crate::refuse_if_broken(b)
                .map_err(|e| format!("{e} (this window keeps the canvas locked until then)")),
            None => Ok(()),
        }
    }

    /// Rename canvas `lease.doc` (held by this process, and not open in a binding here): its
    /// manifest's name and modified time, as one checked commit of `project.json` through the
    /// canvas's write fence (`commit_fenced`: all or, after an error, nothing). Its id, pages,
    /// files and location don't change; the name is data, never part of a path. A commit whose
    /// undo failed fences the canvas: this and every later rename, open or export of it here
    /// is refused with the files named, and its lease is kept until this process ends. Returns
    /// the message to show as it is.
    pub(crate) fn rename_document(&self, lease: Lease, name: &str) -> Result<Manifest, String> {
        let not = |e: String| format!("Not renamed: {e}. Nothing was changed.");
        self.refuse_if_broken(&lease.doc).map_err(not)?;
        lease.check(self).map_err(not)?;
        let mut manifest = self.read_document(&lease.doc).map_err(not)?;
        manifest.name = name.to_string();
        manifest.modified = paper_rs::project::now_ms();
        let writes = [crate::commit::Write {
            path: self.document_dir(&lease.doc).join(MANIFEST),
            data: manifest.to_json().into_bytes(),
        }];
        if let Err(e) = crate::commit_fenced(&self.fence(&lease.doc), &writes) {
            if matches!(e, crate::commit::CommitError::Broken { .. }) {
                self.kept.lock().unwrap().push(lease);
            }
            return Err(format!("Not renamed: {}", e.describe()));
        }
        // Committed. The lease is checked once more: if it no longer checks out, the new name
        // was still written (nothing is undone), only this process's exclusive ownership while
        // it wrote can't be confirmed.
        lease.check(self).map_err(|e| {
            format!(
                "The new name was written to this canvas's {MANIFEST}, but this OpenPaperDesign \
                 window couldn't confirm afterwards that it still held the canvas ({e}). Check \
                 the canvas in the library before changing it again."
            )
        })?;
        Ok(manifest)
    }

    /// Canvas `lease.doc` (held by this process, and not open in a binding here) as a portable
    /// canvas file, read under its lease (no OpenPaperDesign process can write it meanwhile).
    pub(crate) fn export_document(&self, lease: &Lease) -> Result<payload::Exported, String> {
        lease.check(self)?;
        self.check_document(&lease.doc)?;
        payload::export(&self.document_dir(&lease.doc))
    }
}

/// The first page's name in a new library file (`NewDocument::blank`).
pub(crate) const FIRST_PAPER: &str = "Paper 1";

/// The content of a new canvas: a checked manifest, its pages (in manifest order) and the
/// image files under `assets/` (file name, bytes).
pub(crate) struct NewDocument {
    pub manifest: Manifest,
    pub pages: Vec<PageData>,
    pub assets: Vec<(String, Vec<u8>)>,
}

impl NewDocument {
    /// An empty canvas with one blank page, as upstream `Store::create` makes it, except that
    /// its page is named [`FIRST_PAPER`] (people see a canvas's pages as Papers). Only a new
    /// file's name: existing files, imports and upstream's own projects keep theirs. No design
    /// system, direction or token is applied.
    pub(crate) fn blank(name: &str) -> NewDocument {
        let mut manifest = Manifest::new(name);
        manifest.pages[0].name = FIRST_PAPER.to_string();
        let page = PageData {
            id: manifest.pages[0].id.clone(),
            name: manifest.pages[0].name.clone(),
            doc: paper_rs::model::Document {
                name: manifest.pages[0].name.clone(),
                ..Default::default()
            },
            camera: None,
        };
        NewDocument {
            manifest,
            pages: vec![page],
            assets: Vec::new(),
        }
    }

    /// A new library identity: a fresh manifest id and timestamps. Page ids, names, order,
    /// content and images are kept.
    pub(crate) fn fresh(mut self) -> NewDocument {
        let now = paper_rs::project::now_ms();
        self.manifest.id = paper_rs::project::new_id();
        self.manifest.created = now;
        self.manifest.modified = now;
        self
    }
}

/// What one create or import made: each folder (identity taken right after `create_dir`) and
/// each file (identity and revision taken after its write).
#[derive(Default)]
pub(crate) struct Created {
    dirs: Vec<(PathBuf, (u64, u64))>,
    files: Vec<(PathBuf, (u64, u64), String)>,
}

impl Created {
    fn dir(&mut self, path: &Path) {
        if let Ok(m) = real_dir(path) {
            self.dirs.push((path.to_path_buf(), dev_ino(&m)));
        }
    }

    fn file(&mut self, path: &Path) {
        if let Ok(m) = std::fs::symlink_metadata(path)
            && m.is_file()
            && let Ok(bytes) = std::fs::read(path)
        {
            self.files
                .push((path.to_path_buf(), dev_ino(&m), crate::etag(&bytes)));
        }
    }

    /// Remove what is still exactly what was made, files first, then folders deepest first,
    /// and sync the folder holding each removed entry. Returns what stays, and removals that may
    /// not be durable. Only NotFound counts as gone.
    pub(crate) fn undo(&self) -> Vec<String> {
        let mut left = Vec::new();
        let mut parents: Vec<PathBuf> = Vec::new();
        for (f, id, rev) in self.files.iter().rev() {
            let ours = std::fs::symlink_metadata(f)
                .ok()
                .filter(|m| m.is_file())
                .map(|m| dev_ino(&m))
                == Some(*id)
                && std::fs::read(f).ok().map(|b| crate::etag(&b)).as_deref() == Some(rev.as_str());
            if ours && std::fs::remove_file(f).is_ok() {
                parents.extend(f.parent().map(Path::to_path_buf));
            } else if !gone(f) {
                left.push(f.display().to_string());
            }
        }
        for (d, id) in self.dirs.iter().rev() {
            let ours = real_dir(d).ok().map(|m| dev_ino(&m)) == Some(*id)
                && d.canonicalize().ok().as_deref() == Some(d.as_path());
            if ours && std::fs::remove_dir(d).is_ok() {
                parents.extend(d.parent().map(Path::to_path_buf));
            } else if !gone(d) {
                left.push(d.display().to_string());
            }
        }
        parents.sort();
        parents.dedup();
        for p in parents.iter().filter(|p| !gone(p)) {
            if let Err(e) = sync_dir(p) {
                left.push(format!(
                    "{} (a removal in it may not be durable: {e})",
                    p.display()
                ));
            }
        }
        left
    }

    /// The error for a failed creation, after undoing it: says whether anything stayed.
    pub(crate) fn fail(&self, error: &str) -> String {
        let left = self.undo();
        if left.is_empty() {
            format!("{error}. Nothing was created.")
        } else {
            format!(
                "{error}. The new canvas could not be fully removed; these remain or may not be \
                 durably removed (not empty, changed, or not known to be this call's): {}",
                left.join(", ")
            )
        }
    }
}

/// A lease on one canvas id: the open lease file. Its exclusive lock is held from `acquire`
/// until `unlock` (an open keeps it until its binding claimed the editing) and again around
/// each write (`Editing::admit`); see the module docs.
pub struct Lease {
    file: File,
    path: PathBuf,
    id: (u64, u64),
    pub(crate) doc: String,
    /// Set once a write under this lease's exclusive lock couldn't be undone (its server is
    /// `Broken`): the exclusive lock is then kept for as long as this handle lives. Nothing
    /// takes or lets go of it again: a later shared read (which on this same handle would
    /// convert the lock) reads under the exclusive lock it already holds, `unlock` does nothing
    /// and every exclusive take is refused (R78-02).
    poisoned: Mutex<Option<String>>,
}

pub(crate) enum LeaseError {
    /// Another OpenPaperDesign process (or another binding in this one) holds it.
    Busy,
    Failed(String),
}

impl Lease {
    /// Take the lease on canvas `doc`, without waiting. The lease file is created if needed
    /// (and its folder entry synced) and never removed, truncated or replaced afterwards. The
    /// opened file must be the path's own regular single-link file, before and after locking.
    pub(crate) fn acquire(lib: &Library, doc: &str) -> Result<Lease, LeaseError> {
        if !valid_id(doc) {
            return Err(LeaseError::Failed("not a canvas id".into()));
        }
        lib.ensure_hierarchy().map_err(LeaseError::Failed)?;
        let path = lib.lock_path(doc);
        let unusable =
            |e: String| LeaseError::Failed(format!("this canvas's lease file is unusable: {e}"));
        let existed = plain(&path, Kind::File).map_err(unusable)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| unusable(e.to_string()))?;
        if !existed {
            sync_dir(&lib.locks()).map_err(LeaseError::Failed)?;
        }
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(LeaseError::Busy),
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(LeaseError::Failed(format!(
                    "this canvas's lease couldn't be taken: {e}"
                )));
            }
        }
        let id = file
            .metadata()
            .map(|m| dev_ino(&m))
            .map_err(|e| unusable(e.to_string()))?;
        let lease = Lease {
            file,
            path,
            id,
            doc: doc.to_string(),
            poisoned: Mutex::new(None),
        };
        lease.check(lib).map_err(LeaseError::Failed)?;
        Ok(lease)
    }

    /// [`Lease::acquire`], waiting (boundedly, without blocking the executor) while another
    /// process holds the lease for a write, an open or its claim of the editing (see the module
    /// docs). Still busy after that is `Busy`.
    pub(crate) async fn acquire_within(lib: &Library, doc: &str) -> Result<Lease, LeaseError> {
        for _ in 1..LEASE_TRIES {
            match Lease::acquire(lib, doc) {
                Err(LeaseError::Busy) => tokio::time::sleep(LEASE_PAUSE).await,
                other => return other,
            }
        }
        Lease::acquire(lib, doc)
    }

    /// Hold this lease's lock again, waiting boundedly (this thread sleeps meanwhile; it is
    /// called with no await until it is released). `shared` for a read that only must not see
    /// a write half done. Taking it again on this same handle while it is held (the open's
    /// lock, until the claim) succeeds at once: one handle, one lock.
    fn lock(&self, shared: bool) -> Result<(), LeaseError> {
        if let Some(why) = self.poisoned.lock().unwrap().as_ref() {
            // The exclusive lock is still held (see `poisoned`): a read needs nothing more,
            // and no write, claim or record change is admitted.
            return if shared {
                Ok(())
            } else {
                Err(LeaseError::Failed(why.clone()))
            };
        }
        for i in 0..LEASE_TRIES {
            let r = if shared {
                self.file.try_lock_shared()
            } else {
                self.file.try_lock()
            };
            match r {
                Ok(()) => return Ok(()),
                Err(std::fs::TryLockError::WouldBlock) if i + 1 < LEASE_TRIES => {
                    std::thread::sleep(LEASE_PAUSE);
                }
                Err(std::fs::TryLockError::WouldBlock) => return Err(LeaseError::Busy),
                Err(std::fs::TryLockError::Error(e)) => {
                    return Err(LeaseError::Failed(format!(
                        "this canvas's lease couldn't be taken: {e}"
                    )));
                }
            }
        }
        Err(LeaseError::Busy)
    }

    /// Let go of this lease's lock, keeping the open handle (and so the lease's identity). Other
    /// processes can then open and write the canvas between this binding's own writes.
    pub(crate) fn unlock(&self) {
        if self.poisoned.lock().unwrap().is_none() {
            let _ = self.file.unlock();
        }
    }

    /// Keep the exclusive lock this handle holds now for as long as it lives (see `poisoned`).
    /// Only called while it is held exclusively (a write's hold whose server became `Broken`).
    fn poison(&self, why: &str) {
        let mut p = self.poisoned.lock().unwrap();
        if p.is_none() {
            *p = Some(format!(
                "{why} (this window keeps the canvas locked until it is closed, so no other \
                 OpenPaperDesign window writes over those files)"
            ));
        }
    }

    /// A lease on a new, unused canvas id.
    pub(crate) fn fresh(lib: &Library) -> Result<Lease, String> {
        for _ in 0..8 {
            let id = new_id()?;
            if !gone(&lib.document_dir(&id)) || !gone(&lib.lock_path(&id)) {
                continue;
            }
            match Lease::acquire(lib, &id) {
                Ok(l) => return Ok(l),
                Err(LeaseError::Busy) => continue,
                Err(LeaseError::Failed(e)) => return Err(format!("{e}. Nothing was created.")),
            }
        }
        Err("no unused canvas id was found; nothing was created".into())
    }

    /// The library still checks out, and the lease path is still the held file: a regular,
    /// single-link file with the same identity as the open handle.
    pub(crate) fn check(&self, lib: &Library) -> Result<(), String> {
        lib.check()?;
        let replaced = || {
            "this canvas's lease file was replaced, linked or removed, so this process no longer \
             provably owns the canvas"
                .to_string()
        };
        let handle = self.file.metadata().map_err(|_| replaced())?;
        let m = std::fs::symlink_metadata(&self.path).map_err(|_| replaced())?;
        if m.file_type().is_symlink()
            || !m.is_file()
            || !handle.is_file()
            || crate::links(&handle) != 1
            || dev_ino(&m) != self.id
            || dev_ino(&handle) != self.id
        {
            return Err(replaced());
        }
        Ok(())
    }
}

/// The server's guard for a library canvas: the library and the lease are checked on every
/// browser and MCP access. The first failure is kept: from then on every access is refused,
/// even if the identity looks right again. It holds the lease weakly; the binding owns it.
pub(crate) fn guard(lib: Arc<Library>, lease: &Arc<Lease>) -> crate::Guard {
    let lease = Arc::downgrade(lease);
    let failed: Mutex<Option<String>> = Mutex::new(None);
    Arc::new(move || {
        let mut failed = failed.lock().unwrap();
        if let Some(why) = failed.as_ref() {
            return Err(why.clone());
        }
        let r = match lease.upgrade() {
            Some(l) => l.check(&lib),
            None => Err("this canvas was closed".to_string()),
        };
        r.map_err(|e| {
            let why = format!(
                "{e}. This canvas is fenced: nothing more is read or saved through it. Close it \
                 and open it again from the library."
            );
            *failed = Some(why.clone());
            why
        })
    })
}

/// Why a write through a binding that isn't the canvas's editor is refused (see the module
/// docs, "Editing"). Nothing was read or written; the view keeps its unsaved work.
pub(crate) const NOT_EDITING: &str = "This canvas is now being edited in another \
    OpenPaperDesign view (one opened after this one), so this view is read-only and this change \
    was not saved; nothing was written. An unsaved change here stays here. Choose Edit here (or \
    open the canvas again here) to edit in this view again; a change saved there meanwhile is \
    then a conflict, never overwritten.";

/// Why a write, claim or snapshot read waited for the lease in vain (see the module docs).
pub(crate) const LEASE_BUSY: &str = "Another OpenPaperDesign window is holding this canvas right \
    now (in the middle of a save, after a save it couldn't undo, or because it runs an older \
    OpenPaperDesign that keeps a canvas for as long as it has it open). Nothing was written. Try \
    again in a moment.";

/// What `Editing::admit` refuses with.
pub(crate) enum EditError {
    /// Another binding has the editing (`NOT_EDITING`).
    NotEditing,
    /// The lease stayed held by another process (`LEASE_BUSY`).
    Busy,
    /// The lease or the library failed a check or an operation.
    Failed(String),
    /// A record (editing, handover) couldn't be written and was put back: nothing changed.
    Record(String),
    /// This server's writes are fenced (`Broken`): a failed rollback, or a record that couldn't
    /// be put back, may have left the named files either way. Nothing more is written until
    /// they are checked and OpenPaperDesign restarts.
    Fenced(String),
}

impl EditError {
    pub(crate) fn message(&self) -> String {
        match self {
            EditError::NotEditing => NOT_EDITING.into(),
            EditError::Busy => LEASE_BUSY.into(),
            EditError::Failed(e) | EditError::Record(e) | EditError::Fenced(e) => e.clone(),
        }
    }
}

impl From<LeaseError> for EditError {
    fn from(e: LeaseError) -> Self {
        match e {
            LeaseError::Busy => EditError::Busy,
            LeaseError::Failed(e) => EditError::Failed(e),
        }
    }
}

/// One binding's editing of its library canvas (see the module docs, "Editing"): its own random
/// identity in the canvas's record, and its lease (weakly: the binding owns it). Every method
/// that takes the lease's lock is called under the binding's writer fence, so two of this
/// binding's own operations never share (and release) one lock.
pub(crate) struct Editing {
    lib: Arc<Library>,
    lease: std::sync::Weak<Lease>,
    /// Never shown, never a credential: 64 hex digits, unique across processes.
    token: String,
}

/// A held lease lock (`Editing::admit`, `Editing::read`): released when dropped, after the
/// commit or rollback; kept (with this binding's lease, until it is dropped) when the server's
/// writes are `Broken` by then, so no other process writes over files a failed rollback left.
pub(crate) struct Hold {
    lease: Arc<Lease>,
    broken: Option<crate::Broken>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        // An exclusive hold whose server became `Broken` meanwhile keeps the lock on the lease
        // itself, for good (`Lease::poison`), so no later read or cleanup of this binding can
        // let it go (R78-02). A shared hold never poisons; its `unlock` does nothing once the
        // lease is poisoned.
        let broken = self
            .broken
            .as_ref()
            .and_then(|b| crate::refuse_if_broken(b).err());
        match broken {
            Some(why) => self.lease.poison(&why),
            None => self.lease.unlock(),
        }
    }
}

impl Editing {
    pub(crate) fn new(lib: Arc<Library>, lease: &Arc<Lease>) -> Result<Editing, String> {
        Ok(Editing {
            lib,
            lease: Arc::downgrade(lease),
            token: crate::new_token().map_err(|e| format!("no editing identity: {e}"))?,
        })
    }

    fn lease(&self) -> Result<Arc<Lease>, EditError> {
        self.lease
            .upgrade()
            .ok_or_else(|| EditError::Failed("this canvas was closed".into()))
    }

    /// Before a write: the lease's exclusive lock (bounded wait), then this binding (with `view`:
    /// exactly that view of it) must still have the editing. Held until the write's commit or
    /// rollback is done (the returned hold).
    pub(crate) fn admit(
        &self,
        broken: &crate::Broken,
        view: Option<&str>,
    ) -> Result<Hold, EditError> {
        let hold = self.lock_write(broken)?;
        if self.owns(view)? {
            Ok(hold)
        } else {
            Err(EditError::NotEditing)
        }
    }

    /// Whether `record` (an editing record's content) names this binding and, with `view`,
    /// exactly that view of it. A record names `<token>` (the binding, before any view of it
    /// opened, or for an agent's open) or `<token>:<view>` (one view: a panel mount `m<n>`, a
    /// browser tab `t<n>`). A call without a view (an agent, a raw API client) is the binding's.
    fn names(&self, record: &str, view: Option<&str>) -> bool {
        let (token, named) = record.split_once(':').unwrap_or((record, ""));
        token == self.token && view.is_none_or(|v| v == named)
    }

    /// The record's content naming this binding and `view`.
    fn record(&self, view: Option<&str>) -> String {
        match view {
            Some(v) => format!("{}:{v}", self.token),
            None => self.token.clone(),
        }
    }

    /// The lease's exclusive lock, checked, for a call that may or may not write (an agent's):
    /// its reload, checks and any commit then see no other process's write. Whether it may
    /// write is [`Editing::owns`], asked under this hold once the call has something to save.
    pub(crate) fn lock_write(&self, broken: &crate::Broken) -> Result<Hold, EditError> {
        let lease = self.lease()?;
        lease.lock(false)?;
        let hold = Hold {
            lease: lease.clone(),
            broken: Some(broken.clone()),
        };
        lease.check(&self.lib).map_err(EditError::Failed)?;
        Ok(hold)
    }

    /// Whether the canvas's record names this binding (with `view`, that view of it). Asked
    /// under a hold of the lease's exclusive lock ([`Editing::lock_write`]) for a write to rely
    /// on it.
    pub(crate) fn owns(&self, view: Option<&str>) -> Result<bool, EditError> {
        let lease = self.lease()?;
        let editor = self.lib.editor_of(&lease.doc).map_err(EditError::Failed)?;
        Ok(editor.is_some_and(|e| self.names(&e, view)))
    }

    /// Before a snapshot read of the whole canvas: the lease's shared lock, so no other
    /// process's write is half done in it. Needs no editing.
    pub(crate) fn read(&self) -> Result<Hold, EditError> {
        let lease = self.lease()?;
        lease.lock(true)?;
        Ok(Hold {
            lease,
            broken: None,
        })
    }

    /// A record change's outcome for a caller holding the lease's exclusive lock (`hold`): an
    /// uncertain one (the record may be either, R78-06) fences this server's writes (`Broken`,
    /// naming the record), which keeps the lock with the lease when `hold` goes.
    fn recorded(
        &self,
        broken: &crate::Broken,
        r: Result<(), RecordError>,
    ) -> Result<(), EditError> {
        match r {
            Ok(()) => Ok(()),
            Err(RecordError::Failed(e)) => Err(EditError::Record(e)),
            Err(RecordError::Uncertain { error, file }) => {
                crate::set_broken(broken, &file);
                Err(EditError::Fenced(format!(
                    "{error}; {file} may now be either version, so this window's writes to the \
                     canvas are disabled until it is checked and OpenPaperDesign is restarted"
                )))
            }
        }
    }

    /// Take the editing for this binding, or (`view`) for that view of it (an open, a view that
    /// opened, or a view's Edit here), whoever had it: under the lease's exclusive lock, the
    /// record names it. The view that had it (another view, of this binding or another) is
    /// then waiting to hand over what it has (a pending entry in the handover record, see
    /// [`Editing::answer`]), until it answers or ends; the new view is told. What the claiming
    /// view itself had handed over leaves the record: it saves that itself now. A record change
    /// that failed changes nothing (`replace_record`); an uncertain one is `Broken`. Refused
    /// while this server's writes are `Broken`.
    pub(crate) fn claim(
        &self,
        broken: &crate::Broken,
        view: Option<&str>,
    ) -> Result<Option<String>, EditError> {
        crate::refuse_if_broken(broken).map_err(EditError::Fenced)?;
        let hold = self.lock_write(broken)?;
        let doc = hold.lease.doc.clone();
        let record = self.record(view);
        let before = self.lib.editor_of(&doc).map_err(EditError::Failed)?;
        let r = self.lib.set_editor(&doc, Some(&record));
        self.recorded(broken, r)?;
        // Taken. The handover record follows; the claim stands even if it can't be updated (the
        // new view then isn't told to wait, and what the earlier view answers still arrives),
        // and the result says so (`Some`), fenced too when it couldn't be put back.
        let mut h = match self.lib.handover_of(&doc) {
            Ok(h) => h,
            Err(e) => {
                return Ok(Some(format!(
                    "Editing was taken here, but the handover record couldn't be read ({e}), so \
                     the view it came from isn't waited for; what it hands over still arrives."
                )));
            }
        };
        let mut gone = Vec::new();
        if view.is_some() {
            h.pending.retain(|p| p.from != record);
            gone = h.take_offers_from(&record);
        }
        if let Some(prev) = before.filter(|p| *p != record && p.contains(':'))
            && !h.pending.iter().any(|p| p.from == prev)
            && h.pending.len() < MAX_PENDING
            && let Ok(id) = new_id()
        {
            h.pending.push(Pending {
                id,
                from: prev,
                since: now_ms(),
                ended: false,
            });
        }
        #[cfg(test)]
        if crate::test_support::fault(&self.lib.locks(), "handover_write") {
            return Ok(Some(
                "Editing was taken here, but the handover record couldn't be written (injected \
                 failure), so the view it came from isn't waited for; what it hands over still \
                 arrives."
                    .into(),
            ));
        }
        let r = self.lib.set_handover(&doc, &h);
        match self.recorded(broken, r) {
            Ok(()) => {
                for o in gone {
                    self.lib.remove_offer(&doc, &o.id);
                }
                Ok(None)
            }
            Err(EditError::Fenced(e)) => Ok(Some(format!("Editing was taken here, but then {e}"))),
            Err(e) => Ok(Some(format!(
                "Editing was taken here, but the handover record couldn't be updated ({}), so \
                 the view it came from isn't waited for; what it hands over still arrives.",
                e.message()
            ))),
        }
    }

    /// At this binding's end (after its server was retired): clear the record if it still names
    /// this binding (any view of it), never another's, and mark every view of it still pending
    /// a handover as ended (it can't answer any more; what it didn't hand over is gone with it).
    /// Nothing while `Broken` (the lock is kept with the lease), and nothing if the lease can't
    /// be had in time (the record then names no live binding).
    pub(crate) fn release(&self, broken: &crate::Broken) {
        if crate::refuse_if_broken(broken).is_err() {
            return;
        }
        let Ok(hold) = self.lock_write(broken) else {
            return;
        };
        let doc = hold.lease.doc.clone();
        if self
            .lib
            .editor_of(&doc)
            .ok()
            .flatten()
            .is_some_and(|e| self.names(&e, None))
        {
            let r = self.lib.set_editor(&doc, None);
            let _ = self.recorded(broken, r);
        }
        self.end_pending(broken, &doc, |from| self.names(from, None));
    }

    /// A view of this binding left (a browser tab's release, a panel mount's): if it was still
    /// to hand over, it is marked ended. Its record entry, if it edits, stays (nobody edits
    /// until a view takes it). Under the writer fence, as every lease use.
    pub(crate) fn release_view(&self, broken: &crate::Broken, view: &str) {
        if crate::refuse_if_broken(broken).is_err() {
            return;
        }
        let Ok(hold) = self.lock_write(broken) else {
            return;
        };
        let doc = hold.lease.doc.clone();
        let record = self.record(Some(view));
        self.end_pending(broken, &doc, |from| *from == record);
    }

    fn end_pending(&self, broken: &crate::Broken, doc: &str, mine: impl Fn(&String) -> bool) {
        let Ok(mut h) = self.lib.handover_of(doc) else {
            return;
        };
        let mut changed = false;
        for p in h.pending.iter_mut().filter(|p| !p.ended && mine(&p.from)) {
            p.ended = true;
            changed = true;
        }
        if changed {
            let r = self.lib.set_handover(doc, &h);
            let _ = self.recorded(broken, r);
        }
    }

    /// Whether this binding (with `view`, that view of it) has the editing now (the record,
    /// read without the lock: a hint for the views, never what a write relies on).
    pub(crate) fn here(&self, view: Option<&str>) -> bool {
        let Ok(lease) = self.lease() else {
            return false;
        };
        self.lib
            .editor_of(&lease.doc)
            .ok()
            .flatten()
            .is_some_and(|e| self.names(&e, view))
    }

    /// A view that lost the editing answers the handover (issue #23's new-view recovery, R78-03):
    /// it is no longer pending, and what it has unsaved is kept as an offer for the editing
    /// view: `canvas`, its whole canvas as a canvas save on revision `base` (every page with its
    /// content, as it stands without any edit still under way; empty when it has nothing
    /// finished), whether that differs from `base` (`changed`), and `unfinished`, the edits
    /// still under way there (a rename field's text, a text layer being typed, a gesture),
    /// each a continuation the editing view can take up. Nothing in the canvas changes. The
    /// view's later answer replaces its earlier offer. Several views' offers wait side by side
    /// (`MAX_OFFERS`); nothing is merged. The view must be live and not editing (one that edits
    /// saves instead); the caller checks its lifetime under the writer fence. Returns the
    /// offer's id (none for an answer with nothing to hand over).
    pub(crate) fn answer(
        &self,
        broken: &crate::Broken,
        view: &str,
        offer: Option<NewOffer>,
    ) -> Result<Option<String>, EditError> {
        let hold = self.lock_write(broken)?;
        if self.owns(Some(view))? {
            return Err(EditError::Failed(
                "this view has the editing, so its change is saved, not handed over".into(),
            ));
        }
        let doc = hold.lease.doc.clone();
        let from = self.record(Some(view));
        let mut h = self.lib.handover_of(&doc).map_err(EditError::Failed)?;
        h.pending.retain(|p| p.from != from);
        let replaced = h.take_offers_from(&from);
        let mut made = None;
        if let Some(o) = offer {
            if h.offers.len() >= MAX_OFFERS {
                return Err(EditError::Failed(OFFERS_FULL.into()));
            }
            let id = new_id().map_err(EditError::Failed)?;
            if !o.canvas.is_empty() {
                self.lib
                    .write_offer(&doc, &id, o.canvas.as_bytes())
                    .map_err(EditError::Failed)?;
            }
            h.offers.push(Offer {
                id: id.clone(),
                from,
                base: o.base,
                canvas: !o.canvas.is_empty(),
                changed: o.changed,
                unfinished: o.unfinished,
                at: now_ms(),
            });
            made = Some(id);
        }
        let r = self.lib.set_handover(&doc, &h);
        if let Err(e) = self.recorded(broken, r) {
            if let Some(id) = &made {
                self.lib.remove_offer(&doc, id);
            }
            return Err(e);
        }
        for o in replaced {
            self.lib.remove_offer(&doc, &o.id);
        }
        Ok(made)
    }

    /// The handover record as `view` sees it (a hint for the views, read without the lock;
    /// every action re-reads it under the lease).
    pub(crate) fn handover(&self, view: Option<&str>) -> Option<Value> {
        let lease = self.lease().ok()?;
        let h = self.lib.handover_of(&lease.doc).ok()?;
        if h.offers.is_empty() && h.pending.is_empty() {
            return None;
        }
        let mine = |from: &str| view.is_some_and(|v| from == self.record(Some(v)));
        Some(json!({
            "offers": h.offers.iter().map(|o| json!({
                "id": o.id,
                "from_here": mine(&o.from),
                "base": o.base,
                "changed": o.changed && o.canvas,
                "canvas": o.canvas,
                "unfinished": o.unfinished,
                "at": o.at,
            })).collect::<Vec<_>>(),
            "pending": h.pending.iter().map(|p| json!({
                "id": p.id,
                "from_here": mine(&p.from),
                "since": p.since,
                "ended": p.ended,
            })).collect::<Vec<_>>(),
        }))
    }

    /// Offer `id` and its canvas (when it has one), read under `hold` (the caller's admission
    /// of the editing view): `None` when no such offer waits (dismissed, applied or replaced
    /// meanwhile).
    pub(crate) fn find_offer(
        &self,
        hold: &Hold,
        id: &str,
    ) -> Result<Option<(Offer, Option<String>)>, EditError> {
        let doc = &hold.lease.doc;
        let h = self.lib.handover_of(doc).map_err(EditError::Failed)?;
        let Some(o) = h.offers.into_iter().find(|o| o.id == id) else {
            return Ok(None);
        };
        let canvas = if o.canvas {
            Some(self.lib.read_offer(doc, id).map_err(EditError::Failed)?)
        } else {
            None
        };
        Ok(Some((o, canvas)))
    }

    /// Remove offer `id` from the handover record under `hold` (the caller's admission), as a
    /// checked record change; its canvas file goes after (a leftover one is never read).
    /// Nothing if it waits no more.
    pub(crate) fn remove_offer(
        &self,
        hold: &Hold,
        broken: &crate::Broken,
        id: &str,
    ) -> Result<(), EditError> {
        let doc = &hold.lease.doc;
        let mut h = self.lib.handover_of(doc).map_err(EditError::Failed)?;
        let before = h.offers.len();
        h.offers.retain(|o| o.id != id);
        if h.offers.len() == before {
            return Ok(());
        }
        let r = self.lib.set_handover(doc, &h);
        self.recorded(broken, r)?;
        self.lib.remove_offer(doc, id);
        Ok(())
    }

    /// The editing view stops waiting for pending entry `id` (under `hold`, the caller's
    /// admission): whatever that view still has stays there, and is handed over if it answers
    /// later. Returns whether it was waiting.
    pub(crate) fn stop_waiting(
        &self,
        hold: &Hold,
        broken: &crate::Broken,
        id: &str,
    ) -> Result<bool, EditError> {
        let doc = &hold.lease.doc;
        let mut h = self.lib.handover_of(doc).map_err(EditError::Failed)?;
        let before = h.pending.len();
        h.pending.retain(|p| p.id != id);
        if h.pending.len() == before {
            return Ok(false);
        }
        let r = self.lib.set_handover(doc, &h);
        self.recorded(broken, r)?;
        Ok(true)
    }

    /// Keep offer `id`'s canvas as a new library canvas (`name`), with this canvas's images
    /// (`assets`, under `hold`): a separate canvas with the same page ids, names and content,
    /// never merged into this one (R78-03: a handed-over change that conflicts is recovered
    /// this way). Made and checked as a create is (`create_document`, `check_document`); on a
    /// failure nothing of it stays (or what stays is named). Returns the new canvas's id.
    pub(crate) fn keep_as_canvas(
        &self,
        _hold: &Hold,
        doc: NewDocument,
    ) -> Result<String, EditError> {
        let lease = Lease::fresh(&self.lib).map_err(EditError::Failed)?;
        let (_, made) = self
            .lib
            .create_document(&lease, &doc.fresh())
            .map_err(EditError::Failed)?;
        if let Err(e) = self.lib.check_document(&lease.doc) {
            return Err(EditError::Failed(
                made.fail(&format!("the new canvas didn't check out ({e})")),
            ));
        }
        Ok(lease.doc.clone())
    }
}

/// The most offers that wait at once (several views can each hand over what they had).
const MAX_OFFERS: usize = 8;
/// The most views waited for at once.
const MAX_PENDING: usize = 16;
/// The most continuations one offer carries, and their bounded JSON size.
pub(crate) const MAX_CONTINUATIONS: usize = 16;
pub(crate) const MAX_CONTINUATION_BYTES: usize = 64 << 10;

/// Why an offer isn't kept while the record is full (`MAX_OFFERS`): it stays in its view.
pub(crate) const OFFERS_FULL: &str = "Several other views' unsaved changes are already waiting \
    for the editing view, so this one wasn't handed over; it stays here, unsaved.";

/// What a view hands over (see [`Editing::answer`]).
pub(crate) struct NewOffer {
    pub base: String,
    pub canvas: String,
    pub changed: bool,
    pub unfinished: Vec<Value>,
}

/// One waiting offer: which view (`from`, its record content, never shown), the canvas
/// revision it was made on, whether its canvas file exists (`<id>.offer-<offer>`) and differs
/// from that revision, its continuations (opaque to the server: the editor's own JSON, bounded)
/// and when it came.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Offer {
    pub id: String,
    pub from: String,
    pub base: String,
    pub canvas: bool,
    pub changed: bool,
    pub unfinished: Vec<Value>,
    pub at: u64,
}

/// A view that had the editing when another took it, and hasn't answered yet (`ended`: its
/// binding or view ended without answering).
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Pending {
    pub id: String,
    pub from: String,
    pub since: u64,
    pub ended: bool,
}

/// A canvas's handover record (`<id>.handover`): the offers waiting and the views waited for.
#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct Handover {
    pub offers: Vec<Offer>,
    pub pending: Vec<Pending>,
}

impl Handover {
    fn take_offers_from(&mut self, from: &str) -> Vec<Offer> {
        let (gone, kept) = std::mem::take(&mut self.offers)
            .into_iter()
            .partition(|o| o.from == from);
        self.offers = kept;
        gone
    }

    fn to_json(&self) -> String {
        json!({
            "offers": self.offers.iter().map(|o| json!({
                "id": o.id, "from": o.from, "base": o.base, "canvas": o.canvas,
                "changed": o.changed, "unfinished": o.unfinished, "at": o.at,
            })).collect::<Vec<_>>(),
            "pending": self.pending.iter().map(|p| json!({
                "id": p.id, "from": p.from, "since": p.since, "ended": p.ended,
            })).collect::<Vec<_>>(),
        })
        .to_string()
    }

    /// A handover record's content; anything else is an error (never overwritten unread).
    fn from_json(s: &str) -> Option<Handover> {
        let v: Value = serde_json::from_str(s).ok()?;
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let offers = v
            .get("offers")?
            .as_array()?
            .iter()
            .map(|o| {
                Some(Offer {
                    id: text(o, "id").filter(|i| valid_id(i))?,
                    from: text(o, "from")?,
                    base: text(o, "base")?,
                    canvas: o.get("canvas")?.as_bool()?,
                    changed: o.get("changed")?.as_bool()?,
                    unfinished: o.get("unfinished")?.as_array()?.clone(),
                    at: o.get("at")?.as_u64()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let pending = v
            .get("pending")?
            .as_array()?
            .iter()
            .map(|p| {
                Some(Pending {
                    id: text(p, "id").filter(|i| valid_id(i))?,
                    from: text(p, "from")?,
                    since: p.get("since")?.as_u64()?,
                    ended: p.get("ended")?.as_bool()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Handover { offers, pending })
    }
}

fn now_ms() -> u64 {
    paper_rs::project::now_ms() as u64
}

/// What a failed record change left (see [`Library::replace_record`]).
pub(crate) enum RecordError {
    /// Nothing changed: the record is as it was.
    Failed(String),
    /// Putting it back failed too: `file` may be either version.
    Uncertain { error: String, file: String },
}

impl RecordError {
    fn context(self, what: &str) -> RecordError {
        match self {
            RecordError::Failed(e) => RecordError::Failed(format!("{what} ({e})")),
            RecordError::Uncertain { error, file } => RecordError::Uncertain {
                error: format!("{what} ({error})"),
                file,
            },
        }
    }
}
