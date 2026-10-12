//! `openpaperdesign-mcp`: the openpaperdesign plugin's MCP server, over stdio.
//!
//! ```text
//! openpaperdesign-mcp [--workspace <dir>]            MCP over stdio (what the plugin runs)
//! openpaperdesign-mcp doctor [--workspace <dir>]     setup check, as JSON
//! openpaperdesign-mcp --version
//! ```
//!
//! The workspace is the working directory the host starts it in (or `--workspace`). Nothing
//! is open until the agent binds a project deliberately: `bind_project` (an existing paper-rs
//! project inside the workspace) or `create_project` (a new one under `<workspace>/design/`).
//! The bound project is then served by the B2a server, in this process, on 127.0.0.1 with a
//! fresh token. Every upstream tool call goes to that server's host, with the same policy,
//! write lock, atomic saves and revisions as its HTTP `/mcp` endpoint. `open_editor` returns
//! the loopback editor URL (with the token in its fragment) to open in the host's browser;
//! the URL is never written to disk. One project per process; binding again switches.
//!
//! Switching is safe in two ways. The new project is opened and fully checked (containment,
//! `project.json`, page ids, links) before the current one is touched, so a failed bind
//! leaves the current binding, its server and its editor URL as they were. And once the
//! browser editor has been opened, switching stops the server that tab saves to, so an edit
//! there that isn't saved yet could no longer be saved: the switch then needs `force: true`,
//! and the refusal says why. Edits made through the tools are saved by each call. When this
//! process exits, the same applies to an open tab, without a prompt.

use openpaperdesign_serve::agent::offered_tools;
use openpaperdesign_serve::{Server, bundle_file_names, new_token, router};
use paper_rs::mcp::server::to_result;
use paper_rs::mcp::{Out, ToolResult, tools};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

const VERSION: &str = env!("CARGO_PKG_VERSION");

const SWITCH_NEEDS_FORCE: &str = "The browser editor for the bound project was opened \
    (open_editor). Switching stops the server that browser tab saves to, so any edit there that \
    isn't saved yet (the page shows an unsaved change or \"Saving…\") would be lost. Your own \
    tool edits are already saved by each call. Ask the person to let their edits save (the \
    page shows \"Saved\") or to accept losing them, then call again with force: true.";

const NOT_BOUND: &str = "No design project is bound yet. Call bind_project with the path of an \
    existing paper-rs project folder inside this workspace, or create_project to make a new one \
    under design/.";

fn bundle_complete() -> bool {
    let names = bundle_file_names();
    names
        .iter()
        .all(|n| BUNDLE_FILES.iter().any(|(f, _)| f == n))
}

/// Why this workspace can't be used for binding, if it can't: never the filesystem root or
/// the user's home folder itself (that would put every project of the user in reach).
fn workspace_problem(ws: &Path) -> Option<String> {
    if ws.parent().is_none() {
        return Some("the workspace is the filesystem root".into());
    }
    let home = std::env::var_os("HOME").and_then(|h| PathBuf::from(h).canonicalize().ok());
    if home.as_deref() == Some(ws) {
        return Some("the workspace is your home folder".into());
    }
    // A host that starts the server in the installed plugin's folder (how it resolves the
    // server's working directory is not verified) must not get design/ created in there.
    if ws.join("mcp.json").is_file() && ws.join("skills/openpaperdesign/SKILL.md").is_file() {
        return Some(
            "the workspace is the openpaperdesign plugin folder, not your project (the host \
             started the server there)"
                .into(),
        );
    }
    None
}

/// A directory's identity: (device, inode).
#[cfg(unix)]
fn identity(m: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (m.dev(), m.ino())
}
#[cfg(not(unix))]
fn identity(_: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

/// `path` is a real directory (not a symlink to one).
fn real_dir(path: &Path) -> Result<std::fs::Metadata, String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if m.file_type().is_symlink() || !m.is_dir() {
        return Err(format!("{} is not a real directory", path.display()));
    }
    Ok(m)
}

struct Bound {
    server: Arc<Server>,
    port: u16,
    token: String,
    task: tokio::task::JoinHandle<()>,
    /// `open_editor` gave out this server's URL, so a browser tab may be editing it.
    editor_opened: bool,
}

impl Drop for Bound {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone)]
struct Launcher {
    /// Canonical path, recorded at start, and its identity (device, inode) then: re-checked
    /// before every bind and create, so a workspace moved or replaced (for example by a
    /// symlink to somewhere else) is refused instead of followed.
    workspace: PathBuf,
    workspace_id: (u64, u64),
    bound: Arc<tokio::sync::Mutex<Option<Bound>>>,
    /// The one MCP client on stdio, as a session of the bound project's host.
    session: u64,
}

fn text(v: Value) -> ToolResult {
    Ok(vec![Out::Text(serde_json::to_string_pretty(&v).unwrap())])
}

fn own_tools() -> Vec<Tool> {
    [
        json!({
            "name": "bind_project",
            "description": "Bind the design project to edit: an existing paper-rs project folder \
                (it has a project.json) inside this workspace, as a path relative to the \
                workspace or absolute. Paths outside the workspace, symlinks leading out of it, \
                and the home folder are refused. Binding again switches projects; a failed bind \
                keeps the current one. If the browser editor was opened, switching needs \
                force: true, because unsaved edits in that browser tab would be lost. Required \
                before any editing tool.",
            "inputSchema": {"type": "object", "properties": {
                "path": {"type": "string", "description": "Project folder, e.g. design/landing"},
                "force": {"type": "boolean", "description": "Switch even though the browser editor of the current project was opened; unsaved edits in that tab are lost."}
            }, "required": ["path"]}
        }),
        json!({
            "name": "create_project",
            "description": "Create a new, empty paper-rs project under <workspace>/design/<name>/ \
                and bind it. Refuses a name whose folder already exists (bind that instead). \
                Switching away from a project whose browser editor was opened needs force: true, \
                as for bind_project.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"},
                "force": {"type": "boolean", "description": "As for bind_project."}
            }, "required": ["name"]}
        }),
        json!({
            "name": "open_editor",
            "description": "Start (if needed) and return the editable canvas URL for the bound \
                project: http://127.0.0.1:<port>/#token=<token>. Open it in the host's in-app \
                browser; the person's edits there and your tool edits land in the same project \
                and show up in each other live. The URL holds a per-session token: don't write \
                it into files.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "openpaperdesign_status",
            "description": "Report what is set up and what is bound: version, workspace, bound \
                project, whether the browser editor is available.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
    ]
    .into_iter()
    .map(|t| serde_json::from_value(t).expect("valid tool"))
    .collect()
}

impl Launcher {
    fn relative(&self, dir: &Path) -> String {
        let rel = dir
            .strip_prefix(&self.workspace)
            .unwrap_or(dir)
            .display()
            .to_string();
        if rel.is_empty() { ".".into() } else { rel }
    }

    /// The workspace is still the directory recorded at start.
    fn check_workspace(&self) -> Result<(), String> {
        let replaced = || {
            format!(
                "the workspace {} was moved or replaced since this server started; refusing. \
                 Restart the agent in the project folder.",
                self.workspace.display()
            )
        };
        let m = real_dir(&self.workspace).map_err(|_| replaced())?;
        if identity(&m) != self.workspace_id
            || self.workspace.canonicalize().ok().as_deref() != Some(self.workspace.as_path())
        {
            return Err(replaced());
        }
        Ok(())
    }

    /// Create `design/<folder>/pages/` without following links: each component is made with
    /// `create_dir` (which never follows a final symlink) or must already be a real
    /// directory, and the result must still be inside the unchanged workspace.
    fn create_folders(&self, folder: &str) -> Result<PathBuf, String> {
        self.check_workspace()?;
        let design = self.workspace.join("design");
        match std::fs::create_dir(&design) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("design/: {e}")),
        }
        real_dir(&design).map_err(|_| "design/ is not a real folder (a link?); refusing")?;
        let dir = design.join(folder);
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(format!(
                    "design/{folder} already exists; bind it with bind_project"
                ));
            }
            Err(e) => return Err(format!("design/{folder}: {e}")),
        }
        std::fs::create_dir(dir.join(paper_rs::project::PAGES_DIR))
            .map_err(|e| format!("design/{folder}/pages: {e}"))?;
        // Verify what was made before writing any file into it.
        self.check_workspace()?;
        for d in [&design, &dir, &dir.join(paper_rs::project::PAGES_DIR)] {
            real_dir(d)?;
        }
        if dir.canonicalize().ok().as_deref() != Some(dir.as_path()) {
            return Err("the new project folder isn't where it was made; refusing".into());
        }
        Ok(dir)
    }

    fn resolve_inside(&self, path: &str) -> Result<PathBuf, String> {
        self.check_workspace()?;
        if let Some(p) = workspace_problem(&self.workspace) {
            return Err(format!(
                "refusing to bind: {p}. Start the agent in a project folder instead."
            ));
        }
        let p = Path::new(path.trim());
        let joined = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace.join(p)
        };
        let real = joined
            .canonicalize()
            .map_err(|e| format!("{}: {e}", joined.display()))?;
        if !real.starts_with(&self.workspace) {
            return Err(format!(
                "{} is outside the workspace {} (symlinks are followed before this check)",
                joined.display(),
                self.workspace.display()
            ));
        }
        if workspace_problem(&real).is_some() {
            return Err("refusing to bind the home folder or filesystem root".into());
        }
        Ok(real)
    }

    /// Refuse to switch away from a project whose browser editor was opened, unless forced.
    fn may_switch(bound: &Option<Bound>, to: Option<&Path>, force: bool) -> Result<(), String> {
        match bound {
            Some(b) if b.editor_opened && !force && to != Some(b.server.dir()) => {
                Err(SWITCH_NEEDS_FORCE.into())
            }
            _ => Ok(()),
        }
    }

    async fn bind(&self, dir: PathBuf, force: bool) -> ToolResult {
        let mut bound = self.bound.lock().await;
        if let Some(b) = bound.as_ref()
            && b.server.dir() == dir
        {
            return text(
                json!({"bound": self.relative(&dir), "note": "already bound; nothing changed"}),
            );
        }
        Self::may_switch(&bound, Some(&dir), force)?;
        // Open and check the new project completely before touching the current binding: any
        // error below returns with the current one (and its server) unchanged.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = new_token().map_err(|e| e.to_string())?;
        let server = Server::open_embedded(dir, BUNDLE_FILES, port, token.clone())
            .map_err(|e| e.to_string())?;
        // The switch now happens: the project being left releases its layout state, so the
        // new one doesn't inherit it (the new one has already dropped stale entries for its
        // own pages when it opened).
        if let Some(old) = bound.as_ref() {
            old.server.release_layout_state();
        }
        let app = router(server.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        // The summary returned with the bind is not the agent's read of any page: it runs in
        // a one-off session, so the agent's session still has to read a page before changing
        // it.
        let info = server
            .call_tool(
                openpaperdesign_serve::agent::new_session(),
                "get_basic_info",
                json!({}),
            )
            .await;
        let rel = self.relative(server.dir());
        // Only now stop the previous project's server.
        *bound = Some(Bound {
            server,
            port,
            token,
            task,
            editor_opened: false,
        });
        let mut out = vec![Out::Text(
            json!({
                "bound": rel,
                "next": "before changing a page, read it: get_tree_summary with nodeId \"root\" \
                    and its pageId (this summary doesn't count); get_basic_info before renaming, \
                    adding or removing pages. Then edit with the design tools; call open_editor \
                    for the browser canvas.",
            })
            .to_string(),
        )];
        if let Ok(more) = info {
            out.extend(more);
        }
        Ok(out)
    }

    async fn run(&self, name: &str, args: Value) -> ToolResult {
        let arg = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| format!("missing required string argument `{k}`"))
        };
        let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
        match name {
            "bind_project" => {
                let dir = self.resolve_inside(arg("path")?)?;
                if !dir.join(paper_rs::project::MANIFEST).is_file() {
                    return Err(format!(
                        "{} is not a paper-rs project (no project.json). Use create_project for \
                         a new one.",
                        dir.display()
                    ));
                }
                self.bind(dir, force).await
            }
            "create_project" => {
                let name = arg("name")?;
                // Before creating anything.
                Self::may_switch(&*self.bound.lock().await, None, force)?;
                self.check_workspace()?;
                if let Some(p) = workspace_problem(&self.workspace) {
                    return Err(format!("refusing to create a project: {p}"));
                }
                let folder = paper_rs::project::folder_name(name);
                let dir = self.create_folders(&folder)?;
                // As upstream `Store::create`, but into the folder made and checked above.
                let manifest = paper_rs::project::Manifest::new(name.trim());
                let page = paper_rs::project::PageData {
                    id: manifest.pages[0].id.clone(),
                    name: manifest.pages[0].name.clone(),
                    doc: paper_rs::model::Document {
                        name: manifest.pages[0].name.clone(),
                        ..Default::default()
                    },
                    camera: None,
                };
                let p = paper_rs::project::Project { dir, manifest };
                p.save_page(&page).map_err(|e| e.to_string())?;
                p.write_manifest().map_err(|e| e.to_string())?;
                self.bind(p.dir, force).await
            }
            "open_editor" => {
                let mut bound = self.bound.lock().await;
                let Some(b) = bound.as_mut() else {
                    return Err(NOT_BOUND.into());
                };
                if !bundle_complete() {
                    return Err(
                        "This openpaperdesign-mcp was built without the browser editor. Build \
                         the web bundle first (./scripts/build-web.sh), then reinstall \
                         (cargo install --locked --path crates/openpaperdesign-serve). Editing \
                         through the tools works without it."
                            .into(),
                    );
                }
                b.editor_opened = true;
                text(json!({
                    "url": format!("http://127.0.0.1:{}/#token={}", b.port, b.token),
                    "open_in": "the host's in-app browser (loopback only)",
                    "note": "The token in the URL is for this session only; don't write it into files.",
                }))
            }
            "openpaperdesign_status" => {
                let bound = self.bound.lock().await;
                text(json!({
                    "version": VERSION,
                    "workspace": self.workspace,
                    "workspace_usable": workspace_problem(&self.workspace).is_none(),
                    "bound_project": bound.as_ref().map(|b| b.server.dir().to_path_buf()),
                    "editor_opened": bound.as_ref().is_some_and(|b| b.editor_opened),
                    "browser_editor": if bundle_complete() { "built in" } else { "not built (open_editor unavailable)" },
                }))
            }
            _ => {
                let server = {
                    let bound = self.bound.lock().await;
                    match bound.as_ref() {
                        Some(b) => b.server.clone(),
                        None => return Err(NOT_BOUND.into()),
                    }
                };
                server.call_tool(self.session, name, args).await
            }
        }
    }
}

impl ServerHandler for Launcher {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("openpaperdesign", VERSION))
            .with_instructions(format!(
                "openpaperdesign: an editable design canvas (paper-rs) for this workspace. First \
                 bind a project deliberately (bind_project for an existing one inside the \
                 workspace, or create_project under design/). Before changing a page, read \
                 that page in this session (get_tree_summary with nodeId \"root\" and its \
                 pageId); changes to unread or since-changed pages are refused as conflicts. \
                 Then edit with the tools below, and call open_editor to get the canvas URL for \
                 the in-app browser, where the \
                 person can edit too. Export and project switching outside the workspace are \
                 disabled; images must be data: URIs.\n\n{}",
                tools::INSTRUCTIONS
            ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut all = own_tools();
        all.extend(offered_tools());
        Ok(ListToolsResult::with_all_items(all))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = Value::Object(request.arguments.unwrap_or_default());
        Ok(to_result(self.run(&request.name, args).await).into())
    }
}

fn workspace(args: &mut Vec<String>) -> Result<PathBuf, String> {
    let w = match args.iter().position(|a| a == "--workspace") {
        Some(i) => {
            let v = args
                .get(i + 1)
                .cloned()
                .ok_or("--workspace needs a value")?;
            args.drain(i..=i + 1);
            PathBuf::from(v)
        }
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    w.canonicalize()
        .map_err(|e| format!("workspace {}: {e}", w.display()))
}

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("openpaperdesign-mcp {VERSION}");
        return;
    }
    let ws = workspace(&mut args).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    match args.first().map(String::as_str) {
        Some("doctor") => {
            let missing: Vec<&str> = bundle_file_names()
                .into_iter()
                .filter(|n| !BUNDLE_FILES.iter().any(|(f, _)| f == n))
                .collect();
            let report = json!({
                "version": VERSION,
                "workspace": ws,
                "workspace_usable": workspace_problem(&ws).is_none(),
                "workspace_problem": workspace_problem(&ws),
                "browser_editor": if missing.is_empty() { "built in" } else { "missing" },
                "missing_bundle_files": missing,
                "platform_supported": cfg!(unix),
            });
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
            let ok = missing.is_empty() && cfg!(unix);
            std::process::exit(if ok { 0 } else { 1 });
        }
        None => {}
        Some(other) => {
            eprintln!(
                "unknown argument {other:?}\nusage: openpaperdesign-mcp [--workspace <dir>] [doctor] | --version"
            );
            std::process::exit(2);
        }
    }
    // MCP over stdio. Nothing else may write to stdout.
    let workspace_id = match std::fs::symlink_metadata(&ws) {
        Ok(m) => identity(&m),
        Err(e) => {
            eprintln!("workspace {}: {e}", ws.display());
            std::process::exit(2);
        }
    };
    let launcher = Launcher {
        workspace: ws,
        workspace_id,
        bound: Arc::new(tokio::sync::Mutex::new(None)),
        session: openpaperdesign_serve::agent::new_session(),
    };
    match launcher.serve(rmcp::transport::stdio()).await {
        Ok(running) => {
            let _ = running.waiting().await;
        }
        Err(e) => {
            eprintln!("openpaperdesign-mcp: {e}");
            std::process::exit(1);
        }
    }
}
