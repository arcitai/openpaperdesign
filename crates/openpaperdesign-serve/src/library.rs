//! The app library (`openpaperdesign-mcp --app-library <dir>`): canvases OpenPaperDesign itself
//! owns, kept in a data folder the host configures for the plugin (Codex's `${PLUGIN_DATA}`).
//!
//! Layout, created by this module only and never named by tool input:
//!
//! ```text
//! <root>/library/v1/canvases/<id>/    one standard paper-rs project per canvas: the live copy
//! <root>/library/v1/locks/<id>.lock   its lease file: permanent, never removed or truncated
//! <root>/library/v1/appearance        the views' appearance preference (system/light/dark)
//! ```
//!
//! `<id>` is 16 lowercase hex digits made by the server. A canvas's name is data in its
//! `project.json`, never part of a path. Nothing here reads a workspace, a folder named by a tool,
//! the process working directory or any Codex state. `$HOME` and `$PLUGIN_ROOT` are read only to
//! refuse them (or anything inside the package) as the root.
//!
//! Ownership across processes: every Codex chat or panel may run its own `openpaperdesign-mcp`
//! on the same root. A canvas is created or opened only while this process holds the exclusive
//! lock on its lease file (`File::try_lock`, nonblocking). The lock is advisory: it coordinates
//! OpenPaperDesign processes and nothing else (not a native paper-rs app, not other programs).
//! It is taken before upstream `Project::open` (whose stale-temp cleanup deletes every matching
//! temporary file) and before any write, and it is released only when the binding that holds it
//! is dropped, after the server was retired (admission closed, started writers and rollback
//! drained). Lease files stay forever, so every process locks the same file.
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
/// The views' kept appearance preference (`<root>/library/v1/appearance`, see
/// [`Library::appearance`]): one of [`APPEARANCES`], nothing else.
const APPEARANCE_FILE: &str = "appearance";
pub(crate) const APPEARANCES: [&str; 3] = ["system", "light", "dark"];
/// The views' other kept view preferences (`<root>/library/v1/view`, see [`Library::view`]):
/// one small JSON record with two keys, the home's file layout and the editor's canvas
/// background. Its own file, so the appearance's stays exactly as it is.
const VIEW_FILE: &str = "view";
const MAX_VIEW_BYTES: u64 = 256;
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
        let mut out = ViewPrefs::default();
        if self.check().is_err() {
            return out;
        }
        let path = self.version_dir().join(VIEW_FILE);
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() && m.len() <= MAX_VIEW_BYTES => {}
            _ => return out,
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            return out;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return out;
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
        out
    }

    /// Keep `home_layout` and/or `background` (each one of its known values; at least one),
    /// with the other key as it is kept now: the whole record is written to a new file beside
    /// it, made durable and renamed over the old one, so a reader sees the old or the new
    /// record, never part of one. Touches no canvas, lease or binding. Two processes keeping
    /// different keys at the same moment can race (the later record wins whole); each record is
    /// still a complete, valid one.
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
        let mut next = self.view();
        if let Some(l) = layout {
            next.home_layout = l;
        }
        if let Some(b) = pattern {
            next.background = b;
        }
        self.ensure_hierarchy()?;
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

/// The content of a new canvas: a checked manifest, its pages (in manifest order) and the
/// image files under `assets/` (file name, bytes).
pub(crate) struct NewDocument {
    pub manifest: Manifest,
    pub pages: Vec<PageData>,
    pub assets: Vec<(String, Vec<u8>)>,
}

impl NewDocument {
    /// An empty canvas with one blank page, as upstream `Store::create` makes it. No design
    /// system, direction or token is applied.
    pub(crate) fn blank(name: &str) -> NewDocument {
        let manifest = Manifest::new(name);
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

/// A held lease on one canvas id: the open lease file with its exclusive lock.
pub struct Lease {
    file: File,
    path: PathBuf,
    id: (u64, u64),
    pub(crate) doc: String,
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
        };
        lease.check(lib).map_err(LeaseError::Failed)?;
        Ok(lease)
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
