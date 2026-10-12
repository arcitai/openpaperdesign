//! Loopback HTTP server for editing one paper-rs project folder in a browser.
//!
//! - Serves the built `openpaperdesign-web` bundle and a small JSON API.
//! - API calls need the per-launch token (`Authorization: Bearer …`); requests with a foreign
//!   `Host` (DNS rebinding) or a cross-origin `Origin`/`Sec-Fetch-Site` are refused.
//! - Pages are addressed by the ids listed in the project's manifest only; the server builds
//!   every path itself, so no client input reaches the file system as a path.
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
use paper_rs::project::{MANIFEST, Manifest, PageData, Project, now_ms};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Files of the web bundle, by URL path. Nothing else is served from disk.
const BUNDLE: &[(&str, &str, &str)] = &[
    ("/", "index.html", "text/html; charset=utf-8"),
    ("/app.js", "app.js", "text/javascript; charset=utf-8"),
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
    dir: PathBuf,
    web_dir: PathBuf,
    token: String,
    hosts: [String; 2],
    origins: [String; 2],
    /// Serializes revision check + write, so two saves through this server can't interleave.
    write_lock: tokio::sync::Mutex<()>,
}

#[derive(Debug)]
pub enum OpenError {
    NotAProject(PathBuf, String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NotAProject(p, e) => {
                write!(f, "{} is not a paper-rs project folder: {e}", p.display())
            }
        }
    }
}

impl std::error::Error for OpenError {}

impl Server {
    /// Open an existing project folder. Nothing is created: a folder without a valid
    /// `project.json` is refused.
    pub fn open(cfg: Config) -> Result<Arc<Server>, OpenError> {
        let dir = cfg
            .project
            .canonicalize()
            .map_err(|e| OpenError::NotAProject(cfg.project.clone(), e.to_string()))?;
        // Same check (and stale temp-file cleanup) as when the native app opens a project.
        Project::open(&dir).map_err(|e| OpenError::NotAProject(dir.clone(), e.to_string()))?;
        let p = cfg.port;
        Ok(Arc::new(Server {
            dir,
            web_dir: cfg.web_dir,
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

    fn project(&self) -> Result<Project, ApiError> {
        // Re-read per request: the manifest may change while we run (e.g. pages renamed
        // natively). Not `Project::open`, which also deletes other writers' temp files.
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
    let mut res = next.run(req).await;
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
                match std::fs::read(s.web_dir.join(file)) {
                    Ok(b) => ([(header::CONTENT_TYPE, mime)], b).into_response(),
                    Err(e) => error(
                        StatusCode::NOT_FOUND,
                        &format!("web bundle file {file} not found ({e}); build it first"),
                    ),
                }
            }),
        );
    }
    r.layer(middleware::from_fn_with_state(s.clone(), guard))
        .layer(DefaultBodyLimit::max(MAX_PAGE_BYTES))
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
    match std::fs::read(p.page_path(id)) {
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
