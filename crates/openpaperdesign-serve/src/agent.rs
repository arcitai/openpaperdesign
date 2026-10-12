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
//! 3. applies the policy below, runs the tool,
//! 4. refuses the change (and discards it) if it touches a page this MCP session hasn't read,
//!    or whose content or metadata (name, position) changed since the session last read it,
//!    e.g. a browser save landed after the agent looked, and
//! 5. otherwise saves every changed page atomically through upstream `Project::save_page`,
//!    which changes that page's revision, so the browser picks the change up.
//!
//! What a session has read is kept per page: a page counts as read at a revision only when a
//! call that succeeded returned that page's content or state (`read_keys`), or when the
//! session's own successful call wrote that revision. The page list (names, order) is one
//! more item, read only through `get_basic_info`. Failed calls, and calls that can't be
//! attributed to a page, acknowledge nothing. A change must find every page it writes (and
//! the page list, if it changes it) read at its current revision. A session that hasn't read
//! (a new one, or one whose record was dropped because more than `MAX_SESSIONS` sessions
//! were seen) can't change existing pages until it reads; it can still create new ones.
//!
//! Policy: `export` and `export_combined_pdf` are refused (they write files where the agent
//! says), and so is `create_file`. `list_resources`, `open_file` and any `fileId` argument only
//! accept the bound project. Image references (`<img src>`, CSS `url(...)`) must be `data:`
//! URIs, http(s) URLs (never fetched upstream) or ids of assets already loaded (the bound
//! project's own `assets/`, or images an agent sent as `data:` URIs). Inline `<svg>` must be
//! well-formed XML with no DTD, no `@import` and no `<image>`/`<feImage>` elements (usvg reads
//! their href as a file name when it isn't a valid data URI; use HTML `<img>` with a `data:`
//! URI instead), and every `href` or `url(...)` must be `#id` of an element in that SVG.
//! Anything else is refused before upstream code runs, so no local path is ever read.

use crate::{
    ASSETS, Kind, check_project_dir, check_root, etag, page_prefix, plain, read_manifest_checked,
    valid_page_id,
};
use paper_rs::editor::Editor;
use paper_rs::mcp::html::{self, Html};
use paper_rs::mcp::server::{to_result, tool_list};
use paper_rs::mcp::{Call, ToolResult, exec, tools};
use paper_rs::project::format::page_from_json;
use paper_rs::project::{PAGES_DIR, PageData, PageRef, Project, Store, now_ms};
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
    let ids: std::collections::HashSet<&str> = doc
        .descendants()
        .filter_map(|n| n.attribute("id"))
        .collect();
    // `#id` must name an element of this same SVG.
    let internal = |v: &str| {
        v.trim()
            .strip_prefix('#')
            .is_some_and(|id| !id.is_empty() && ids.contains(id))
    };
    for n in doc.descendants() {
        let tag = n.tag_name().name();
        if tag.eq_ignore_ascii_case("image") || tag.eq_ignore_ascii_case("feimage") {
            return Err(format!(
                "inline SVG <{tag}> is refused: put images in HTML <img src=\"data:...\"> instead"
            ));
        }
        for a in n.attributes() {
            if a.name() == "href" && !internal(a.value()) {
                return Err(format!(
                    "inline SVG reference `{}` refused: only #id of an element in the same SVG \
                     is allowed",
                    a.value().trim()
                ));
            }
            for u in css_urls(a.value()) {
                if !internal(&u) {
                    return Err(format!("inline SVG url({u}) refused"));
                }
            }
        }
        if let Some(t) = n.text() {
            for u in css_urls(t) {
                if !internal(&u) {
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
    /// Page revisions each MCP session last saw (see the module docs, step 4).
    seen: std::collections::BTreeMap<u64, HashMap<String, String>>,
    gpu: Option<Headless>,
    gpu_tried: bool,
}

const MANIFEST_KEY: &str = "\0manifest";

/// Key of the page list (ids, names and order: the manifest's page metadata) in the revision
/// maps, observed and checked as one item.
const PAGES_KEY: &str = "\0pages";

/// The page list's revision: the hash of a canonical JSON array of `[id, name]` pairs, so
/// no two different lists share an encoding (whatever characters names contain).
fn pages_rev<'a>(pages: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    let list: Vec<[&str; 2]> = pages.map(|(id, name)| [id, name]).collect();
    etag(&serde_json::to_vec(&list).expect("strings serialize"))
}

/// A listed page that has no file yet: still a page whose content can be stale.
const NO_FILE: &str = "\0none";

/// Argument key the MCP handler sets to its session number (overwriting any client value);
/// stripped before the call is checked or run.
pub const SESSION_ARG: &str = "_openpaperdesignSession";
/// Sessions whose reads are remembered. Beyond this the oldest is forgotten, and must read
/// again before it changes a page (never a silent pass).
const MAX_SESSIONS: usize = 256;

/// The project's pages, read safely: ids come from the checked manifest, and `pages/` and each
/// page file are checked (no links, a single hard link) before any byte is read. Each page
/// takes its id and name from its manifest entry. Also returns the
/// revision of each page file (from the same bytes) and of the manifest.
fn read_pages(project: &Project) -> Result<(Vec<PageData>, HashMap<String, String>), String> {
    let mut revs = HashMap::new();
    let m = std::fs::read(project.dir.join(paper_rs::project::MANIFEST))
        .map_err(|e| format!("project.json: {e}"))?;
    revs.insert(MANIFEST_KEY.to_string(), etag(&m));
    let have_dir = plain(&project.dir.join(PAGES_DIR), Kind::Dir)?;
    let mut pages = Vec::new();
    // Distinct pages must be distinct files (backs up the case-folded id check).
    let mut files = std::collections::HashSet::new();
    for r in &project.manifest.pages {
        if !valid_page_id(&r.id) {
            return Err(format!("unsafe page id {:?}", r.id));
        }
        let path = project.page_path(&r.id);
        let mut page = if have_dir && plain(&path, Kind::File)? {
            let m = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", r.id))?;
            if !files.insert(crate::dev_ino(&m)) {
                return Err(format!("page {:?} is the same file as another page", r.id));
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", r.id))?;
            revs.insert(r.id.clone(), etag(&bytes));
            let text = String::from_utf8(bytes).map_err(|e| format!("{}: {e}", r.id))?;
            page_from_json(&text).map_err(|e| format!("{}: {e}", r.id))?
        } else {
            // A listed page without a file starts empty, as in upstream `load_pages`.
            revs.insert(r.id.clone(), NO_FILE.to_string());
            PageData {
                id: String::new(),
                name: r.name.clone(),
                doc: paper_rs::model::Document {
                    name: r.name.clone(),
                    ..Default::default()
                },
                camera: None,
            }
        };
        // The manifest entry owns the page's identity and name (renames only write the
        // manifest), whatever the file says; as upstream `Project::load_page`.
        page.id = r.id.clone();
        page.name = r.name.clone();
        page.doc.name = r.name.clone();
        pages.push(page);
    }
    revs.insert(
        PAGES_KEY.to_string(),
        pages_rev(
            project
                .manifest
                .pages
                .iter()
                .map(|p| (p.id.as_str(), p.name.as_str())),
        ),
    );
    Ok((pages, revs))
}

fn disk_revisions(project: &Project) -> Result<HashMap<String, String>, String> {
    read_pages(project).map(|(_, revs)| revs)
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
            seen: Default::default(),
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
        let (pages, revs) = read_pages(&self.project)?;
        let active = self.project.manifest.active_page.clone();
        self.ed.open_pages(pages, active.as_deref());
        for i in 0..self.ed.pages.len() {
            self.ed.mark_page_saved(i);
        }
        self.known = revs;
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
    /// Each written page gets a new revision (its on-disk hash).
    pub fn save_changes(&mut self) -> Result<(), String> {
        let ed = &mut self.ed;
        ed.commit_pending();
        let dirty = ed.dirty_pages();
        if dirty.is_empty() && !ed.meta_dirty {
            return Ok(());
        }
        check_project_dir(&self.project.dir)?;
        plain(&self.project.dir.join(ASSETS), Kind::Dir)?;
        plain(&self.project.dir.join(PAGES_DIR), Kind::Dir)?;
        // Write only to safe names (listed pages come from the checked manifest, new ones get
        // upstream's random hex ids) and never through a link.
        if let Some(bad) = ed.pages.iter().find(|p| !valid_page_id(&p.id)) {
            return Err(format!(
                "refusing to save a page with unsafe id {:?}",
                bad.id
            ));
        }
        let mut folded = std::collections::HashSet::new();
        if let Some(dup) = ed
            .pages
            .iter()
            .find(|p| !folded.insert(p.id.to_ascii_lowercase()))
        {
            return Err(format!(
                "refusing to save: page id {:?} differs from another only in case",
                dup.id
            ));
        }
        let mut prefixes = std::collections::HashSet::new();
        if let Some(dup) = ed
            .pages
            .iter()
            .find(|p| !prefixes.insert(page_prefix(&p.id)))
        {
            return Err(format!(
                "refusing to save: page id {:?} shares its first six characters with another",
                dup.id
            ));
        }
        for &i in &dirty {
            let data = ed.page_data(i);
            plain(&self.project.page_path(&data.id), Kind::File)?;
            paper_rs::assets::save_to_dir(
                &self.project.dir.join(ASSETS),
                paper_rs::assets::used_by(&data.doc),
            )
            .map_err(|e| e.to_string())?;
            self.project.save_page(&data).map_err(|e| e.to_string())?;
            ed.mark_page_saved(i);
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
        Ok(())
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

/// Why this call may not change what it changed, if so. Each existing page whose content it
/// changed or which it removed must have been observed by `session` at its current revision,
/// and so must the page list if the call changed it (a rename, a new or removed page, a new
/// order).
fn stale_change(
    host: &mut BoundHost,
    session: u64,
    before: &HashMap<String, String>,
) -> Option<String> {
    host.ed.commit_pending();
    let empty = HashMap::new();
    let seen = host.seen.get(&session).unwrap_or(&empty);
    let ed = &host.ed;
    let now: std::collections::HashSet<&str> = ed.pages.iter().map(|p| p.id.as_str()).collect();
    let mut keys: Vec<String> = ed
        .dirty_pages()
        .into_iter()
        .map(|i| ed.pages[i].id.clone())
        .chain(
            before
                .keys()
                .filter(|k| !k.starts_with('\0') && !now.contains(k.as_str()))
                .cloned(),
        )
        .collect();
    let list = pages_rev(ed.pages.iter().map(|p| (p.id.as_str(), p.name.as_str())));
    if before.get(PAGES_KEY) != Some(&list) {
        keys.push(PAGES_KEY.to_string());
    }
    for k in keys {
        // Pages new in this call have nothing before it.
        let Some(rev) = before.get(&k) else { continue };
        let what = if k == PAGES_KEY {
            "the page list (names, order)".to_string()
        } else {
            host.project
                .manifest
                .pages
                .iter()
                .find(|p| p.id == k)
                .map_or(format!("page {k}"), |p| format!("page {} ({k})", p.name))
        };
        match seen.get(&k) {
            Some(s) if s == rev => {}
            Some(_) => {
                return Some(format!(
                    "conflict: {what} was changed by someone else (e.g. in the browser) since \
                     you last read it"
                ));
            }
            None => {
                return Some(format!(
                    "conflict: {what} hasn't been read in this MCP session yet (or the \
                     session's reads were forgotten)"
                ));
            }
        }
    }
    None
}

/// What a successful read call returned, as revision keys: the pages whose content or state
/// it returned (resolved like upstream: `pageId`, else the page the user is viewing; node
/// keys `<page prefix>:<n>` name their own page), and the page list for `get_basic_info`.
/// Calls that can't be attributed to specific pages, or that return matches or a selection
/// rather than a page's content (`find_nodes`, `get_selection`, guides, fonts, ...), return
/// nothing here, so they acknowledge nothing. See also `returned`.
fn read_keys(ed: &Editor, name: &str, args: &Value) -> Vec<String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim);
    let find = |key: &str| -> Option<usize> {
        ed.pages
            .iter()
            .position(|p| p.id == key)
            .or_else(|| {
                ed.pages
                    .iter()
                    .position(|p| p.id.starts_with(key) && key.len() >= 4)
            })
            .or_else(|| {
                ed.pages
                    .iter()
                    .position(|p| p.name.eq_ignore_ascii_case(key))
            })
    };
    let default = match s("pageId") {
        Some(p) if !p.is_empty() => find(p),
        _ => Some(ed.active_page),
    };
    let node = |key: &str| -> Option<usize> {
        let key = key.trim().trim_start_matches('#');
        match key.split_once(':') {
            Some((p, _)) => find(p),
            None => default,
        }
    };
    let root_or_node = |k: Option<&str>| match k.filter(|k| !k.is_empty() && *k != "root") {
        Some(k) => node(k),
        None => default,
    };
    let pages: Vec<Option<usize>> = match name {
        "get_basic_info" => vec![default],
        "get_node_info" | "get_jsx" | "get_fill_image" => vec![s("nodeId").and_then(node)],
        "get_children" | "get_tree_summary" | "get_screenshot" => vec![root_or_node(s("nodeId"))],
        "get_computed_styles" => match args.get("nodeIds").and_then(Value::as_array) {
            Some(ids) => ids
                .iter()
                .map(|v| match v {
                    Value::String(k) => node(k),
                    Value::Number(_) => default,
                    _ => None,
                })
                .collect(),
            None => vec![None],
        },
        _ => vec![None],
    };
    if pages.iter().any(Option::is_none) {
        return Vec::new();
    }
    let mut keys: Vec<String> = pages
        .into_iter()
        .flatten()
        .filter_map(|i| ed.pages.get(i).map(|p| p.id.clone()))
        .collect();
    if name == "get_basic_info" {
        keys.push(PAGES_KEY.to_string());
    }
    keys
}

/// Whether a successful read's output really contains page `key`, for tools whose output
/// can leave a requested page out: `get_computed_styles` answers with one map keyed by node
/// id, so a page counts only if one of its node ids is a key there.
fn returned(name: &str, out: &[paper_rs::mcp::Out], key: &str) -> bool {
    if name != "get_computed_styles" || key.starts_with('\0') {
        return true;
    }
    let prefix = page_prefix(key);
    out.iter().any(|o| match o {
        paper_rs::mcp::Out::Text(t) => serde_json::from_str::<Map<String, Value>>(t)
            .map(|m| {
                m.keys().any(|k| {
                    k.split_once(':')
                        .is_some_and(|(p, _)| p.to_ascii_lowercase() == prefix)
                })
            })
            .unwrap_or(false),
        _ => false,
    })
}

/// One tool call on the host thread, with the write lock held by the caller.
pub fn run(host: &mut BoundHost, root_id: (u64, u64), name: &str, args: &Value) -> ToolResult {
    let dir = host.project.dir.clone();
    check_root(&dir, root_id)?;
    check_project_dir(&dir)?;
    host.sync()?;
    let mut args = args.clone();
    let session = args
        .as_object_mut()
        .and_then(|a| a.remove(SESSION_ARG))
        .and_then(|v| v.as_u64());
    let before = host.known.clone();
    let args = check_call(name, &args, &host.bound())?;
    // Worked out before the call, on the pages as they are; used only if it succeeds.
    let reads = read_keys(&host.ed, name, &args);
    // Nothing reaches the disk during the call (`Host::save` is a no-op, export and file
    // creation are refused): changes are saved below, only after a successful result.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match name {
        "list_resources" => list_resources(host),
        "open_file" => {
            // Upstream resolves an absolute project path itself: hand it only ours.
            let mut a = args.as_object().cloned().unwrap_or_default();
            a.insert("fileId".into(), json!(dir.to_string_lossy()));
            exec::call(host, name, &Value::Object(a))
        }
        _ => exec::call(host, name, &args),
    }))
    .unwrap_or_else(|_| Err(format!("{name} failed unexpectedly")));
    if r.is_err() {
        // A call that fails changes nothing: drop whatever it changed in memory (the editor
        // was in step with the disk before the call, so reloading restores it), save
        // nothing and acknowledge nothing.
        host.load()?;
        return r;
    }
    if let Some(s) = session
        && let Some(why) = stale_change(host, s, &before)
    {
        // Drop the change: it wasn't decided on the page as it is now.
        host.load()?;
        return Err(format!(
            "{why}, so this change was not applied and nothing was written. Read every page \
             you change (get_tree_summary with nodeId \"root\" and its pageId, or get_node_info \
             on its nodes; get_basic_info for the page list) and redo the change on what is \
             there now."
        ));
    }
    let saved = host.save_changes();
    if saved.is_err() {
        // Not saved: drop the call's changes, as for a failed call.
        host.load()?;
    }
    // Only a call that succeeded acknowledges anything: a failed or refused call, or a
    // failed save, leaves what the session has observed as it was.
    if let Some(s) = session
        && saved.is_ok()
        && r.is_ok()
    {
        if !host.seen.contains_key(&s) && host.seen.len() >= MAX_SESSIONS {
            // Forgetting fails closed: that session must read again before changing a page.
            host.seen.pop_first();
        }
        let seen = host.seen.entry(s).or_default();
        if before == host.known {
            // Nothing written: acknowledge exactly the pages (and page list) it returned.
            let out = r.as_deref().unwrap_or(&[]);
            for k in reads.iter().filter(|k| returned(name, out, k)) {
                if let Some(v) = host.known.get(k) {
                    seen.insert(k.clone(), v.clone());
                }
            }
        } else {
            // What this call wrote is observed as written; nothing else is acknowledged.
            for (k, v) in &host.known {
                if k != MANIFEST_KEY && before.get(k) != Some(v) {
                    seen.insert(k.clone(), v.clone());
                }
            }
        }
    }
    match saved {
        Ok(()) => r,
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
                    run(&mut host, root_id, &call.name, &call.args)
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
    /// One per MCP session (the service makes a handler per session).
    session: u64,
}

impl AgentServer {
    pub fn new(calls: Sender<Call>) -> AgentServer {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        AgentServer {
            calls: Arc::new(Mutex::new(calls)),
            session: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
        let mut args = request.arguments.unwrap_or_default();
        args.insert(SESSION_ARG.into(), json!(self.session));
        let call = Call {
            name: request.name.to_string(),
            args: Value::Object(args),
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

    #[test]
    fn svg_image_elements_are_refused_and_references_stay_inside() {
        let b = bound();
        let svg = |s: &str| check_call("write_html", &json!({"html": s}), &b);
        // Review round 1 finding 2 (`#private.png`) and round 2 finding 1 (usvg reads a
        // malformed data: href as a file name): no image element is accepted at all.
        for bad in [
            "<svg xmlns='http://www.w3.org/2000/svg' width='30' height='20'><image href='#private.png' width='30' height='20'/></svg>",
            "<svg xmlns='http://www.w3.org/2000/svg' width='30' height='20'><image href='data:private.png' width='30' height='20'/></svg>",
            "<svg><image href='data:'/></svg>",
            "<svg><image href=' data:image/png;base64,!!!'/></svg>",
            "<svg><image href=\"data:image/png;base64,iVBORw0KGgo=\"/></svg>",
            "<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\"><image xlink:href=\"data:private.png\"/></svg>",
            "<svg><filter id=\"f\"><feImage href=\"data:private.png\"/></filter></svg>",
            "<svg><filter id=\"f\"><FEIMAGE href=\"#f\"/></filter></svg>",
            "<svg><use href=\"data:private.png\"/></svg>",
            "<svg><use href=\"#missing.png\"/></svg>",
            "<svg><rect fill=\"url(#missing.png)\"/></svg>",
            "<svg><rect fill=\"url(data:private.png)\"/></svg>",
            "<svg><use href=\"#\"/></svg>",
        ] {
            assert!(svg(bad).is_err(), "{bad}");
        }
        assert!(svg("<svg><defs><rect id=\"r\" width=\"5\" height=\"5\"/></defs><use href=\"#r\"/></svg>").is_ok());
        assert!(svg("<svg viewBox=\"0 0 10 10\"><defs><linearGradient id=\"g\"/></defs><rect width=\"10\" height=\"10\" fill=\"url(#g)\"/></svg>").is_ok());
    }
}
