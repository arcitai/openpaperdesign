//! Loopback HTTP server for editing one paper-rs project folder in a browser.
//!
//! - Serves the built `openpaperdesign-web` bundle and a small JSON API.
//! - API calls need the per-launch token (`Authorization: Bearer …`); requests with a foreign
//!   `Host` (DNS rebinding) or a cross-origin `Origin`/`Sec-Fetch-Site` are refused. One
//!   exception: a top-level document navigation to `/` from another site (a link or redirect,
//!   such as the host opening the editor URL) gets the static bootstrap page, which holds no
//!   project data. The editor's capability stays in the URL fragment, never sent over HTTP; the
//!   page's own API and MCP calls still need it as the Bearer token (see `top_level_entry`).
//! - Pages are addressed by the ids listed in the project's manifest only; the server builds
//!   every path itself, so no client input reaches the file system as a path.
//! - Symbolic links are refused, fail closed: `project.json`, `pages/`, every page file and
//!   every bundle file must be a real file or directory (and files must have a single hard
//!   link). The project root itself must stay the directory that was opened: on every access
//!   it must still be a real directory with the same canonical path and the same (device,
//!   inode). This is checked before upstream `Project::open` runs (its stale-temp cleanup must
//!   never follow a link out of the project) and again on every read and write. Atomic saves
//!   rename over the page file, so a write never lands outside `pages/`. Residual race: a
//!   directory or link swapped in between a check and the following file operation can still
//!   redirect that read or write.
//! - The web bundle (eight allowlisted files) is checked and read into memory once at startup,
//!   from a folder outside the project; static requests never touch the file system.
//! - Unix only (Linux, macOS): the root identity and hard-link checks use Unix metadata, so the
//!   server refuses to start elsewhere.
//! - Writes are revision-checked: a page's revision (ETag) is a hash of its file **as it is
//!   on disk**, `PUT` needs a matching `If-Match`, and a stale one gets `409 Conflict` with
//!   the file left untouched. Saves write upstream's own page and manifest formats
//!   (`project::format`), so the native app reads them as its own.
//! - The editors load and save the whole canvas (`/api/canvas`): every listed page, the page
//!   list (names, order) and the active page, under one canvas revision that covers the
//!   manifest's exact bytes and every page file. A save names the revision it was based on and
//!   carries the full page list, the active page and the content of each page that changed; it
//!   is one commit of those pages and the manifest, refused unwritten when anything in the
//!   canvas changed since (conservatively: any page, the list or the active page; nothing is
//!   merged). A page removed from the list keeps its file on disk, unreferenced (as upstream's
//!   own project saves do): nothing reads it by id any more, and it isn't erased. The per-page
//!   API (`/api/pages/{id}`) stays as it was.
//!
//! Boundary: only writers that go through this server are serialized and revision-checked.
//! Because the revision is computed from the file on disk, a change made by another writer
//! (e.g. a running native paper-rs) before the browser's next save makes that save fail
//! with 409 instead of overwriting it. Nothing stops the other writer from overwriting the
//! browser's saves, and a write landing between this server's check and its rename is not
//! detected. Run one writer at a time.

pub mod agent;
mod commit;
/// A project design guide's bytes and design values: a pure codec, no I/O (R19–R21 precursor).
pub mod guide;
#[doc(hidden)]
pub mod launcher;
pub(crate) mod library;
/// Test-only: `library.rs` exactly as it was frozen for runs 192 and 193 (the handover of an
/// older OpenPaperDesign process sharing a library canvas), so tests run that old code against
/// records the current one writes (R84-01, run 195). Never part of a build of the product;
/// see `library_compat_tests.rs` for its provenance and hash.
#[cfg(test)]
#[allow(dead_code, unused, clippy::all)]
mod library_192;
mod lifecycle;
#[cfg(test)]
mod test_support;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use paper_rs::model::Document;
use paper_rs::project::format::{page_from_json, page_to_json};
use paper_rs::project::{
    ASSETS_DIR, MANIFEST, Manifest, PAGES_DIR, PageData, PageRef, Project, now_ms,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) const ASSETS: &str = ASSETS_DIR;
/// The MCP endpoint (same listener, same token).
pub const MCP_PATH: &str = "/mcp";

/// Files of the web bundle, by URL path. Nothing else is served from disk. `panel.js` is the
/// in-panel editor's own part; the launcher inlines it (with the glue, `session.js`, `page.js`
/// and `editor.js`) into the home view, and serving it here gives the browser nothing new.
const BUNDLE: &[(&str, &str, &str)] = &[
    ("/", "index.html", "text/html; charset=utf-8"),
    ("/app.js", "app.js", "text/javascript; charset=utf-8"),
    ("/page.js", "page.js", "text/javascript; charset=utf-8"),
    ("/editor.js", "editor.js", "text/javascript; charset=utf-8"),
    ("/panel.js", "panel.js", "text/javascript; charset=utf-8"),
    (
        "/session.js",
        "session.js",
        "text/javascript; charset=utf-8",
    ),
    (
        "/openpaperdesign_web.js",
        "openpaperdesign_web.js",
        "text/javascript; charset=utf-8",
    ),
    (
        "/openpaperdesign_web_bg.wasm",
        "openpaperdesign_web_bg.wasm",
        "application/wasm",
    ),
];

/// Same-origin only; wasm needs `wasm-unsafe-eval`, the page's `<style>` block needs inline
/// styles. Not embeddable in other sites' frames.
const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; \
    base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// Revision of a page whose file does not exist yet (listed in the manifest, never saved).
pub const ABSENT: &str = "\"absent\"";
pub(crate) const MAX_PAGE_BYTES: usize = 64 << 20;
/// The most pages a canvas save may list.
pub(crate) const MAX_PAGES: usize = 1000;
/// The longest page name (characters) a canvas save may give a page.
pub(crate) const MAX_PAGE_NAME: usize = 200;

/// An extra identity check run before every read and write of a bound project, from the
/// browser API and the MCP host alike (the app library's root and lease, see `library`). Once
/// it fails it keeps failing.
pub(crate) type Guard = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// What the browser editor reported to this server: that it loaded the page and its renderer
/// is Ready, and how many of its saves were committed. Never the URL or anything else.
#[derive(Default)]
pub(crate) struct EditorState {
    ready: std::sync::atomic::AtomicBool,
    saves: std::sync::atomic::AtomicU64,
}

pub struct Config {
    /// The project folder (must contain an upstream `project.json`).
    pub project: PathBuf,
    /// Directory with the built web bundle.
    pub web_dir: PathBuf,
    /// The port the listener is bound to (used for the Host/Origin allow-list).
    pub port: u16,
    pub token: String,
}

pub struct Server {
    /// The web bundle, read once at startup: (URL path, content type, bytes). Static files are
    /// never read from disk per request.
    bundle: Bundle,
    /// Canonical project root at startup.
    dir: PathBuf,
    /// (device, inode) of the root at startup.
    root_id: (u64, u64),
    token: String,
    hosts: [String; 2],
    origins: [String; 2],
    /// Serializes revision check + write for every writer through this server: browser saves
    /// and agent calls (the MCP host thread takes the same lock).
    lifecycle: Arc<lifecycle::Lifecycle>,
    /// Tool calls for the MCP host thread.
    calls: agent::Calls,
    /// Set when a failed write couldn't be undone; every later write is refused.
    broken: Broken,
    /// Every page id the host has held, released only after retirement drains its writers.
    held: HeldPages,
    /// The app library's identity check for this project, if it is a library canvas.
    guard: Option<Guard>,
    /// A library canvas's editing (issue #23, see `library`): every write takes its lease and
    /// must be the canvas's editor; a snapshot read takes the lease shared.
    editing: Option<Arc<library::Editing>>,
    /// The browser tabs that opened this binding's editor (`POST /api/view`): each is a view of
    /// its own (`t<n>`), as each in-panel editor mount is (`m<n>`), live until it is released.
    views: Views,
    editor: EditorState,
}

/// The browser views this binding issued (R78-01): numbered, each with its lifetime flag (as a
/// panel mount has), kept after release so a released view is told apart from one never issued.
#[derive(Default)]
struct Views {
    next: std::sync::atomic::AtomicU64,
    issued: std::sync::Mutex<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
}

/// The most browser views one binding issues (tabs opened, reloads included).
const MAX_VIEWS: usize = 4096;

/// A library canvas's browser request names no view (R78-01): it can't change anything.
const VIEW_MISSING: &str = "This editor tab didn't say which view of the canvas it is \
    (X-OPD-View), so it can't change this canvas; it may be from an older OpenPaperDesign. \
    Reload this tab to edit here. Nothing was written.";
const VIEW_MALFORMED: &str = "This editor tab's view (X-OPD-View) isn't one this canvas's \
    editor issues. Reload this tab to edit here. Nothing was written.";
const VIEW_UNISSUED: &str = "This editor tab's view wasn't issued by this canvas's editor (it \
    may be from before OpenPaperDesign restarted or the canvas was reopened). Reload this tab to \
    edit here. Nothing was written.";
const VIEW_RELEASED: &str = "This editor tab was closed (its view was released), so it changes \
    nothing any more. Nothing was written.";

/// Fully loaded but not yet active. Preparation has no effect on the layout registry.
/// The launcher retires its old server before consuming this value.
pub(crate) struct PreparedServer(Arc<Server>);

impl PreparedServer {
    /// Start with fresh layout state for this project's ids. For a replacement, the previous
    /// server must have completed `retire` first. Arbitrary concurrent hosts are unsupported.
    pub(crate) fn activate(self) -> Arc<Server> {
        self.0.lifecycle.activate(&self.0.held);
        self.0
    }
}

/// Page ids a host has held (shared between the server and its host thread).
pub type HeldPages = Arc<std::sync::Mutex<std::collections::HashSet<String>>>;

/// Fail-closed state shared by every writer of one server: once a failed multi-file write
/// could not be rolled back (see `commit`), the message naming the files is kept here and
/// every further write, browser or MCP, is refused with it. Restarting the server after
/// checking those files clears it.
pub type Broken = Arc<std::sync::Mutex<Option<String>>>;

pub(crate) fn set_broken(b: &Broken, why: &str) {
    *b.lock().unwrap() = Some(why.to_string());
}

pub(crate) fn refuse_if_broken(b: &Broken) -> Result<(), String> {
    match b.lock().unwrap().as_ref() {
        Some(files) => Err(format!(
            "writes are disabled: a failed save could not be undone, so these files may be \
             inconsistent: {files}. Check or restore them, then restart the server"
        )),
        None => Ok(()),
    }
}

/// The one write of every save, browser or MCP: `writes` as one commit (`commit::commit`),
/// all of them or, after an error, none. A failure whose rollback failed too fences every later
/// write (`Broken`).
pub(crate) fn commit_fenced(
    b: &Broken,
    writes: &[commit::Write],
) -> Result<(), commit::CommitError> {
    commit::commit(writes).inspect_err(|e| {
        if let commit::CommitError::Broken { files, .. } = e {
            set_broken(b, &files.join(", "));
        }
    })
}

#[derive(Debug)]
pub enum OpenError {
    NotAProject(PathBuf, String),
    Unsafe(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NotAProject(p, e) => {
                write!(f, "{} is not a paper-rs project folder: {e}", p.display())
            }
            OpenError::Unsafe(e) => write!(f, "refusing to serve this project: {e}"),
        }
    }
}

impl std::error::Error for OpenError {}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    File,
    Dir,
}

/// `Ok(true)` if `path` is a real file/dir of the wanted kind, `Ok(false)` if it doesn't
/// exist, and an error for a symlink, the wrong kind, or a file with more than one hard link
/// (which could alias a file outside the project).
pub(crate) fn plain(path: &Path, want: Kind) -> Result<bool, String> {
    plain_seen(path, want).map_err(NotPlain::message)
}

/// Why [`plain_seen`] refused a path: it couldn't be looked at (an I/O error other than
/// NotFound: what is there isn't known), or it was, and isn't what was wanted.
pub(crate) enum NotPlain {
    Unseen(String),
    Seen(String),
}

impl NotPlain {
    pub(crate) fn message(self) -> String {
        match self {
            NotPlain::Unseen(m) | NotPlain::Seen(m) => m,
        }
    }
}

/// [`plain`], saying which kind of refusal it is (run 206, LEAD205-01).
pub(crate) fn plain_seen(path: &Path, want: Kind) -> Result<bool, NotPlain> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let m = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(NotPlain::Unseen(format!("{name}: {e}"))),
    };
    let t = m.file_type();
    if t.is_symlink() {
        return Err(NotPlain::Seen(format!("{name} is a symbolic link")));
    }
    match want {
        Kind::Dir if !t.is_dir() => Err(NotPlain::Seen(format!("{name} is not a directory"))),
        Kind::File if !t.is_file() => Err(NotPlain::Seen(format!("{name} is not a regular file"))),
        Kind::File if links(&m) > 1 => Err(NotPlain::Seen(format!(
            "{name} has more than one hard link"
        ))),
        _ => Ok(true),
    }
}

#[cfg(unix)]
pub(crate) fn links(m: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.nlink()
}

#[cfg(unix)]
pub(crate) fn dev_ino(m: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (m.dev(), m.ino())
}

// Non-Unix: `Server::open` refuses to start, so these never run; they fail closed anyway.
#[cfg(not(unix))]
pub(crate) fn links(_m: &std::fs::Metadata) -> u64 {
    u64::MAX
}

#[cfg(not(unix))]
pub(crate) fn check_root(_dir: &Path, _id: (u64, u64)) -> Result<(), String> {
    Err("only supported on Unix".into())
}

/// The root must still be the directory opened at startup: a real directory (not a link),
/// with the same canonical path (catches swapped ancestors too) and the same (device, inode).
#[cfg(unix)]
pub(crate) fn check_root(dir: &Path, id: (u64, u64)) -> Result<(), String> {
    let m = std::fs::symlink_metadata(dir).map_err(|e| format!("project folder: {e}"))?;
    if m.file_type().is_symlink() || !m.is_dir() {
        return Err("the project folder was replaced (not a real directory any more)".into());
    }
    let now = dir
        .canonicalize()
        .map_err(|e| format!("project folder: {e}"))?;
    if now != dir || dev_ino(&m) != id {
        return Err("the project folder was replaced since the server started".into());
    }
    Ok(())
}

/// Web bundle files compiled into a binary: (file name, bytes).
pub type EmbeddedBundle = &'static [(&'static str, &'static [u8])];

/// The bundle file names a complete build has (see `scripts/build-web.sh`).
pub fn bundle_file_names() -> Vec<&'static str> {
    BUNDLE.iter().map(|(_, f, _)| *f).collect()
}

enum BundleSource {
    Dir(PathBuf),
    Embedded(EmbeddedBundle),
}

/// (URL path, content type, bytes) of each bundle file found at startup.
type Bundle = Vec<(&'static str, &'static str, Vec<u8>)>;

/// The bundle folder's real path (symlinks resolved by the OS), or `None` when it doesn't
/// exist, in which case nothing is served. No lexical guessing: a path that doesn't exist and
/// contains `..` is refused, because `..` after a symlink can't be resolved without it.
fn resolve(path: &Path) -> Result<Option<PathBuf>, String> {
    match path.canonicalize() {
        Ok(p) => Ok(Some(p)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                Err("a web bundle folder that doesn't exist may not contain `..`".into())
            } else {
                Ok(None)
            }
        }
        Err(e) => Err(format!("web bundle folder: {e}")),
    }
}

/// Read the bundle allowlist into memory. The bundle directory must not be (or be inside) the
/// project folder, and every file present must be a plain single-link file. A missing
/// directory or file is not an error: nothing is served for it (404 "build it first").
fn load_bundle(web_dir: &Path, project: &Path) -> Result<Bundle, String> {
    let Some(web) = resolve(web_dir)? else {
        return Ok(Vec::new());
    };
    if web.starts_with(project) {
        return Err("the web bundle folder must not be inside the project folder".into());
    }
    if !plain(&web, Kind::Dir)? {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for &(url, file, mime) in BUNDLE {
        let path = web.join(file);
        if plain(&path, Kind::File)? {
            let bytes = std::fs::read(&path).map_err(|e| format!("{file}: {e}"))?;
            out.push((url, mime, bytes));
        }
    }
    Ok(out)
}

/// The project's own entries must be real: `project.json` (required) and `pages/` (optional).
pub(crate) fn check_project_dir(dir: &Path) -> Result<(), String> {
    if !plain(&dir.join(MANIFEST), Kind::File)? {
        return Err(format!("{MANIFEST} is missing"));
    }
    plain(&dir.join(PAGES_DIR), Kind::Dir)?;
    plain(&dir.join(ASSETS), Kind::Dir)?;
    Ok(())
}

/// A page id that can safely name a file under `pages/`: `[A-Za-z0-9_-]{1,64}` (no
/// separators, dots or absolute paths). Upstream ids are 16 hex digits.
/// A page id's node-id prefix (upstream `<first six characters>:<n>`), case-folded.
pub(crate) fn page_prefix(id: &str) -> String {
    id.chars().take(6).collect::<String>().to_ascii_lowercase()
}

pub(crate) fn valid_page_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Page ids a save may write as one project's pages, checked before anything is written: each
/// a safe file name (`valid_page_id`), none equal to another ignoring case (one file on a
/// case-insensitive file system), none sharing its first six characters with another ignoring
/// case (node ids name their page by those, see `page_prefix`). Shared by the agent's saves
/// and the editors' canvas saves.
pub(crate) fn check_page_ids(ids: &[&str]) -> Result<(), String> {
    if let Some(bad) = ids.iter().find(|id| !valid_page_id(id)) {
        return Err(format!("refusing to save a page with unsafe id {bad:?}"));
    }
    let mut folded = std::collections::HashSet::new();
    if let Some(dup) = ids
        .iter()
        .find(|id| !folded.insert(id.to_ascii_lowercase()))
    {
        return Err(format!(
            "refusing to save: page id {dup:?} differs from another only in case"
        ));
    }
    let mut prefixes = std::collections::HashSet::new();
    if let Some(dup) = ids.iter().find(|id| !prefixes.insert(page_prefix(id))) {
        return Err(format!(
            "refusing to save: page id {dup:?} shares its first six characters with another"
        ));
    }
    Ok(())
}

/// The manifest, read only from a real `project.json`, with every page id checked (and no
/// duplicates) before any page is touched.
pub(crate) fn read_manifest_checked(dir: &Path) -> Result<Manifest, String> {
    read_manifest_bytes_checked(dir).map(|(m, _)| m)
}

/// [`read_manifest_checked`], with the exact bytes it was read from (for the canvas revision).
pub(crate) fn read_manifest_bytes_checked(dir: &Path) -> Result<(Manifest, Vec<u8>), String> {
    let path = dir.join(MANIFEST);
    if !plain(&path, Kind::File)? {
        return Err(format!("{MANIFEST} is missing"));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let m = std::str::from_utf8(&bytes)
        .map_err(|e| e.to_string())
        .and_then(|s| Manifest::from_json(s).map_err(|e| e.to_string()))?;
    let mut seen = std::collections::HashSet::new();
    let mut prefixes = std::collections::HashSet::new();
    for p in &m.pages {
        if !valid_page_id(&p.id) {
            return Err(format!("{MANIFEST} lists an unsafe page id {:?}", p.id));
        }
        // Compared case-folded: on a case-insensitive file system (macOS APFS by default)
        // `CasePage` and `casepage` are the same page file.
        if !seen.insert(p.id.to_ascii_lowercase()) {
            return Err(format!(
                "{MANIFEST} lists page id {:?} twice (page ids must differ in more than case)",
                p.id
            ));
        }
        // Node ids name their page by its first six characters (`<prefix>:<n>`), so two
        // pages sharing them would be ambiguous to every tool.
        if !prefixes.insert(page_prefix(&p.id)) {
            return Err(format!(
                "{MANIFEST} lists page id {:?}, whose first six characters (ignoring case) \
                 are another page's; node ids couldn't tell the pages apart",
                p.id
            ));
        }
    }
    Ok((m, bytes))
}

fn forbidden(e: String) -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, format!("refused: {e}"))
}

impl Server {
    /// Open an existing project folder. Nothing is created: a folder without a valid
    /// `project.json` is refused.
    pub fn open(cfg: Config) -> Result<Arc<Server>, OpenError> {
        #[cfg(not(unix))]
        return Err(OpenError::Unsafe(
            "only supported on Unix (Linux, macOS): the project-root identity and hard-link \
             checks need Unix metadata"
                .into(),
        ));
        #[cfg(unix)]
        Self::open_unix(
            cfg.project,
            BundleSource::Dir(cfg.web_dir),
            cfg.port,
            cfg.token,
            None,
            None,
        )
        .map(PreparedServer::activate)
    }

    /// Like [`Server::open`], with the web bundle compiled into the binary (`files` = file
    /// name and bytes; only the allowlisted names are served). Nothing is read from disk for
    /// static files, at startup or later.
    pub fn open_embedded(
        project: PathBuf,
        files: EmbeddedBundle,
        port: u16,
        token: String,
    ) -> Result<Arc<Server>, OpenError> {
        #[cfg(not(unix))]
        return Err(OpenError::Unsafe(
            "only supported on Unix (Linux, macOS)".into(),
        ));
        #[cfg(unix)]
        Self::prepare_embedded(project, files, port, token).map(PreparedServer::activate)
    }

    /// Validate and load a replacement without changing the active binding's layout state.
    /// The returned server admits no calls until `activate` consumes it.
    pub(crate) fn prepare_embedded(
        project: PathBuf,
        files: EmbeddedBundle,
        port: u16,
        token: String,
    ) -> Result<PreparedServer, OpenError> {
        #[cfg(not(unix))]
        return Err(OpenError::Unsafe(
            "only supported on Unix (Linux, macOS)".into(),
        ));
        #[cfg(unix)]
        Self::open_unix(
            project,
            BundleSource::Embedded(files),
            port,
            token,
            None,
            None,
        )
    }

    /// Like [`Server::prepare_embedded`], for an app-library canvas: `guard` (its library and
    /// lease check, see `library::guard`) runs before upstream `Project::open` and then before
    /// every browser and MCP read and write; `editing` admits every write (see `library`,
    /// "Editing"). Prepared under the lease the open holds.
    pub(crate) fn prepare_library(
        project: PathBuf,
        files: EmbeddedBundle,
        port: u16,
        token: String,
        guard: Guard,
        editing: Arc<library::Editing>,
    ) -> Result<PreparedServer, OpenError> {
        #[cfg(not(unix))]
        return Err(OpenError::Unsafe(
            "only supported on Unix (Linux, macOS)".into(),
        ));
        #[cfg(unix)]
        Self::open_unix(
            project,
            BundleSource::Embedded(files),
            port,
            token,
            Some(guard),
            Some(editing),
        )
    }

    #[cfg(unix)]
    fn open_unix(
        project: PathBuf,
        bundle: BundleSource,
        port: u16,
        token: String,
        guard: Option<Guard>,
        editing: Option<Arc<library::Editing>>,
    ) -> Result<PreparedServer, OpenError> {
        if let Some(g) = &guard {
            g().map_err(OpenError::Unsafe)?;
        }
        let dir = project
            .canonicalize()
            .map_err(|e| OpenError::NotAProject(project.clone(), e.to_string()))?;
        let root_id = std::fs::symlink_metadata(&dir)
            .map(|m| dev_ino(&m))
            .map_err(|e| OpenError::NotAProject(dir.clone(), e.to_string()))?;
        // Before upstream `Project::open`: its stale-temp cleanup deletes files in `pages/`, so
        // that must not be a link to somewhere else.
        check_project_dir(&dir).map_err(OpenError::Unsafe)?;
        // Same check (and stale temp-file cleanup) as when the native app opens a project.
        Project::open(&dir).map_err(|e| OpenError::NotAProject(dir.clone(), e.to_string()))?;
        let bundle = match bundle {
            BundleSource::Dir(web_dir) => load_bundle(&web_dir, &dir).map_err(OpenError::Unsafe)?,
            BundleSource::Embedded(files) => BUNDLE
                .iter()
                .filter_map(|&(url, file, mime)| {
                    files
                        .iter()
                        .find(|(name, _)| *name == file)
                        .map(|(_, bytes)| (url, mime, bytes.to_vec()))
                })
                .collect(),
        };
        let lifecycle = Arc::new(lifecycle::Lifecycle::default());
        let broken = Broken::default();
        let held = HeldPages::default();
        let calls = agent::spawn_host_guarded(
            dir.clone(),
            root_id,
            lifecycle.clone(),
            broken.clone(),
            held.clone(),
            guard.clone(),
            editing.clone(),
        )
        .map_err(|e| OpenError::NotAProject(dir.clone(), e))?;
        let p = port;
        Ok(PreparedServer(Arc::new(Server {
            dir,
            root_id,
            bundle,
            token,
            hosts: [format!("127.0.0.1:{p}"), format!("localhost:{p}")],
            origins: [
                format!("http://127.0.0.1:{p}"),
                format!("http://localhost:{p}"),
            ],
            lifecycle,
            calls,
            broken,
            held,
            guard,
            editing,
            views: Views::default(),
            editor: EditorState::default(),
        })))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Permanently close admission, wait for the current writer (including rollback), and
    /// release this host's page ids. Queued and late HTTP/stdio calls cannot execute, and
    /// browser saves check the same fence. Retained handles stay retired; this is idempotent.
    /// A library canvas's editing record is cleared after that, only if it still names this
    /// binding (`library::Editing::release`): closing a view that isn't editing never takes the
    /// editing from the view that is. If that end couldn't record that every view of this
    /// binding left (a write put back) or this server is fenced, the canvas's lease outlives the
    /// binding, still locked (`library::Editing::outlive`, R92-01): no other window is promised
    /// an answer from a view that ended here.
    pub async fn retire(&self) {
        self.lifecycle.retire(&self.held).await;
        self.calls.stop();
        if let Some(e) = &self.editing {
            if let Err(library::EditError::Busy) = e.release(&self.broken) {
                // Another window holds the lease past the ordinary wait: this end stays under
                // way until it lets it go, then records that every view of this binding ended
                // (run 202, R91-01), off the executor. Its admission is closed already; nothing
                // of this binding runs meanwhile.
                let (e, broken) = (e.clone(), self.broken.clone());
                let _ = tokio::task::spawn_blocking(move || e.release_waiting(&broken)).await;
            }
            e.outlive(&self.broken);
        }
    }

    /// What this binding's end (`retire`) didn't record, as a release says it (`note`, the
    /// condition's `cleanup_status`): `None` when it recorded everything or hasn't ended.
    pub(crate) fn retirement_outcome(&self) -> Option<serde_json::Value> {
        let e = self.editing.as_ref()?;
        let Some(Err(err)) = e.retirement() else {
            return None;
        };
        let a = edit_refusal(err);
        Some(serde_json::json!({
            "note": format!(
                "The canvas was closed here, but recording that its views left didn't \
                 complete: {}.",
                a.msg.trim_end_matches('.')
            ),
            "cleanup_status": a.status.as_u16(),
        }))
    }

    /// Take a library canvas's editing for this binding (an open, or a view's Edit here; see
    /// `library`, "Editing"), under this binding's writer fence (bounded, as a save's start):
    /// a write of this binding under way finishes first, and none is started meanwhile.
    /// Without editing (a workspace project), there is nothing to take.
    /// `view` (a panel mount `m<n>` or a browser tab `t<n>` of this binding) takes it: its saves
    /// are then the only ones of this binding admitted, and every other view of it is read-only
    /// too (issue #23: the newest view edits, within one binding as across processes). Without
    /// a view (an open, an agent's open), the binding takes it, for its next view to take.
    /// `live` is the view's lifetime (a mount's, a tab's), checked again under the fence: a
    /// claim of a view released while it waited never runs (R78-05).
    /// Its outcome keeps the condition (R185 CLAIM contract, the same statuses for the panel and
    /// the browser): `503` the canvas stayed busy (the writer fence or the lease: it couldn't
    /// start, nothing changed, try again) or this window's writes are fenced (`Broken`: nothing
    /// written, the files named); `410` the binding retired or the view was released while it
    /// waited (permanent: nothing changed); `403` a library, lease or folder check failed (the
    /// canvas is fenced); `500` the editing record couldn't be written and was put back (nothing
    /// changed). Once the record names this view the claim stands: `Claimed::note` then says
    /// what of the handover bookkeeping after it didn't happen (fenced too, when it couldn't be
    /// put back), never a refusal.
    pub(crate) async fn claim(
        &self,
        view: Option<&str>,
        live: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<Claimed, ApiError> {
        self.claim_within(SAVE_START, view, live).await
    }

    /// [`Server::claim`] with its own bound on the wait for the writer fence.
    pub(crate) async fn claim_within(
        &self,
        start_within: std::time::Duration,
        view: Option<&str>,
        live: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<Claimed, ApiError> {
        let Some(e) = &self.editing else {
            return Ok(Claimed { note: None });
        };
        let _write = tokio::time::timeout(start_within, self.lifecycle.write.lock())
            .await
            .map_err(|_| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the canvas was busy for too long, so editing wasn't taken here; nothing was \
                     changed. Try again",
                )
            })?;
        self.lifecycle.check_active().map_err(|_| {
            ApiError::new(
                StatusCode::GONE,
                "this canvas was closed here (its editor ended), so editing wasn't taken; nothing \
                 was changed",
            )
        })?;
        check_live(live)?;
        self.check()?;
        let note = e.claim(&self.broken, view).map_err(edit_refusal)?;
        Ok(Claimed { note })
    }

    /// A browser tab of this binding's editor opened (`POST /api/view`): it is issued a view of
    /// its own (`t<n>`, live until released) and, as the newest view, takes the editing (see
    /// [`Server::claim`]). If that is refused now (the canvas busy), the tab is still its own
    /// view, read-only, and says so. `None` when this binding issued all the views it can.
    pub(crate) async fn open_tab(&self) -> Option<String> {
        let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let view = {
            let mut issued = self.views.issued.lock().unwrap();
            if issued.len() >= MAX_VIEWS {
                return None;
            }
            let n = self
                .views
                .next
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                .saturating_add(1);
            let view = format!("t{n}");
            issued.insert(view.clone(), live.clone());
            view
        };
        let _ = self.claim(Some(&view), Some(&live)).await;
        Some(view)
    }

    /// A browser tab left (`POST /api/view/release`, as the page goes away): its view changes
    /// nothing any more (every later request naming it is refused as released), and a save of
    /// it already under way finishes first. Then its record cleanup, as [`Server::release_view`].
    pub(crate) async fn release_tab(&self, view: &str) -> Result<(), ApiError> {
        if let Some(l) = self.views.issued.lock().unwrap().get(view) {
            l.store(false, std::sync::atomic::Ordering::Release);
        }
        self.release_view(view).await
    }

    /// A view of this binding left (a tab's release, a panel mount's; its caller revoked it
    /// already, for good): after any write of it under way (the writer fence), its record
    /// cleanup (`library::Editing::release_view`: it never edits or is waited for again). `Err`
    /// is what of that cleanup didn't happen, with its condition's status (R89-01, run 199):
    /// `503` busy or fenced (`Broken`, naming the record that may be either version), `500` a
    /// record write put back, `403` the canvas or a record no longer checks out. The view stays
    /// released either way; unless fenced, its cleanup is done at this binding's next hold of
    /// the lease. A binding already retired did its own cleanup (`Editing::release`). The lease
    /// held by another window past the ordinary wait is never answered as busy (run 201,
    /// R90-01): the release stays under way, this binding's writer fence held, until that window
    /// lets it go, then records that the view left (`Editing::release_view_waiting`).
    pub(crate) async fn release_view(&self, view: &str) -> Result<(), ApiError> {
        let Some(e) = &self.editing else {
            return Ok(());
        };
        // Owed from now on, whatever happens below (R90-03).
        e.depart(view);
        let write = self.lifecycle.write.clone().lock_owned().await;
        if self.lifecycle.check_active().is_err() {
            // Closed here: that close did (or didn't do) every view's cleanup; said as it was.
            return match e.retirement() {
                Some(Ok(())) => Ok(()),
                Some(Err(why)) => {
                    // Its own condition (status, named files), as that close found it (and,
                    // when it keeps the canvas locked, what records it later: R92-01).
                    let kept = why.message().contains("keeps the canvas locked");
                    let mut a = edit_refusal(why);
                    a.msg = format!(
                        "this canvas was closed here, and recording at that close that its \
                         views left didn't complete ({}){}",
                        a.msg,
                        if kept {
                            ""
                        } else {
                            "; nothing more is recorded from this window"
                        }
                    );
                    Err(a)
                }
                None => Err(ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "this canvas is being closed here; that close records that its views left",
                )),
            };
        }
        if let Err(c) = self.check() {
            return Err(ApiError::new(
                c.status,
                format!(
                    "{}; so nothing more is recorded from this window (a view of another \
                     window that takes the editing may be told to wait for it: Stop waiting there)",
                    c.msg
                ),
            ));
        }
        let done = match e.release_view(&self.broken, view) {
            Err(library::EditError::Busy) => {
                // Another window holds the lease: wait for it, off the executor, with this
                // binding's writer fence moved along (it stays held until the cleanup is done,
                // even if this request goes away), then do the cleanup.
                let (e, broken) = (e.clone(), self.broken.clone());
                tokio::task::spawn_blocking(move || {
                    let _write = write;
                    e.release_view_waiting(&broken)
                })
                .await
                .unwrap_or_else(|j| {
                    Err(library::EditError::Failed(format!(
                        "recording that it left stopped unexpectedly ({j})"
                    )))
                })
            }
            other => other,
        };
        done.map_err(|err| {
            let retry = match &err {
                // Fenced: its message says what follows. Noted: its message says it is safe.
                library::EditError::Fenced(_) => false,
                library::EditError::Record(m) | library::EditError::Failed(m) => {
                    !m.contains("no view waits")
                        && !m.contains("no view can wait")
                        && !m.contains("keeps the canvas locked")
                }
                _ => true,
            };
            let mut a = edit_refusal(err);
            if retry {
                a.msg = format!(
                    "{}; it is done at this window's next change or check of the canvas, or its \
                     close (until then a view of another window that takes the editing may be \
                     told to wait for it: Stop waiting there)",
                    a.msg
                );
            }
            a
        })
    }

    /// At a poll of this binding's views: a released view's departure not yet noted is recorded
    /// now if nothing else is under way (the writer fence free, the lease free at once; no wait,
    /// no retry beyond the views' own polls: R90-01).
    fn settle_departures(&self) {
        let Some(e) = &self.editing else {
            return;
        };
        if !e.departure_unsafe() {
            return;
        }
        let Ok(_write) = self.lifecycle.write.try_lock() else {
            return;
        };
        if self.lifecycle.check_active().is_ok() && self.check().is_ok() {
            e.settle_now(&self.broken);
        }
    }

    /// The browser view a library canvas's request names (R78-01): an issued, live tab, or the
    /// exact refusal (missing, malformed, never issued, released). Every library mutation of the
    /// HTTP API needs one: a missing or bad header is never taken for the binding. A workspace
    /// project has no views (`None`, as before).
    fn tab_of(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<(String, Arc<std::sync::atomic::AtomicBool>)>, ApiError> {
        if self.editing.is_none() {
            return Ok(None);
        }
        let Some(raw) = headers.get(VIEW_HEADER) else {
            return Err(ApiError::new(
                StatusCode::PRECONDITION_REQUIRED,
                VIEW_MISSING,
            ));
        };
        let Some(view) = raw.to_str().ok().filter(|v| valid_view(v)) else {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, VIEW_MALFORMED));
        };
        match self.views.issued.lock().unwrap().get(view) {
            None => Err(ApiError::new(StatusCode::GONE, VIEW_UNISSUED)),
            Some(l) if !l.load(std::sync::atomic::Ordering::Acquire) => {
                Err(ApiError::new(StatusCode::GONE, VIEW_RELEASED))
            }
            Some(l) => Ok(Some((view.to_string(), l.clone()))),
        }
    }

    /// For a read: the issued, live browser view a request names, if any (a read needs none; a
    /// library read without one is told it doesn't edit).
    fn tab_read(&self, headers: &HeaderMap) -> Option<String> {
        self.tab_of(headers).ok().flatten().map(|(v, _)| v)
    }

    /// Whether a library canvas's browser view (none: a request that named none) edits it now;
    /// a workspace project is always edited here.
    fn tab_editing(&self, view: Option<&str>) -> bool {
        match &self.editing {
            None => true,
            Some(e) => view.is_some_and(|v| e.here(Some(v))),
        }
    }

    /// Whether this binding (with `view`, that view of it) may write its canvas now: always for
    /// a workspace project; for a library canvas, whether its record names it (a hint for the
    /// views; every write checks it again under the lease).
    pub(crate) fn editing_here(&self, view: Option<&str>) -> bool {
        self.editing.as_ref().is_none_or(|e| e.here(view))
    }

    /// The editing's view of a write's admission (see [`Server::start_write`]): with `view`,
    /// only that view of this binding may write.
    fn admit_edit(&self, view: Option<&str>) -> Result<Option<library::Hold>, ApiError> {
        let Some(e) = &self.editing else {
            return Ok(None);
        };
        e.admit(&self.broken, view).map(Some).map_err(edit_refusal)
    }

    /// The handover record (issue #23's new-view recovery, see `library::Editing::answer`) as
    /// `view` sees it, for a revision poll: `null` when nothing waits. A released view's
    /// departure still owed is recorded first, if it can be at once (`settle_departures`).
    pub(crate) fn handover_json(&self, view: Option<&str>) -> serde_json::Value {
        self.settle_departures();
        self.editing
            .as_ref()
            .and_then(|e| e.handover(view))
            .unwrap_or(serde_json::Value::Null)
    }

    /// `view`, which lost the editing, answers the handover (see `library::Editing::answer`):
    /// with `offer`, what it has unsaved, checked first (its canvas a whole canvas save, every
    /// page with its content, bounded and parsed as a save is; its continuations bounded JSON
    /// objects); without, that it has nothing. Admitted under the writer fence with its lifetime
    /// (`live`) checked again there (R78-05). Nothing in the canvas changes. Returns the
    /// offer's id.
    pub(crate) async fn answer(
        &self,
        view: &str,
        live: Option<&std::sync::atomic::AtomicBool>,
        offer: Option<library::NewOffer>,
    ) -> Result<Option<String>, ApiError> {
        let Some(e) = &self.editing else {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "only a library canvas hands over between views; nothing was handed over",
            ));
        };
        if let Some(o) = &offer {
            check_offer(o)?;
        }
        let _write = tokio::time::timeout(SAVE_START, self.lifecycle.write.lock())
            .await
            .map_err(|_| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the canvas was busy; the change wasn't handed over and stays in its view",
                )
            })?;
        self.admit()?;
        check_live(live)?;
        self.check()?;
        e.answer(&self.broken, view, offer).map_err(edit_refusal)
    }

    /// The editing `view` applies offer `id` (R78-04): admitted as one write of that view (the
    /// writer fence, its lifetime, its editing under the lease's exclusive lock), the offer is
    /// read under that same hold (so a dismissal or replacement queued before it is seen: `404`,
    /// nothing written), its canvas saved on the revision it was made on (strict: a change
    /// saved since is `409`, nothing written, the offer kept), and then removed from the
    /// handover record, all before the hold goes. Once the save is committed the result says
    /// so (`Applied::revision`), whether or not the offer could be removed after it
    /// (`Applied::left`): a removal that failed leaves the offer waiting (dismiss it), never a
    /// refusal for a change that was written.
    pub(crate) async fn apply_offer(
        &self,
        view: Option<&str>,
        live: Option<&std::sync::atomic::AtomicBool>,
        id: &str,
        resolved: &[String],
    ) -> Result<Applied, ApiError> {
        let e = self.library_editing()?;
        let (write, hold) = self.start_write(SAVE_START, live, view).await?;
        let _write = write;
        let hold = hold.ok_or_else(|| forbidden("no editing hold".into()))?;
        let (offer, canvas) = self.offer_for(e, &hold, id)?;
        let recoveries = offer_guard(&offer, resolved)?;
        let Some(canvas) = canvas.filter(|_| offer.changed) else {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "that view handed over no finished change to apply; nothing was written",
            ));
        };
        let save = parse_canvas_save(&canvas)?;
        let revision = self.commit_canvas(save, &offer.base)?;
        // Committed: nothing below turns it into a refusal.
        let left = e
            .remove_offer(&hold, &self.broken, id, &recoveries)
            .err()
            .map(|e| e.message());
        Ok(Applied { revision, left })
    }

    /// The editing `view` keeps offer `id`'s canvas as a new library canvas (R78-03: a change
    /// that conflicts, or one to keep apart, is recovered whole, never merged into this
    /// canvas), with this canvas's images; then the offer leaves the handover record (as for
    /// Apply, a removal that failed is said, the new canvas stays). Admitted as `apply_offer`
    /// is. Returns the new canvas's id and name.
    pub(crate) async fn keep_offer(
        &self,
        view: Option<&str>,
        live: Option<&std::sync::atomic::AtomicBool>,
        id: &str,
        resolved: &[String],
    ) -> Result<Kept, ApiError> {
        let e = self.library_editing()?;
        let (write, hold) = self.start_write(SAVE_START, live, view).await?;
        let _write = write;
        let hold = hold.ok_or_else(|| forbidden("no editing hold".into()))?;
        let (offer, canvas) = self.offer_for(e, &hold, id)?;
        // The new canvas is only the offer's settled canvas: its kept recoveries go with Keep
        // only to an editor that says it holds them (R83-02).
        let recoveries = offer_guard(&offer, resolved)?;
        let Some(canvas) = canvas else {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "that view handed over no canvas to keep; nothing was created",
            ));
        };
        let doc = self.kept_document(&canvas)?;
        let name = doc.manifest.name.clone();
        let canvas_id = e.keep_as_canvas(&hold, doc).map_err(edit_refusal)?;
        let left = e
            .remove_offer(&hold, &self.broken, id, &recoveries)
            .err()
            .map(|e| e.message());
        Ok(Kept {
            id: canvas_id,
            name,
            left,
        })
    }

    /// The editing `view` dismisses offer `id`, under the same admission (`404` when it waits no
    /// more: nothing else is removed). Its unsaved work stays in the view that handed it over,
    /// if that view is still open.
    pub(crate) async fn dismiss_offer(
        &self,
        view: Option<&str>,
        live: Option<&std::sync::atomic::AtomicBool>,
        id: &str,
        resolved: &[String],
    ) -> Result<(), ApiError> {
        let e = self.library_editing()?;
        let (write, hold) = self.start_write(SAVE_START, live, view).await?;
        let _write = write;
        let hold = hold.ok_or_else(|| forbidden("no editing hold".into()))?;
        let (offer, _) = self.offer_for(e, &hold, id)?;
        let recoveries = offer_guard(&offer, resolved)?;
        e.remove_offer(&hold, &self.broken, id, &recoveries)
            .map_err(edit_refusal)
    }

    /// The editing `view` stops waiting for pending view `id` (it may be hidden, asleep,
    /// disconnected or ended): under the same admission. What that view has stays there, and
    /// is handed over if it answers later.
    pub(crate) async fn stop_waiting(
        &self,
        view: Option<&str>,
        live: Option<&std::sync::atomic::AtomicBool>,
        id: &str,
    ) -> Result<(), ApiError> {
        let e = self.library_editing()?;
        let (write, hold) = self.start_write(SAVE_START, live, view).await?;
        let _write = write;
        let hold = hold.ok_or_else(|| forbidden("no editing hold".into()))?;
        match e.stop_waiting(&hold, &self.broken, id) {
            Ok(true) => Ok(()),
            Ok(false) => Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "that view isn't waited for any more; nothing was changed",
            )),
            Err(e) => Err(edit_refusal(e)),
        }
    }

    fn library_editing(&self) -> Result<&Arc<library::Editing>, ApiError> {
        self.editing.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "only a library canvas hands over between views; nothing was changed",
            )
        })
    }

    /// Offer `id`, under the caller's hold: `404` when it waits no more.
    fn offer_for(
        &self,
        e: &library::Editing,
        hold: &library::Hold,
        id: &str,
    ) -> Result<(library::Offer, Option<String>), ApiError> {
        e.find_offer(hold, id).map_err(edit_refusal)?.ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "that handed-over change isn't waiting any more (it was applied, kept, dismissed \
                 or replaced); nothing was changed",
            )
        })
    }

    /// A new canvas from an offer's whole canvas (every page with its content) and this
    /// canvas's images, named after it ("… (handed over)").
    fn kept_document(&self, canvas: &str) -> Result<library::NewDocument, ApiError> {
        let save = parse_canvas_save(canvas)?;
        let bad =
            |m: &str| ApiError::new(StatusCode::BAD_REQUEST, format!("{m}; nothing was created"));
        let current = self.project()?.manifest;
        let base = save.name.clone().unwrap_or_else(|| current.name.clone());
        let suffix = " (handed over)";
        let keep = library::MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
        let name: String = base.chars().take(keep).collect::<String>() + suffix;
        let mut manifest = Manifest::new(&name);
        let mut pages = Vec::with_capacity(save.pages.len());
        let mut refs = Vec::with_capacity(save.pages.len());
        for p in save.pages {
            let Some(mut page) = p.page else {
                return Err(bad("the handed-over canvas is missing a page's content"));
            };
            page.name = p.name.clone();
            page.doc.name = p.name.clone();
            refs.push(PageRef {
                id: p.id,
                name: p.name,
            });
            pages.push(page);
        }
        manifest.pages = refs;
        manifest.active_page = Some(save.active);
        let assets = read_assets(&self.dir).map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("this canvas's images couldn't be read ({e}); nothing was created"),
            )
        })?;
        Ok(library::NewDocument {
            manifest,
            pages,
            assets,
        })
    }

    /// The shared lease for a snapshot read of a library canvas (no other process's write half
    /// done in it).
    fn read_lease(&self) -> Result<Option<library::Hold>, ApiError> {
        let Some(e) = &self.editing else {
            return Ok(None);
        };
        e.read().map(Some).map_err(|e| {
            let status = match e {
                library::EditError::Failed(_) => StatusCode::FORBIDDEN,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            ApiError::new(status, e.message())
        })
    }

    /// Run a tool call against this project's host, exactly as `/mcp` does (same policy,
    /// lock, save, revision and stale-write check). `session` is the caller's MCP session,
    /// from [`agent::new_session`].
    pub async fn call_tool(
        &self,
        session: u64,
        name: &str,
        args: serde_json::Value,
    ) -> paper_rs::mcp::ToolResult {
        agent::dispatch(&self.calls, session, name.to_string(), args).await
    }

    /// How many bundle files were found at startup (0 = not built).
    pub fn bundle_files(&self) -> usize {
        self.bundle.len()
    }

    /// The bound project's manifest, behind the same boundary as every API read: the root is
    /// still the directory that was bound (a real directory with the same canonical path and
    /// device/inode, so a replaced or linked root is refused) and its own entries are real.
    /// Reading it acknowledges no page.
    pub(crate) fn bound_manifest(&self) -> Result<Manifest, String> {
        check_root(&self.dir, self.root_id)?;
        self.check_guard()?;
        check_project_dir(&self.dir)?;
        read_manifest_checked(&self.dir)
    }

    fn check_guard(&self) -> Result<(), String> {
        self.guard.as_ref().map_or(Ok(()), |g| g())
    }

    /// Root identity, the library guard (for a library canvas) and the project's own entries;
    /// run before every read and write.
    fn check(&self) -> Result<(), ApiError> {
        check_root(&self.dir, self.root_id).map_err(forbidden)?;
        self.check_guard().map_err(forbidden)?;
        check_project_dir(&self.dir).map_err(forbidden)
    }

    /// Whether the browser editor reported its page loaded and Ready, and how many browser
    /// saves were committed. A navigation request never sets these.
    pub(crate) fn editor_state(&self) -> (bool, u64) {
        use std::sync::atomic::Ordering;
        (
            self.editor.ready.load(Ordering::Acquire),
            self.editor.saves.load(Ordering::Acquire),
        )
    }

    /// The saved canvas as a portable file (`library::payload`): one coherent snapshot of the
    /// files on disk, read under this binding's writer fence, so no browser or MCP save is
    /// half-written in it (it waits for one under way). Edits still only in a browser tab are
    /// not on disk and not in it. The fence is released before the text is handed on.
    pub(crate) async fn saved_snapshot(&self) -> Result<library::payload::Exported, String> {
        let _write = self.lifecycle.write.lock().await;
        self.lifecycle.check_active()?;
        let _read = self.read_lease().map_err(|e| e.msg)?;
        check_root(&self.dir, self.root_id)?;
        self.check_guard()?;
        check_project_dir(&self.dir)?;
        library::payload::export(&self.dir)
    }

    fn project(&self) -> Result<Project, ApiError> {
        // Re-read per request: the manifest may change while we run (e.g. pages renamed
        // natively). Not `Project::open`, which also deletes other writers' temp files.
        self.check()?;
        let manifest = read_manifest_checked(&self.dir)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok(Project {
            dir: self.dir.clone(),
            manifest,
        })
    }

    /// The canvas as it is on disk now: the checked manifest with its exact bytes, and the
    /// current bytes of every listed page file (`None` for a listed page without one), in list
    /// order. Same checks as every read.
    fn canvas_files(&self) -> Result<CanvasFiles, ApiError> {
        self.check()?;
        let (manifest, bytes) = read_manifest_bytes_checked(&self.dir)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let project = Project {
            dir: self.dir.clone(),
            manifest,
        };
        let pages = project
            .manifest
            .pages
            .iter()
            .map(|r| read_page(&project, &r.id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CanvasFiles {
            project,
            manifest: bytes,
            pages,
        })
    }
}

/// A refusal of a library canvas's editing as an API error: `423 Locked` when another view
/// edits (nothing was read or written), 503 when the lease stayed busy, 403 when a check failed.
fn edit_refusal(e: library::EditError) -> ApiError {
    let status = match e {
        library::EditError::NotEditing => StatusCode::LOCKED,
        library::EditError::Busy | library::EditError::Fenced(_) => StatusCode::SERVICE_UNAVAILABLE,
        library::EditError::Failed(_) => StatusCode::FORBIDDEN,
        library::EditError::Record(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ApiError::new(status, e.message())
}

/// A claim that took the editing (see [`Server::claim`]); `note`: what of its handover
/// bookkeeping didn't happen after it, if anything.
pub(crate) struct Claimed {
    pub note: Option<String>,
}

/// A view's lifetime (a panel mount's, a browser tab's), checked under the writer fence: one
/// released while its call waited never acts (R78-05).
fn check_live(live: Option<&std::sync::atomic::AtomicBool>) -> Result<(), ApiError> {
    if live.is_some_and(|l| !l.load(std::sync::atomic::Ordering::Acquire)) {
        return Err(ApiError::new(
            StatusCode::GONE,
            "this editor was left (released) before this could start; it was not run and \
             nothing was changed",
        ));
    }
    Ok(())
}

/// An offer as a view sends it, checked before anything is kept: a canvas is a whole canvas
/// save (every page with its content), bounded and parsed as a save is; a change needs one;
/// continuations are bounded JSON objects.
/// Whether continuation `c` is a kept recovery (an editor's `{kind: "recovery", id, …}`, run
/// 193), and its id. The server checks only its kind and id; what it guards is the editors'.
fn recovery_id(c: &serde_json::Value) -> Option<&str> {
    (c.get("kind")?.as_str()? == "recovery")
        .then(|| c.get("id")?.as_str())
        .flatten()
}

/// Why an offer carrying kept recoveries isn't applied, kept or dismissed for an editor that
/// doesn't say it holds them (R83-03, run 194).
const RECOVERY_UNHELD: &str = "this handed-over change carries kept unfinished edits that this \
    editor doesn't keep (an older OpenPaperDesign), so it can't be applied, kept or dismissed \
    here; open the canvas in a newer OpenPaperDesign to continue or dismiss them; nothing was \
    changed";

/// The kept recoveries offer `o` carries, admitted for an action that says it resolves them
/// (`resolved`: applies, keeps or dismisses them, R83): every one must be named, else `426
/// Upgrade Required` and nothing changes (an older editor, which would consume the offer and
/// leave its recoveries nowhere: it doesn't know them). Returns their ids, for receipts.
fn offer_guard(o: &library::Offer, resolved: &[String]) -> Result<Vec<String>, ApiError> {
    let ids: Vec<String> = o
        .unfinished
        .iter()
        .filter_map(recovery_id)
        .map(str::to_string)
        .collect();
    if ids.iter().any(|i| !resolved.contains(i)) {
        return Err(ApiError::new(StatusCode::UPGRADE_REQUIRED, RECOVERY_UNHELD));
    }
    Ok(ids)
}

/// An action's `resolved` list (run 194): the kept recoveries' ids it says it resolves.
pub(crate) fn resolved_of(v: Option<&serde_json::Value>) -> Result<Vec<String>, ApiError> {
    let bad = || {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "`resolved` is a list of up to 16 ids; nothing was changed",
        )
    };
    let Some(v) = v else {
        return Ok(Vec::new());
    };
    let list = v.as_array().ok_or_else(bad)?;
    if list.len() > library::MAX_CONTINUATIONS {
        return Err(bad());
    }
    list.iter()
        .map(|x| {
            x.as_str()
                .filter(|s| !s.is_empty() && s.len() <= 64)
                .map(str::to_string)
                .ok_or_else(bad)
        })
        .collect()
}

fn check_offer(o: &library::NewOffer) -> Result<(), ApiError> {
    let bad = |m: &str| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("{m}; nothing was handed over"),
        )
    };
    if o.canvas.len() > MAX_PAGE_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "this change is too large to hand over; it stays in its view",
        ));
    }
    if o.canvas.is_empty() {
        if o.changed {
            return Err(bad("a finished change comes with its canvas"));
        }
    } else if parse_canvas_save(&o.canvas)?
        .pages
        .iter()
        .any(|p| p.page.is_none())
    {
        return Err(bad("a handed-over canvas carries every page's content"));
    }
    if o.canvas.is_empty() && o.unfinished.is_empty() {
        return Err(bad("there is nothing to hand over"));
    }
    let size: usize = o.unfinished.iter().map(|c| c.to_string().len()).sum();
    if o.unfinished.len() > library::MAX_CONTINUATIONS
        || size > library::MAX_CONTINUATION_BYTES
        || o.unfinished.iter().any(|c| !c.is_object())
    {
        return Err(bad(
            "its unfinished edits are not what an editor hands over",
        ));
    }
    if o.base.is_empty() || o.base.len() > 64 {
        return Err(bad("the revision it was made on is missing"));
    }
    // A kept recovery is handed over on its own (R83, run 194): with a canvas, a change or
    // another unfinished edit, an editor that doesn't know it could apply, keep or take up the
    // rest and drop the whole offer, recovery and all.
    let recoveries = o
        .unfinished
        .iter()
        .filter(|c| recovery_id(c).is_some())
        .count();
    if recoveries > 0 && (!o.canvas.is_empty() || o.changed || recoveries != o.unfinished.len()) {
        return Err(bad(
            "a kept recovery is handed over on its own, without a canvas, change or other edit",
        ));
    }
    Ok(())
}

/// An applied offer (see [`Server::apply_offer`]): the canvas revision written, and why the
/// offer couldn't be removed afterwards, if it couldn't (it then still waits).
pub(crate) struct Applied {
    pub revision: String,
    pub left: Option<String>,
}

/// An offer kept as a new canvas (see [`Server::keep_offer`]).
pub(crate) struct Kept {
    pub id: String,
    pub name: String,
    pub left: Option<String>,
}

/// A canvas's image files (`assets/`, plain files only, bounded), for a canvas made from it.
fn read_assets(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let assets = dir.join(ASSETS_DIR);
    if !plain(&assets, Kind::Dir)? {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&assets).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Err("an image's file name isn't text".into());
        };
        // Not a file, or a commit's temporary file (`.<name>.<pid>-<n>.tmp`).
        if name.starts_with('.') || !plain(&path, Kind::File)? {
            continue;
        }
        if out.len() >= library::MAX_ASSETS {
            return Err("more images than a canvas can have".into());
        }
        let len = std::fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .len();
        if len > library::MAX_ASSET_BYTES {
            return Err(format!("{name} is larger than an image can be"));
        }
        out.push((name, std::fs::read(&path).map_err(|e| e.to_string())?));
    }
    Ok(out)
}

/// See [`Server::canvas_files`].
struct CanvasFiles {
    project: Project,
    manifest: Vec<u8>,
    pages: Vec<Option<Vec<u8>>>,
}

impl CanvasFiles {
    /// The revision of these exact files (see [`canvas_revision`]).
    fn revision(&self) -> String {
        let revs: Vec<String> = self.pages.iter().map(revision).collect();
        let pages: Vec<(&str, &str)> = self
            .project
            .manifest
            .pages
            .iter()
            .zip(&revs)
            .map(|(r, rev)| (r.id.as_str(), rev.as_str()))
            .collect();
        canvas_revision(&self.manifest, &pages)
    }
}

/// The canvas revision: one change detector (like `etag`) for the manifest's exact bytes (its
/// page list, names, order, active page and everything else) together with each listed page's
/// id and revision, in list order. Any change to any of them changes it.
fn canvas_revision(manifest: &[u8], pages: &[(&str, &str)]) -> String {
    etag(&serde_json::to_vec(&(etag(manifest), pages)).expect("strings serialize"))
}

/// A random per-launch API token (32 bytes, hex).
pub fn new_token() -> Result<String, getrandom::Error> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Revision of page file bytes: 64-bit SipHash (std `DefaultHasher`, fixed keys) of the
/// exact bytes. A change detector for honest writers, not a cryptographic digest.
pub fn etag(bytes: &[u8]) -> String {
    let mut h = std::hash::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("\"{:016x}\"", h.finish())
}

fn error(status: StatusCode, msg: &str) -> Response {
    ApiError::new(status, msg).into_response()
}

/// An API failure: status, message and (for 409) the page's current revision. The in-panel
/// page tools (`launcher_app.rs`) report the same three.
pub(crate) struct ApiError {
    status: StatusCode,
    msg: String,
    etag: Option<String>,
}

impl ApiError {
    fn new(status: StatusCode, msg: impl Into<String>) -> ApiError {
        ApiError {
            status,
            msg: msg.into(),
            etag: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.msg }).to_string();
        let mut r = (
            self.status,
            [(header::CONTENT_TYPE, "application/json")],
            Body::from(body),
        )
            .into_response();
        if let Some(e) = self.etag.and_then(|e| HeaderValue::from_str(&e).ok()) {
            r.headers_mut().insert(header::ETAG, e);
        }
        r
    }
}

fn header_str(h: &HeaderMap, name: impl header::AsHeaderName) -> Option<&str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The one request from another site that is let through: the browser's top-level navigation
/// to the editor page itself, as when a link or redirect on the host's own site (its open-link)
/// opens the editor URL. Only `GET /`, marked by the browser as a navigation
/// (`Sec-Fetch-Mode: navigate`) of a top-level document (`Sec-Fetch-Dest: document`) from a
/// same-site or cross-site page. It returns the static bootstrap page only: no project data,
/// file operation or state. The editor's capability is the URL fragment (`#token=…`), which the
/// browser never sends, so nothing here checks or needs it; every API and MCP call still needs
/// it as a Bearer token from the page's own origin. Frames, fetches, subresources, other paths
/// and methods stay refused, and a foreign `Origin` or `Host` is refused before this.
fn top_level_entry(req: &Request) -> bool {
    let h = req.headers();
    req.method() == axum::http::Method::GET
        && req.uri().path() == "/"
        && header_str(h, "sec-fetch-mode") == Some("navigate")
        && header_str(h, "sec-fetch-dest") == Some("document")
        && matches!(
            header_str(h, "sec-fetch-site"),
            Some("same-site" | "cross-site")
        )
}

/// Host / Origin / fetch-site checks for every request, token check for `/api/`.
async fn guard(State(s): State<Arc<Server>>, req: Request, next: Next) -> Response {
    let h = req.headers();
    match header_str(h, header::HOST) {
        Some(host) if s.hosts.iter().any(|a| a == host) => {}
        _ => return error(StatusCode::FORBIDDEN, "unexpected Host"),
    }
    if let Some(origin) = h.get(header::ORIGIN)
        && !s.origins.iter().any(|a| origin == a.as_str())
    {
        return error(StatusCode::FORBIDDEN, "cross-origin request");
    }
    if let Some(site) = header_str(h, "sec-fetch-site")
        && !matches!(site, "same-origin" | "none")
        && !top_level_entry(&req)
    {
        return error(StatusCode::FORBIDDEN, "cross-site request");
    }
    let path = req.uri().path();
    if path.starts_with("/api/") || path == MCP_PATH || path.starts_with("/mcp/") {
        let ok = header_str(h, header::AUTHORIZATION)
            .and_then(|v| v.strip_prefix("Bearer "))
            .is_some_and(|t| same_bytes(t.as_bytes(), s.token.as_bytes()));
        if !ok {
            return error(StatusCode::UNAUTHORIZED, "missing or wrong token");
        }
    }
    if let Err(e) = s.lifecycle.check_active() {
        return error(StatusCode::SERVICE_UNAVAILABLE, &e);
    }
    next.run(req).await
}

/// On every response, including refusals from `guard`, unmatched paths and body-limit
/// errors.
async fn security_headers(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    res
}

/// Streamable HTTP clients must accept JSON and SSE; add the header when a client omits it
/// (as upstream's MCP server does).
async fn fix_accept(mut req: Request, next: Next) -> Response {
    if req.method() == axum::http::Method::POST {
        let accept = header_str(req.headers(), header::ACCEPT).unwrap_or("");
        if !(accept.contains("application/json") && accept.contains("text/event-stream")) {
            req.headers_mut().insert(
                header::ACCEPT,
                HeaderValue::from_static("application/json, text/event-stream"),
            );
        }
    }
    next.run(req).await
}

pub fn router(s: Arc<Server>) -> Router {
    let calls = s.calls.clone();
    let config = StreamableHttpServerConfig::default()
        .with_allowed_origins(s.origins.clone())
        .enforce_origin_validation()
        .with_max_request_body_bytes(MAX_PAGE_BYTES);
    let mcp: StreamableHttpService<agent::AgentServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(agent::AgentServer::new(calls.clone())),
            Default::default(),
            config,
        );
    let mut r = Router::new()
        .route("/api/project", get(get_project))
        .route("/api/pages/{id}", get(get_page).put(put_page))
        .route("/api/pages/{id}/revision", get(get_revision))
        .route("/api/canvas", get(get_canvas).put(put_canvas))
        .route("/api/canvas/revision", get(get_canvas_revision))
        .route("/api/ready", axum::routing::post(post_ready))
        .route("/api/claim", axum::routing::post(post_claim))
        .route("/api/view", axum::routing::post(post_view))
        .route("/api/view/release", axum::routing::post(post_view_release))
        .route("/api/draft", axum::routing::post(post_draft))
        .route("/api/draft/apply", axum::routing::post(post_draft_apply))
        .route("/api/draft/keep", axum::routing::post(post_draft_keep))
        .route("/api/draft/stop", axum::routing::post(post_draft_stop))
        .route(
            "/api/draft/dismiss",
            axum::routing::post(post_draft_dismiss),
        )
        .nest_service(
            MCP_PATH,
            tower::ServiceBuilder::new()
                .layer(middleware::from_fn(fix_accept))
                .service(mcp),
        );
    for &(url, file, mime) in BUNDLE {
        r = r.route(
            url,
            get(move |State(s): State<Arc<Server>>| async move {
                match s.bundle.iter().find(|(u, _, _)| *u == url) {
                    Some((_, _, b)) => ([(header::CONTENT_TYPE, mime)], b.clone()).into_response(),
                    None => error(
                        StatusCode::NOT_FOUND,
                        &format!("web bundle file {file} not found; build it first"),
                    ),
                }
            }),
        );
    }
    r.fallback(|| async { error(StatusCode::NOT_FOUND, "not found") })
        .layer(middleware::from_fn_with_state(s.clone(), guard))
        .layer(DefaultBodyLimit::max(MAX_PAGE_BYTES))
        .layer(middleware::map_response(security_headers))
        .with_state(s)
}

async fn get_project(State(s): State<Arc<Server>>) -> Response {
    let p = match s.project() {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let m = &p.manifest;
    let body = serde_json::json!({
        "name": m.name,
        "pages": m.pages.iter().map(|r| serde_json::json!({"id": r.id, "name": r.name})).collect::<Vec<_>>(),
        "active_page": m.active_page,
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// The browser editor's own report that it loaded its page and its renderer is Ready (token
/// and origin checked like every API call). It carries nothing and changes no file.
async fn post_ready(State(s): State<Arc<Server>>) -> Response {
    match s.mark_ready() {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

/// How long a save may wait for the writer fence before it is given up, unrun (as for MCP
/// calls, see `agent`).
const SAVE_START: std::time::Duration = std::time::Duration::from_secs(120);

/// One page as a reader gets it: its manifest identity, the canvas name, its revision and its
/// file bytes (a listed page without a file reads as upstream's empty page).
pub(crate) struct PageRead {
    pub id: String,
    pub name: String,
    pub project: String,
    pub revision: String,
    pub bytes: Vec<u8>,
}

/// The whole canvas as the editors load it (see [`Server::canvas`]).
pub(crate) struct CanvasRead {
    pub project: String,
    pub revision: String,
    pub active_page: String,
    pub pages: Vec<PageRead>,
}

impl CanvasRead {
    /// The editors' canvas envelope, around upstream's own page files: `{canvas: {name},
    /// revision, active_page, pages: [{id, name, revision, text}]}`, the pages in list order,
    /// each `text` an upstream page file. A page file that isn't UTF-8 text is refused (422):
    /// it can't be edited here, and nothing is changed.
    pub(crate) fn to_json(&self) -> Result<serde_json::Value, ApiError> {
        let mut pages = Vec::with_capacity(self.pages.len());
        for p in &self.pages {
            let text = std::str::from_utf8(&p.bytes).map_err(|_| {
                ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!(
                        "page {:?}'s file isn't UTF-8 text, so the canvas can't be edited \
                         here; nothing was changed",
                        p.id
                    ),
                )
            })?;
            pages.push(serde_json::json!({
                "id": p.id,
                "name": p.name,
                "revision": p.revision,
                "text": text,
            }));
        }
        Ok(serde_json::json!({
            "canvas": {"name": self.project},
            "revision": self.revision,
            "active_page": self.active_page,
            "pages": pages,
        }))
    }
}

/// The manifest's active page: the one it names if that is listed, else its first page.
fn active_id(m: &Manifest) -> Result<String, ApiError> {
    m.active_page
        .clone()
        .filter(|a| m.pages.iter().any(|r| &r.id == a))
        .or_else(|| m.pages.first().map(|r| r.id.clone()))
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "this canvas has no pages"))
}

/// A listed page without a file, as upstream `Project::load_pages` reads it: empty.
fn empty_page(id: &str, name: &str) -> Vec<u8> {
    page_to_json(&PageData {
        id: id.to_string(),
        name: name.to_string(),
        doc: Document {
            name: name.to_string(),
            ..Default::default()
        },
        camera: None,
    })
    .into_bytes()
}

/// A page name a canvas save may set: 1 to `MAX_PAGE_NAME` characters, without leading or
/// trailing spaces (upstream's rename trims) and without control characters. A listed page's
/// unchanged name is kept whatever it is.
fn check_page_name(name: &str) -> Result<(), String> {
    let n = name.chars().count();
    if n == 0 || n > MAX_PAGE_NAME || name.trim() != name || name.chars().any(char::is_control) {
        return Err(format!(
            "a page name has 1 to {MAX_PAGE_NAME} characters, without control characters or \
             leading and trailing spaces"
        ));
    }
    Ok(())
}

/// A file name a canvas save may set, by the app library's rename rule: 1 to
/// `library::MAX_NAME_CHARS` characters after trimming, without control characters. Returns
/// the trimmed name.
fn check_canvas_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty()
        || n.chars().count() > library::MAX_NAME_CHARS
        || n.chars().any(char::is_control)
    {
        return Err(format!(
            "a file name has 1 to {} characters, without control characters",
            library::MAX_NAME_CHARS
        ));
    }
    Ok(n.to_string())
}

/// A canvas save, checked (see [`parse_canvas_save`]).
struct CanvasSave {
    active: String,
    pages: Vec<SavedPage>,
    /// The file's new name, when the save renames it (the editor's title).
    name: Option<String>,
}

/// One page of a canvas save: its id and name, and its content when it changed.
struct SavedPage {
    id: String,
    name: String,
    page: Option<PageData>,
}

/// A canvas save as the editors send it: `{active_page, pages: [{id, name, text?}], name?}`, the
/// whole page list in its new order, with `text` (an upstream page file) for each page whose
/// content is saved, and the file's new `name` when the editor renamed it (an older editor
/// never sends it, and its saves keep the name as it is). Only those fields; 1 to `MAX_PAGES`
/// pages; ids checked as one project's (`check_page_ids`); the active page one of them; each
/// page file parsed and validated by upstream (`page_from_json`: format, version, node graph)
/// and naming its own id; a name by `check_canvas_name`. Checked completely, with no file
/// touched, before a save takes the writer fence.
fn parse_canvas_save(body: &str) -> Result<CanvasSave, ApiError> {
    use serde_json::Value;
    let bad = |msg: String| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("{msg}; nothing was written"),
        )
    };
    let v: Value =
        serde_json::from_str(body).map_err(|e| bad(format!("the canvas save isn't JSON ({e})")))?;
    let obj = v
        .as_object()
        .ok_or_else(|| bad("the canvas save must be an object".into()))?;
    if let Some(k) = obj
        .keys()
        .find(|k| !matches!(k.as_str(), "active_page" | "pages" | "name"))
    {
        return Err(bad(format!(
            "the canvas save has an unexpected field `{k}`"
        )));
    }
    let name = match obj.get("name") {
        None => None,
        Some(Value::String(n)) => Some(check_canvas_name(n).map_err(bad)?),
        Some(_) => return Err(bad("the file's `name` must be text".into())),
    };
    let list = obj
        .get("pages")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("the canvas save needs its page list (`pages`)".into()))?;
    if list.is_empty() || list.len() > MAX_PAGES {
        return Err(bad(format!(
            "a canvas has 1 to {MAX_PAGES} pages, not {}",
            list.len()
        )));
    }
    let mut entries = Vec::with_capacity(list.len());
    for e in list {
        let e = e
            .as_object()
            .ok_or_else(|| bad("each page of the list must be an object".into()))?;
        if let Some(k) = e
            .keys()
            .find(|k| !matches!(k.as_str(), "id" | "name" | "text"))
        {
            return Err(bad(format!("a page has an unexpected field `{k}`")));
        }
        let id = e
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("each page needs its `id`".into()))?;
        let name = e
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(format!("page {id:?} needs its `name`")))?;
        let text = match e.get("text") {
            None => None,
            Some(Value::String(t)) => Some(t.as_str()),
            Some(_) => return Err(bad(format!("page {id:?}: `text` must be a page file"))),
        };
        entries.push((id, name, text));
    }
    let ids: Vec<&str> = entries.iter().map(|(id, _, _)| *id).collect();
    check_page_ids(&ids).map_err(bad)?;
    let active = obj
        .get("active_page")
        .and_then(Value::as_str)
        .filter(|a| ids.contains(a))
        .ok_or_else(|| bad("`active_page` must be one of the listed pages".into()))?;
    let mut pages = Vec::with_capacity(entries.len());
    for (id, name, text) in entries {
        let page = match text {
            None => None,
            Some(t) => {
                let page = page_from_json(t).map_err(|e| {
                    ApiError::new(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("page {id:?}: {e}; nothing was written"),
                    )
                })?;
                if page.id != id {
                    return Err(bad(format!(
                        "page {id:?}: the page file names another page"
                    )));
                }
                Some(page)
            }
        };
        pages.push(SavedPage {
            id: id.to_string(),
            name: name.to_string(),
            page,
        });
    }
    Ok(CanvasSave {
        active: active.to_string(),
        pages,
        name,
    })
}

/// The page and canvas operations shared by the browser's HTTP API and the app's in-panel
/// tools (`launcher_app.rs`). Each one checks admission (an active binding) itself, so no
/// caller relies on the HTTP middleware for it; the reads and the saves also check the root
/// identity, the library guard and the project's own entries. `mark_ready` reads and writes no
/// file and checks only admission (the in-panel caller re-checks the canvas, library and lease
/// first). HTTP keeps its own Host/Origin/token checks in front of these.
impl Server {
    fn admit(&self) -> Result<(), ApiError> {
        self.lifecycle
            .check_active()
            .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))
    }

    /// Page `id` of the manifest, or with `None` the manifest's active page (its first page
    /// when none is set or the set one isn't listed), as the browser editor picks it.
    pub(crate) fn page(&self, id: Option<&str>) -> Result<PageRead, ApiError> {
        self.admit()?;
        let p = self.project()?;
        let id = match id {
            Some(id) => id.to_string(),
            None => active_id(&p.manifest)?,
        };
        let name = page_ref(&p, &id)?;
        let bytes = read_page(&p, &id)?;
        let rev = revision(&bytes);
        Ok(PageRead {
            bytes: bytes.unwrap_or_else(|| empty_page(&id, &name)),
            id,
            name,
            project: p.manifest.name.clone(),
            revision: rev,
        })
    }

    /// The whole canvas as the editors load it: every listed page (as [`Server::page`] reads
    /// each), the active page and the canvas revision of exactly those files, read as one
    /// snapshot under the writer fence (it waits, boundedly, for a save or agent call under way,
    /// so nothing in it is half-written). The page files together are bounded by
    /// `MAX_PAGE_BYTES`: a larger canvas isn't opened in the editors (its pages still are, one
    /// at a time, through the per-page API). Reading acknowledges nothing and changes nothing.
    pub(crate) async fn canvas(&self) -> Result<CanvasRead, ApiError> {
        let _write = tokio::time::timeout(SAVE_START, self.lifecycle.write.lock())
            .await
            .map_err(|_| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the canvas could not be read in time (it was busy); try again",
                )
            })?;
        self.admit()?;
        // Across processes too: no other OpenPaperDesign process's write half done in it.
        let _read = self.read_lease()?;
        let f = self.canvas_files()?;
        let revision_now = f.revision();
        let m = &f.project.manifest;
        let active_page = active_id(m)?;
        let mut total = 0usize;
        let mut pages = Vec::with_capacity(m.pages.len());
        for (r, bytes) in m.pages.iter().zip(f.pages) {
            let rev = revision(&bytes);
            let bytes = bytes.unwrap_or_else(|| empty_page(&r.id, &r.name));
            total = total.saturating_add(bytes.len());
            if total > MAX_PAGE_BYTES {
                return Err(ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!(
                        "this canvas's pages are larger than {MAX_PAGE_BYTES} bytes together, \
                         more than its editor opens at once; nothing was changed"
                    ),
                ));
            }
            pages.push(PageRead {
                id: r.id.clone(),
                name: r.name.clone(),
                project: m.name.clone(),
                revision: rev,
                bytes,
            });
        }
        Ok(CanvasRead {
            project: m.name.clone(),
            revision: revision_now,
            active_page,
            pages,
        })
    }

    /// The canvas revision now (cheap polling; see [`canvas_revision`]). Read without the
    /// writer fence: during a commit it may describe files of two moments, which matches no
    /// state at all, so the editor that polls only reloads (under the fence) or gets a
    /// conflict; it is never taken for a newer state that was saved.
    pub(crate) fn canvas_revision(&self) -> Result<String, ApiError> {
        self.admit()?;
        Ok(self.canvas_files()?.revision())
    }

    /// The current revision of page `id` (cheap polling).
    pub(crate) fn page_revision(&self, id: &str) -> Result<String, ApiError> {
        self.admit()?;
        let p = self.project()?;
        page_ref(&p, id)?;
        Ok(revision(&read_page(&p, id)?))
    }

    /// Save page `id` from `body` (an upstream page file), only if `if_match` is its revision
    /// on disk now (the per-page API, kept as it was; the editors save the whole canvas, see
    /// [`Server::save_canvas_within`]): the page and the manifest (its `modified` time only;
    /// the page list, names, order, active page, other pages and assets as they are on disk
    /// now) are one commit, rolled back on any error. A
    /// failed rollback fences every later write (`Broken`). The wait for the writer fence is
    /// bounded (`SAVE_START`): a save that couldn't start, or whose caller stopped waiting
    /// before it started, never runs. Once it holds the fence nothing awaits any more, so a
    /// started save always finishes its commit or rollback, and retirement waits for it.
    /// Returns the revision of the bytes written. (The routes call [`Server::save_page_within`]
    /// with the requesting view; this is the same save without one.)
    #[cfg(test)]
    pub(crate) async fn save_page(
        &self,
        id: &str,
        if_match: &str,
        body: &str,
    ) -> Result<String, ApiError> {
        self.save_page_within(SAVE_START, None, None, id, if_match, body)
            .await
    }

    /// [`Server::save_page`] for an editor whose lifetime can be revoked (`live`, an in-panel
    /// editor's mount): it is checked again under the writer fence, so a save of an editor
    /// that was released before the save started never runs. Revoke it, then
    /// [`Server::drain_writer`], to know no save of that editor is still under way. `view` is
    /// that editor's own view (`m<n>`): only the view that has the editing saves.
    pub(crate) async fn save_page_while(
        &self,
        live: &std::sync::atomic::AtomicBool,
        view: &str,
        id: &str,
        if_match: &str,
        body: &str,
    ) -> Result<String, ApiError> {
        self.save_page_within(SAVE_START, Some(live), Some(view), id, if_match, body)
            .await
    }

    /// Wait until no writer (a started save, its commit or rollback, or an agent call) holds
    /// the writer fence any more. Changes nothing.
    pub(crate) async fn drain_writer(&self) {
        drop(self.lifecycle.write.lock().await);
    }

    /// [`Server::save_page`] with its own bound on the wait for the writer fence, an optional
    /// revocable editor lifetime and an optional view of its own (see
    /// [`Server::save_page_while`]; a browser tab sends its view too).
    pub(crate) async fn save_page_within(
        &self,
        start_within: std::time::Duration,
        live: Option<&std::sync::atomic::AtomicBool>,
        view: Option<&str>,
        id: &str,
        if_match: &str,
        body: &str,
    ) -> Result<String, ApiError> {
        if body.len() > MAX_PAGE_BYTES {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("the page is larger than {MAX_PAGE_BYTES} bytes; nothing was written"),
            ));
        }
        let (write, hold) = self.start_write(start_within, live, view).await?;
        // Declared in this order so the editing's hold goes first, after the commit.
        let _write = write;
        let _hold = hold;
        let mut p = self.project()?;
        let name = page_ref(&p, id)?;
        // Strict: the save must be based on the revision on disk now (whoever wrote it: the
        // agent through this server, or anything else). A stale one is refused, unwritten.
        let current = revision(&read_page(&p, id)?);
        if if_match != current {
            return Err(ApiError {
                etag: Some(current),
                ..ApiError::new(
                    StatusCode::CONFLICT,
                    "the page changed since it was loaded; nothing was written",
                )
            });
        }
        let mut page = page_from_json(body)
            .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
        if page.id != id {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "page id in the body does not match the URL or page_id",
            ));
        }
        // The manifest owns page names (as in upstream `Project::load_page`).
        page.name = name.clone();
        page.doc.name = name;
        // Checked again just before writing (`read_page` above checked the page file).
        self.check()?;
        refuse_if_broken(&self.broken)
            .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
        // Like the native session: a page write bumps the project's modified time. The page
        // and the manifest are written as one commit: on an error neither changed.
        p.manifest.modified = now_ms();
        let writes = [
            commit::Write {
                path: p.page_path(&page.id),
                data: page_to_json(&page).into_bytes(),
            },
            commit::Write {
                path: p.dir.join(MANIFEST),
                data: p.manifest.to_json().into_bytes(),
            },
        ];
        commit_fenced(&self.broken, &writes)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.describe()))?;
        // Committed: the revision is the hash of the bytes just written. Nothing that can
        // fail comes after the commit, so a saved page is never reported as not saved.
        let rev = etag(&writes[0].data);
        self.editor
            .saves
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(rev)
    }

    /// The start of every browser and in-panel save: the bounded wait for the writer fence
    /// (`start_within`; a save that couldn't start, or whose caller stopped waiting before it
    /// started, never runs), then, under the fence, admission again (a close may have retired
    /// the binding meanwhile) and the editor's own lifetime (`live`: it may have been released
    /// meanwhile). Once this returns the save has started: its caller awaits nothing more, so
    /// it runs to its commit or rollback, and retirement and a release (`drain_writer`) wait
    /// for it. For a library canvas the write is then admitted by its editing (`admit_edit`:
    /// the lease's exclusive lock across processes, and this binding must still be the
    /// canvas's editor, and with `view`, exactly that view of it; else `423 Locked`, nothing read
    /// or written), held until the caller's commit or rollback is done (the returned hold,
    /// dropped first).
    async fn start_write(
        &self,
        start_within: std::time::Duration,
        live: Option<&std::sync::atomic::AtomicBool>,
        view: Option<&str>,
    ) -> Result<(tokio::sync::MutexGuard<'_, ()>, Option<library::Hold>), ApiError> {
        #[cfg(test)]
        test_support::hit(&self.dir, "browser_queued");
        let write = tokio::time::timeout(start_within, self.lifecycle.write.lock())
            .await
            .map_err(|_| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the save could not start in time (the canvas was busy); it was not run and \
                     nothing was written",
                )
            })?;
        self.admit()?;
        if live.is_some_and(|l| !l.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(ApiError::new(
                StatusCode::GONE,
                "this editor was left (released) before its save could start; it was not run \
                 and nothing was written",
            ));
        }
        // Fenced writes are refused as every save refuses them (`503`, the files named), before
        // the lease (which a fenced binding keeps, `Lease::poison`) is asked.
        refuse_if_broken(&self.broken)
            .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
        let hold = self.admit_edit(view)?;
        #[cfg(test)]
        test_support::hit(&self.dir, "save_started");
        Ok((write, hold))
    }

    /// Save the canvas from `body` (a canvas save, see [`parse_canvas_save`]), only if
    /// `if_match` is the canvas revision on disk now. Like [`Server::save_page`]: the same
    /// bounded start, fence, checks, commit, rollback and `Broken` rule. Returns the canvas
    /// revision of what was written. (As `save_page`, without a view; the routes name theirs.)
    #[cfg(test)]
    pub(crate) async fn save_canvas(&self, if_match: &str, body: &str) -> Result<String, ApiError> {
        self.save_canvas_within(SAVE_START, None, None, if_match, body)
            .await
    }

    /// [`Server::save_canvas`] for an editor whose lifetime can be revoked (as
    /// [`Server::save_page_while`]), of its own `view`.
    pub(crate) async fn save_canvas_while(
        &self,
        live: &std::sync::atomic::AtomicBool,
        view: &str,
        if_match: &str,
        body: &str,
    ) -> Result<String, ApiError> {
        self.save_canvas_within(SAVE_START, Some(live), Some(view), if_match, body)
            .await
    }

    /// The canvas save: every page whose content was sent, and the manifest with the sent page
    /// list (ids, names, order) and active page, as one commit, rolled back on any error. The
    /// manifest's identity, canvas name, format, version and creation time stay the server's;
    /// only its page list, active page and modified time change. Everything the request says is
    /// checked before the writer fence is taken (`parse_canvas_save`: shape, bounds, ids, page
    /// files and their graphs), and again against the canvas as it is under the fence: the
    /// revision must be the current one (any change to any page, the list or the active page
    /// since it was loaded is a conflict, `409` with the current revision; nothing merges),
    /// every page new to the list must come with its content (a listed id never reaches an
    /// unlisted file left on disk), and every name that changed must be a page name
    /// (`check_page_name`). A page left out of the list keeps its file, unreferenced; nothing is
    /// deleted.
    pub(crate) async fn save_canvas_within(
        &self,
        start_within: std::time::Duration,
        live: Option<&std::sync::atomic::AtomicBool>,
        view: Option<&str>,
        if_match: &str,
        body: &str,
    ) -> Result<String, ApiError> {
        if body.len() > MAX_PAGE_BYTES {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "the canvas save is larger than {MAX_PAGE_BYTES} bytes; nothing was \
                     written"
                ),
            ));
        }
        let save = parse_canvas_save(body)?;
        let (write, hold) = self.start_write(start_within, live, view).await?;
        // Declared in this order so the editing's hold goes first, after the commit.
        let _write = write;
        let _hold = hold;
        self.commit_canvas(save, if_match)
    }

    /// The part of a canvas save after its admission (`start_write`, whose fence and hold the
    /// caller keeps until this returns): the strict revision check, every check against the
    /// canvas as it is now, and the one commit. Nothing after the commit can fail.
    fn commit_canvas(&self, save: CanvasSave, if_match: &str) -> Result<String, ApiError> {
        let f = self.canvas_files()?;
        let current = f.revision();
        if if_match != current {
            return Err(ApiError {
                etag: Some(current),
                ..ApiError::new(
                    StatusCode::CONFLICT,
                    "the canvas changed since it was loaded (a page, the page list or the \
                     active page); nothing was written",
                )
            });
        }
        let listed = &f.project.manifest.pages;
        let bad = |msg: String| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("{msg}; nothing was written"),
            )
        };
        let mut writes = Vec::new();
        let mut list = Vec::with_capacity(save.pages.len());
        let mut revs = Vec::with_capacity(save.pages.len());
        for s in save.pages {
            let was = listed.iter().position(|r| r.id == s.id);
            if was.is_none_or(|i| listed[i].name != s.name) {
                check_page_name(&s.name).map_err(|e| bad(format!("page {:?}: {e}", s.id)))?;
            }
            let rev = match (s.page, was) {
                (Some(mut page), _) => {
                    // The sent list owns the names (as the manifest does on every read).
                    page.name = s.name.clone();
                    page.doc.name = s.name.clone();
                    let path = f.project.page_path(&s.id);
                    plain(&path, Kind::File).map_err(forbidden)?;
                    let data = page_to_json(&page).into_bytes();
                    let rev = etag(&data);
                    writes.push(commit::Write { path, data });
                    rev
                }
                (None, Some(i)) => revision(&f.pages[i]),
                (None, None) => {
                    return Err(bad(format!(
                        "page {:?} is new to the canvas, so its content must be sent",
                        s.id
                    )));
                }
            };
            revs.push((s.id.clone(), rev));
            list.push(PageRef {
                id: s.id,
                name: s.name,
            });
        }
        let mut manifest = f.project.manifest.clone();
        manifest.pages = list;
        manifest.active_page = Some(save.active);
        // A rename from the editor's title lands in this same commit and revision.
        if let Some(name) = save.name {
            manifest.name = name;
        }
        manifest.modified = now_ms();
        let manifest_bytes = manifest.to_json().into_bytes();
        writes.push(commit::Write {
            path: self.dir.join(MANIFEST),
            data: manifest_bytes.clone(),
        });
        // Checked again just before writing (`canvas_files` above checked every page file).
        self.check()?;
        refuse_if_broken(&self.broken)
            .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
        // New page files go into `pages/`, made (never through a link) if a canvas has none
        // yet, as the agent's saves do.
        let pages_dir = self.dir.join(PAGES_DIR);
        if !plain(&pages_dir, Kind::Dir).map_err(forbidden)? {
            std::fs::create_dir(&pages_dir).map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("pages: {e}; nothing was written"),
                )
            })?;
        }
        commit_fenced(&self.broken, &writes)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.describe()))?;
        // Committed: the revision of exactly what is on disk now (nothing else wrote under the
        // fence). Nothing that can fail comes after the commit.
        let pages: Vec<(&str, &str)> = revs
            .iter()
            .map(|(id, rev)| (id.as_str(), rev.as_str()))
            .collect();
        let rev = canvas_revision(&manifest_bytes, &pages);
        self.editor
            .saves
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(rev)
    }

    /// Rename the bound canvas: its manifest's name and modified time, through the same writer
    /// fence, checks, commit, rollback and `Broken` rule as every save. Pages and files don't
    /// change; an editor of this canvas sees a new canvas revision (a clean one reloads, a dirty
    /// one's next save is a conflict, nothing merges).
    pub(crate) async fn rename_canvas(&self, name: &str) -> Result<(), ApiError> {
        let (write, hold) = self.start_write(SAVE_START, None, None).await?;
        // Declared in this order so the editing's hold goes first, after the commit.
        let _write = write;
        let _hold = hold;
        let mut p = self.project()?;
        p.manifest.name = name.to_string();
        p.manifest.modified = now_ms();
        self.check()?;
        refuse_if_broken(&self.broken)
            .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
        let writes = [commit::Write {
            path: p.dir.join(MANIFEST),
            data: p.manifest.to_json().into_bytes(),
        }];
        commit_fenced(&self.broken, &writes)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.describe()))
    }

    /// An editor of this binding (the browser tab or an in-panel editor) reports that it loaded
    /// its page and its renderer is Ready. Changes no file.
    pub(crate) fn mark_ready(&self) -> Result<(), ApiError> {
        self.admit()?;
        self.editor
            .ready
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
}

/// The manifest's entry for `id`, or an error response. Ids are checked against the
/// manifest (and for a plain character set), never used as client-chosen paths.
fn page_ref(p: &Project, id: &str) -> Result<String, ApiError> {
    if !valid_page_id(id) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid page id"));
    }
    p.manifest
        .pages
        .iter()
        .find(|r| r.id == id)
        .map(|r| r.name.clone())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no such page in this project"))
}

/// Current file bytes of a page (None when it was never written).
fn read_page(p: &Project, id: &str) -> Result<Option<Vec<u8>>, ApiError> {
    let path = p.page_path(id);
    if !plain(&p.dir.join(PAGES_DIR), Kind::Dir).map_err(forbidden)?
        || !plain(&path, Kind::File).map_err(forbidden)?
    {
        return Ok(None);
    }
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        )),
    }
}

fn revision(bytes: &Option<Vec<u8>>) -> String {
    bytes.as_deref().map_or_else(|| ABSENT.to_string(), etag)
}

async fn get_page(State(s): State<Arc<Server>>, UrlPath(id): UrlPath<String>) -> Response {
    match s.page(Some(&id)) {
        Ok(page) => (
            [
                (header::CONTENT_TYPE, "application/json".to_string()),
                (header::ETAG, page.revision),
            ],
            page.bytes,
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// The page's current revision only (cheap polling for the browser).
async fn get_revision(State(s): State<Arc<Server>>, UrlPath(id): UrlPath<String>) -> Response {
    match s.page_revision(&id) {
        Ok(rev) => (
            [(header::ETAG, rev.clone())],
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "revision": rev }).to_string(),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

async fn put_page(
    State(s): State<Arc<Server>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(if_match) = header_str(&headers, header::IF_MATCH).map(str::to_string) else {
        return error(
            StatusCode::PRECONDITION_REQUIRED,
            "If-Match with the page revision is required",
        );
    };
    let tab = match s.tab_of(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    let (view, live) = split_tab(&tab);
    match s
        .save_page_within(SAVE_START, live, view, &id, &if_match, &body)
        .await
    {
        Ok(rev) => (
            StatusCode::OK,
            [(header::ETAG, rev.clone())],
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "revision": rev }).to_string(),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// A browser tab's own view (`X-OPD-View`, issued by `POST /api/view`, issue #23): `t<n>`. On a
/// library canvas every mutation needs an issued, live one (`Server::tab_of`, R78-01); the
/// chat's agent writes through its MCP binding, never through these routes.
const VIEW_HEADER: header::HeaderName = header::HeaderName::from_static("x-opd-view");

/// A view as a client sends it back: `t` and 1 to 20 digits.
fn valid_view(v: &str) -> bool {
    v.strip_prefix('t')
        .is_some_and(|n| !n.is_empty() && n.len() <= 20 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// A resolved tab as the writes take it: its view and lifetime.
fn split_tab(
    tab: &Option<(String, Arc<std::sync::atomic::AtomicBool>)>,
) -> (Option<&str>, Option<&std::sync::atomic::AtomicBool>) {
    match tab {
        Some((v, l)) => (Some(v.as_str()), Some(l.as_ref())),
        None => (None, None),
    }
}

/// A browser tab of the editor opened (issue #23): it is issued its own view and, as the newest
/// view, takes the canvas's editing (any other tab or panel of it, here or in another window, is
/// read-only from now on). It carries nothing. Refused (`503`) when no view can be issued: the
/// tab then can't edit, and says so.
async fn post_view(State(s): State<Arc<Server>>) -> Response {
    if let Err(e) = s.admit() {
        return e.into_response();
    }
    let Some(view) = s.open_tab().await else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "this canvas's editor has issued as many views as it can; close and reopen the \
             canvas to edit here",
        );
    };
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "view": view, "editing": s.tab_editing(Some(&view)) }).to_string(),
    )
        .into_response()
}

/// The tab is going away (`POST /api/view/release`): its view changes nothing any more.
async fn post_view_release(State(s): State<Arc<Server>>, headers: HeaderMap) -> Response {
    match s.tab_of(&headers) {
        Ok(Some((view, _))) => match s.release_tab(&view).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            // Released all the same (never a refusal: it can't be undone), with what of its
            // cleanup didn't happen (R89-01).
            Err(e) => (
                [(header::CONTENT_TYPE, "application/json")],
                release_outcome(&e).to_string(),
            )
                .into_response(),
        },
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

/// A release whose record cleanup didn't complete (see [`Server::release_view`]), as both
/// transports say it: released (for good), a `note` saying what didn't happen and what follows,
/// and that condition's status (`cleanup_status`).
pub(crate) fn release_outcome(e: &ApiError) -> serde_json::Value {
    // The condition's message says what follows (`Server::release_view`): noted (no view waits
    // for it), owed to this window's next change, check or close, or fenced.
    serde_json::json!({
        "released": true,
        "note": format!(
            "This view was released (it can't change anything any more), but recording that it \
             left didn't complete: {}.",
            e.msg.trim_end_matches('.')
        ),
        "cleanup_status": e.status.as_u16(),
    })
}

/// A tab that lost the editing answers the handover (`POST /api/draft`, see
/// [`Server::answer`]): `{base, canvas, changed, unfinished}` (what it has unsaved), or `{}`
/// when it has nothing. Returns the offer's id.
async fn post_draft(State(s): State<Arc<Server>>, headers: HeaderMap, body: String) -> Response {
    let tab = match s.tab_of(&headers) {
        Ok(Some(t)) => t,
        Ok(None) => return error(StatusCode::NOT_FOUND, "nothing is handed over here"),
        Err(e) => return e.into_response(),
    };
    let offer = match parse_offer(&body) {
        Ok(o) => o,
        Err(e) => return e.into_response(),
    };
    match s.answer(&tab.0, Some(&tab.1), offer).await {
        Ok(id) => (
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "offer": id }).to_string(),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// An answer's body: `{}` (nothing to hand over) or `{base, canvas, changed, unfinished}`.
pub(crate) fn parse_offer(body: &str) -> Result<Option<library::NewOffer>, ApiError> {
    let bad = |m: &str| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("{m}; nothing was handed over"),
        )
    };
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|_| bad("the answer isn't JSON"))?;
    let o = v
        .as_object()
        .ok_or_else(|| bad("the answer isn't an object"))?;
    if o.is_empty() {
        return Ok(None);
    }
    let text = |k: &str| {
        o.get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    let offer = library::NewOffer {
        base: text("base").ok_or_else(|| bad("`base` is required"))?,
        canvas: text("canvas").unwrap_or_default(),
        changed: o
            .get("changed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        unfinished: match o.get("unfinished") {
            None => Vec::new(),
            Some(u) => u
                .as_array()
                .cloned()
                .ok_or_else(|| bad("`unfinished` is a list"))?,
        },
    };
    Ok((!offer.canvas.is_empty() || !offer.unfinished.is_empty()).then_some(offer))
}

/// An offer action's body: `{"id": "<offer or pending id>"}`.
fn action_id(body: &str) -> Result<String, ApiError> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .filter(|i| library::valid_id(i))
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "the change to act on (`id`) is required; nothing was changed",
            )
        })
}

/// What the editing tab does with a handed-over change or a pending view (R78-03/04): apply,
/// keep as a new canvas, dismiss, or stop waiting. Each needs this tab's issued, live view and
/// the change's (or view's) `id`.
async fn post_draft_action(
    s: &Arc<Server>,
    headers: &HeaderMap,
    body: &str,
    action: &str,
) -> Response {
    let tab = match s.tab_of(headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    let (view, live) = split_tab(&tab);
    let id = match action_id(body) {
        Ok(i) => i,
        Err(e) => return e.into_response(),
    };
    // The kept recoveries the action says it resolves (`{"id", "resolved": [...]}`, R83).
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let resolved = match resolved_of(parsed.as_ref().and_then(|v| v.get("resolved"))) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let json = |v: serde_json::Value| {
        ([(header::CONTENT_TYPE, "application/json")], v.to_string()).into_response()
    };
    match action {
        "apply" => match s.apply_offer(view, live, &id, &resolved).await {
            Ok(a) => json(serde_json::json!({
                "applied": true, "revision": a.revision, "left": a.left,
            })),
            Err(e) => e.into_response(),
        },
        "keep" => match s.keep_offer(view, live, &id, &resolved).await {
            Ok(k) => json(serde_json::json!({
                "kept": {"id": k.id, "name": k.name}, "left": k.left,
            })),
            Err(e) => e.into_response(),
        },
        "dismiss" => match s.dismiss_offer(view, live, &id, &resolved).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => e.into_response(),
        },
        _ => match s.stop_waiting(view, live, &id).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => e.into_response(),
        },
    }
}

async fn post_draft_apply(State(s): State<Arc<Server>>, h: HeaderMap, body: String) -> Response {
    post_draft_action(&s, &h, &body, "apply").await
}

async fn post_draft_keep(State(s): State<Arc<Server>>, h: HeaderMap, body: String) -> Response {
    post_draft_action(&s, &h, &body, "keep").await
}

async fn post_draft_dismiss(State(s): State<Arc<Server>>, h: HeaderMap, body: String) -> Response {
    post_draft_action(&s, &h, &body, "dismiss").await
}

async fn post_draft_stop(State(s): State<Arc<Server>>, h: HeaderMap, body: String) -> Response {
    post_draft_action(&s, &h, &body, "stop").await
}

/// The whole canvas, for the editors (see [`Server::canvas`]); its revision is the ETag.
async fn get_canvas(State(s): State<Arc<Server>>, headers: HeaderMap) -> Response {
    let read = match s.canvas().await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    match read.to_json() {
        Ok(body) => (
            [
                (header::CONTENT_TYPE, "application/json".to_string()),
                (header::ETAG, read.revision.clone()),
                // Whether this view may save it (issue #23; see `library`, "Editing"; a library
                // request naming no issued view is told it doesn't): the body stays exactly
                // the canvas the editor loads.
                (
                    EDITING_HEADER,
                    editing_value(&s, s.tab_read(&headers).as_deref()).to_string(),
                ),
            ],
            body.to_string(),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// Whether a library canvas's view (or binding) has its editing (`here`) or another view has it
/// (`elsewhere`); a workspace project is always `here`. A hint for the editors, read with the
/// canvas and its revision; every save checks it again.
const EDITING_HEADER: header::HeaderName = header::HeaderName::from_static("x-opd-editing");

fn editing_value(s: &Server, view: Option<&str>) -> &'static str {
    if s.tab_editing(view) {
        "here"
    } else {
        "elsewhere"
    }
}

/// The canvas revision only (cheap polling for the editors), whether this view has the canvas's
/// editing (`editing`, see [`EDITING_HEADER`]), and the handover record (`handover`, see
/// [`Server::handover_json`]; `view` says why a request naming no usable view can't edit:
/// `missing`, `malformed`, `unissued` or `released`).
async fn get_canvas_revision(State(s): State<Arc<Server>>, headers: HeaderMap) -> Response {
    let (view, problem) = match s.tab_of(&headers) {
        Ok(t) => (t.map(|(v, _)| v), serde_json::Value::Null),
        Err(e) => (
            None,
            serde_json::json!(match e.status {
                StatusCode::PRECONDITION_REQUIRED => "missing",
                StatusCode::BAD_REQUEST => "malformed",
                _ if e.msg == VIEW_RELEASED => "released",
                _ => "unissued",
            }),
        ),
    };
    match s.canvas_revision() {
        Ok(rev) => (
            [(header::ETAG, rev.clone())],
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "revision": rev,
                "editing": s.tab_editing(view.as_deref()),
                "handover": s.handover_json(view.as_deref()),
                "view": problem,
            })
            .to_string(),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// The browser editor's Edit here (issue #23): take the canvas's editing for this tab (its
/// issued, live `X-OPD-View`; a library request naming none is refused, R78-01; see
/// [`Server::claim`]). It carries nothing; a refusal changes nothing.
async fn post_claim(State(s): State<Arc<Server>>, headers: HeaderMap) -> Response {
    // Admission (a retired binding: `410`) is the claim's own, under its fence.
    let tab = match s.tab_of(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    let (view, live) = split_tab(&tab);
    // The refusal as it came (see `Server::claim`), never a blanket conflict.
    match s.claim(view, live).await {
        Ok(c) => (
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "editing": true, "note": c.note }).to_string(),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// A canvas save (see [`Server::save_canvas_within`]), based on the canvas revision in
/// `If-Match`.
async fn put_canvas(State(s): State<Arc<Server>>, headers: HeaderMap, body: String) -> Response {
    let Some(if_match) = header_str(&headers, header::IF_MATCH).map(str::to_string) else {
        return error(
            StatusCode::PRECONDITION_REQUIRED,
            "If-Match with the canvas revision is required",
        );
    };
    let tab = match s.tab_of(&headers) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    let (view, live) = split_tab(&tab);
    match s
        .save_canvas_within(SAVE_START, live, view, &if_match, &body)
        .await
    {
        Ok(rev) => (
            StatusCode::OK,
            [(header::ETAG, rev.clone())],
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "revision": rev }).to_string(),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    const TOKEN: &str = "unit-test-token-0123456789abcdef";

    async fn put(s: &Arc<Server>, id: &str, tag: &str, body: String) -> (StatusCode, String) {
        use http_body_util::BodyExt;
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/pages/{id}"))
            .header(header::HOST, "127.0.0.1:1")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::IF_MATCH, tag)
            .body(Body::from(body))
            .unwrap();
        let res = router(s.clone()).oneshot(req).await.unwrap();
        let st = res.status();
        let b = res.into_body().collect().await.unwrap().to_bytes();
        (st, String::from_utf8_lossy(&b).into_owned())
    }

    /// Review round 4, F1, browser side: the page and the manifest are one commit, so a
    /// failure at any step leaves both byte-identical; after an undoable failure, every
    /// later save is refused.
    #[tokio::test(flavor = "current_thread")]
    async fn browser_saves_are_all_or_nothing_and_fail_closed() {
        use std::hash::{BuildHasher, RandomState};
        let root = std::env::temp_dir().join(format!(
            "opd-unit-put-{:016x}",
            RandomState::new().hash_one(std::time::SystemTime::now())
        ));
        let (p, _) = paper_rs::project::Store::open(&root)
            .unwrap()
            .create("Put")
            .unwrap();
        let id = p.manifest.pages[0].id.clone();
        let s = Server::open(Config {
            project: p.dir.clone(),
            web_dir: root.join("no-bundle"),
            port: 1,
            token: TOKEN.into(),
        })
        .unwrap();
        let files = || {
            (
                std::fs::read(p.page_path(&id)).unwrap(),
                std::fs::read(p.dir.join(MANIFEST)).unwrap(),
            )
        };
        let mut page = p.load_page(&id).unwrap();
        page.doc.next_id += 1;
        let body = page_to_json(&page);
        // Steps: stage page (0), stage manifest (1), rename page (2), rename manifest (3),
        // sync pages/ (4), sync the project folder (5).
        for at in 0..6 {
            let before = files();
            let tag = etag(&before.0);
            commit::fault::fail_at(&[at]);
            let (st, msg) = put(&s, &id, &tag, body.clone()).await;
            commit::fault::fail_at(&[]);
            assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "step {at}");
            assert!(msg.contains("nothing was written"), "step {at}: {msg}");
            assert_eq!(files(), before, "step {at}: nothing written");
        }
        // Rename of the manifest fails, and restoring the page fails too: fail closed, and
        // the answer says the page may be inconsistent (review round 5, F3).
        let tag = etag(&files().0);
        commit::fault::fail_at(&[3, 4]);
        let (st, msg) = put(&s, &id, &tag, body.clone()).await;
        commit::fault::fail_at(&[]);
        assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            msg.contains("may now be inconsistent")
                && msg.contains(&format!("{id}.json"))
                && msg.contains("disabled")
                && !msg.contains("nothing was written"),
            "{msg}"
        );
        assert!(s.broken.lock().unwrap().is_some());
        let before = files();
        let (st, msg) = put(&s, &id, &etag(&before.0), body).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
        assert!(msg.contains(&format!("{id}.json")), "{msg}");
        assert_eq!(files(), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- the editors' canvas API (`/api/canvas`): every page, the list and the active page

    /// A fresh upstream project (one page) and its server.
    fn canvas_project(tag: &str) -> (PathBuf, Project, Arc<Server>) {
        use std::hash::{BuildHasher, RandomState};
        let root = std::env::temp_dir().join(format!(
            "opd-unit-canvas-{tag}-{:016x}",
            RandomState::new().hash_one(std::time::SystemTime::now())
        ));
        let (p, _) = paper_rs::project::Store::open(&root)
            .unwrap()
            .create("Canvas")
            .unwrap();
        let s = Server::open(Config {
            project: p.dir.clone(),
            web_dir: root.join("no-bundle"),
            port: 1,
            token: TOKEN.into(),
        })
        .unwrap();
        (root, p, s)
    }

    /// One API request: (status, ETag, body).
    async fn request(
        s: &Arc<Server>,
        method: &str,
        uri: &str,
        if_match: Option<&str>,
        body: String,
    ) -> (StatusCode, Option<String>, String) {
        use http_body_util::BodyExt;
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "127.0.0.1:1")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
        if let Some(tag) = if_match {
            req = req.header(header::IF_MATCH, tag);
        }
        let res = router(s.clone())
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let st = res.status();
        let tag = res
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let b = res.into_body().collect().await.unwrap().to_bytes();
        (st, tag, String::from_utf8_lossy(&b).into_owned())
    }

    /// The canvas revision now, as the editors poll it.
    async fn canvas_rev(s: &Arc<Server>) -> String {
        let (st, tag, body) = request(s, "GET", "/api/canvas/revision", None, String::new()).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        tag.unwrap()
    }

    /// A page file as the editor sends it: page `id` with `rects` rectangles.
    fn page_text(id: &str, rects: usize) -> String {
        let mut doc = Document::default();
        for i in 0..rects {
            let n = doc.alloc_id();
            let r = paper_rs::geom::Rect::new(i as f64 * 10.0, 0.0, 5.0, 5.0);
            doc.nodes.insert(
                n,
                paper_rs::model::Node::new(n, paper_rs::model::NodeKind::Rect, r),
            );
            doc.root.push(n);
        }
        page_to_json(&PageData {
            id: id.into(),
            name: "named by the list".into(),
            doc,
            camera: None,
        })
    }

    /// A page id as long as `id` whose first six characters all differ from `id`'s.
    fn other_id(id: &str) -> String {
        id.chars()
            .enumerate()
            .map(|(i, c)| match (i < 6, c) {
                (true, '0') => '1',
                (true, _) => '0',
                (false, c) => c,
            })
            .collect()
    }

    /// Every file of the project that a save could touch (the manifest and everything in
    /// `pages/`, stray temporary files included), byte for byte.
    fn disk(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = vec![(
            MANIFEST.to_string(),
            std::fs::read(dir.join(MANIFEST)).unwrap(),
        )];
        let mut pages: Vec<PathBuf> = std::fs::read_dir(dir.join(PAGES_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        pages.sort();
        for p in pages {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read(&p).unwrap()));
        }
        out
    }

    /// The editors' load and save of the whole canvas: one commit of the changed pages and the
    /// manifest's list, names, order and active page; the canvas identity stays the server's.
    /// The remembered active page is every reader's default afterwards, the agent host's
    /// included (its sync sees the manifest change). An active-page-only save writes only the
    /// manifest. A page left out of the list keeps its file, unreferenced and unreadable by id,
    /// and listing that id again without its content is refused.
    #[tokio::test(flavor = "current_thread")]
    async fn canvas_saves_every_changed_page_the_list_and_the_active_page_as_one_commit() {
        let (root, p, s) = canvas_project("save");
        let first = p.manifest.pages[0].id.clone();
        let (st, tag, body) = request(&s, "GET", "/api/canvas", None, String::new()).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let tag = tag.unwrap();
        assert_eq!(v["revision"], serde_json::json!(tag));
        assert_eq!(v["active_page"], serde_json::json!(first));
        assert_eq!(v["canvas"]["name"], "Canvas");
        let pages = v["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(
            pages[0]["text"].as_str().unwrap().as_bytes(),
            std::fs::read(p.page_path(&first)).unwrap().as_slice(),
            "each page as its upstream file"
        );
        assert_eq!(canvas_rev(&s).await, tag, "the poll agrees with the read");
        // The agent's host on the same project, as the MCP thread holds it.
        let mut host = agent::BoundHost::open(&p.dir).unwrap();
        assert_eq!(host.ed.active_page_id(), first);
        // Rename page 1 (its content unchanged, so not sent), add page 2 with its content,
        // and make page 2 the active one.
        let second = other_id(&first);
        let page1 = std::fs::read(p.page_path(&first)).unwrap();
        let save = serde_json::json!({"active_page": second, "pages": [
            {"id": first, "name": "Plan"},
            {"id": second, "name": "Notes", "text": page_text(&second, 2)},
        ]});
        let (st, rev, body) = request(&s, "PUT", "/api/canvas", Some(&tag), save.to_string()).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        let rev = rev.unwrap();
        let m = read_manifest_checked(&p.dir).unwrap();
        let list: Vec<(&str, &str)> = m
            .pages
            .iter()
            .map(|r| (r.id.as_str(), r.name.as_str()))
            .collect();
        assert_eq!(list, [(first.as_str(), "Plan"), (second.as_str(), "Notes")]);
        assert_eq!(m.active_page.as_deref(), Some(second.as_str()));
        assert_eq!(
            (&m.id, &m.name, m.created, &m.format, m.version),
            (
                &p.manifest.id,
                &p.manifest.name,
                p.manifest.created,
                &p.manifest.format,
                p.manifest.version
            ),
            "the canvas identity is the server's"
        );
        assert_eq!(std::fs::read(p.page_path(&first)).unwrap(), page1);
        let written = std::fs::read_to_string(p.page_path(&second)).unwrap();
        let written = page_from_json(&written).unwrap();
        assert_eq!(written.name, "Notes", "the list names the page");
        assert_eq!(written.doc.nodes.len(), 2);
        // The answer is the revision of exactly what is on disk.
        assert_eq!(canvas_rev(&s).await, rev);
        let (_, again, body) = request(&s, "GET", "/api/canvas", None, String::new()).await;
        assert_eq!(again.as_deref(), Some(rev.as_str()));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["active_page"], serde_json::json!(second));
        assert_eq!(v["pages"][1]["name"], "Notes");
        // Every reader's default page is the remembered one: the per-page API's, and the
        // agent's after its sync.
        assert_eq!(s.page(None).ok().map(|r| r.id), Some(second.clone()));
        host.sync().unwrap();
        assert_eq!(host.ed.active_page_id(), second);
        assert_eq!(host.ed.pages.len(), 2);
        // Only the active page changes: only the manifest is written.
        let page2 = std::fs::read(p.page_path(&second)).unwrap();
        let save = serde_json::json!({"active_page": first, "pages": [
            {"id": first, "name": "Plan"}, {"id": second, "name": "Notes"},
        ]});
        let (st, rev, body) = request(&s, "PUT", "/api/canvas", Some(&rev), save.to_string()).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert_eq!(std::fs::read(p.page_path(&first)).unwrap(), page1);
        assert_eq!(std::fs::read(p.page_path(&second)).unwrap(), page2);
        let m = read_manifest_checked(&p.dir).unwrap();
        assert_eq!(m.active_page.as_deref(), Some(first.as_str()));
        host.sync().unwrap();
        assert_eq!(
            host.ed.active_page_id(),
            first,
            "no stale active page for the agent"
        );
        // Page 2 left out of the list: its file stays, unreferenced, and isn't read by id.
        let save =
            serde_json::json!({"active_page": first, "pages": [{"id": first, "name": "Plan"}]});
        let (st, rev, body) =
            request(&s, "PUT", "/api/canvas", rev.as_deref(), save.to_string()).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert_eq!(std::fs::read(p.page_path(&second)).unwrap(), page2);
        let uri = format!("/api/pages/{second}");
        let (st, _, _) = request(&s, "GET", &uri, None, String::new()).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        // Listing it again without its content would make that file a page again: refused.
        let before = disk(&p.dir);
        let save = serde_json::json!({"active_page": first, "pages": [
            {"id": first, "name": "Plan"}, {"id": second, "name": "Back"},
        ]});
        let (st, _, body) =
            request(&s, "PUT", "/api/canvas", rev.as_deref(), save.to_string()).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.contains("new to the canvas"), "{body}");
        assert_eq!(disk(&p.dir), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The editor's title rename travels in its canvas save (revision 2, slice 6): the file's
    /// name and a changed page are one commit and one revision; a save without `name` (an older
    /// editor) keeps the name; an invalid name is refused with nothing written; and a rename
    /// made elsewhere since the editor loaded the canvas (the library's Rename of the open
    /// canvas) is a conflict: neither the editor's name nor its content is written.
    #[tokio::test(flavor = "current_thread")]
    async fn a_canvas_save_renames_the_file_with_its_pages_in_one_commit() {
        let (root, p, s) = canvas_project("name");
        let first = p.manifest.pages[0].id.clone();
        let rev = canvas_rev(&s).await;
        let save = serde_json::json!({"active_page": first, "name": "  Launch plan ", "pages": [
            {"id": first, "name": "Page 1", "text": page_text(&first, 2)},
        ]});
        let (st, rev, body) = request(&s, "PUT", "/api/canvas", Some(&rev), save.to_string()).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        let rev = rev.unwrap();
        let m = read_manifest_checked(&p.dir).unwrap();
        assert_eq!(m.name, "Launch plan", "trimmed, as Rename keeps it");
        assert_eq!(m.id, p.manifest.id, "the identity stays the server's");
        let written = std::fs::read_to_string(p.page_path(&first)).unwrap();
        let written = page_from_json(&written).unwrap();
        assert_eq!(written.doc.nodes.len(), 2, "the page in the same commit");
        assert_eq!(canvas_rev(&s).await, rev, "one revision for both");
        let (_, _, body) = request(&s, "GET", "/api/canvas", None, String::new()).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["canvas"]["name"], "Launch plan");
        // An older editor's save (no `name`) keeps the name as it is.
        let same = serde_json::json!({"active_page": first, "pages": [
            {"id": first, "name": "Page 1"},
        ]});
        let (st, rev, body) = request(&s, "PUT", "/api/canvas", Some(&rev), same.to_string()).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        let rev = rev.unwrap();
        assert_eq!(read_manifest_checked(&p.dir).unwrap().name, "Launch plan");
        // Invalid names, each on the current revision: refused, nothing written.
        let before = disk(&p.dir);
        let names = [
            (serde_json::json!(""), "1 to 60 characters"),
            (serde_json::json!("   "), "1 to 60 characters"),
            (serde_json::json!("x".repeat(61)), "1 to 60 characters"),
            (serde_json::json!("a\u{7}b"), "control characters"),
            (serde_json::json!(7), "must be text"),
        ];
        for (name, why) in names {
            let save = serde_json::json!({"active_page": first, "name": name, "pages": [
                {"id": first, "name": "Page 1"},
            ]});
            let (st, _, body) =
                request(&s, "PUT", "/api/canvas", Some(&rev), save.to_string()).await;
            assert_eq!(st, StatusCode::BAD_REQUEST, "{why}: {body}");
            assert!(body.contains(why), "{why}: {body}");
            assert!(body.contains("nothing was written"), "{why}: {body}");
            assert_eq!(disk(&p.dir), before, "{why}");
        }
        // The library renames the open canvas meanwhile (`_app_rename` goes through this same
        // fence): the editor's rename and change, based on what it loaded, are a conflict.
        assert!(s.rename_canvas("Renamed in the library").await.is_ok());
        let before = disk(&p.dir);
        let mine = serde_json::json!({"active_page": first, "name": "Mine", "pages": [
            {"id": first, "name": "Page 1", "text": page_text(&first, 3)},
        ]});
        let (st, tag, body) = request(&s, "PUT", "/api/canvas", Some(&rev), mine.to_string()).await;
        assert_eq!(st, StatusCode::CONFLICT, "{body}");
        let now = canvas_rev(&s).await;
        assert_eq!(tag.as_deref(), Some(now.as_str()), "the current revision");
        assert_eq!(disk(&p.dir), before, "nothing of the stale save");
        let name = read_manifest_checked(&p.dir).unwrap().name;
        assert_eq!(name, "Renamed in the library");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A canvas save is based on the whole canvas it loaded: a change since to another page
    /// (an agent's, or a per-page save), to the page list or to the active page is a conflict
    /// (409 with the current canvas revision) and nothing is written. A save that isn't a
    /// valid canvas save is refused before anything is written, whatever it gets wrong.
    #[tokio::test(flavor = "current_thread")]
    async fn stale_or_invalid_canvas_saves_write_nothing() {
        let (root, p, s) = canvas_project("stale");
        let first = p.manifest.pages[0].id.clone();
        let second = other_id(&first);
        let two = serde_json::json!({"active_page": first, "pages": [
            {"id": first, "name": "One"},
            {"id": second, "name": "Two", "text": page_text(&second, 1)},
        ]});
        let rev = canvas_rev(&s).await;
        let (st, _, body) = request(&s, "PUT", "/api/canvas", Some(&rev), two.to_string()).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        // This editor loaded the canvas now and changes page 1 only.
        let loaded = canvas_rev(&s).await;
        let mine = serde_json::json!({"active_page": first, "pages": [
            {"id": first, "name": "One", "text": page_text(&first, 3)},
            {"id": second, "name": "Two"},
        ]})
        .to_string();
        // Meanwhile page 2 (which this save doesn't send) changes through the per-page API.
        let page2 = std::fs::read(p.page_path(&second)).unwrap();
        let (st, _) = put(&s, &second, &etag(&page2), page_text(&second, 2)).await;
        assert_eq!(st, StatusCode::OK);
        let before = disk(&p.dir);
        let (st, tag, body) = request(&s, "PUT", "/api/canvas", Some(&loaded), mine.clone()).await;
        assert_eq!(st, StatusCode::CONFLICT, "{body}");
        assert!(body.contains("nothing was written"), "{body}");
        let now = canvas_rev(&s).await;
        assert_eq!(tag.as_deref(), Some(now.as_str()), "the current revision");
        assert_eq!(
            disk(&p.dir),
            before,
            "another page's change is never overwritten"
        );
        // The page list changes (another editor renames page 2): conflict again.
        let rename = serde_json::json!({"active_page": first, "pages": [
            {"id": first, "name": "One"}, {"id": second, "name": "Renamed"},
        ]});
        let (st, _, _) = request(&s, "PUT", "/api/canvas", Some(&now), rename.to_string()).await;
        assert_eq!(st, StatusCode::OK);
        let before = disk(&p.dir);
        let (st, _, _) = request(&s, "PUT", "/api/canvas", Some(&now), mine.clone()).await;
        assert_eq!(st, StatusCode::CONFLICT);
        assert_eq!(disk(&p.dir), before);
        // The active page changes alone: conflict too.
        let now = canvas_rev(&s).await;
        let switch = serde_json::json!({"active_page": second, "pages": [
            {"id": first, "name": "One"}, {"id": second, "name": "Renamed"},
        ]});
        let (st, _, _) = request(&s, "PUT", "/api/canvas", Some(&now), switch.to_string()).await;
        assert_eq!(st, StatusCode::OK);
        let before = disk(&p.dir);
        let (st, _, _) = request(&s, "PUT", "/api/canvas", Some(&now), mine).await;
        assert_eq!(st, StatusCode::CONFLICT);
        assert_eq!(disk(&p.dir), before);
        // Invalid saves, each on the current revision: refused, nothing written.
        let rev = canvas_rev(&s).await;
        let page = |id: &str, name: &str| serde_json::json!({"id": id, "name": name});
        let with = |id: &str, name: &str, text: String| serde_json::json!({"id": id, "name": name, "text": text});
        let save = |active: &str, pages: Vec<serde_json::Value>| {
            serde_json::json!({"active_page": active, "pages": pages}).to_string()
        };
        let graph = format!(
            "{{\"format\":\"paper-rs-page\",\"version\":3,\"id\":\"{first}\",\"name\":\"x\",\
             \"next_id\":9,\"root\":[5],\"nodes\":[]}}"
        );
        let many: Vec<serde_json::Value> = (0..MAX_PAGES + 1)
            .map(|i| page(&format!("{i:06}x"), "P"))
            .collect();
        let cases: Vec<(String, StatusCode, &str)> = vec![
            ("not json".into(), StatusCode::BAD_REQUEST, "isn't JSON"),
            (
                serde_json::json!({"active_page": first, "pages": [page(&first, "One")], "extra": 1})
                    .to_string(),
                StatusCode::BAD_REQUEST,
                "unexpected field `extra`",
            ),
            (save(&first, vec![]), StatusCode::BAD_REQUEST, "1 to 1000 pages"),
            (save("x00000", many), StatusCode::BAD_REQUEST, "1 to 1000 pages"),
            (
                save(&first, vec![page(&first, "One"), with("../x", "Up", page_text("../x", 0))]),
                StatusCode::BAD_REQUEST,
                "unsafe id",
            ),
            (
                save(&first, vec![page(&first, "One"), page(&first, "Again")]),
                StatusCode::BAD_REQUEST,
                "only in case",
            ),
            (
                save(
                    &first,
                    vec![
                        page(&first, "One"),
                        with("Abcdef01", "A", page_text("Abcdef01", 0)),
                        with("abcdef01", "B", page_text("abcdef01", 0)),
                    ],
                ),
                StatusCode::BAD_REQUEST,
                "only in case",
            ),
            (
                save(
                    &first,
                    vec![
                        page(&first, "One"),
                        with("abcdef01", "A", page_text("abcdef01", 0)),
                        with("ABCDEF02", "B", page_text("ABCDEF02", 0)),
                    ],
                ),
                StatusCode::BAD_REQUEST,
                "first six characters",
            ),
            (
                save("elsewhere", vec![page(&first, "One"), page(&second, "Renamed")]),
                StatusCode::BAD_REQUEST,
                "active_page",
            ),
            (
                save(&first, vec![with(&first, "One", page_text(&second, 0)), page(&second, "Renamed")]),
                StatusCode::BAD_REQUEST,
                "names another page",
            ),
            (
                save(&first, vec![with(&first, "One", graph), page(&second, "Renamed")]),
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid document",
            ),
            (
                save(&first, vec![page(&first, "a\u{7}b"), page(&second, "Renamed")]),
                StatusCode::BAD_REQUEST,
                "control characters",
            ),
            (
                save(&first, vec![page(&first, " padded"), page(&second, "Renamed")]),
                StatusCode::BAD_REQUEST,
                "leading and trailing spaces",
            ),
        ];
        let before = disk(&p.dir);
        for (body, want, why) in cases {
            let (st, _, answer) = request(&s, "PUT", "/api/canvas", Some(&rev), body).await;
            assert_eq!(st, want, "{why}: {answer}");
            assert!(answer.contains(why), "{why}: {answer}");
            assert!(answer.contains("nothing was written"), "{why}: {answer}");
            assert_eq!(disk(&p.dir), before, "{why}");
        }
        let (st, _, _) = request(&s, "PUT", "/api/canvas", None, save(&first, vec![])).await;
        assert_eq!(st, StatusCode::PRECONDITION_REQUIRED);
        let big = "x".repeat(MAX_PAGE_BYTES + 1);
        let e = s.save_canvas(&rev, &big).await.err().unwrap();
        assert_eq!(e.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(disk(&p.dir), before);
        assert_eq!(canvas_rev(&s).await, rev, "nothing changed");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The canvas save is one commit: a failure at any step leaves every file byte-identical
    /// (no page or manifest half-written, no temporary file left); after a failure that couldn't
    /// be rolled back every later write, canvas or page, is refused.
    #[tokio::test(flavor = "current_thread")]
    async fn canvas_saves_are_all_or_nothing_and_fail_closed() {
        let (root, p, s) = canvas_project("faults");
        let first = p.manifest.pages[0].id.clone();
        let body = serde_json::json!({"active_page": first, "pages": [
            {"id": first, "name": "Edited", "text": page_text(&first, 1)},
        ]})
        .to_string();
        // Steps: stage page (0), stage manifest (1), rename page (2), rename manifest (3),
        // sync pages/ (4), sync the project folder (5).
        for at in 0..6 {
            let before = disk(&p.dir);
            let rev = canvas_rev(&s).await;
            commit::fault::fail_at(&[at]);
            let (st, _, msg) = request(&s, "PUT", "/api/canvas", Some(&rev), body.clone()).await;
            commit::fault::fail_at(&[]);
            assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "step {at}");
            assert!(msg.contains("nothing was written"), "step {at}: {msg}");
            assert_eq!(disk(&p.dir), before, "step {at}: nothing written");
            assert_eq!(canvas_rev(&s).await, rev, "step {at}");
        }
        let rev = canvas_rev(&s).await;
        commit::fault::fail_at(&[3, 4]);
        let (st, _, msg) = request(&s, "PUT", "/api/canvas", Some(&rev), body.clone()).await;
        commit::fault::fail_at(&[]);
        assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            msg.contains("may now be inconsistent") && msg.contains("disabled"),
            "{msg}"
        );
        assert!(s.broken.lock().unwrap().is_some());
        let before = disk(&p.dir);
        let rev = canvas_rev(&s).await;
        let (st, _, _) = request(&s, "PUT", "/api/canvas", Some(&rev), body.clone()).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
        let page = std::fs::read(p.page_path(&first)).unwrap();
        let (st, _) = put(&s, &first, &etag(&page), page_text(&first, 2)).await;
        assert_eq!(
            st,
            StatusCode::SERVICE_UNAVAILABLE,
            "page saves are fenced too"
        );
        assert_eq!(disk(&p.dir), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A canvas save that can't take the writer fence in time, whose editor was released, or
    /// that comes after the binding was retired never runs; neither does a canvas read then.
    #[tokio::test(flavor = "current_thread")]
    async fn canvas_saves_that_never_started_write_nothing() {
        let (root, p, s) = canvas_project("fence");
        let first = p.manifest.pages[0].id.clone();
        let body = serde_json::json!({"active_page": first, "pages": [
            {"id": first, "name": "Edited", "text": page_text(&first, 1)},
        ]})
        .to_string();
        let rev = canvas_rev(&s).await;
        let before = disk(&p.dir);
        let held = s.lifecycle.write.lock().await;
        let e = s
            .save_canvas_within(
                std::time::Duration::from_millis(20),
                None,
                None,
                &rev,
                &body,
            )
            .await
            .err()
            .unwrap();
        assert_eq!(e.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(e.msg.contains("could not start in time"), "{}", e.msg);
        drop(held);
        let released = std::sync::atomic::AtomicBool::new(false);
        let e = s
            .save_canvas_while(&released, "m1", &rev, &body)
            .await
            .err()
            .unwrap();
        assert_eq!(e.status, StatusCode::GONE);
        assert_eq!(disk(&p.dir), before);
        s.retire().await;
        let e = s.save_canvas(&rev, &body).await.err().unwrap();
        assert_eq!(e.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(s.canvas().await.is_err() && s.canvas_revision().is_err());
        assert_eq!(disk(&p.dir), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Operator Mac finding on cffeec6: a server that stops for good releases the stretch
    /// entries of every page it held, and only those.
    #[tokio::test]
    async fn releasing_a_server_drops_only_its_pages_layout_state() {
        use std::hash::{BuildHasher, RandomState};
        let root = std::env::temp_dir().join(format!(
            "opd-unit-release-{:016x}",
            RandomState::new().hash_one(std::time::SystemTime::now())
        ));
        let (p, _) = paper_rs::project::Store::open(&root)
            .unwrap()
            .create("Release")
            .unwrap();
        let id = p.manifest.pages[0].id.clone();
        let s = Server::open(Config {
            project: p.dir.clone(),
            web_dir: root.join("no-bundle"),
            port: 1,
            token: TOKEN.into(),
        })
        .unwrap();
        let other = format!("other-{id}");
        let n = paper_rs::model::NodeId(7);
        paper_rs::mcp::exec::with_no_stretch(|set| {
            set.insert((id.clone(), n));
            set.insert((other.clone(), n));
        });
        s.retire().await;
        let (mine, theirs) = paper_rs::mcp::exec::with_no_stretch(|set| {
            (
                set.contains(&(id.clone(), n)),
                set.contains(&(other.clone(), n)),
            )
        });
        assert!(!mine && theirs);
        paper_rs::mcp::exec::with_no_stretch(|set| set.remove(&(other.clone(), n)));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Retirement releases the lifetime union, including pages no longer in the manifest.
    /// The manifest edit is a sequential reload fixture, not a concurrent external writer.
    #[tokio::test]
    async fn retirement_includes_created_then_removed_pages_and_preserves_other_keys() {
        use paper_rs::mcp::Out;
        use paper_rs::model::NodeId;
        use serde_json::{Value, json};
        fn value(r: paper_rs::mcp::ToolResult) -> Value {
            r.unwrap()
                .iter()
                .find_map(|o| match o {
                    Out::Text(t) => serde_json::from_str(t).ok(),
                    _ => None,
                })
                .unwrap()
        }
        use std::hash::{BuildHasher, RandomState};
        let root = std::env::temp_dir().join(format!(
            "opd-held-{:016x}",
            RandomState::new().hash_one(std::time::SystemTime::now())
        ));
        let (p, _) = paper_rs::project::Store::open(&root)
            .unwrap()
            .create("Held")
            .unwrap();
        let first = p.manifest.pages[0].id.clone();
        let other = format!("other-{first}");
        let node = NodeId(991);
        paper_rs::mcp::exec::with_no_stretch(|s| {
            s.insert((first.clone(), node));
            s.insert((other.clone(), node));
        });
        let server = Server::open(Config {
            project: p.dir.clone(),
            web_dir: root.join("no-web"),
            port: 1,
            token: TOKEN.into(),
        })
        .unwrap();
        assert!(paper_rs::mcp::exec::with_no_stretch(|s| !s
            .contains(&(first.clone(), node))
            && s.contains(&(other.clone(), node))));
        let session = agent::new_session();
        server
            .call_tool(session, "get_basic_info", json!({}))
            .await
            .unwrap();
        let added = value(
            server
                .call_tool(session, "create_page", json!({"name":"Temporary"}))
                .await,
        )["pageId"]
            .as_str()
            .unwrap()
            .to_string();
        server
            .call_tool(
                session,
                "get_tree_summary",
                json!({"nodeId":"root","pageId":added}),
            )
            .await
            .unwrap();
        server.call_tool(session,"create_artboard",json!({"pageId":added,"styles":{"display":"flex","flexDirection":"column","alignItems":"flex-start","width":"450px","height":"200px"}})).await.unwrap();
        assert!(paper_rs::mcp::exec::with_no_stretch(|s| s
            .iter()
            .any(|(p, _)| p == &added)));
        let manifest = p.dir.join(MANIFEST);
        let mut m: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        m["pages"]
            .as_array_mut()
            .unwrap()
            .retain(|v| v["id"] != added);
        std::fs::write(&manifest, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
        let now = value(server.call_tool(session, "get_basic_info", json!({})).await);
        assert_eq!(now["pages"].as_array().unwrap().len(), 1);
        paper_rs::mcp::exec::with_no_stretch(|s| {
            s.insert((first.clone(), node));
        });
        server.retire().await;
        paper_rs::mcp::exec::with_no_stretch(|s| {
            assert!(!s.iter().any(|(p, _)| p == &first || p == &added));
            assert!(s.remove(&(other, node)));
        });
        let _ = std::fs::remove_dir_all(root);
    }

    /// Final review, P2 2: once both files are committed the save is reported as saved, with
    /// the revision of the bytes written; no fallible step follows the commit (a failure
    /// injected right after it, step 6, has nothing to hit).
    #[tokio::test(flavor = "current_thread")]
    async fn a_completed_browser_save_is_reported_as_saved() {
        use std::hash::{BuildHasher, RandomState};
        let root = std::env::temp_dir().join(format!(
            "opd-unit-put2-{:016x}",
            RandomState::new().hash_one(std::time::SystemTime::now())
        ));
        let (mut p, _) = paper_rs::project::Store::open(&root)
            .unwrap()
            .create("Put")
            .unwrap();
        // Make the before timestamp distinct without depending on clock resolution.
        p.manifest.modified = 0;
        p.write_manifest().unwrap();
        let id = p.manifest.pages[0].id.clone();
        let s = Server::open(Config {
            project: p.dir.clone(),
            web_dir: root.join("no-bundle"),
            port: 1,
            token: TOKEN.into(),
        })
        .unwrap();
        let before = (
            std::fs::read(p.page_path(&id)).unwrap(),
            std::fs::read(p.dir.join(MANIFEST)).unwrap(),
        );
        let mut page = p.load_page(&id).unwrap();
        page.doc.next_id += 1;
        commit::fault::fail_at(&[6]);
        let (st, msg) = put(&s, &id, &etag(&before.0), page_to_json(&page)).await;
        commit::fault::fail_at(&[]);
        let page_now = std::fs::read(p.page_path(&id)).unwrap();
        assert_ne!(page_now, before.0);
        assert_ne!(std::fs::read(p.dir.join(MANIFEST)).unwrap(), before.1);
        assert_eq!(st, StatusCode::OK, "{msg}");
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["revision"], serde_json::json!(etag(&page_now)), "{msg}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
