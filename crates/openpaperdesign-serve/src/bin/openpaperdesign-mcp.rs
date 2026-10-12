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
    None
}

struct Bound {
    server: Arc<Server>,
    port: u16,
    token: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Bound {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone)]
struct Launcher {
    workspace: PathBuf,
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
                and the home folder are refused. Binding again switches projects (everything is \
                already saved). Required before any editing tool.",
            "inputSchema": {"type": "object", "properties": {
                "path": {"type": "string", "description": "Project folder, e.g. design/landing"}
            }, "required": ["path"]}
        }),
        json!({
            "name": "create_project",
            "description": "Create a new, empty paper-rs project under <workspace>/design/<name>/ \
                and bind it. Refuses a name whose folder already exists (bind that instead).",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"}
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
    fn resolve_inside(&self, path: &str) -> Result<PathBuf, String> {
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

    async fn bind(&self, dir: PathBuf) -> ToolResult {
        let mut bound = self.bound.lock().await;
        // Switching: the previous project's changes are already saved; stop its server.
        *bound = None;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = new_token().map_err(|e| e.to_string())?;
        let server = Server::open_embedded(dir, BUNDLE_FILES, port, token.clone())
            .map_err(|e| e.to_string())?;
        let app = router(server.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let info = server
            .call_tool(self.session, "get_basic_info", json!({}))
            .await;
        let rel = server
            .dir()
            .strip_prefix(&self.workspace)
            .unwrap_or(server.dir())
            .display()
            .to_string();
        *bound = Some(Bound {
            server,
            port,
            token,
            task,
        });
        let mut out = vec![Out::Text(
            json!({
                "bound": if rel.is_empty() { ".".to_string() } else { rel },
                "next": "edit with the design tools; call open_editor for the browser canvas",
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
                self.bind(dir).await
            }
            "create_project" => {
                let name = arg("name")?;
                if let Some(p) = workspace_problem(&self.workspace) {
                    return Err(format!("refusing to create a project: {p}"));
                }
                let design = self.workspace.join("design");
                if design.exists() {
                    let real = design.canonicalize().map_err(|e| e.to_string())?;
                    if !real.starts_with(&self.workspace) || !real.is_dir() {
                        return Err("design/ leads outside the workspace; refusing".into());
                    }
                }
                let folder = paper_rs::project::folder_name(name);
                if design.join(&folder).exists() {
                    return Err(format!(
                        "design/{folder} already exists; bind it with bind_project"
                    ));
                }
                let store = paper_rs::project::Store::open(&design).map_err(|e| e.to_string())?;
                let (p, _) = store.create(name).map_err(|e| e.to_string())?;
                self.bind(p.dir).await
            }
            "open_editor" => {
                let bound = self.bound.lock().await;
                let Some(b) = bound.as_ref() else {
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
                 workspace, or create_project under design/). Then edit with the tools below, \
                 and call open_editor to get the canvas URL for the in-app browser, where the \
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
    let launcher = Launcher {
        workspace: ws,
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
