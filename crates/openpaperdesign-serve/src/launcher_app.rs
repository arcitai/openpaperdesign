//! The app library's direct actions (`openpaperdesign-mcp --app-library <dir>`): list, create,
//! open, close, reopen, import and export canvases the app owns (see `crate::library`), from
//! the home view, with no model turn and no message.
//!
//! In an app-library process only these tools, the home, its resource,
//! `openpaperdesign_status` (this mode's own report) and the read-only Codex project inventory
//! are answered. Every project route (`bind_project`, `create_project`, `open_editor`, the
//! workspace canvases list, the `_ui_*` workspace actions, the folder picker and the upstream
//! design tools) is refused without being admitted and without touching any state, so a call
//! without workspace context can neither use nor retire anything here. No Workspace is ever
//! made for the library, and no argument names a path: only a server-made canvas id, a bounded
//! name, a `force` flag, a server-made binding identity or a canvas file's bytes reach it.
//!
//! One canvas is bound per process, through the same prepare/install/retire handoff as project
//! mode; its lease moves with the binding and is released only after the old server drained.
//! Creation and import run in an owned task, so a caller that goes away can't strand a
//! half-made canvas, its cleanup or the handoff. A lost reply doesn't mean nothing happened:
//! the canvas then shows up in the list. Nothing is retried automatically.
//!
//! Several views (the sidebar, chat tabs) can share this process's one binding. Each binding
//! has its own identity (`binding`, `<canvas id>.<n>`), returned with every editor result.
//! Close, export and the readiness poll name the binding they were shown for, and a confirmed
//! switch (`force`) names the binding it confirmed replacing (`replace`); each is compared
//! under the launcher's state lock before anything happens, and a stale one is refused with
//! the current canvas, its files, URL and lease untouched. This only catches stale views: it
//! is not a credential, a path or any authority.

use super::*;
use crate::library::payload::MAX_PAYLOAD_BYTES;
use crate::library::{Lease, LeaseError, MAX_NAME_CHARS, NewDocument, payload, valid_id};

pub(super) const LIST: &str = "openpaperdesign_app_canvases";
pub(super) const CREATE: &str = "openpaperdesign_app_create";
pub(super) const OPEN: &str = "openpaperdesign_app_open";
pub(super) const EDITOR: &str = "openpaperdesign_app_editor";
pub(super) const CLOSE: &str = "openpaperdesign_app_close";
pub(super) const IMPORT: &str = "openpaperdesign_app_import";
pub(super) const EXPORT: &str = "openpaperdesign_app_export";
pub(super) const TOOLS: [&str; 7] = [LIST, CREATE, OPEN, EDITOR, CLOSE, IMPORT, EXPORT];

pub(super) const NOT_LIBRARY: &str = "This OpenPaperDesign server wasn't started for the app \
    library (it serves workspace projects), so app-library actions aren't available here. \
    Nothing was read or changed.";
const LIBRARY_ONLY: &str = "This OpenPaperDesign server runs the app library: its canvases \
    live in OpenPaperDesign's own data folder, and project tools aren't available here. Nothing \
    was read or changed. Workspace projects need a separately started `openpaperdesign-mcp \
    --codex-context` (or `--workspace <dir>`) server.";
const NO_CANVAS: &str = "No canvas is open in this OpenPaperDesign window. Open one from the \
    library. Nothing was changed.";
const BUSY: &str = "This canvas is open in another OpenPaperDesign window (another Codex chat \
    or panel). Close it there first. Nothing was changed here.";
const CLOSE_NEEDS_FORCE: &str = "The editor of this canvas was opened. Closing stops the server \
    its browser tab saves to, so an edit there that isn't saved yet (the page shows an unsaved \
    change or \"Saving…\") would be lost. Let it show \"Saved\", or accept losing it, then close \
    again with force: true.";
const STALE: &str = "The canvas this view shows isn't the one open in this OpenPaperDesign \
    window any more: another view changed it. Nothing was done, and the open canvas is \
    unchanged. Go back to your canvases to see what is open.";

/// A new binding's identity: its canvas id and a number unique in this process.
pub(super) fn binding_id(doc: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{doc}.{n}")
}

/// The shape of [`binding_id`]: a canvas id, a dot and 1 to 20 digits.
fn valid_binding(s: &str) -> bool {
    s.split_once('.').is_some_and(|(id, n)| {
        valid_id(id) && !n.is_empty() && n.len() <= 20 && n.bytes().all(|b| b.is_ascii_digit())
    })
}

/// The current library binding's identity, if a canvas is open.
fn current_binding(state: &LauncherState) -> Option<&str> {
    state.bound.as_ref().and_then(|b| b.binding.as_deref())
}

/// A switch away from the open canvas (create, import or open of another one). Once its editor
/// link was given, it needs `force` together with `replace`, the binding the person confirmed
/// losing; the refusal names the current binding so a view can confirm exactly that one. A
/// `replace` that isn't the current binding is stale and refused.
fn library_switch(state: &LauncherState, force: bool, replace: Option<&str>) -> Result<(), String> {
    let current = current_binding(state);
    if let Some(r) = replace
        && current != Some(r)
    {
        return Err(STALE.into());
    }
    match state.bound.as_ref() {
        Some(b) if b.editor_opened && (!force || replace.is_none()) => Err(format!(
            "{SWITCH_NEEDS_FORCE} (open canvas: {})",
            current.unwrap_or("none")
        )),
        _ => Ok(()),
    }
}

/// An action on the canvas a view shows (`binding`): there must be an open library canvas, and
/// it must be exactly that binding.
fn expect_binding(state: &LauncherState, binding: Option<&str>) -> Result<(), String> {
    match current_binding(state) {
        None => Err(NO_CANVAS.into()),
        Some(c) if binding == Some(c) => Ok(()),
        Some(_) => Err(STALE.into()),
    }
}

pub(super) const INSTRUCTIONS: &str = "openpaperdesign (app library): the OpenPaperDesign home \
    in the Codex sidebar creates, opens, closes and reopens canvases that OpenPaperDesign keeps \
    in its own data folder, and opens them in the loopback browser editor. Those actions are \
    the person's, from the view; they need no model call. This server has no workspace and no \
    project editing tools: openpaperdesign_status reports what it serves. Workspace design \
    projects (design/<name>/ in a repository) need a separately started `openpaperdesign-mcp \
    --codex-context` server; don't claim this one can bind or edit them.";

/// The tools of an app-library process: the home, this mode's status, the inventory and the
/// app-only library actions. No project or upstream design tool.
pub(super) fn listed_tools() -> Vec<Tool> {
    let none = json!({"type": "object", "properties": {}, "additionalProperties": false});
    let mut out = vec![
        json!({
            "name": HOME_TOOL,
            "title": "OpenPaperDesign",
            "description": "Open the OpenPaperDesign home (the Codex sidebar entry): the app's \
                own canvas library. The person creates, opens and closes canvases there \
                directly. Calling this changes nothing.",
            "inputSchema": none,
            "icons": icons(),
            "_meta": {
                "ui": {"resourceUri": HOME_URI},
                "openai/ui": {"entrypoints": [{"type": "global"}, {"type": "thread"}]}
            }
        }),
        json!({
            "name": "openpaperdesign_status",
            "description": "Report what this OpenPaperDesign server serves: the app library \
                (its folder and the canvas open in this window), and that it has no workspace \
                or project editing tools.",
            "inputSchema": none,
        }),
    ];
    out.extend(
        own_tools()
            .into_iter()
            .filter(|t| t.name == PROJECTS_TOOL)
            .map(|t| serde_json::to_value(t).expect("tool serializes")),
    );
    out.extend(tools());
    out.into_iter()
        .map(|t| serde_json::from_value(t).expect("valid tool"))
        .collect()
}

/// The app-only library actions. Their schemas match what the server enforces.
fn tools() -> Vec<Value> {
    let app = json!({"ui": {"visibility": ["app"]}});
    let none = json!({"type": "object", "properties": {}, "additionalProperties": false});
    let force = json!({"type": "boolean", "description": "Go ahead even though the open \
        canvas's browser tab may still have unsaved edits; they are lost. A switch also needs \
        `replace`."});
    let id = json!({"type": "string", "pattern": "^[0-9a-f]{16}$", "description": "A canvas id from the library list."});
    let binding = json!({"type": "string", "pattern": "^[0-9a-f]{16}\\.[0-9]{1,20}$",
        "description": "The binding this view was shown (from an editor result). A stale one is refused."});
    let replace = json!({"type": "string", "pattern": "^[0-9a-f]{16}\\.[0-9]{1,20}$",
        "description": "With force: the binding the person confirmed replacing. A stale one is refused."});
    vec![
        json!({"name": LIST, "_meta": app, "inputSchema": none,
            "description": "For the home view: the canvases in OpenPaperDesign's app library \
                (read-only: nothing is created, opened or locked)."}),
        json!({"name": CREATE, "_meta": app,
            "description": "For the home view: create a new, empty canvas in the app library \
                and open it in the editor. No design system or direction is applied.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string", "minLength": 1, "maxLength": MAX_NAME_CHARS},
                "force": force,
                "replace": replace
            }, "required": ["name"], "additionalProperties": false}}),
        json!({"name": OPEN, "_meta": app,
            "description": "For the home view: open a library canvas in the editor.",
            "inputSchema": {"type": "object", "properties": {"id": id, "force": force, "replace": replace},
                "required": ["id"], "additionalProperties": false}}),
        json!({"name": EDITOR, "_meta": app,
            "description": "For the home view: the editor link and state of the canvas open \
                in this window; with `binding`, only if that is still the one open.",
            "inputSchema": {"type": "object", "properties": {"binding": binding},
                "additionalProperties": false}}),
        json!({"name": CLOSE, "_meta": app,
            "description": "For the home view: close the canvas this view shows (its editor \
                server stops). Saved edits stay in the library; closing saves nothing.",
            "inputSchema": {"type": "object", "properties": {"binding": binding, "force": force},
                "required": ["binding"], "additionalProperties": false}}),
        json!({"name": IMPORT, "_meta": app,
            "description": "For the home view: import an OpenPaperDesign canvas file (its \
                text, chosen by the person) as a new library canvas, and open it. The file is \
                checked completely first; nothing existing is overwritten or merged.",
            "inputSchema": {"type": "object", "properties": {
                "payload": {"type": "string", "maxLength": MAX_PAYLOAD_BYTES},
                "force": force,
                "replace": replace
            }, "required": ["payload"], "additionalProperties": false}}),
        json!({"name": EXPORT, "_meta": app,
            "description": "For the home view: the canvas this view shows, as last saved, as \
                a canvas file (canvas only: no design direction or tokens).",
            "inputSchema": {"type": "object", "properties": {"binding": binding},
                "required": ["binding"], "additionalProperties": false}}),
    ]
}

/// A new canvas's name: 1 to 60 characters after trimming, without control characters.
fn canvas_name(name: Option<&str>) -> Result<String, String> {
    let n = name.map(str::trim).unwrap_or("");
    if n.is_empty() || n.chars().count() > MAX_NAME_CHARS || n.chars().any(char::is_control) {
        return Err(format!(
            "A canvas name of 1 to {MAX_NAME_CHARS} characters, without control characters, is \
             required. Nothing was created."
        ));
    }
    Ok(n.to_string())
}

impl Launcher {
    /// Every call in an app-library process.
    pub(super) async fn library_call(
        &self,
        lib: &Arc<Library>,
        name: &str,
        args: Option<serde_json::Map<String, Value>>,
        context: &RequestContext<RoleServer>,
    ) -> ToolResult {
        match name {
            HOME_TOOL => {
                if args.as_ref().is_some_and(|a| !a.is_empty()) {
                    return Err(format!(
                        "{HOME_TOOL} takes no arguments; it only opens the home view."
                    ));
                }
                text(json!({
                    "surface": "home",
                    "version": VERSION,
                    "mode": "app-library",
                    "note": "Static home view. Nothing was read or changed.",
                }))
            }
            PROJECTS_TOOL => {
                let ct = context.ct.clone();
                projects(args.as_ref(), async move { ct.cancelled().await }).await
            }
            "openpaperdesign_status" => {
                if args.as_ref().is_some_and(|a| !a.is_empty()) {
                    return Err("openpaperdesign_status takes no arguments.".into());
                }
                let state = self.state.lock().await;
                let problem = lib.check().err();
                let open = state.bound.as_ref().and_then(|b| b.doc.clone());
                text(json!({
                    "version": VERSION,
                    "mode": "app-library",
                    "library": lib.root(),
                    "library_usable": problem.is_none(),
                    "library_problem": problem,
                    "open_canvas": open,
                    "editor_opened": state.bound.as_ref().is_some_and(|b| b.editor_opened),
                    "browser_editor": if bundle_complete() { "built in" } else { "not built" },
                    "workspace": null,
                    "workspace_usable": false,
                    "workspace_problem": "This server runs the app library and has no workspace \
                        or project tools; workspace projects need a separately started \
                        openpaperdesign-mcp --codex-context server.",
                }))
            }
            n if TOOLS.contains(&n) => {
                self.app(lib, n, Value::Object(args.unwrap_or_default()))
                    .await
            }
            _ => Err(LIBRARY_ONLY.into()),
        }
    }

    pub(super) async fn app(&self, lib: &Arc<Library>, name: &str, args: Value) -> ToolResult {
        let (strings, flags): (&[&str], &[&str]) = match name {
            CREATE => (&["name", "replace"], &["force"]),
            OPEN => (&["id", "replace"], &["force"]),
            EDITOR | EXPORT => (&["binding"], &[]),
            CLOSE => (&["binding"], &["force"]),
            IMPORT => (&["payload", "replace"], &["force"]),
            _ => (&[], &[]),
        };
        let map = direct::strict(&args, strings, flags)?;
        let get = |k: &str| map.get(k).and_then(Value::as_str);
        let force = map.get("force").and_then(Value::as_bool).unwrap_or(false);
        for k in ["binding", "replace"] {
            if get(k).is_some_and(|b| !valid_binding(b)) {
                return Err(format!(
                    "`{k}` isn't a binding from an editor result; nothing was read or changed"
                ));
            }
        }
        let (binding, replace) = (get("binding"), get("replace"));
        match name {
            LIST => {
                let state = self.state.lock().await;
                let open = state.bound.as_ref().and_then(|b| b.doc.clone());
                drop(state);
                text(lib.list(open.as_deref())?)
            }
            CREATE => {
                let name = canvas_name(get("name"))?;
                self.app_create(lib, NewDocument::blank(&name), force, replace, "created")
                    .await
            }
            OPEN => {
                let id = get("id")
                    .filter(|s| valid_id(s))
                    .ok_or("A canvas id from the library list is required. Nothing was opened.")?;
                self.app_open(lib, id, force, replace).await
            }
            EDITOR => {
                let state = self.state.clone().lock_owned().await;
                let mut state = Self::fence_library(state).await?;
                // Without `binding`: whichever canvas is open (the menu's "Open current canvas").
                if binding.is_some() {
                    expect_binding(&state, binding)?;
                }
                text(Self::library_editor(&mut state, lib, "reopened")?)
            }
            CLOSE => self.app_close(binding, force).await,
            IMPORT => {
                let file =
                    get("payload").ok_or("A canvas file is required. Nothing was imported.")?;
                // Checked completely before anything is locked or written.
                let doc = payload::validate(file)?;
                self.app_create(lib, doc, force, replace, "imported").await
            }
            EXPORT => self.app_export(binding).await,
            _ => Err(format!("unknown tool {name}")),
        }
    }

    /// An open canvas whose library or lease identity no longer checks out is closed through
    /// the retirement fence before anything else happens, and the action is refused.
    async fn fence_library(
        state: OwnedMutexGuard<LauncherState>,
    ) -> Result<OwnedMutexGuard<LauncherState>, String> {
        let invalid = state
            .bound
            .as_ref()
            .filter(|b| b.doc.is_some())
            .and_then(|b| b.server.bound_manifest().err());
        match invalid {
            None => Ok(state),
            Some(why) => {
                Self::retire_session(state, why.clone()).await?;
                Err(format!(
                    "The open canvas can't be used any more ({why}), so it was closed: nothing \
                     more is read or saved through it. Nothing else was changed."
                ))
            }
        }
    }

    /// The open library canvas's editor link and identity, from this same binding. Reading its
    /// manifest first re-checks the canvas, the library and the lease.
    fn library_editor(
        state: &mut LauncherState,
        lib: &Library,
        action: &str,
    ) -> Result<Value, String> {
        let Some(b) = state.bound.as_mut() else {
            return Err(NO_CANVAS.into());
        };
        let Some(doc) = b.doc.clone() else {
            return Err(NO_CANVAS.into());
        };
        let manifest = b.server.bound_manifest().map_err(|e| {
            format!("The open canvas can't be shown: {e}. Nothing was opened; close it and open it again.")
        })?;
        b.editor_opened = true;
        let (loaded, saves) = b.server.editor_state();
        Ok(json!({
            "action": action,
            "storage": "app-library",
            "url": format!("http://127.0.0.1:{}/#token={}", b.port, b.token),
            "browser_editor": if bundle_complete() { "built in" } else { "not built" },
            "canvas": {
                "id": doc,
                "name": manifest.name,
                "pages": manifest.pages.len(),
                "modified": manifest.modified,
            },
            "binding": b.binding,
            "location": b.server.dir(),
            "library": lib.root(),
            "editor_loaded": loaded,
            "browser_saves": saves,
            "open_in": "the host's in-app browser (loopback only)",
            "note": "Saved edits are kept in OpenPaperDesign's app library. The token in the URL \
                is for this window only; don't write it into files.",
        }))
    }

    /// Create (or import) a canvas and open it. Fallible preparation (listener, lease, files,
    /// checks, server) finishes before the open canvas, if any, is retired; any failure before
    /// the handoff keeps the open canvas, its URL and lease, and removes exactly what this call
    /// made (or names what stays). The work runs as an owned task.
    async fn app_create(
        &self,
        lib: &Arc<Library>,
        doc: NewDocument,
        force: bool,
        replace: Option<&str>,
        action: &'static str,
    ) -> ToolResult {
        let state = self.state.clone().lock_owned().await;
        let state = Self::fence_library(state).await?;
        library_switch(&state, force, replace)?;
        let lib2 = lib.clone();
        let id = tokio::spawn(async move {
            let lib = lib2;
            let listening = Self::listen(lib.root())
                .await
                .map_err(|e| format!("{e}. Nothing was created."))?;
            let lease = Lease::fresh(&lib)?;
            let id = lease.doc.clone();
            let (dir, created) = lib.create_document(&lease, &doc.fresh())?;
            lib.check_document(&id).map_err(|e| created.fail(&e))?;
            #[cfg(test)]
            if crate::test_support::fault(lib.root(), "app_prepare_fails") {
                return Err(created.fail("injected test failure"));
            }
            let lease = Arc::new(lease);
            let guard = crate::library::guard(lib.clone(), &lease);
            let install = Self::prepare_library_install(dir, &id, listening, lease, guard)
                .map_err(|e| created.fail(&e))?;
            Self::install(state, install, None, Owner::Library).await?;
            Ok::<_, String>(id)
        })
        .await
        .map_err(|e| {
            format!("creating the canvas stopped unexpectedly ({e}); check the library list")
        })??;
        let mut state = self.state.clone().lock_owned().await;
        if state.bound.as_ref().and_then(|b| b.doc.as_deref()) != Some(id.as_str()) {
            return Err(format!(
                "The canvas was {action}, but this window's open canvas changed before it could \
                 be shown. It is in the library list."
            ));
        }
        text(Self::library_editor(&mut state, lib, action)?)
    }

    /// Open a library canvas. Opening the canvas that is already open here is a checked no-op;
    /// a busy, invalid or failing target leaves the open canvas, its URL and lease as they were.
    async fn app_open(
        &self,
        lib: &Arc<Library>,
        id: &str,
        force: bool,
        replace: Option<&str>,
    ) -> ToolResult {
        let state = self.state.clone().lock_owned().await;
        let mut state = Self::fence_library(state).await?;
        if replace.is_some_and(|r| current_binding(&state) != Some(r)) {
            // A confirmation shown for another binding never acts on this one.
            return Err(STALE.into());
        }
        if state.bound.as_ref().and_then(|b| b.doc.as_deref()) == Some(id) {
            return text(Self::library_editor(&mut state, lib, "already-open")?);
        }
        // Checked before a lease file is made for it: only an existing library canvas.
        lib.read_document(id)
            .map_err(|e| format!("This canvas can't be opened: {e}. Nothing was changed."))?;
        library_switch(&state, force, replace)?;
        let listening = Self::listen(lib.root())
            .await
            .map_err(|e| format!("{e}. Nothing was opened; the open canvas is unchanged."))?;
        let lease = match Lease::acquire(lib, id) {
            Ok(l) => l,
            Err(LeaseError::Busy) => return Err(BUSY.into()),
            Err(LeaseError::Failed(e)) => {
                return Err(format!(
                    "{e}. Nothing was opened; the open canvas is unchanged."
                ));
            }
        };
        // Under the lease, before upstream `Project::open` and its stale-temp cleanup.
        lib.check_document(id)
            .map_err(|e| format!("This canvas can't be opened: {e}. Nothing was changed."))?;
        let lease = Arc::new(lease);
        let guard = crate::library::guard(lib.clone(), &lease);
        let install =
            Self::prepare_library_install(lib.document_dir(id), id, listening, lease, guard)?;
        Self::install(state, install, None, Owner::Library).await?;
        let mut state = self.state.clone().lock_owned().await;
        if state.bound.as_ref().and_then(|b| b.doc.as_deref()) != Some(id) {
            return Err(
                "The canvas was opened, but this window's open canvas changed before it could be \
                 shown. Open it again from the library."
                    .into(),
            );
        }
        text(Self::library_editor(&mut state, lib, "opened")?)
    }

    fn prepare_library_install(
        dir: PathBuf,
        doc: &str,
        (listener, port, token): (Listener, u16, String),
        lease: Arc<Lease>,
        guard: crate::Guard,
    ) -> Result<Install, String> {
        let prepared = Server::prepare_library(dir, BUNDLE_FILES, port, token.clone(), guard)
            .map_err(|e| e.to_string())?;
        Ok(Install {
            listener,
            port,
            token,
            rel: doc.to_string(),
            prepared,
            doc: Some(doc.to_string()),
            lease: Some(lease),
        })
    }

    /// Close the canvas the view shows (`binding`, checked under the state lock): admission
    /// closes, started writers and rollback drain, then the binding and its lease go. Owned, so
    /// a caller leaving can't stop it halfway. Closing saves nothing that is only in a browser
    /// tab. Its `force` confirmation is bound to the same `binding`.
    async fn app_close(&self, binding: Option<&str>, force: bool) -> ToolResult {
        let state = self.state.clone().lock_owned().await;
        expect_binding(&state, binding)?;
        let Some(b) = state.bound.as_ref() else {
            return Err(NO_CANVAS.into());
        };
        let Some(doc) = b.doc.clone() else {
            return Err(NO_CANVAS.into());
        };
        if b.editor_opened && !force {
            return Err(CLOSE_NEEDS_FORCE.into());
        }
        tokio::spawn(async move {
            let mut state = state;
            if let Some(b) = state.bound.as_ref() {
                b.server.retire().await;
            }
            state.bound = None;
        })
        .await
        .map_err(|e| format!("closing stopped unexpectedly: {e}"))?;
        text(json!({
            "action": "closed",
            "canvas": {"id": doc},
            "note": "The canvas's editor server was stopped. Everything saved before this is in \
                the library; an edit still unsaved in a browser tab was not saved by closing.",
        }))
    }

    /// The open canvas as last saved, as a canvas file. The snapshot is taken under its lease
    /// and writer fence (it waits for a save under way); the fence is released before the text
    /// is returned. Only the open canvas the view shows (`binding`) can be exported; a binding
    /// replaced meanwhile is retired, so its snapshot is refused rather than read from the
    /// replacement.
    async fn app_export(&self, binding: Option<&str>) -> ToolResult {
        let (server, doc, current) = {
            let state = self.state.lock().await;
            let b = state
                .bound
                .as_ref()
                .filter(|b| b.doc.is_some())
                .ok_or("Open the canvas to export first. Nothing was exported.")?;
            expect_binding(&state, binding)?;
            (
                b.server.clone(),
                b.doc.clone().unwrap_or_default(),
                b.binding.clone(),
            )
        };
        let e = server
            .saved_snapshot()
            .await
            .map_err(|e| format!("Not exported: {e}"))?;
        let stem: String = e
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .take(40)
            .collect();
        let stem = stem.trim_matches('-');
        text(json!({
            "action": "exported",
            "binding": current,
            "canvas": {"id": doc, "name": e.name},
            "content": payload::CANVAS_ONLY,
            "file_name": format!("{}.opdcanvas.json", if stem.is_empty() { "canvas" } else { stem }),
            "bytes": e.text.len(),
            "payload": e.text,
            "note": payload::NOTE,
        }))
    }
}
