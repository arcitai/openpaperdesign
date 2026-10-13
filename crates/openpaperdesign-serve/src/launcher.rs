//! `openpaperdesign-mcp`: the openpaperdesign plugin's MCP server, over stdio.
//!
//! ```text
//! openpaperdesign-mcp --codex-context                the packaged Codex plugin
//! openpaperdesign-mcp [--workspace <dir>]            standalone MCP over stdio
//! openpaperdesign-mcp doctor [--workspace <dir>]     setup check, as JSON
//! openpaperdesign-mcp --version
//! ```
//!
//! The plugin pins workspace and supported permissions from Codex's per-call context.
//! Standalone mode uses the working directory (or deliberate `--workspace`). Nothing
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

use crate::agent::offered_tools;
use crate::{Server, bundle_file_names, new_token, router};
use paper_rs::mcp::server::to_result;
use paper_rs::mcp::{Out, ToolResult, tools};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    RequestMetaObject, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(not(test))]
include!(concat!(env!("OUT_DIR"), "/bundle.rs"));
#[cfg(test)]
use tests::BUNDLE_FILES;

#[path = "launcher_inventory.rs"]
mod inventory;
#[path = "launcher_scope.rs"]
mod scope;
use scope::{CAPABILITY, Workspace};
use tokio::sync::{Mutex, OwnedMutexGuard};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const SWITCH_NEEDS_FORCE: &str = "The browser editor for the bound project was opened \
    (open_editor). Switching stops the server that browser tab saves to, so any edit there that \
    isn't saved yet (the page shows an unsaved change or \"Saving…\") would be lost. Your own \
    tool edits are already saved by each call. Ask the person to let their edits save (the \
    page shows \"Saved\") or to accept losing them, then call again with force: true.";

/// The home: one global-entry tool and its static MCP App view, plus the read-only native
/// project inventory it shows. All three are answered before workspace admission because they
/// touch none of this session's state: no context, binding, project, listener or workspace
/// file. They never read or change it, so they cannot pin, inspect or retire a binding. The
/// inventory reads Codex's own project list through its CLI; a listed root is discovery, never
/// authority. Every other tool keeps per-call admission unchanged.
const HOME_TOOL: &str = "openpaperdesign_home";
const PROJECTS_TOOL: &str = "openpaperdesign_projects";
const INVENTORY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);
/// One inventory child process at a time per session.
static INVENTORY_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const HOME_URI: &str = "ui://openpaperdesign/home.html";
const HOME_MIME: &str = "text/html;profile=mcp-app";
const HOME_HTML: &str = include_str!("home.html");

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
    // Neither standalone cwd nor a host-selected scope may use the plugin folder.
    if ws.join("mcp.json").is_file() && ws.join("skills/openpaperdesign/SKILL.md").is_file() {
        return Some("the workspace is the openpaperdesign plugin folder, not your project".into());
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
    state: Arc<Mutex<LauncherState>>,
    /// The one MCP client on stdio, as a session of the bound project's host.
    session: u64,
}

struct LauncherState {
    host_context: bool,
    workspace: Option<Workspace>,
    invalidated: Option<String>,
    bound: Option<Bound>,
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
            "inputSchema": {"type": "object", "properties": {}},
            "_meta": result_view()
        }),
        json!({
            "name": "openpaperdesign_status",
            "description": "Report what is set up and what is bound: version, workspace, bound \
                project, workspace provenance/permission admission or refusal reason, and \
                whether the browser editor is available.",
            "inputSchema": {"type": "object", "properties": {}},
            "_meta": result_view()
        }),
        json!({
            "name": HOME_TOOL,
            "title": "OpenPaperDesign",
            "description": "Open the OpenPaperDesign home: a static view (the Codex sidebar entry). \
                It reads no workspace, project or binding and changes nothing. To work on a \
                canvas use openpaperdesign_status, then bind_project or create_project, then \
                open_editor.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "_meta": {
                "ui": {"resourceUri": HOME_URI},
                // Global: the sidebar. Thread: a tab inside a chat, whose model calls carry
                // that chat's own workspace context. Neither entry grants a workspace.
                "openai/ui": {"entrypoints": [{"type": "global"}, {"type": "thread"}]}
            }
        }),
        json!({
            "name": "openpaperdesign_canvases",
            "description": "List the existing design canvases (paper-rs projects) directly under \
                this workspace's design/ folder: project path, canvas name and id, page count, \
                and which one is bound. Read-only: it creates nothing (not even design/), binds \
                nothing, follows no links and counts as reading no page. Needs the same \
                workspace context as every other tool.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "_meta": result_view()
        }),
        json!({
            "name": PROJECTS_TOOL,
            "description": "List the Codex projects saved on this computer (read-only, from the \
                Codex CLI's app-server: project/list, or project/read with projectId), each \
                with its id and every root folder. Discovery only: a listed root is not this \
                chat's workspace and grants nothing; openpaperdesign_status shows the \
                workspace you can use. Reports available: false with a reason when the list \
                can't be read.",
            "inputSchema": {"type": "object", "properties": {
                "projectId": {"type": "string", "maxLength": 256, "description": "Read just this project (an id from the list)."}
            }, "additionalProperties": false}
        }),
    ]
    .into_iter()
    .map(|t| serde_json::from_value(t).expect("valid tool"))
    .collect()
}

/// The home view also presents the model's own status and editor results. Model-only: the
/// view itself never calls these tools (it has no workspace context and would be refused).
fn result_view() -> Value {
    json!({"ui": {"resourceUri": HOME_URI, "visibility": ["model"]}})
}

/// `openpaperdesign_home`: answered without workspace admission and without touching state.
fn home(args: Option<&serde_json::Map<String, Value>>) -> ToolResult {
    if args.is_some_and(|a| !a.is_empty()) {
        return Err(format!(
            "{HOME_TOOL} takes no arguments; it only opens the home view."
        ));
    }
    text(json!({
        "surface": "home",
        "version": VERSION,
        "note": "Static home view. Nothing was read or changed. Use openpaperdesign_status to check this chat's workspace.",
    }))
}

/// `openpaperdesign_projects`: the native inventory, answered without workspace admission and
/// without touching state. Only `projectId` is accepted. `cancelled` is the MCP request's
/// cancellation: the inventory child is stopped and waited for before the inventory is free
/// again (a cancelled request's reply is not sent).
async fn projects(
    args: Option<&serde_json::Map<String, Value>>,
    cancelled: impl std::future::Future<Output = ()>,
) -> ToolResult {
    let empty = serde_json::Map::new();
    let args = args.unwrap_or(&empty);
    if let Some(k) = args.keys().find(|k| *k != "projectId") {
        return Err(format!(
            "{PROJECTS_TOOL} accepts only projectId, not `{k}`."
        ));
    }
    let id = match args.get("projectId") {
        None => None,
        Some(Value::String(s))
            if !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control) =>
        {
            Some(s.as_str())
        }
        Some(_) => return Err("projectId must be a non-empty project id string.".into()),
    };
    use std::sync::atomic::Ordering;
    if INVENTORY_BUSY.swap(true, Ordering::AcqRel) {
        return Err("The Codex project list is already being read; try again.".into());
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            INVENTORY_BUSY.store(false, Ordering::Release);
        }
    }
    let _release = Release;
    let query = id.map_or(inventory::Query::List, inventory::Query::Read);
    // The normal command route: `codex` on this process's PATH. No fixed install path.
    let read = inventory::read(
        std::ffi::OsStr::new("codex"),
        query,
        INVENTORY_DEADLINE,
        cancelled,
    );
    match read.await {
        Ok(v) => text(v),
        Err(why) => text(why.to_json()),
    }
}

impl Workspace {
    fn relative(&self, dir: &Path) -> String {
        let rel = dir
            .strip_prefix(&self.path)
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
                self.path.display()
            )
        };
        let m = real_dir(&self.path).map_err(|_| replaced())?;
        if identity(&m) != self.id
            || self.path.canonicalize().ok().as_deref() != Some(self.path.as_path())
        {
            return Err(replaced());
        }
        Ok(())
    }

    /// The existing paper-rs projects directly under `<workspace>/design/`, read-only. Runs only
    /// after workspace admission. Each candidate must be a real directory on the workspace's
    /// file system (links are reported, never followed) with a real `project.json` that passes
    /// the same checks as binding. Nothing is created: no `design/`, project, server, layout
    /// entry or page-read acknowledgement. The scan is bounded and says when it stopped early.
    fn canvases(&self, bound: Option<&Path>) -> ToolResult {
        const MAX_SCAN: usize = 1000;
        const MAX_CANVASES: usize = 200;
        self.check_workspace()?;
        if let Some(p) = workspace_problem(&self.path) {
            return Err(format!("refusing to list canvases: {p}"));
        }
        let design = self.path.join("design");
        self.project_path(&design)?;
        let mut out = serde_json::Map::new();
        out.insert("workspace".into(), json!(self.path));
        let present = crate::plain(&design, crate::Kind::Dir)
            .map_err(|e| format!("design/ can't be listed: {e}"))?;
        out.insert("design_present".into(), json!(present));
        let (mut canvases, mut skipped, mut complete) = (vec![], vec![], true);
        if present {
            let design_dev = identity(&real_dir(&design)?).0;
            let entries = std::fs::read_dir(&design).map_err(|e| format!("design/: {e}"))?;
            for (n, entry) in entries.enumerate() {
                if n >= MAX_SCAN || canvases.len() >= MAX_CANVASES {
                    complete = false;
                    break;
                }
                let entry = entry.map_err(|e| format!("design/: {e}"))?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    skipped.push(
                        json!({"project": null, "problem": "a folder name that isn't valid UTF-8"}),
                    );
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                let rel = format!("design/{name}");
                let dir = design.join(name);
                let mut skip =
                    |problem: String| skipped.push(json!({"project": rel, "problem": problem}));
                if let Err(e) = self.project_path(&dir) {
                    skip(e);
                    continue;
                }
                let m = match std::fs::symlink_metadata(&dir) {
                    Ok(m) => m,
                    Err(e) => {
                        skip(e.to_string());
                        continue;
                    }
                };
                if m.file_type().is_symlink() {
                    skip("a symbolic link (not followed)".into());
                    continue;
                }
                if !m.is_dir() {
                    continue;
                }
                if identity(&m).0 != design_dev {
                    skip("on another file system (not followed)".into());
                    continue;
                }
                match crate::plain(&dir.join(paper_rs::project::MANIFEST), crate::Kind::File) {
                    Ok(false) => continue, // not a paper-rs project
                    Err(e) => {
                        skip(e);
                        continue;
                    }
                    Ok(true) => {}
                }
                let checked = crate::check_project_dir(&dir)
                    .and_then(|()| crate::read_manifest_checked(&dir));
                match checked {
                    Ok(manifest) => canvases.push(json!({
                        "project": rel,
                        "path": dir,
                        "bound": bound == Some(dir.as_path()),
                        "canvas": {
                            "id": manifest.id,
                            "name": manifest.name,
                            "pages": manifest.pages.len(),
                            "modified": manifest.modified,
                        },
                    })),
                    Err(e) => skip(e),
                }
            }
        }
        canvases.sort_by(|a, b| a["project"].as_str().cmp(&b["project"].as_str()));
        skipped.sort_by(|a, b| a["project"].as_str().cmp(&b["project"].as_str()));
        out.insert("complete".into(), json!(complete));
        out.insert("canvases".into(), json!(canvases));
        out.insert("skipped".into(), json!(skipped));
        out.insert(
            "note".into(),
            json!(
                "Read-only discovery: nothing was created or bound, and no page counts as read. \
                   To open one, bind_project its project path, read the page you will change \
                   (get_tree_summary root with its pageId), then call open_editor."
            ),
        );
        text(Value::Object(out))
    }

    /// Create `design/<folder>/pages/` without following links: each component is made with
    /// `create_dir` (which never follows a final symlink) or must already be a real
    /// directory, and the result must still be inside the unchanged workspace.
    fn create_folders(&self, folder: &str) -> Result<PathBuf, String> {
        self.check_workspace()?;
        self.project_path(&self.path.join("design").join(folder))?;
        let design = self.path.join("design");
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
        if let Some(p) = workspace_problem(&self.path) {
            return Err(format!(
                "refusing to bind: {p}. Start the agent in a project folder instead."
            ));
        }
        let p = Path::new(path.trim());
        self.project_path(p)?;
        let joined = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.path.join(p)
        };
        let real = joined
            .canonicalize()
            .map_err(|e| format!("{}: {e}", joined.display()))?;
        if !real.starts_with(&self.path) {
            return Err(format!(
                "{} is outside the workspace {} (symlinks are followed before this check)",
                joined.display(),
                self.path.display()
            ));
        }
        self.project_path(&real)?;
        if workspace_problem(&real).is_some() {
            return Err("refusing to bind the home folder or filesystem root".into());
        }
        Ok(real)
    }
}

impl Launcher {
    fn new(workspace: Option<Workspace>) -> Self {
        Self {
            state: Arc::new(Mutex::new(LauncherState {
                host_context: workspace.is_none(),
                workspace,
                invalidated: None,
                bound: None,
            })),
            session: crate::agent::new_session(),
        }
    }

    fn status(state: &LauncherState, problem: Option<&str>) -> ToolResult {
        let current_problem = problem.map(str::to_owned).or_else(|| {
            state.workspace.as_ref().and_then(|ws| {
                ws.check_workspace()
                    .err()
                    .or_else(|| workspace_problem(&ws.path))
            })
        });
        text(json!({
            "version": VERSION,
            "workspace": state.workspace.as_ref().map(|ws| &ws.path),
            "workspace_source": state.workspace.as_ref().map_or(CAPABILITY, |ws| ws.source),
            "workspace_state": if state.invalidated.is_some() { "retired" } else if state.workspace.is_some() { "pinned" } else { "unresolved" },
            "workspace_usable": state.workspace.is_some() && current_problem.is_none() && state.invalidated.is_none(),
            "workspace_permission_mode": state.workspace.as_ref().filter(|_| current_problem.is_none() && state.invalidated.is_none()).map(|ws| ws.permission),
            "workspace_problem": current_problem,
            "bound_project": state.bound.as_ref().map(|b| b.server.dir()),
            "editor_opened": state.bound.as_ref().is_some_and(|b| b.editor_opened),
            "browser_editor": if bundle_complete() { "built in" } else { "not built (open_editor unavailable)" },
        }))
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

    async fn bind(
        mut state: OwnedMutexGuard<LauncherState>,
        ws: Workspace,
        dir: PathBuf,
        force: bool,
    ) -> ToolResult {
        if let Some(b) = state.bound.as_ref()
            && b.server.dir() == dir
        {
            return text(
                json!({"bound": ws.relative(&dir), "note": "already bound; nothing changed"}),
            );
        }
        Self::may_switch(&state.bound, Some(&dir), force)?;
        // Open and check the new project completely before touching the current binding: any
        // error below returns with the current one (and its server) unchanged.
        #[cfg(test)]
        use tests::Listener;
        #[cfg(not(test))]
        use tokio::net::TcpListener as Listener;
        let listener = Listener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = new_token().map_err(|e| e.to_string())?;
        let rel = ws.relative(&dir);
        let prepared = Server::prepare_embedded(dir, BUNDLE_FILES, port, token.clone())
            .map_err(|e| e.to_string())?;
        // No fallible preparation remains. Own the transition independently of the request:
        // cancelling a bind response must not strand the current binding halfway retired.
        tokio::spawn(async move {
            if let Some(old) = state.bound.as_ref() {
                old.server.retire().await;
                old.task.abort();
            }
            // Every old writer/rollback has finished; queued and retained old handles are
            // permanently fenced. Only now may the replacement reset its same-id entries.
            let server = prepared.activate();
            let app = router(server.clone());
            let task = tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            state.bound = Some(Bound {
                server: server.clone(),
                port,
                token,
                task,
                editor_opened: false,
            });
            // This summary is not the agent's read: its session must still read each page.
            let info = server
                .call_tool(crate::agent::new_session(), "get_basic_info", json!({}))
                .await;
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
        })
        .await
        .unwrap_or_else(|e| Err(format!("binding handoff failed unexpectedly: {e}")))
    }

    async fn run(&self, name: &str, args: Value, meta: &RequestMetaObject) -> ToolResult {
        let mut state = self.state.clone().lock_owned().await;
        if state.host_context {
            let context = if let Some(reason) = &state.invalidated {
                Err(reason.clone())
            } else {
                Workspace::from_meta(meta).and_then(|ws| {
                    if let Some(pinned) = &state.workspace
                        && (ws.path != pinned.path || ws.id != pinned.id)
                    {
                        return Err("Codex workspace changed or was moved/replaced; this session cannot change workspace".into());
                    }
                    Ok(ws)
                })
            };
            match context {
                Ok(ws) => state.workspace = Some(ws),
                Err(reason) => {
                    if state.workspace.is_some() {
                        state.invalidated = Some(reason.clone());
                    }
                    let status = name == "openpaperdesign_status";
                    // Own invalidation independently of the caller, just like a bind
                    // handoff. Keep admission locked until the shared fence is drained.
                    return tokio::spawn(async move {
                        if let Some(old) = state.bound.as_ref() {
                            old.server.retire().await;
                        }
                        state.bound = None;
                        if status {
                            Self::status(&state, Some(&reason))
                        } else {
                            Err(format!("{reason}. No call was run. If this session had a workspace, it is retired; start a fresh supported host session."))
                        }
                    }).await.unwrap_or_else(|e| Err(format!("workspace retirement failed: {e}")));
                }
            }
        }
        if name == "openpaperdesign_status" {
            return Self::status(&state, None);
        }
        let ws = state
            .workspace
            .as_ref()
            .expect("admitted workspace")
            .clone();
        let arg = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| format!("missing required string argument `{k}`"))
        };
        let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
        match name {
            "bind_project" => {
                let dir = ws.resolve_inside(arg("path")?)?;
                if !dir.join(paper_rs::project::MANIFEST).is_file() {
                    return Err(format!(
                        "{} is not a paper-rs project (no project.json). Use create_project for \
                         a new one.",
                        dir.display()
                    ));
                }
                Self::bind(state, ws, dir, force).await
            }
            "create_project" => {
                let name = arg("name")?;
                // Before creating anything.
                Self::may_switch(&state.bound, None, force)?;
                ws.check_workspace()?;
                if let Some(p) = workspace_problem(&ws.path) {
                    return Err(format!("refusing to create a project: {p}"));
                }
                let folder = paper_rs::project::folder_name(name);
                let dir = ws.create_folders(&folder)?;
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
                Self::bind(state, ws, p.dir, force).await
            }
            "open_editor" => {
                let Some(b) = state.bound.as_mut() else {
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
                // Self-contained identity from this admitted call: the validated workspace and
                // the bound project, whose manifest is read with the binding checks. Reading
                // it acknowledges no page.
                let dir = b.server.dir().to_path_buf();
                let canvas = crate::read_manifest_checked(&dir)
                    .ok()
                    .map(|m| json!({"id": m.id, "name": m.name, "pages": m.pages.len()}));
                text(json!({
                    "url": format!("http://127.0.0.1:{}/#token={}", b.port, b.token),
                    "workspace": ws.path,
                    "project": ws.relative(&dir),
                    "project_path": dir,
                    "canvas": canvas,
                    "open_in": "the host's in-app browser (loopback only)",
                    "note": "The token in the URL is for this session only; don't write it into files.",
                }))
            }
            "openpaperdesign_canvases" => {
                if args.as_object().is_some_and(|a| !a.is_empty()) {
                    return Err("openpaperdesign_canvases takes no arguments; it lists this workspace's design/ folder.".into());
                }
                let bound = state.bound.as_ref().map(|b| b.server.dir().to_path_buf());
                ws.canvases(bound.as_deref())
            }
            _ => {
                let server = {
                    match state.bound.as_ref() {
                        Some(b) => b.server.clone(),
                        None => return Err(NOT_BOUND.into()),
                    }
                };
                drop(state);
                server.call_tool(self.session, name, args).await
            }
        }
    }
}

impl ServerHandler for Launcher {
    fn get_info(&self) -> ServerConfig {
        let mut capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        // Safe to advertise in standalone mode too; only --codex-context consumes it.
        capabilities.experimental = Some([(CAPABILITY.to_string(), Default::default())].into());
        ServerConfig::new(capabilities)
            .with_server_info(Implementation::new("openpaperdesign", VERSION))
            .with_instructions(format!(
                "openpaperdesign: an editable design canvas (paper-rs) for this workspace. \
                 Check openpaperdesign_status first. In packaged Codex mode every call except \
                 the static openpaperdesign_home view and the read-only openpaperdesign_projects \
                 list needs host-generated workspace and supported permission context; tool \
                 arguments cannot supply it, and a listed Codex project folder is not the \
                 workspace. If workspace_usable is false, report workspace_problem \
                 and stop editing. First \
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

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let home = serde_json::from_value(json!({
            "uri": HOME_URI,
            "name": "openpaperdesign-home",
            "title": "OpenPaperDesign",
            "mimeType": HOME_MIME,
        }))
        .expect("valid resource");
        Ok(ListResourcesResult::with_all_items(vec![home]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        // Static and exact: no other URI is resolved, and no state or file is read.
        if request.uri != HOME_URI {
            return Err(ErrorData::resource_not_found("unknown resource", None));
        }
        let contents = serde_json::from_value(json!({
            "uri": HOME_URI,
            "mimeType": HOME_MIME,
            "text": HOME_HTML,
            "_meta": {"openai/ui": {"availableDisplayModes": ["inline", "fullscreen"]}},
        }))
        .expect("valid resource contents");
        Ok(ReadResourceResult::new(vec![contents]).into())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name == HOME_TOOL {
            return Ok(to_result(home(request.arguments.as_ref())).into());
        }
        if request.name == PROJECTS_TOOL {
            // rmcp cancels `ct` on `notifications/cancelled` but keeps running this handler.
            let ct = context.ct.clone();
            let cancelled = async move { ct.cancelled().await };
            return Ok(to_result(projects(request.arguments.as_ref(), cancelled).await).into());
        }
        let args = Value::Object(request.arguments.unwrap_or_default());
        Ok(to_result(self.run(&request.name, args, &context.meta).await).into())
    }
}

fn workspace(args: &mut Vec<String>) -> Result<Option<Workspace>, String> {
    if args.iter().any(|a| a == "--codex-context") {
        if args.iter().filter(|a| *a == "--codex-context").count() != 1
            || args.iter().any(|a| a == "--workspace")
        {
            return Err("--codex-context cannot be repeated or combined with --workspace".into());
        }
        args.retain(|a| a != "--codex-context");
        return Ok(None);
    }
    let (path, source) = match args.iter().position(|a| a == "--workspace") {
        Some(i) => {
            let v = args
                .get(i + 1)
                .cloned()
                .filter(|v| !v.starts_with("--"))
                .ok_or("--workspace needs a value")?;
            args.drain(i..=i + 1);
            (PathBuf::from(v), "explicit --workspace")
        }
        None => (
            std::env::current_dir().map_err(|e| e.to_string())?,
            "process cwd",
        ),
    };
    Workspace::new(&path, source).map(Some)
}

/// Entrypoint for the stdio launcher binary.
pub async fn main() {
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
                "workspace": ws.as_ref().map(|w| &w.path),
                "workspace_source": ws.as_ref().map_or(CAPABILITY, |w| w.source),
                "workspace_usable": ws.as_ref().is_some_and(|w| workspace_problem(&w.path).is_none()),
                "workspace_problem": ws.as_ref().map_or_else(|| Some("awaiting per-call Codex workspace and permission context".to_string()), |w| workspace_problem(&w.path)),
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
                "unknown argument {other:?}\nusage: openpaperdesign-mcp [--workspace <dir> | --codex-context] [doctor] | --version"
            );
            std::process::exit(2);
        }
    }
    // MCP over stdio. Nothing else may write to stdout.
    let launcher = Launcher::new(ws);
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

#[cfg(test)]
#[path = "launcher_tests.rs"]
mod tests;
