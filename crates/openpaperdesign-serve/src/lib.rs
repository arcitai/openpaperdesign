//! Loopback HTTP server for editing one paper-rs project folder in a browser.
//!
//! - Serves the built `openpaperdesign-web` bundle and a small JSON API.
//! - API calls need the per-launch token (`Authorization: Bearer …`); requests with a foreign
//!   `Host` (DNS rebinding) or a cross-origin `Origin`/`Sec-Fetch-Site` are refused.
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
//! - The web bundle (six allowlisted files) is checked and read into memory once at startup,
//!   from a folder outside the project; static requests never touch the file system.
//! - Unix only (Linux, macOS): the root identity and hard-link checks use Unix metadata, so the
//!   server refuses to start elsewhere.
//! - Writes are revision-checked: a page's revision (ETag) is a hash of its file **as it is
//!   on disk**, `PUT` needs a matching `If-Match`, and a stale one gets `409 Conflict` with
//!   the file left untouched. Saves go through upstream `project::Project::save_page`
//!   (format + atomic write), so the native app reads them as its own.
//!
//! Boundary: only writers that go through this server are serialized and revision-checked.
//! Because the revision is computed from the file on disk, a change made by another writer
//! (e.g. a running native paper-rs) before the browser's next save makes that save fail
//! with 409 instead of overwriting it. Nothing stops the other writer from overwriting the
//! browser's saves, and a write landing between this server's check and its rename is not
//! detected. Run one writer at a time.

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use paper_rs::model::Document;
use paper_rs::project::format::{page_from_json, page_to_json};
use paper_rs::project::{MANIFEST, Manifest, PAGES_DIR, PageData, Project, now_ms};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Files of the web bundle, by URL path. Nothing else is served from disk.
const BUNDLE: &[(&str, &str, &str)] = &[
    ("/", "index.html", "text/html; charset=utf-8"),
    ("/app.js", "app.js", "text/javascript; charset=utf-8"),
    ("/page.js", "page.js", "text/javascript; charset=utf-8"),
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
const MAX_PAGE_BYTES: usize = 64 << 20;

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
    /// Serializes revision check + write, so two saves through this server can't interleave.
    write_lock: tokio::sync::Mutex<()>,
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
enum Kind {
    File,
    Dir,
}

/// `Ok(true)` if `path` is a real file/dir of the wanted kind, `Ok(false)` if it doesn't
/// exist, and an error for a symlink, the wrong kind, or a file with more than one hard link
/// (which could alias a file outside the project).
fn plain(path: &Path, want: Kind) -> Result<bool, String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let m = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{name}: {e}")),
    };
    let t = m.file_type();
    if t.is_symlink() {
        return Err(format!("{name} is a symbolic link"));
    }
    match want {
        Kind::Dir if !t.is_dir() => Err(format!("{name} is not a directory")),
        Kind::File if !t.is_file() => Err(format!("{name} is not a regular file")),
        Kind::File if links(&m) > 1 => Err(format!("{name} has more than one hard link")),
        _ => Ok(true),
    }
}

#[cfg(unix)]
fn links(m: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.nlink()
}

#[cfg(unix)]
fn dev_ino(m: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (m.dev(), m.ino())
}

// Non-Unix: `Server::open` refuses to start, so these never run; they fail closed anyway.
#[cfg(not(unix))]
fn links(_m: &std::fs::Metadata) -> u64 {
    u64::MAX
}

#[cfg(not(unix))]
fn check_root(_dir: &Path, _id: (u64, u64)) -> Result<(), String> {
    Err("only supported on Unix".into())
}

/// The root must still be the directory opened at startup: a real directory (not a link),
/// with the same canonical path (catches swapped ancestors too) and the same (device, inode).
#[cfg(unix)]
fn check_root(dir: &Path, id: (u64, u64)) -> Result<(), String> {
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
fn check_project_dir(dir: &Path) -> Result<(), String> {
    if !plain(&dir.join(MANIFEST), Kind::File)? {
        return Err(format!("{MANIFEST} is missing"));
    }
    plain(&dir.join(PAGES_DIR), Kind::Dir)?;
    Ok(())
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
        Self::open_unix(cfg)
    }

    #[cfg(unix)]
    fn open_unix(cfg: Config) -> Result<Arc<Server>, OpenError> {
        let dir = cfg
            .project
            .canonicalize()
            .map_err(|e| OpenError::NotAProject(cfg.project.clone(), e.to_string()))?;
        let root_id = std::fs::symlink_metadata(&dir)
            .map(|m| dev_ino(&m))
            .map_err(|e| OpenError::NotAProject(dir.clone(), e.to_string()))?;
        // Before upstream `Project::open`: its stale-temp cleanup deletes files in `pages/`, so
        // that must not be a link to somewhere else.
        check_project_dir(&dir).map_err(OpenError::Unsafe)?;
        // Same check (and stale temp-file cleanup) as when the native app opens a project.
        Project::open(&dir).map_err(|e| OpenError::NotAProject(dir.clone(), e.to_string()))?;
        let bundle = load_bundle(&cfg.web_dir, &dir).map_err(OpenError::Unsafe)?;
        let p = cfg.port;
        Ok(Arc::new(Server {
            dir,
            root_id,
            bundle,
            token: cfg.token,
            hosts: [format!("127.0.0.1:{p}"), format!("localhost:{p}")],
            origins: [
                format!("http://127.0.0.1:{p}"),
                format!("http://localhost:{p}"),
            ],
            write_lock: tokio::sync::Mutex::new(()),
        }))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// How many bundle files were found at startup (0 = not built).
    pub fn bundle_files(&self) -> usize {
        self.bundle.len()
    }

    /// Root identity plus the project's own entries; run before every read and write.
    fn check(&self) -> Result<(), ApiError> {
        check_root(&self.dir, self.root_id).map_err(forbidden)?;
        check_project_dir(&self.dir).map_err(forbidden)
    }

    fn project(&self) -> Result<Project, ApiError> {
        // Re-read per request: the manifest may change while we run (e.g. pages renamed
        // natively). Not `Project::open`, which also deletes other writers' temp files.
        self.check()?;
        let manifest = std::fs::read_to_string(self.dir.join(MANIFEST))
            .map_err(|e| e.to_string())
            .and_then(|s| Manifest::from_json(&s).map_err(|e| e.to_string()))
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok(Project {
            dir: self.dir.clone(),
            manifest,
        })
    }
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

/// An API failure: status, message and (for 409) the page's current revision.
struct ApiError {
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
    {
        return error(StatusCode::FORBIDDEN, "cross-site request");
    }
    if req.uri().path().starts_with("/api/") {
        let ok = header_str(h, header::AUTHORIZATION)
            .and_then(|v| v.strip_prefix("Bearer "))
            .is_some_and(|t| same_bytes(t.as_bytes(), s.token.as_bytes()));
        if !ok {
            return error(StatusCode::UNAUTHORIZED, "missing or wrong token");
        }
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

pub fn router(s: Arc<Server>) -> Router {
    let mut r = Router::new()
        .route("/api/project", get(get_project))
        .route("/api/pages/{id}", get(get_page).put(put_page));
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

/// The manifest's entry for `id`, or an error response. Ids are checked against the
/// manifest (and for a plain character set), never used as client-chosen paths.
fn page_ref(p: &Project, id: &str) -> Result<String, ApiError> {
    let plain = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !plain {
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
    let result = (|| {
        let p = s.project()?;
        let name = page_ref(&p, &id)?;
        let bytes = read_page(&p, &id)?;
        let rev = revision(&bytes);
        let body = match bytes {
            Some(b) => b,
            // Same as upstream `Project::load_pages`: a listed page without a file is empty.
            None => page_to_json(&PageData {
                id: id.clone(),
                name: name.clone(),
                doc: Document {
                    name: name.clone(),
                    ..Default::default()
                },
                camera: None,
            })
            .into_bytes(),
        };
        Ok::<_, ApiError>(
            (
                [
                    (header::CONTENT_TYPE, "application/json".to_string()),
                    (header::ETAG, rev),
                ],
                body,
            )
                .into_response(),
        )
    })();
    result.unwrap_or_else(IntoResponse::into_response)
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
    let _write = s.write_lock.lock().await;
    let result = (|| {
        let mut p = s.project()?;
        let name = page_ref(&p, &id)?;
        let current = revision(&read_page(&p, &id)?);
        if if_match != current {
            return Err(ApiError {
                etag: Some(current),
                ..ApiError::new(
                    StatusCode::CONFLICT,
                    "the page changed since it was loaded; nothing was written",
                )
            });
        }
        let mut page = page_from_json(&body)
            .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
        if page.id != id {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "page id in the body does not match the URL",
            ));
        }
        // The manifest owns page names (as in upstream `Project::load_page`).
        page.name = name.clone();
        page.doc.name = name;
        // Checked again just before writing (`read_page` above checked the page file).
        s.check()?;
        p.save_page(&page)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        // Like the native session: a page write bumps the project's modified time.
        p.manifest.modified = now_ms();
        p.write_manifest()
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let rev = revision(&read_page(&p, &id)?);
        Ok((
            StatusCode::OK,
            [(header::ETAG, rev.clone())],
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "revision": rev }).to_string(),
        )
            .into_response())
    })();
    result.unwrap_or_else(IntoResponse::into_response)
}
