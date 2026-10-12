//! Agent edits over MCP, on the bound project only.
//!
//! The MCP endpoint (`/mcp`) reuses upstream's tool catalogue and `mcp::exec::call`. One host
//! thread owns an upstream `Editor` holding the bound project's pages (the `Editor` isn't
//! `Send`, so like upstream's `mcp::serve_host` it lives on its own thread). Each call takes
//! the same write lock as browser saves and:
//!
//! 1. re-verifies the project root and entries (same checks as every browser request),
//! 2. reloads the project if any page file or the manifest changed on disk since the host last
//!    read or wrote it (e.g. a browser save),
//! 3. applies the policy below, runs the tool, and
//! 4. saves every changed page atomically through upstream `Project::save_page`, which changes
//!    that page's revision, so the browser picks the change up.
//!
//! Policy: `export` and `export_combined_pdf` are refused (they write files where the agent
//! says), and so is `create_file`. `list_resources`, `open_file` and any `fileId` argument only
//! accept the bound project. Image references (`<img src>`, CSS `url(...)`) must be `data:`
//! URIs, http(s) URLs (never fetched upstream) or ids of assets already loaded (the bound
//! project's own `assets/`, or images an agent sent as `data:` URIs). Inline `<svg>` must be
//! well-formed XML whose `href`s and `url(...)`s point inside the SVG (`#id`) or are `data:`
//! URIs, with no DTD and no `@import`. Anything else is refused before upstream code runs, so
//! no local path is ever read.

use crate::{
    ASSETS, Kind, Revisions, check_project_dir, check_root, etag, plain, read_manifest_checked,
};
use paper_rs::editor::Editor;
use paper_rs::mcp::html::{self, Html};
use paper_rs::mcp::server::{to_result, tool_list};
use paper_rs::mcp::{Call, ToolResult, exec, tools};
use paper_rs::project::{PageRef, Project, Store, now_ms};
use paper_rs::render::gpu::Headless;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Tools that are not offered at all.
pub const REFUSED_TOOLS: &[&str] = &["export", "export_combined_pdf", "create_file"];

const CALL_TIMEOUT: Duration = Duration::from_secs(120);

// ------------------------------------------------------------------------------ policy

/// The identity of the bound project, for `fileId` checks.
#[derive(Debug, Clone)]
pub struct Bound {
    pub dir: PathBuf,
    pub id: String,
    pub name: String,
}

impl Bound {
    fn matches(&self, key: &str) -> bool {
        let key = key.trim().trim_start_matches("/file/");
        key == self.id
            || key == self.name
            || Path::new(key).canonicalize().is_ok_and(|p| p == self.dir)
    }
}

/// Whether an image reference can be used without reading a local file.
fn allowed_image(src: &str) -> bool {
    let s = src.trim();
    if s.starts_with("data:") || s.starts_with("http://") || s.starts_with("https://") {
        return true;
    }
    // Same lookup as upstream `image_asset`: an id already registered.
    let id = s
        .strip_prefix("paper-asset://")
        .or_else(|| s.strip_prefix("file://"))
        .unwrap_or(s);
    !id.is_empty() && paper_rs::assets::get(id).is_some()
}

/// Every `url(...)` target in a string (case-insensitive, quotes trimmed).
fn css_urls(s: &str) -> Vec<String> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find("url(") {
        let start = at + i + 4;
        let end = s[start..].find(')').map_or(s.len(), |e| start + e);
        out.push(
            s[start..end]
                .trim()
                .trim_matches(['"', '\''])
                .trim()
                .to_string(),
        );
        at = end.min(s.len());
        if at <= start {
            break;
        }
    }
    out
}

fn refuse_image(src: &str) -> String {
    format!(
        "image reference `{src}` refused: openpaperdesign-serve doesn't read local files for \
         agents. Use a data: URI (or an image already in this project's assets)."
    )
}

/// Inline SVG must not reach outside itself.
fn check_svg(raw: &str) -> Result<(), String> {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("<!doctype") || lower.contains("<!entity") {
        return Err("inline SVG with a DTD is refused".into());
    }
    if lower.contains("@import") {
        return Err("inline SVG with @import is refused".into());
    }
    let doc = roxmltree::Document::parse(raw)
        .map_err(|e| format!("inline SVG must be well-formed XML ({e})"))?;
    let local = |v: &str| v.trim_start().starts_with('#') || v.trim_start().starts_with("data:");
    for n in doc.descendants() {
        for a in n.attributes() {
            if a.name() == "href" && !local(a.value()) {
                return Err(format!(
                    "inline SVG reference `{}` refused: only #id and data: URIs are allowed",
                    a.value()
                ));
            }
            for u in css_urls(a.value()) {
                if !local(&u) {
                    return Err(format!("inline SVG url({u}) refused"));
                }
            }
        }
        if let Some(t) = n.text() {
            for u in css_urls(t) {
                if !local(&u) {
                    return Err(format!("inline SVG url({u}) refused"));
                }
            }
        }
    }
    Ok(())
}

fn check_html(src: &str) -> Result<(), String> {
    fn walk(nodes: &[Html]) -> Result<(), String> {
        for n in nodes {
            match n {
                Html::Text(t) => {
                    for u in css_urls(t) {
                        if !allowed_image(&u) {
                            return Err(refuse_image(&u));
                        }
                    }
                }
                Html::Element(e) => {
                    for (k, v) in &e.attrs {
                        if (k.eq_ignore_ascii_case("src") || k.eq_ignore_ascii_case("srcset"))
                            && !allowed_image(v)
                        {
                            return Err(refuse_image(v));
                        }
                        for u in css_urls(v) {
                            if !allowed_image(&u) {
                                return Err(refuse_image(&u));
                            }
                        }
                    }
                    if e.tag == "svg" {
                        check_svg(e.raw.as_deref().unwrap_or(""))?;
                    }
                    walk(&e.children)?;
                }
            }
        }
        Ok(())
    }
    walk(&html::parse(src))
}

fn check_strings(v: &Value) -> Result<(), String> {
    match v {
        Value::String(s) => {
            for u in css_urls(s) {
                if !allowed_image(&u) {
                    return Err(refuse_image(&u));
                }
            }
            Ok(())
        }
        Value::Array(a) => a.iter().try_for_each(check_strings),
        Value::Object(o) => o.values().try_for_each(check_strings),
        _ => Ok(()),
    }
}

/// Check one call against the policy. Returns the arguments to pass on (with `fileId`
/// removed once it has been checked).
pub fn check_call(name: &str, args: &Value, bound: &Bound) -> Result<Value, String> {
    if REFUSED_TOOLS.contains(&name) {
        return Err(format!(
            "`{name}` is disabled in openpaperdesign-serve: it would create or write files \
             outside the project this server is bound to"
        ));
    }
    let mut args = match args {
        Value::Object(o) => o.clone(),
        Value::Null => Map::new(),
        _ => return Err("arguments must be an object".into()),
    };
    if let Some(key) = args.get("fileId").and_then(Value::as_str)
        && !key.trim().is_empty()
        && !bound.matches(key)
    {
        return Err(format!(
            "`{key}` is not the project this server is bound to ({}); only that project can \
             be used",
            bound.name
        ));
    }
    args.remove("fileId");
    // `html` is parsed (inline SVG has its own rules); every other string is scanned as CSS.
    for (k, v) in &args {
        match (k.as_str(), v) {
            ("html", Value::String(h)) => check_html(h)?,
            _ => check_strings(v)?,
        }
    }
    Ok(Value::Object(args))
}

// ------------------------------------------------------------------------------ host

/// The upstream `exec::Host` for the bound project.
pub struct BoundHost {
    pub ed: Editor,
    project: Project,
    store: Store,
    /// Revision of each page file (and of the manifest) as the editor last read or wrote it.
    known: HashMap<String, String>,
    gpu: Option<Headless>,
    gpu_tried: bool,
}

const MANIFEST_KEY: &str = "\0manifest";

fn disk_revisions(project: &Project) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    let m = std::fs::read(project.dir.join(paper_rs::project::MANIFEST))
        .map_err(|e| format!("project.json: {e}"))?;
    out.insert(MANIFEST_KEY.to_string(), etag(&m));
    for r in &project.manifest.pages {
        let path = project.page_path(&r.id);
        if plain(&path, Kind::File)? {
            let b = std::fs::read(&path).map_err(|e| format!("{}: {e}", r.id))?;
            out.insert(r.id.clone(), etag(&b));
        }
    }
    Ok(out)
}

/// Register the project's own images: plain single-link files in `assets/` only.
fn load_assets(dir: &Path) -> Result<(), String> {
    let assets = dir.join(ASSETS);
    if !plain(&assets, Kind::Dir)? {
        return Ok(());
    }
    let rd = std::fs::read_dir(&assets).map_err(|e| format!("assets: {e}"))?;
    for e in rd.flatten() {
        if plain(&e.path(), Kind::File).unwrap_or(false)
            && let Ok(bytes) = std::fs::read(e.path())
        {
            let _ = paper_rs::assets::register(&bytes);
        }
    }
    Ok(())
}

impl BoundHost {
    pub fn open(dir: &Path) -> Result<BoundHost, String> {
        let mut h = BoundHost {
            ed: Editor::default(),
            project: Project {
                dir: dir.to_path_buf(),
                manifest: read_manifest_checked(dir)?,
            },
            // Never listed or searched: project tools are answered by the adapter.
            store: Store {
                root: dir.to_path_buf(),
            },
            known: HashMap::new(),
            gpu: None,
            gpu_tried: false,
        };
        h.load()?;
        Ok(h)
    }

    pub fn bound(&self) -> Bound {
        Bound {
            dir: self.project.dir.clone(),
            id: self.project.manifest.id.clone(),
            name: self.project.manifest.name.clone(),
        }
    }

    /// (Re)load every page from disk. Undo history starts over.
    fn load(&mut self) -> Result<(), String> {
        self.project.manifest = read_manifest_checked(&self.project.dir)?;
        load_assets(&self.project.dir)?;
        let pages = self.project.load_pages().map_err(|e| e.to_string())?;
        let active = self.project.manifest.active_page.clone();
        self.ed.open_pages(pages, active.as_deref());
        for i in 0..self.ed.pages.len() {
            self.ed.mark_page_saved(i);
        }
        self.known = disk_revisions(&self.project)?;
        Ok(())
    }

    /// Reload if anything changed on disk since the editor last read or wrote it.
    pub fn sync(&mut self) -> Result<(), String> {
        let manifest = read_manifest_checked(&self.project.dir)?;
        let now = disk_revisions(&Project {
            dir: self.project.dir.clone(),
            manifest,
        })?;
        if now != self.known {
            self.load()?;
        }
        Ok(())
    }

    /// Save every changed page (and the manifest when needed) like the native session does.
    /// Returns (page id, revision, bytes) of each page written.
    pub fn save_changes(&mut self) -> Result<Vec<(String, String, Vec<u8>)>, String> {
        let ed = &mut self.ed;
        ed.commit_pending();
        let dirty = ed.dirty_pages();
        if dirty.is_empty() && !ed.meta_dirty {
            return Ok(Vec::new());
        }
        check_project_dir(&self.project.dir)?;
        plain(&self.project.dir.join(ASSETS), Kind::Dir)?;
        let mut written = Vec::new();
        for &i in &dirty {
            let data = ed.page_data(i);
            paper_rs::assets::save_to_dir(
                &self.project.dir.join(ASSETS),
                paper_rs::assets::used_by(&data.doc),
            )
            .map_err(|e| e.to_string())?;
            self.project.save_page(&data).map_err(|e| e.to_string())?;
            ed.mark_page_saved(i);
            let path = self.project.page_path(&data.id);
            let b = std::fs::read(&path).map_err(|e| e.to_string())?;
            written.push((data.id.clone(), etag(&b), b));
        }
        self.project.manifest.pages = ed
            .pages
            .iter()
            .map(|s| PageRef {
                id: s.id.clone(),
                name: s.name.clone(),
            })
            .collect();
        self.project.manifest.modified = now_ms();
        self.project.write_manifest().map_err(|e| e.to_string())?;
        ed.meta_dirty = false;
        self.known = disk_revisions(&self.project)?;
        Ok(written)
    }
}

impl exec::Host for BoundHost {
    fn editor(&mut self) -> &mut Editor {
        &mut self.ed
    }
    fn store(&self) -> &Store {
        &self.store
    }
    fn project(&self) -> Option<&Project> {
        Some(&self.project)
    }
    fn canvas_open(&self) -> bool {
        true
    }
    fn open_project(&mut self, dir: &Path) -> Result<(), String> {
        if dir.canonicalize().ok().as_deref() == Some(self.project.dir.as_path()) {
            Ok(())
        } else {
            Err("only the project this server is bound to can be opened".into())
        }
    }
    fn save(&mut self) -> Result<(), String> {
        // Changes are saved after every call anyway (see `run`).
        Ok(())
    }
    fn gpu(&mut self) -> Option<Headless> {
        if !self.gpu_tried {
            self.gpu_tried = true;
            self.gpu = Headless::new().ok();
        }
        self.gpu.as_ref().map(|g| Headless {
            device: g.device.clone(),
            queue: g.queue.clone(),
            adapter_info: g.adapter_info.clone(),
        })
    }
}

fn list_resources(host: &BoundHost) -> ToolResult {
    let p = &host.project;
    Ok(vec![paper_rs::mcp::Out::Text(
        json!({
            "items": [{
                "id": p.manifest.id,
                "type": "file",
                "name": p.manifest.name,
                "pages": p.manifest.pages.len(),
                "updatedAt": p.manifest.modified,
                "open": true,
                "active": true,
            }],
            "note": "openpaperdesign-serve is bound to this one project",
        })
        .to_string(),
    )])
}

/// One tool call on the host thread, with the write lock held by the caller.
pub fn run(
    host: &mut BoundHost,
    root_id: (u64, u64),
    revs: &Mutex<Revisions>,
    name: &str,
    args: &Value,
) -> ToolResult {
    let dir = host.project.dir.clone();
    check_root(&dir, root_id)?;
    check_project_dir(&dir)?;
    host.sync()?;
    let args = check_call(name, args, &host.bound())?;
    let r = match name {
        "list_resources" => list_resources(host),
        "open_file" => {
            // Upstream resolves an absolute project path itself: hand it only ours.
            let mut a = args.as_object().cloned().unwrap_or_default();
            a.insert("fileId".into(), json!(dir.to_string_lossy()));
            exec::call(host, name, &Value::Object(a))
        }
        _ => exec::call(host, name, &args),
    };
    let written = host.save_changes();
    let mut revs = revs.lock().unwrap();
    match written {
        Ok(pages) => {
            for (id, tag, bytes) in pages {
                revs.record(&id, &tag, bytes);
            }
            r
        }
        Err(e) => Err(match r {
            Ok(_) => format!("the change was made but could not be saved: {e}"),
            Err(r) => format!("{r}; saving also failed: {e}"),
        }),
    }
}

/// Start the host thread. Calls are run one at a time under `lock`.
pub fn spawn_host(
    dir: PathBuf,
    root_id: (u64, u64),
    lock: Arc<tokio::sync::Mutex<()>>,
    revs: Arc<Mutex<Revisions>>,
) -> Result<Sender<Call>, String> {
    let (tx, rx) = std::sync::mpsc::channel::<Call>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("opd-mcp-host".into())
        .spawn(move || {
            let host = {
                let _g = lock.blocking_lock();
                BoundHost::open(&dir)
            };
            let mut host = match host {
                Ok(h) => {
                    let _ = ready_tx.send(Ok(()));
                    h
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            while let Ok(call) = rx.recv() {
                let r = {
                    let _g = lock.blocking_lock();
                    run(&mut host, root_id, &revs, &call.name, &call.args)
                };
                let _ = call.reply.send(r);
            }
        })
        .map_err(|e| e.to_string())?;
    ready_rx
        .recv()
        .map_err(|_| "the MCP host thread stopped".to_string())??;
    Ok(tx)
}

// ------------------------------------------------------------------------------ MCP

/// The tools offered: upstream's catalogue without the refused ones.
pub fn offered_tools() -> Vec<Tool> {
    tool_list()
        .into_iter()
        .filter(|t| !REFUSED_TOOLS.contains(&t.name.as_ref()))
        .collect()
}

#[derive(Clone)]
pub struct AgentServer {
    calls: Arc<Mutex<Sender<Call>>>,
}

impl AgentServer {
    pub fn new(calls: Sender<Call>) -> AgentServer {
        AgentServer {
            calls: Arc::new(Mutex::new(calls)),
        }
    }
}

impl ServerHandler for AgentServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "openpaperdesign",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(format!(
                "{}\n\nopenpaperdesign-serve: these tools edit the one project this server is \
                 bound to; a person may be editing it in a browser at the same time and sees \
                 your changes live. Projects can't be created or switched, export is disabled, \
                 and images must be data: URIs.",
                tools::INSTRUCTIONS
            ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(offered_tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        let call = Call {
            name: request.name.to_string(),
            args: Value::Object(request.arguments.unwrap_or_default()),
            reply,
        };
        let sent = self.calls.lock().unwrap().send(call).is_ok();
        let r = if !sent {
            Err("the project host has stopped".to_string())
        } else {
            match tokio::time::timeout(CALL_TIMEOUT, rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => Err("the project host dropped the call".into()),
                Err(_) => Err("the call did not finish in time".into()),
            }
        };
        Ok(to_result(r).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound() -> Bound {
        Bound {
            dir: PathBuf::from("/nonexistent/bound"),
            id: "abc".into(),
            name: "Bound".into(),
        }
    }

    #[test]
    fn refused_tools_and_other_projects() {
        for t in REFUSED_TOOLS {
            assert!(check_call(t, &json!({}), &bound()).is_err(), "{t}");
            assert!(!offered_tools().iter().any(|x| x.name == *t), "{t} offered");
        }
        assert!(check_call("get_basic_info", &json!({"fileId": "abc"}), &bound()).is_ok());
        assert!(check_call("get_basic_info", &json!({"fileId": "Bound"}), &bound()).is_ok());
        for other in ["other", "/tmp", "../x", "/file/zzz"] {
            assert!(
                check_call("get_basic_info", &json!({"fileId": other}), &bound()).is_err(),
                "{other}"
            );
        }
        let a = check_call("get_basic_info", &json!({"fileId": "abc"}), &bound()).unwrap();
        assert!(a.get("fileId").is_none(), "checked fileId is removed");
    }

    #[test]
    fn local_image_paths_are_refused() {
        let b = bound();
        let html = |h: &str| check_call("write_html", &json!({"html": h}), &b);
        let style = |s: &str| {
            check_call(
                "update_styles",
                &json!({"updates": [{"nodeIds": ["1"], "styles": {"background-image": s}}]}),
                &b,
            )
        };
        for bad in [
            "/etc/passwd",
            "../secret.png",
            "file:///etc/passwd",
            "paper-asset:///home/x/a.png",
            "a.png",
            "#frag",
            "DATA:image/png;base64,AA==",
        ] {
            assert!(html(&format!("<img src=\"{bad}\">")).is_err(), "img {bad}");
            assert!(
                html(&format!("<div style=\"background:url('{bad}')\"></div>")).is_err(),
                "css {bad}"
            );
            assert!(
                style(&format!("url({bad})")).is_err(),
                "update_styles {bad}"
            );
        }
        // Entity-encoded attributes are checked after decoding.
        assert!(html("<div style=\"background:u&#114;l(/etc/passwd)\"></div>").is_err());
        assert!(html("<IMG SRC=/etc/passwd>").is_err());
        // Allowed: data: URIs and remote URLs (upstream never fetches those).
        assert!(html("<img src=\"data:image/png;base64,iVBORw0KGgo=\">").is_ok());
        assert!(style("url(data:image/png;base64,iVBORw0KGgo=)").is_ok());
        assert!(html("<div style=\"color:red\">no images</div>").is_ok());
    }

    #[test]
    fn inline_svg_must_stay_inside_itself() {
        let b = bound();
        let svg = |s: &str| check_call("write_html", &json!({"html": s}), &b);
        assert!(svg("<svg viewBox=\"0 0 10 10\"><rect width=\"10\" height=\"10\" fill=\"url(#g)\"/><defs><linearGradient id=\"g\"/></defs></svg>").is_ok());
        for bad in [
            "<svg><image href=\"/etc/passwd\"/></svg>",
            "<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\"><image xlink:href=\"file:///etc/x.png\"/></svg>",
            "<svg><image href=\"../x.png\"/></svg>",
            "<svg><rect style=\"fill:url(/etc/x)\"/></svg>",
            "<svg><style>@import url(x.css);</style></svg>",
            "<!DOCTYPE svg [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><svg>&x;</svg>",
            "<svg><rect></svg>",
        ] {
            assert!(svg(bad).is_err(), "{bad}");
        }
    }
}
