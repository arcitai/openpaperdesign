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
//!                                         (versioned, kept once written: `HANDOVER_FORMAT`)
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
//! as busy, with nothing changed. Except a view's release and a binding's end (runs 201–202):
//! after that ordinary wait, those wait for it with no bound (`Lease::lock_wait`: another
//! process that keeps it for its whole life keeps them under way), then record that the views
//! left; and a window that couldn't record that keeps it (`Lease::kept`) until it can, past the
//! binding's own end too (`Library::owed`, run 203). The lock is advisory: it coordinates
//! OpenPaperDesign processes and nothing else (not a native paper-rs app, not other programs).
//! Lease files stay forever, so every process locks the same file.
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
//! (`Lease::poison`: no later read, claim or cleanup lets it go; past the binding's end, until
//! this process ends: `Library::owed`). A view can also take the
//! editing back explicitly ("Edit here", or opening the canvas again); its next save is then
//! revision-checked against whatever was saved meanwhile, so nothing is overwritten. Reads never
//! need the editing: every open view keeps showing what is saved. A binding's close or end clears
//! the record only if it still names that binding (a view that isn't editing never takes the
//! editing from the one that is), and marks its views still waited for as ended; a view's own
//! release (a mount's, a tab's) marks it ended if it was waited for, and if it edits, leaves the
//! record naming only its binding, so no later view waits for it to hand over. A cleanup that
//! fails is said, and for a view's release kept to be done again; if the editing record can't
//! be rewritten (put back), the handover record notes that the view it names left (an ended
//! entry), so no claim waits for it; if the handover record can't be written either (put back),
//! the window keeps the canvas locked (`Lease::kept`), past the binding's end too
//! (`Library::owed`), so no other window takes the editing or changes the canvas while the
//! records say a view of it may answer (they are told it is held), until that is recorded
//! (`Editing::release_view`, `Editing::release`); a record that can't be put back fences the
//! server instead (`Broken`, the file named). A departure that needs noting where the
//! handover record already lists the most views it can (`MAX_PENDING`) is not recorded either,
//! and is handled the same way (`no_room`, run 204): no entry is removed to make room. Nor is a
//! cleanup whose editing record couldn't be read (run 205): whether it still names the view
//! isn't known, so the lease is kept the same way, the cause said (`Failed`). Nor one whose
//! handover record couldn't be read (run 206: an I/O error, `ReadError::Unread`): its entries
//! may well read later; only one seen and unusable (not a plain file, too large, a form this
//! version doesn't read) is left as it is, since no claim can wait on anything in it. A process
//! that ends without closing (a crash, or a write failing until it ends) leaves its records as
//! they were: the editing record may still name one of its views, and nothing tells that view
//! apart from a hidden, asleep or disconnected one. It is waited for like one: the next view to
//! take the editing gets a pending entry for it (unless one is already there), shown as not
//! answered until the editing view stops waiting (see DESIGN.md). Nothing here detects that a
//! process ended. What only lived in an ended process's memory and was never handed over (an
//! unsaved edit in a view whose host went away first) is not on disk and can't be recovered
//! from here.
//!
//! Identity: the root, the `library/v1/{canvases,locks}` folders and the held lease file are
//! pinned (device, inode) when first seen and checked again on every library action and, through
//! the server's guard, on every browser and MCP access to the open canvas. A changed identity
//! fences that canvas for good: every later read and write through it is refused. The check and
//! the file operation after it can still race, as for project roots (see `lib.rs`).

use crate::{Kind, NotPlain, dev_ino, plain, plain_seen};
use paper_rs::project::{ASSETS_DIR, MANIFEST, Manifest, PAGES_DIR, PageData};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[path = "library_payload.rs"]
pub(crate) mod payload;

#[cfg(test)]
#[path = "library_compat_tests.rs"]
mod compat_tests;

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
/// How long an open, a write or a claim waits for another process's hold of a canvas's lease
/// (see the module docs): `LEASE_TRIES` × `LEASE_PAUSE`, one second. A view's release and a
/// binding's end wait this long first, then for as long as it is held (`Lease::lock_wait`).
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
    /// Bindings of this process that ended (Close, Switch, the end of its service) while their
    /// canvas's records still said that a view of theirs might answer (R92-01, run 203): their
    /// lease, still locked, outlives them here (`Editing::outlive`), so no other window takes
    /// the editing or changes the canvas while those records say so (they are told it is held).
    /// One whose writes were put back is recorded again (`settle_owed`) at this process's next
    /// use of the library (its list, an open, rename or export of a canvas) and at its end, and
    /// let go once recorded; one fenced (`Broken`) is kept until this process ends.
    owed: Mutex<Vec<Owed>>,
}

/// A binding's lease kept past its end (see `Library::owed`), with the end's record cleanup to
/// do again (`None`: fenced; nothing more is written from it).
struct Owed {
    lease: Arc<Lease>,
    cleanup: Option<(Arc<Editing>, crate::Broken)>,
    /// What its end said it didn't record, as it ended.
    why: String,
}

impl Owed {
    /// Why it is still kept: its latest attempt's outcome, else what its end said.
    fn reason(&self) -> String {
        match &self.cleanup {
            Some((e, _)) => e
                .retirement()
                .and_then(Result::err)
                .map_or_else(|| self.why.clone(), |e| e.message()),
            None => self.why.clone(),
        }
    }
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
            owed: Mutex::default(),
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

    /// Whether canvas `id`'s lease is held exclusively right now, by any handle (another
    /// window's, or this process's own during a change): asked on a handle of its own, opened
    /// for this and closed again (never a binding's, whose own lock a shared take would
    /// convert). A hint only; nothing relies on it. False when it can't be asked.
    fn lease_held_now(&self, id: &str) -> bool {
        let path = self.lock_path(id);
        if !matches!(plain(&path, Kind::File), Ok(true)) {
            return false;
        }
        let Ok(f) = File::open(&path) else {
            return false;
        };
        match f.try_lock_shared() {
            Ok(()) => {
                let _ = f.unlock();
                false
            }
            Err(std::fs::TryLockError::WouldBlock) => true,
            Err(std::fs::TryLockError::Error(_)) => false,
        }
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
        // (Tests: `editor_read` fails the next read of an existing record once, as an I/O error
        // that isn't NotFound would; the record is left as it is. Run 205, R94-01.)
        #[cfg(test)]
        if crate::test_support::fault(&self.locks(), "editor_read") {
            return Err("this canvas's editing record: injected read failure".into());
        }
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
        self.handover_read(id).map_err(ReadError::message)
    }

    /// [`Library::handover_of`], saying which kind of failure it is (run 206, LEAD205-01): a
    /// record seen and not one this version reads (`Unusable`: not a plain file, too large, a
    /// form it doesn't know or that doesn't parse), which every claim leaves as it is and can
    /// wait on nothing in; or one that couldn't be read (`Unread`: an I/O error other than
    /// NotFound), whose content isn't known, so a claim may well read and wait on it later.
    fn handover_read(&self, id: &str) -> Result<Handover, ReadError> {
        let path = self.handover_path(id);
        let named = |e: String| format!("this canvas's handover record: {e}");
        let bytes = match self.record_read(&path) {
            Ok(b) => b,
            Err(ReadError::Unusable(e)) => return Err(ReadError::Unusable(named(e))),
            Err(ReadError::Unread(e)) => return Err(ReadError::Unread(named(e))),
        };
        // (Tests: `handover_read` fails the next read of an existing regular record once, as an
        // I/O error that isn't NotFound would; the record is left as it is. Run 206, LEAD205-01.)
        #[cfg(test)]
        if bytes.is_some() && crate::test_support::fault(&self.locks(), "handover_read") {
            return Err(ReadError::Unread(named("injected read failure".into())));
        }
        match bytes {
            None => Ok(Handover::default()),
            Some(b) => std::str::from_utf8(&b)
                .ok()
                .and_then(Handover::from_json)
                .ok_or_else(|| {
                    ReadError::Unusable("this canvas's handover record is unreadable".into())
                }),
        }
    }

    /// Replace a canvas's handover record, only under its lease's exclusive lock, as a checked
    /// record change. Always in this version's form ([`HANDOVER_FORMAT`]), and kept when empty:
    /// once written, an older OpenPaperDesign can't read it, so it neither consumes nor rewrites
    /// what it holds (R84-01).
    fn set_handover(&self, id: &str, h: &Handover) -> Result<(), RecordError> {
        let text = h.to_json();
        if text.len() as u64 > MAX_DRAFT_BYTES {
            return Err(RecordError::Failed(
                "this change is too large to hand over; it stays in its view".into(),
            ));
        }
        self.replace_record(&self.handover_path(id), Some(text.as_bytes()))
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
        // (Tests: `record_sync`/`record_restore` fail the next record's write/put-back,
        // `handover_sync`/`handover_restore` only a handover record's, run 203.)
        #[cfg(test)]
        let handover = path.extension().is_some_and(|e| e == "handover");
        let done = self.put_record(path, bytes).and_then(|()| {
            #[cfg(test)]
            if crate::test_support::fault(&self.locks(), "record_sync")
                || (handover && crate::test_support::fault(&self.locks(), "handover_sync"))
            {
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
            if crate::test_support::fault(&self.locks(), "record_restore")
                || (handover && crate::test_support::fault(&self.locks(), "handover_restore"))
            {
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
        self.record_read(path).map_err(ReadError::message)
    }

    /// [`Library::record_bytes`], saying which kind of failure it is (see `ReadError`).
    fn record_read(&self, path: &Path) -> Result<Option<Vec<u8>>, ReadError> {
        match plain_seen(path, Kind::File) {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(NotPlain::Seen(e)) => return Err(ReadError::Unusable(e)),
            Err(NotPlain::Unseen(e)) => return Err(ReadError::Unread(e)),
        }
        let mut bytes = Vec::new();
        let read = File::open(path).and_then(|f| {
            use std::io::Read;
            f.take(MAX_DRAFT_BYTES + 1).read_to_end(&mut bytes)
        });
        match read {
            Ok(_) if bytes.len() as u64 > MAX_DRAFT_BYTES => Err(ReadError::Unusable(format!(
                "{} is larger than a record can be",
                self.label(path)
            ))),
            Ok(_) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ReadError::Unread(e.to_string())),
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

    /// Record again, without waiting, what each binding of this process that ended still owes
    /// (see `owed`): its lease is still locked by this process, so nothing else can have
    /// changed the records meanwhile. One recorded lets its lease go; one fenced, or failing
    /// again, stays. Called at this process's next use of the library (the launcher: before a
    /// list, an open, rename or export) and at its end; never by a timer. Returns, for each
    /// such canvas, what came of it (`canvas`, `done`, `note`): what a list says (`cleanup`).
    /// Only the editing and handover records are written; no canvas's drawing is.
    pub(crate) fn settle_owed(&self) -> Vec<Value> {
        let mut said = Vec::new();
        self.owed.lock().unwrap().retain(|o| {
            let kept = match &o.cleanup {
                None => true,
                Some((editing, broken)) => {
                    let _ = editing.release(broken);
                    o.lease.is_kept() || o.lease.is_poisoned()
                }
            };
            said.push(if kept {
                json!({"canvas": o.lease.doc, "done": false, "note": format!(
                    "This window still keeps this canvas locked: when it was closed here, \
                     recording that its views left didn't complete ({}).",
                    o.reason().trim_end_matches('.')
                )})
            } else {
                json!({"canvas": o.lease.doc, "done": true, "note": "This window recorded that \
                    the views it closed of this canvas left (its editing and handover records), \
                    and let the canvas go."})
            });
            kept
        });
        said
    }

    /// The public list (the home's and the model's: R93-02, run 204): first what this process
    /// still owes for canvases it closed is recorded where it can be (`settle_owed`: their
    /// editing and handover records written, a canvas this window kept locked for that let
    /// go), then the canvases are listed (`list`, which writes nothing). No canvas is created,
    /// opened or locked by it, and no drawing is changed. What the first step came to is the
    /// result's `cleanup`.
    pub(crate) fn list_settling(&self, open: Option<&str>) -> Result<Value, String> {
        let cleanup = self.settle_owed();
        let mut listed = self.list(open)?;
        listed["cleanup"] = json!(cleanup);
        listed["note"] = json!(
            "Listing creates, opens or locks no canvas and never changes a drawing. First it \
             finishes this window's own bookkeeping of canvases it closed while their editing \
             and handover records couldn't be written: it writes those two records then, and \
             lets go of a canvas it kept locked for that; one it can't record yet stays locked \
             (each is in `cleanup`, with what came of it). These canvases belong to \
             OpenPaperDesign's own library, not to a project folder."
        );
        Ok(listed)
    }

    /// The canvases in the library, read-only and bounded: nothing is created, opened, locked
    /// or written. `open` is the canvas open in this process, if any. (The public lists go
    /// through `list_settling`, which may write ownership records first.)
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
    /// names the files that may be inconsistent. Every other canvas is unaffected. Refused too
    /// while a binding of it that ended here still keeps its lease (`owed`): says why.
    pub(crate) fn refuse_if_broken(&self, id: &str) -> Result<(), String> {
        if let Some(o) = self.owed.lock().unwrap().iter().find(|o| o.lease.doc == id) {
            return Err(format!(
                "when it was closed here, recording that its views left didn't complete ({}), \
                 and this window keeps it locked until then",
                o.reason().trim_end_matches('.')
            ));
        }
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
    /// Set while a view released here couldn't be recorded as gone (its write was put back:
    /// `Editing::forget_view`, run 202; or at the binding's end, `Editing::release`, run 203):
    /// the exclusive lock this handle holds is kept (no unlock, no conversion by a shared read)
    /// so no other window can take the editing or change the canvas while the record still says
    /// that view may answer; this binding's own calls go on under it (each first tries to record
    /// that view again), and past its end the library keeps the handle (`Library::owed`).
    /// Cleared once it is recorded.
    kept: Mutex<bool>,
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
            kept: Mutex::new(false),
        };
        lease.check(lib).map_err(LeaseError::Failed)?;
        Ok(lease)
    }

    /// Whether this handle keeps its exclusive lock for a departure (see `kept`).
    fn is_kept(&self) -> bool {
        *self.kept.lock().unwrap()
    }

    /// Keep the exclusive lock this handle holds now until `unkeep` (see `kept`). Only called
    /// under a hold of it.
    fn keep(&self) {
        *self.kept.lock().unwrap() = true;
    }

    /// Stop keeping it: the next hold's end lets it go as usual.
    fn unkeep(&self) {
        *self.kept.lock().unwrap() = false;
    }

    /// Whether this handle keeps its exclusive lock for good (see `poisoned`).
    fn is_poisoned(&self) -> bool {
        self.poisoned.lock().unwrap().is_some()
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
        // Kept for a departure (see `kept`): this handle holds it exclusively already; a read or
        // a write of this binding goes on under it, never converting or letting it go.
        if self.is_kept() {
            return Ok(());
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

    /// The exclusive lock, waiting (blocking this thread) for as long as another process holds
    /// it: one wait on the lock itself, no retries or timer. Only for a released view's record
    /// cleanup (`Editing::release_view_waiting`), off the async executor and under its
    /// binding's writer fence. A poisoned handle admits no change (as `lock`).
    fn lock_wait(&self) -> Result<(), LeaseError> {
        if let Some(why) = self.poisoned.lock().unwrap().as_ref() {
            return Err(LeaseError::Failed(why.clone()));
        }
        if self.is_kept() {
            return Ok(());
        }
        self.file
            .lock()
            .map_err(|e| LeaseError::Failed(format!("this canvas's lease couldn't be taken: {e}")))
    }

    /// The exclusive lock if it can be had right now, without waiting (`false`: another process
    /// holds it, or this handle is poisoned and admits no change). For a poll's best effort only
    /// (`Editing::settle_now`).
    fn try_lock_now(&self) -> Result<bool, LeaseError> {
        if self.poisoned.lock().unwrap().is_some() {
            return Ok(false);
        }
        if self.is_kept() {
            return Ok(true);
        }
        match self.file.try_lock() {
            Ok(()) => Ok(true),
            Err(std::fs::TryLockError::WouldBlock) => Ok(false),
            Err(std::fs::TryLockError::Error(e)) => Err(LeaseError::Failed(format!(
                "this canvas's lease couldn't be taken: {e}"
            ))),
        }
    }

    /// Let go of this lease's lock, keeping the open handle (and so the lease's identity). Other
    /// processes can then open and write the canvas between this binding's own writes.
    pub(crate) fn unlock(&self) {
        if self.poisoned.lock().unwrap().is_none() && !self.is_kept() {
            let _ = self.file.unlock();
        }
    }

    /// Keep the exclusive lock this handle holds now for as long as it lives (see `poisoned`).
    /// Only called while it is held exclusively (a write's hold whose server became `Broken`).
    fn poison(&self, why: &str) {
        let mut p = self.poisoned.lock().unwrap();
        if p.is_none() {
            *p = Some(format!(
                "{why} (this window keeps the canvas locked until OpenPaperDesign is \
                 restarted, so no other OpenPaperDesign window writes over those files)"
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
#[derive(Clone)]
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
    /// Views of this binding that were released (they can never act again) and whose record
    /// cleanup (`forget_view`) isn't done yet (R89-01/R90, runs 199–200): it is done under this
    /// binding's next hold of the lease (`lock_write`), which refuses what would rely on the
    /// editing record until it is (or until the view's departure is at least noted where every
    /// claim reads it: `Departed::safe`), and tried at a poll of its views (`settle_now`).
    departed: Mutex<Vec<Departed>>,
    /// What this binding's own end (`release`) did: `None` until it ran.
    retired: Mutex<Option<Result<(), EditError>>>,
}

/// A released view whose record cleanup isn't done (see `Editing::departed`). `safe`, once the
/// handover record says that it left (or no claim could wait for it anyway: an unreadable
/// handover record), with why its cleanup is still incomplete: no claim, of this binding or any
/// other, then waits for it.
#[derive(Clone)]
struct Departed {
    view: String,
    safe: bool,
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
            departed: Mutex::default(),
            retired: Mutex::default(),
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
        let hold = self.lock_raw(broken)?;
        // A released view's record cleanup is a prerequisite of anything that relies on the
        // editing record (R90-03): done first, under this hold, or at least its departure noted
        // where every claim reads it; else this call is refused with why, and nothing of it
        // runs. A fence established by it stops everything after it.
        self.settle_departed(broken, &hold.lease.doc)?;
        crate::refuse_if_broken(broken).map_err(EditError::Fenced)?;
        Ok(hold)
    }

    /// The lease's exclusive lock, checked (see [`Editing::lock_write`]), nothing more.
    fn lock_raw(&self, broken: &crate::Broken) -> Result<Hold, EditError> {
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
        // Taken. The handover record follows; the claim stands even if it can't be updated, and
        // the result says what that leaves (`Some`), never that a handover will still arrive
        // (R85-01): a record this version can't read (unknown or malformed) is left as it is,
        // and every answer is refused while it stays so; one whose write failed and was put back
        // took no part of this claim (nor, before this version first wrote it, its form: the
        // first successfully written record is what keeps older processes out). Fenced too
        // when it couldn't be put back.
        let mut h = match self.lib.handover_of(&doc) {
            Ok(h) => h,
            Err(e) => {
                return Ok(Some(format!(
                    "Editing was taken here, but this canvas's handover record couldn't be read \
                     ({e}), so it was left as it is: the view it came from isn't waited for, and \
                     nothing can be handed over to this view while the record can't be read."
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
                "Editing was taken here, but the handover record couldn't be updated (injected \
                 failure), so the view it came from isn't waited for, and a handover from it \
                 isn't confirmed here."
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
                 the view it came from isn't waited for, and a handover from it isn't confirmed \
                 here.",
                e.message()
            ))),
        }
    }

    /// At this binding's end (after its server was retired): clear the record if it still names
    /// this binding (any view of it), never another's, and mark every view of it still pending
    /// a handover as ended (it can't answer any more; what it didn't hand over is gone with it).
    /// If the record's write is put back, the handover record also gets an ended entry for the
    /// view it still names, in the same write, so no claim waits for it (R92-01, run 203). If
    /// that handover write is put back too, or the record has no room for that entry (it lists
    /// `MAX_PENDING` already: `no_room`, run 204), or the editing record couldn't be read
    /// (`Failed`, run 205), the lease is kept locked (`Lease::kept`), and past the binding's
    /// end the library keeps it (`outlive`) until this is recorded. Nothing while
    /// `Broken` (the lock is kept with the lease, then by the library until this process ends).
    /// What it did is kept (`retirement`): a view released after it is told, never that its
    /// cleanup completed when this didn't (R90-03). Every view of this binding is covered,
    /// released ones included. `Busy` (another window holds the lease past the ordinary wait)
    /// is not its end: its caller waits for the lease then (`release_waiting`, run 202: R91-01),
    /// as a view's release does; nothing is kept as done meanwhile. A later fence (a record that
    /// may be either version) is what is said over an earlier record put back.
    pub(crate) fn release(&self, broken: &crate::Broken) -> Result<(), EditError> {
        let done = self.release_now(broken, false);
        if !matches!(done, Err(EditError::Busy)) {
            self.retired_as(done.clone());
        }
        done
    }

    /// [`Editing::release`] whose lease was busy: waits for it (this thread blocked on the lease
    /// itself), then the same cleanup. Its caller holds nothing else of this binding.
    pub(crate) fn release_waiting(&self, broken: &crate::Broken) -> Result<(), EditError> {
        let done = self.release_now(broken, true);
        self.retired_as(done.clone());
        done
    }

    fn retired_as(&self, done: Result<(), EditError>) {
        if done.is_ok() {
            self.departed.lock().unwrap().clear();
        }
        *self.retired.lock().unwrap() = Some(done);
    }

    fn release_now(&self, broken: &crate::Broken, wait: bool) -> Result<(), EditError> {
        crate::refuse_if_broken(broken).map_err(EditError::Fenced)?;
        let hold = if wait {
            self.lock_waiting(broken)?
        } else {
            self.lock_raw(broken)?
        };
        let doc = hold.lease.doc.clone();
        let mut first = None;
        // The view of this binding the editing record still names when its write was put back
        // (R92-01, run 203): the handover record then says that it left (an ended entry, as a
        // released view's `forget_view` notes it), so no claim waits for it.
        let mut stays = None;
        // The editing record couldn't be read (R94-01, run 205): whether it still names a view
        // of this binding isn't known, so neither is that no claim will wait for one.
        let mut unread = false;
        match self.lib.editor_of(&doc) {
            Ok(Some(e)) if self.names(&e, None) => {
                let r = self.lib.set_editor(&doc, None);
                if let Err(err) = self.recorded(broken, r) {
                    if matches!(err, EditError::Record(_)) && e.contains(':') {
                        stays = Some(e);
                    }
                    first = Some(err);
                }
            }
            Ok(_) => {}
            Err(e) => {
                unread = true;
                first = Some(EditError::Failed(e));
            }
        }
        // Nothing more after a fence; a fence is what is said.
        crate::refuse_if_broken(broken).map_err(|f| match &first {
            Some(EditError::Fenced(e)) => EditError::Fenced(e.clone()),
            _ => EditError::Fenced(f),
        })?;
        match self.end_pending(
            broken,
            &doc,
            |from| self.names(from, None),
            stays.as_deref(),
        ) {
            Ok(None) => {
                if stays.is_some() {
                    first = first.map(|e| {
                        EditError::Record(format!(
                            "{}; the handover record notes that its views left, so no view \
                             waits for them",
                            e.message()
                        ))
                    });
                }
            }
            // Unreadable: no claim can wait on anything in it (said, as it is).
            Ok(Some(why)) => {
                first.get_or_insert(EditError::Failed(why));
            }
            Err(e @ EditError::Fenced(_)) => return Err(e),
            // The handover record couldn't be written (put back): it still says that a view of
            // this binding may answer (an existing wait, or no note of the view the editing
            // record names). The lease is kept locked (R92-01, run 203): no other window takes
            // the editing or changes the canvas meanwhile, and they are told it is held. Past
            // this binding's end the library keeps it (`outlive`) until this is recorded.
            // Or it couldn't be read (run 206, LEAD205-01: `Failed`, its own cause and kind kept,
            // never a put-back): the same, since it may well read later with those entries.
            Err(e) => {
                hold.lease.keep();
                let what = match &first {
                    Some(f) => format!("{}; and {}", f.message(), e.message()),
                    None => e.message(),
                };
                let said = format!(
                    "{what}; so this window keeps the canvas locked until it records that its \
                     views left (no other window can take the editing or change the canvas \
                     meanwhile, and they are told it is held): it tries again whenever it next \
                     lists, opens, renames or exports a canvas, and when it ends"
                );
                return Err(match e {
                    EditError::Record(_) => EditError::Record(said),
                    _ => EditError::Failed(said),
                });
            }
        }
        // Unread (R94-01): kept as a put-back is (the same `outlive` past this binding's end, the
        // same retry), its own cause said (`Failed`): nothing is inferred from a record that
        // wasn't read, and it isn't rewritten unread. Done once the record reads.
        if unread {
            hold.lease.keep();
            return Err(EditError::Failed(format!(
                "{}; so this window keeps the canvas locked until it can read it and record \
                 that its views left (no other window can take the editing or change the canvas \
                 meanwhile, and they are told it is held): it tries again whenever it next \
                 lists, opens, renames or exports a canvas, and when it ends",
                first.map_or_else(String::new, |e| e.message())
            )));
        }
        hold.lease.unkeep();
        first.map_or(Ok(()), Err)
    }

    /// After this binding's end (`release`, `release_waiting`): its lease, if that end left it
    /// kept (the records still say that a view of it may answer: their writes were put back)
    /// or poisoned (`Broken`), outlives it in the library (`Library::owed`, R92-01, run 203).
    /// The canvas stays locked, and every view is told it is held, until the end's cleanup is
    /// recorded (tried again at this process's next use of the library and at its end), or,
    /// fenced, until this process ends. Nothing of this binding's admission comes back.
    pub(crate) fn outlive(self: &Arc<Self>, broken: &crate::Broken) {
        let Ok(lease) = self.lease() else {
            return;
        };
        if !lease.is_kept() && !lease.is_poisoned() {
            return;
        }
        let mut owed = self.lib.owed.lock().unwrap();
        if owed.iter().any(|o| Arc::ptr_eq(&o.lease, &lease)) {
            return;
        }
        let why = self
            .retirement()
            .and_then(Result::err)
            .map_or_else(String::new, |e| e.message());
        // A fenced one writes nothing more: only its lease is kept.
        let cleanup = (!lease.is_poisoned()).then(|| (self.clone(), broken.clone()));
        owed.push(Owed {
            lease,
            cleanup,
            why,
        });
    }

    /// The lease's exclusive lock, waiting for as long as another process holds it (see
    /// `Lease::lock_wait`), checked.
    fn lock_waiting(&self, broken: &crate::Broken) -> Result<Hold, EditError> {
        let lease = self.lease()?;
        lease.lock_wait()?;
        let hold = Hold {
            lease: lease.clone(),
            broken: Some(broken.clone()),
        };
        lease.check(&self.lib).map_err(EditError::Failed)?;
        Ok(hold)
    }

    /// What this binding's end did (`release`, `release_waiting`): `None` until it was done
    /// (also while it waits for the lease).
    pub(crate) fn retirement(&self) -> Option<Result<(), EditError>> {
        self.retired.lock().unwrap().clone()
    }

    /// Owe view `view`'s record cleanup (see [`Editing::release_view`]) from now on, whatever
    /// happens next: registered before anything is checked or locked (R90-03).
    pub(crate) fn depart(&self, view: &str) {
        let mut d = self.departed.lock().unwrap();
        if !d.iter().any(|x| x.view == view) {
            d.push(Departed {
                view: view.to_string(),
                safe: false,
            });
        }
    }

    /// Whether a released view's departure isn't even noted yet (see `Departed::safe`).
    pub(crate) fn departure_unsafe(&self) -> bool {
        self.departed.lock().unwrap().iter().any(|d| !d.safe)
    }

    /// At a poll of this binding's views (no wait for anything; R90-01): a released view's
    /// departure not yet noted is recorded now if the lease can be had at once. What fails stays
    /// owed (and fences this server if it must: `forget_view`).
    pub(crate) fn settle_now(&self, broken: &crate::Broken) {
        // Ended: its own end did that cleanup, and what it still owes is the library's to
        // record again (`outlive`), never a late poll's.
        if !self.departure_unsafe()
            || crate::refuse_if_broken(broken).is_err()
            || self.retirement().is_some()
        {
            return;
        }
        let Ok(lease) = self.lease() else {
            return;
        };
        if !matches!(lease.try_lock_now(), Ok(true)) {
            return;
        }
        let hold = Hold {
            lease: lease.clone(),
            broken: Some(broken.clone()),
        };
        if lease.check(&self.lib).is_ok() {
            let _ = self.settle_departed(broken, &hold.lease.doc);
        }
    }

    /// A view of this binding left (a browser tab's release, a panel mount's): if it was still
    /// to hand over, it is marked ended. If it edits, the record then names only this binding,
    /// as before any of its views opened (its agent still acts for it; no view of it edits until
    /// one takes the editing), never the view that left: the next view to take the editing would
    /// otherwise wait for that view to hand over, which it never can (run 198: the parent's
    /// actual197 Home then reopen waited, with no end, for the released mount). A record naming
    /// another view or binding is left as it is. Under the writer fence, as every lease use; a
    /// record write that fails is put back (or fences this server: `Broken`), as every one.
    ///
    /// The view is released already (its caller revoked it, and registered it: `depart`;
    /// nothing undoes that). `Err` says what of its cleanup isn't done, as it happened (R89-01,
    /// R90): the lease busy, a record unreadable, a write put back (`Record`), or a fence
    /// (`Fenced`, naming the records). A cleanup not done stays owed: done under this binding's
    /// next hold of the lease (which refuses what relies on the editing record until it is) or
    /// at a poll of its views. If the editing record couldn't be rewritten, the handover record
    /// says that the view left (an ended pending entry for it), which every claim of any
    /// binding reads: none waits for it. If neither can be written (both put back), or an
    /// existing wait for the view can't be ended, this window keeps the canvas locked
    /// (`Lease::kept`, not `Broken`: both records are as they were), so no other window claims
    /// meanwhile and each is told the canvas is held; past this binding's end, until recorded
    /// (`Editing::release`, `outlive`). A record that can't be put back fences it (`Broken`).
    pub(crate) fn release_view(&self, broken: &crate::Broken, view: &str) -> Result<(), EditError> {
        self.depart(view);
        crate::refuse_if_broken(broken).map_err(EditError::Fenced)?;
        let hold = self.lock_raw(broken)?;
        match self.settle_departed(broken, &hold.lease.doc)? {
            None => Ok(()),
            Some(partial) => Err(partial),
        }
    }

    /// [`Editing::release_view`] whose lease was busy (another window holds it, past the
    /// ordinary wait: run 201, R90-01): the release stays under way and waits for that window to
    /// let the lease go (its operation ends, or that window does), then does the same cleanup.
    /// Never returns busy: until it can record that the view left, nothing else of this binding
    /// writes (its caller holds the writer fence), and no other window can take the editing
    /// either (it would need the same lease); a claim that wins the lease first is answered by
    /// this cleanup right after it (its wait for the view is ended). Blocks this thread.
    pub(crate) fn release_view_waiting(&self, broken: &crate::Broken) -> Result<(), EditError> {
        crate::refuse_if_broken(broken).map_err(EditError::Fenced)?;
        let hold = self.lock_waiting(broken)?;
        match self.settle_departed(broken, &hold.lease.doc)? {
            None => Ok(()),
            Some(partial) => Err(partial),
        }
    }

    /// Do the record cleanup of every departed view (see `departed`) under the caller's hold of
    /// the lease. Each one done leaves the list; one whose departure is only noted stays
    /// (`safe`) and is said (`Ok(Some)`, the first); the first that can't even be noted stops
    /// this (nothing after it is tried, so no later fence can hide behind it) and is returned.
    fn settle_departed(
        &self,
        broken: &crate::Broken,
        doc: &str,
    ) -> Result<Option<EditError>, EditError> {
        let list = self.departed.lock().unwrap().clone();
        let mut partial = None;
        for d in list {
            match self.forget_view(broken, doc, &d.view)? {
                None => self.departed.lock().unwrap().retain(|x| x.view != d.view),
                Some(why) => {
                    for x in self.departed.lock().unwrap().iter_mut() {
                        if x.view == d.view {
                            x.safe = true;
                        }
                    }
                    partial.get_or_insert(why);
                }
            }
        }
        // Every departure recorded (or noted): the lease kept for one goes as usual again.
        if !self.departure_unsafe()
            && let Ok(lease) = self.lease()
        {
            lease.unkeep();
        }
        Ok(partial)
    }

    /// Departed view `view`'s record cleanup (see [`Editing::release_view`]): the editing record
    /// names only this binding if it named that view, and its pending entry, if any, is ended;
    /// if the editing record couldn't be rewritten (put back), the handover record notes that
    /// the view left (`Ok(Some)`, why it isn't done); if that fails too, fenced.
    fn forget_view(
        &self,
        broken: &crate::Broken,
        doc: &str,
        view: &str,
    ) -> Result<Option<EditError>, EditError> {
        crate::refuse_if_broken(broken).map_err(EditError::Fenced)?;
        let record = self.record(Some(view));
        let named = match self.lib.editor_of(doc) {
            Ok(named) => named,
            // Unread (R94-01, run 205): whether it still names the view isn't known, so neither
            // is that no claim will wait for it. Kept as a put-back is, its own cause said
            // (`Failed`); nothing is written from a record that wasn't read. Done once it reads.
            Err(e) => {
                self.lease()?.keep();
                return Err(EditError::Failed(format!(
                    "{e}; so this window keeps the canvas locked until it can read it and record \
                     that the view left (no other window can take the editing or change the \
                     canvas meanwhile, and they are told it is held): it tries again at this \
                     window's next change or check of the canvas, or its close"
                )));
            }
        };
        let mut kept = None;
        if named.as_deref() == Some(record.as_str()) {
            let r = self.lib.set_editor(doc, Some(&self.record(None)));
            match self.recorded(broken, r) {
                Ok(()) => {}
                Err(EditError::Record(e)) => kept = Some(e),
                Err(e) => return Err(e),
            }
        }
        match self.end_departed(broken, doc, &record, kept.is_some()) {
            Ok(None) => Ok(kept.map(|e| {
                EditError::Record(format!(
                    "{e}; the handover record notes that the view left, so no view waits for it, \
                     and the editing record is rewritten at this window's next change or check \
                     of the canvas, or its close"
                ))
            })),
            Ok(Some(why)) => Ok(Some(EditError::Failed(why))),
            // The handover record couldn't be written either (put back, so it is as it was), or
            // had no room for the note (`no_room`, R93-01): it still says the view may answer
            // (its existing wait, or no note of it while the editing record names it, which a
            // claim could wait for). This window keeps the canvas locked until it records
            // that (run 202, R91-02): no other window can take the editing or change the canvas
            // meanwhile, and every view is told the canvas is held (`Editing::handover`).
            // Or it couldn't be read (run 206, LEAD205-01: `Failed`, its kind kept): the same,
            // since it may well read later with the view's entry in it.
            Err(e2 @ (EditError::Record(_) | EditError::Failed(_))) => {
                self.lease()?.keep();
                let what = match kept {
                    Some(e1) => format!("{e1}; and {}", e2.message()),
                    None => e2.message(),
                };
                let said = format!(
                    "{what}; so this window keeps the canvas locked until it records that the \
                     view left (no other window can take the editing or change the canvas \
                     meanwhile, and they are told it is held): it tries again at this window's \
                     next change or check of the canvas, or its close"
                );
                Err(match e2 {
                    EditError::Record(_) => EditError::Record(said),
                    _ => EditError::Failed(said),
                })
            }
            Err(e) => Err(e),
        }
    }

    /// The pending entries from `record` ended; with `note`, one ended entry for it added if it
    /// has none, so no claim waits for it (claims add no entry for a record that has one). A
    /// handover record seen and unusable (not a plain file, too large, a form this version
    /// doesn't read) is left as it is: no claim can wait on anything in it (`Ok(Some)`, said).
    /// One that couldn't be read (an I/O error) is an error, `Failed` (run 206): it may well read
    /// later, entries and all. A full list has no room for the note: that is not recorded, and is
    /// an error (`no_room`), never said to be safe, since a claim that first removes its own
    /// entry would then have room to wait for the view (R93-01, run 204).
    fn end_departed(
        &self,
        broken: &crate::Broken,
        doc: &str,
        record: &str,
        note: bool,
    ) -> Result<Option<String>, EditError> {
        let mut h = match self.lib.handover_read(doc) {
            Ok(h) => h,
            Err(ReadError::Unusable(e)) => {
                return Ok(Some(format!(
                    "{e}, so it was left as it is (no view can wait for anything in it)"
                )));
            }
            // Not read (run 206, LEAD205-01): what it holds isn't known, so neither is that no
            // claim will wait on an entry in it once it reads. Not recorded, and an error.
            Err(ReadError::Unread(e)) => return Err(EditError::Failed(e)),
        };
        let mut changed = false;
        let mut found = false;
        for p in h.pending.iter_mut().filter(|p| p.from == record) {
            found = true;
            if !p.ended {
                p.ended = true;
                changed = true;
            }
        }
        let full = note && !found && h.pending.len() >= MAX_PENDING;
        if note && !found && !full {
            h.pending.push(Pending {
                id: new_id().map_err(EditError::Failed)?,
                from: record.to_string(),
                since: now_ms(),
                ended: true,
            });
            changed = true;
        }
        if changed {
            #[cfg(test)]
            if note && crate::test_support::fault(&self.lib.locks(), "departure_note") {
                return Err(EditError::Record(
                    "this canvas's handover record couldn't be written (injected failure; it \
                     was put back as it was)"
                        .into(),
                ));
            }
            let r = self.lib.set_handover(doc, &h);
            self.recorded(broken, r)?;
        }
        if full {
            return Err(no_room());
        }
        Ok(None)
    }

    /// Mark the pending entries `mine` names ended, and with `note` (a view the editing record
    /// still names) add an ended entry for it if it has none, so no claim waits for it (as
    /// `end_departed` notes one): one checked record change. A handover record that can't be
    /// read is left as it is (`Ok(Some)`, said): no claim can wait on anything in it. A full
    /// list has no room for the note: the rest is written, and the missing note is an error
    /// (`no_room`), never success (R93-01, run 204).
    fn end_pending(
        &self,
        broken: &crate::Broken,
        doc: &str,
        mine: impl Fn(&String) -> bool,
        note: Option<&str>,
    ) -> Result<Option<String>, EditError> {
        let mut h = match self.lib.handover_read(doc) {
            Ok(h) => h,
            Err(ReadError::Unusable(e)) => {
                return Ok(Some(format!(
                    "{e}, so it was left as it is (no view can wait for anything in it)"
                )));
            }
            // Not read (run 206, LEAD205-01): what it holds isn't known, so neither is that no
            // claim will wait on an entry in it once it reads. Not recorded, and an error.
            Err(ReadError::Unread(e)) => return Err(EditError::Failed(e)),
        };
        let mut changed = false;
        for p in h.pending.iter_mut().filter(|p| !p.ended && mine(&p.from)) {
            p.ended = true;
            changed = true;
        }
        let needed = note.filter(|view| !h.pending.iter().any(|p| p.from == *view));
        let full = needed.is_some() && h.pending.len() >= MAX_PENDING;
        if let Some(view) = needed
            && !full
        {
            h.pending.push(Pending {
                id: new_id().map_err(EditError::Failed)?,
                from: view.to_string(),
                since: now_ms(),
                ended: true,
            });
            changed = true;
        }
        if changed {
            let r = self.lib.set_handover(doc, &h);
            self.recorded(broken, r)?;
        }
        if full {
            return Err(no_room());
        }
        Ok(None)
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
        if h.offers.is_empty() && h.pending.is_empty() && h.resolved.is_empty() {
            return None;
        }
        let mine = |from: &str| view.is_some_and(|v| from == self.record(Some(v)));
        // A view still waited for while the canvas's lease is held exclusively right now (a
        // change under way, or a window keeping it until it records that a view left, R91-02):
        // nothing can be handed over while it is, so the views say so rather than promise an
        // answer (`held`). Read on a handle of its own, never this binding's.
        let held = h.pending.iter().any(|p| !p.ended) && self.lib.lease_held_now(&lease.doc);
        Some(json!({
            "held": held,
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
            // Which kept recoveries the editing view resolved, by offer (R83, run 194).
            "resolved": h.resolved.iter().map(|r| json!({
                "offer": r.offer, "recovery": r.recovery,
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
    /// checked record change, with the receipts of the kept recoveries it resolves (`resolved`,
    /// in the same write, R83); its canvas file goes after (a leftover one is never read).
    /// Nothing if it waits no more.
    pub(crate) fn remove_offer(
        &self,
        hold: &Hold,
        broken: &crate::Broken,
        id: &str,
        resolved: &[String],
    ) -> Result<(), EditError> {
        let doc = &hold.lease.doc;
        let mut h = self.lib.handover_of(doc).map_err(EditError::Failed)?;
        let before = h.offers.len();
        h.offers.retain(|o| o.id != id);
        if h.offers.len() == before {
            return Ok(());
        }
        h.add_receipts(id, resolved);
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

/// That the editing view resolved kept recovery `recovery` (an editor's continuation of kind
/// `recovery`, by its `id`) that offer `offer` carried, by applying, keeping or dismissing it
/// (run 194, R83): the view that handed it over lets its own copy go only on this receipt, never
/// because its offer is merely gone (a claim, an older editor or a full record also remove it).
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Receipt {
    pub offer: String,
    pub recovery: String,
}

/// The most receipts kept (the oldest go first: a view that misses one keeps its copy and hands
/// it over again, so nothing is lost by that).
const MAX_RECEIPTS: usize = 32;

/// The form of the handover record this version writes (run 195, R84-01):
/// `{"format": 2, "handover": {"offers": [...], "pending": [...], "resolved": [...]}}`. The
/// handover of OpenPaperDesign before it (runs 192–194: `{"offers": [...], "pending": [...]}`
/// at the top, receipts optional) is read and written in this form at this version's next
/// record change; nothing of it is dropped. An older OpenPaperDesign needs `offers` at the top,
/// so it finds this one unreadable, and never overwrites one it can't read: its answer, Apply,
/// Keep, Dismiss and stop waiting are refused before anything is written, its claim stands with
/// a note and leaves the record as it is, its view is shown no offer, and its close or a view's
/// release leaves the record too. So a recovery a view kept (R82-01) can't be consumed by an
/// older editor's Apply, Keep or Continue once this version has written the record (an older
/// one's record before that is carried over whole). Any other `format` is a record this version
/// doesn't know: never read as empty, never overwritten.
pub(crate) const HANDOVER_FORMAT: u64 = 2;

/// A canvas's handover record (`<id>.handover`, see [`HANDOVER_FORMAT`]): the offers waiting,
/// the views waited for, and the receipts of kept recoveries resolved (`resolved`; a record from
/// before run 194 has none).
#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct Handover {
    pub offers: Vec<Offer>,
    pub pending: Vec<Pending>,
    pub resolved: Vec<Receipt>,
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
            "format": HANDOVER_FORMAT,
            "handover": {
                "offers": self.offers.iter().map(|o| json!({
                    "id": o.id, "from": o.from, "base": o.base, "canvas": o.canvas,
                    "changed": o.changed, "unfinished": o.unfinished, "at": o.at,
                })).collect::<Vec<_>>(),
                "pending": self.pending.iter().map(|p| json!({
                    "id": p.id, "from": p.from, "since": p.since, "ended": p.ended,
                })).collect::<Vec<_>>(),
                "resolved": self.resolved.iter().map(|r| json!({
                    "offer": r.offer, "recovery": r.recovery,
                })).collect::<Vec<_>>(),
            },
        })
        .to_string()
    }

    /// Record that offer `offer` resolved kept recoveries `recoveries` (bounded, oldest first
    /// out).
    fn add_receipts(&mut self, offer: &str, recoveries: &[String]) {
        for r in recoveries {
            if !self
                .resolved
                .iter()
                .any(|x| x.offer == offer && x.recovery == *r)
            {
                self.resolved.push(Receipt {
                    offer: offer.to_string(),
                    recovery: r.clone(),
                });
            }
        }
        let over = self.resolved.len().saturating_sub(MAX_RECEIPTS);
        self.resolved.drain(..over);
    }

    /// A handover record's content, in this version's form or the older one (see
    /// [`HANDOVER_FORMAT`]); anything else is an error (never overwritten unread).
    fn from_json(s: &str) -> Option<Handover> {
        let v: Value = serde_json::from_str(s).ok()?;
        match v.get("format") {
            None => Self::from_value(&v),
            Some(f) if f.as_u64() == Some(HANDOVER_FORMAT) => Self::from_value(v.get("handover")?),
            Some(_) => None,
        }
    }

    fn from_value(v: &Value) -> Option<Handover> {
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
        // Optional (records before run 194 have none); present, it must read.
        let resolved = match v.get("resolved") {
            None => Vec::new(),
            Some(r) => r
                .as_array()?
                .iter()
                .map(|x| {
                    Some(Receipt {
                        offer: text(x, "offer").filter(|i| valid_id(i))?,
                        recovery: text(x, "recovery").filter(|i| i.len() <= 64)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        };
        Some(Handover {
            offers,
            pending,
            resolved,
        })
    }
}

fn now_ms() -> u64 {
    paper_rs::project::now_ms() as u64
}

/// A departure that needed noting where the handover record has no room for it (it already
/// lists `MAX_PENDING` views): that note was not written (no entry is removed to make room),
/// so the departure isn't recorded; handled as a write that was put back (R93-01, run 204).
fn no_room() -> EditError {
    EditError::Record(format!(
        "this canvas's handover record can't wait for more views (it already lists \
         {MAX_PENDING}), so it couldn't note that the view left"
    ))
}

/// Why a record in the locks folder couldn't be had (`Library::record_read`, run 206,
/// LEAD205-01), from the I/O error kinds themselves, never from a message.
enum ReadError {
    /// What is there was seen, and isn't a record this version uses: not a plain file (a link,
    /// a folder, more than one hard link), larger than a record can be, or (a handover record)
    /// a form it doesn't know or that doesn't parse. It stays so until something replaces it.
    Unusable(String),
    /// It couldn't be looked at or read (an I/O error other than NotFound): what it holds isn't
    /// known, and a later read may well succeed.
    Unread(String),
}

impl ReadError {
    fn message(self) -> String {
        match self {
            ReadError::Unusable(m) | ReadError::Unread(m) => m,
        }
    }
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
