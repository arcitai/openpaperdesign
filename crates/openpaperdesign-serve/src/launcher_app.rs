//! The app library's direct actions (`openpaperdesign-mcp --app-library <dir>`): list, create,
//! open, close, reopen, rename, import and export canvases the app owns (see `crate::library`),
//! from the home view, with no model turn and no message.
//!
//! In an app-library process only these tools, the home, its resource,
//! `openpaperdesign_status` (this mode's own report), the read-only Codex project inventory and
//! the agent's access to the same canvases (`launcher_library_agent.rs`: model-visible library
//! controls and upstream's design tools, each naming the open canvas's binding) are answered.
//! Every project route (`bind_project`, `create_project`, `open_editor`, the workspace
//! canvases list, the `_ui_*` workspace actions and the folder picker) is refused without being
//! admitted and without touching any state, so a call without workspace context can neither
//! use nor retire anything here. No Workspace is ever
//! made for the library, and no argument names a path: only a server-made canvas id, a bounded
//! name, a `force` flag, a server-made binding identity or a canvas file's bytes reach it.
//!
//! One canvas is bound per process, through the same prepare/install/retire handoff as project
//! mode; its lease's handle moves with the binding and is dropped only after the old server
//! drained. Several processes (Codex chats, panels) may bind the same canvas (issue #23): each
//! open takes its editing, and so does each new view of a binding (an in-panel mount, a browser
//! tab); every write is admitted by it under the lease, and a view that lost it is read-only
//! until it takes it back, handing its unsaved change over meanwhile (see `library`,
//! "Editing").
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

#[path = "launcher_library_agent.rs"]
mod agent_api;
pub(super) use agent_api::AGENT_TOOLS;
#[cfg(test)]
pub(super) use agent_api::{
    AGENT_CLOSE, AGENT_CREATE, AGENT_CURRENT, AGENT_EDITOR, AGENT_LIST, AGENT_OPEN,
};

pub(super) const LIST: &str = "openpaperdesign_app_canvases";
pub(super) const CREATE: &str = "openpaperdesign_app_create";
pub(super) const OPEN: &str = "openpaperdesign_app_open";
pub(super) const EDITOR: &str = "openpaperdesign_app_editor";
pub(super) const CLOSE: &str = "openpaperdesign_app_close";
pub(super) const IMPORT: &str = "openpaperdesign_app_import";
pub(super) const EXPORT: &str = "openpaperdesign_app_export";
pub(super) const RENAME: &str = "openpaperdesign_app_rename";
/// The views' appearance preference (System, Light or Dark), kept in the library's own data
/// folder; the list reports it. A view preference: no canvas, lease or binding is involved.
pub(super) const APPEARANCE: &str = "openpaperdesign_app_appearance";
/// The views' other view preferences (the home's layout and the editor's canvas background),
/// one small record in the library's own data folder; the list reports them. As the
/// appearance: no canvas, lease or binding is involved.
pub(super) const VIEW: &str = "openpaperdesign_app_view";
// The in-panel editor's transport (the home view's own editor, see `panel_call`).
pub(super) const ASSET: &str = "openpaperdesign_app_editor_asset";
pub(super) const PAGE: &str = "openpaperdesign_app_page";
pub(super) const REVISION: &str = "openpaperdesign_app_page_revision";
pub(super) const SAVE: &str = "openpaperdesign_app_page_save";
pub(super) const READY: &str = "openpaperdesign_app_ready";
pub(super) const RELEASE: &str = "openpaperdesign_app_page_release";
// The whole canvas (every page, the page list and the active page) for the in-panel editor,
// as the browser editor's `/api/canvas`; the per-page tools above stay as they were.
pub(super) const CANVAS: &str = "openpaperdesign_app_canvas";
pub(super) const CANVAS_REVISION: &str = "openpaperdesign_app_canvas_revision";
pub(super) const CANVAS_SAVE: &str = "openpaperdesign_app_canvas_save";
// The in-panel editor's Edit here (issue #23): take the canvas's editing back for this view.
pub(super) const CLAIM: &str = "openpaperdesign_app_claim";
// Issue #23's new-view recovery: a view that lost the editing hands its unsaved change over
// (`offer`), and the editing view applies or dismisses it.
pub(super) const DRAFT: &str = "openpaperdesign_app_draft";
pub(super) const PANEL_TOOLS: [&str; 11] = [
    ASSET,
    PAGE,
    REVISION,
    SAVE,
    READY,
    RELEASE,
    CANVAS,
    CANVAS_REVISION,
    CANVAS_SAVE,
    CLAIM,
    DRAFT,
];
pub(super) const TOOLS: [&str; 21] = [
    LIST,
    CREATE,
    OPEN,
    EDITOR,
    CLOSE,
    IMPORT,
    EXPORT,
    RENAME,
    APPEARANCE,
    VIEW,
    ASSET,
    PAGE,
    REVISION,
    SAVE,
    READY,
    RELEASE,
    CANVAS,
    CANVAS_REVISION,
    CANVAS_SAVE,
    CLAIM,
    DRAFT,
];

/// The packaged renderer goes to the in-panel editor in parts of this many bytes (the last one
/// shorter), in order; a renderer larger than `MAX_ASSET_BYTES` isn't sent at all.
pub(super) const PART_BYTES: usize = 256 * 1024;
/// The largest renderer sent: 18 MiB (revision 23; it was 16 MiB). The renderer embeds every
/// bundled font face, and Geist Mono's four (613,604 bytes) took the measured 16,460,800-byte
/// renderer past the old 16 MiB. A fixed bound for this closed bundle, checked at both ends
/// (the view's `panel.js` refuses a larger one too), not room for a font loader; the parts, the
/// panel message bound, the host allowance and the library's own image limit are unchanged.
pub(super) const MAX_ASSET_BYTES: usize = 18 * 1024 * 1024;
/// The largest page message the in-panel editor exchanges, counted as the serialized JSON
/// (UTF-8, escaping included): a page read's result, or a save's arguments. A deliberately
/// conservative choice, well below the 64 MiB page limit, because the host's message limits
/// are unknown (rmcp's own stdio codec has none by default; that says nothing about the
/// host). A larger page isn't opened in the panel, and a larger save isn't sent; neither
/// changes any file. The browser editor link still works for them.
pub(super) const MAX_PANEL_MESSAGE: usize = 4 * 1024 * 1024;
/// Room left within `MAX_PANEL_MESSAGE` for what a host adds around a tool request or result
/// it forwards (its own ids, metadata and framing). An explicit, bounded allowance, not a known
/// host limit; the view's `panel.js` uses the same number.
pub(super) const HOST_OVERHEAD: usize = 16 * 1024;

/// The size of `r` as this server answers it: `to_result` (what `call_tool` sends) inside a
/// JSON-RPC response with the widest id, every level of escaping included, plus
/// `HOST_OVERHEAD`.
pub(super) fn response_bytes(r: &ToolResult) -> usize {
    let wire = json!({"jsonrpc": "2.0", "id": u64::MAX, "result": to_result(r.clone())});
    serde_json::to_vec(&wire)
        .map_or(usize::MAX, |v| v.len())
        .saturating_add(HOST_OVERHEAD)
}

/// The size of a `tools/call` request for `name` with `args` as the view sends it (and a host
/// forwards it): a JSON-RPC request with the widest id, plus `HOST_OVERHEAD`.
pub(super) fn request_bytes(name: &str, args: &Value) -> usize {
    let wire = json!({"jsonrpc": "2.0", "id": u64::MAX, "method": "tools/call",
        "params": {"name": name, "arguments": args}});
    serde_json::to_vec(&wire)
        .map_or(usize::MAX, |v| v.len())
        .saturating_add(HOST_OVERHEAD)
}
/// JSON integers are exact up to 2^53 - 1.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

pub(super) const NOT_LIBRARY: &str = "This OpenPaperDesign server wasn't started for the app \
    library (it serves workspace projects), so app-library actions aren't available here. \
    Nothing was read or changed.";
const LIBRARY_ONLY: &str = "This OpenPaperDesign server runs the app library: its canvases \
    live in OpenPaperDesign's own data folder, and project tools aren't available here. Nothing \
    was read or changed. Workspace projects need a separately started `openpaperdesign-mcp \
    --codex-context` (or `--workspace <dir>`) server.";
const NO_CANVAS: &str = "No canvas is open in this OpenPaperDesign window. Open one from the \
    library. Nothing was changed.";
/// Another view having the canvas open is no refusal any more (issue #23: opening it here takes
/// its editing). This is only another process's hold of its lease that outlasted the bounded
/// wait: a long save, a save there that couldn't be undone (that window keeps the canvas closed
/// to everyone), or an older OpenPaperDesign that keeps a canvas for as long as it is open.
const BUSY: &str = "This canvas is held right now by another OpenPaperDesign window (another \
    Codex chat or panel): it is in the middle of a save, a save there couldn't be undone, or it \
    runs an older OpenPaperDesign that keeps a canvas for as long as it has it open. Nothing was \
    changed here. Try again in a moment; if it stays like this, update OpenPaperDesign there or \
    end that chat's OpenPaperDesign.";
/// A Rename from the list of a canvas another view is editing (see `app_rename`).
const RENAME_ELSEWHERE: &str = "This canvas is open for editing in another OpenPaperDesign view \
    (or was, in a window that ended without closing it). Nothing was renamed. Open it here (its \
    editing then moves here), then rename it.";
const CLOSE_NEEDS_FORCE: &str = "The editor of this canvas was opened (its browser tab, or the \
    editor in another OpenPaperDesign view). Closing stops the server it saves to, so an edit \
    there that isn't saved yet (the page shows an unsaved change or \"Saving…\") would be lost. \
    Let it show \"Saved\", or accept losing it, then close again with force: true.";
const UNMOUNTED: &str = "This in-panel editor isn't open on the canvas any more (it was left, \
    or the canvas was closed or replaced). Nothing was read or changed.";
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

/// Whether an editor may hold unsaved edits of the open canvas: its browser link was given, or
/// an in-panel editor is mounted on it (in this view or another).
fn editors_in_use(b: &Bound) -> bool {
    b.editor_opened || !b.mounts.is_empty()
}

/// A switch away from the open canvas (create, import or open of another one). Once its editor
/// link was given, or while an in-panel editor is mounted on it, it needs `force` together with
/// `replace`, the binding the person confirmed losing; the refusal names the current binding so
/// a view can confirm exactly that one. A `replace` that isn't the current binding is stale and
/// refused.
fn library_switch(state: &LauncherState, force: bool, replace: Option<&str>) -> Result<(), String> {
    let current = current_binding(state);
    if let Some(r) = replace
        && current != Some(r)
    {
        return Err(STALE.into());
    }
    match state.bound.as_ref() {
        Some(b) if editors_in_use(b) && (!force || replace.is_none()) => Err(format!(
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
    in the Codex sidebar is the person's library of canvases that OpenPaperDesign keeps in its \
    own data folder (they belong to no Codex project). There they create, open, rename, export \
    and import canvases, and edit the open one right in that view, in paper-rs's own workspace \
    (a build without that in-view editor hands the canvas to its loopback web editor instead). \
    Going back to the library saves what is left and closes the canvas. Those actions are the \
    person's, from the view; they need no model call.\n\n\
    You can work on the same canvases: openpaperdesign_library_current shows the canvas open \
    here (id, binding, pages); openpaperdesign_library_canvases, _create, _open and _close pick \
    one, and _editor gives its loopback browser editor link if the person wants a browser tab. \
    The paper-rs design tools below edit it; pass the open canvas's `binding` with every \
    call (a stale one, such as after the person went back to their library, is refused with no \
    effect). Read a page before changing it \
    (get_tree_summary with nodeId \"root\" and its pageId); changes to an unread or \
    since-changed page are refused as conflicts. Each successful change is saved at once and \
    shows in the person's editor when it has no unsaved change of its own. A canvas whose editor may hold unsaved changes \
    can't be switched or closed from here: ask the person. The same canvas may be open in \
    several OpenPaperDesign views (chats, panels, browser tabs); the one opened last edits it, \
    the others are read-only. Your changes are this chat's: admitted while any view of this \
    chat's canvas edits it. A change refused because another view now edits it changed nothing: \
    open the canvas again here (openpaperdesign_library_open) only if the person wants editing \
    to move here, then read the page again. In this server list_resources and \
    open_file only name the open canvas; create_file and export aren't offered; images must be \
    data: URIs. Workspace design projects (design/<name>/ in a repository) need a separately \
    started `openpaperdesign-mcp --codex-context` server; don't claim this one can bind or edit \
    them.";

/// The tools of an app-library process: the home, this mode's status, the inventory, the
/// app-only library actions, and the agent's library controls and design tools.
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
                or workspace project tools.",
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
    out.extend(agent_api::agent_tools());
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
    let panel = json!({"type": "boolean", "description": "The view edits the canvas in its own \
        panel: the result has no browser editor link, and none counts as given."});
    let page_id = json!({"type": "string", "pattern": "^[A-Za-z0-9_-]{1,64}$",
        "description": "The page this in-panel editor read first (from its first page result)."});
    let mount = json!({"type": "integer", "minimum": 1, "maximum": MAX_SAFE_INTEGER,
        "description": "This in-panel editor's number (from its first page result). A released one is refused."});
    let doc = |props: Value, required: &[&str]| json!({"type": "object", "properties": props, "required": required, "additionalProperties": false});
    vec![
        json!({"name": LIST, "_meta": app, "inputSchema": none,
            "description": "For the home view: the canvases in OpenPaperDesign's app library, \
                and the views' kept appearance. It creates, opens or locks no canvas and never \
                changes a drawing. Before listing, it finishes this window's own bookkeeping of \
                canvases it closed while their editing and handover records couldn't be \
                written: it writes those two records then, and lets go of a canvas it kept \
                locked for that (`cleanup` in the result)."}),
        json!({"name": APPEARANCE, "_meta": app,
            "description": "For the home view: keep the views' appearance (system follows the \
                OS; light or dark override it). A view preference in the library's own folder: \
                no canvas is read or changed.",
            "inputSchema": doc(json!({"appearance": {"type": "string",
                "enum": crate::library::APPEARANCES}}), &["appearance"])}),
        json!({"name": VIEW, "_meta": app,
            "description": "For the home view: keep the views' other preferences, the home's file \
                layout (grid or list) and the editor's canvas background (none, dots or lines). \
                View preferences in the library's own folder: no canvas is read or changed.",
            "inputSchema": {"type": "object", "properties": {
                "home_layout": {"type": "string", "enum": crate::library::HOME_LAYOUTS},
                "background": {"type": "string", "enum": crate::library::BACKGROUNDS}
            }, "minProperties": 1, "additionalProperties": false}}),
        json!({"name": CREATE, "_meta": app,
            "description": "For the home view: create a new, empty canvas in the app library \
                and open it in the editor. No design system or direction is applied.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string", "minLength": 1, "maxLength": MAX_NAME_CHARS},
                "force": force,
                "replace": replace,
                "panel": panel
            }, "required": ["name"], "additionalProperties": false}}),
        json!({"name": OPEN, "_meta": app,
            "description": "For the home view: open a library canvas in the editor. Another \
                OpenPaperDesign view (another Codex chat or panel) may have it open too: this \
                view then takes its editing, and that one shows it read-only and keeps its \
                unsaved change. Opening the canvas already open here takes its editing back.",
            "inputSchema": {"type": "object", "properties": {"id": id, "force": force, "replace": replace, "panel": panel},
                "required": ["id"], "additionalProperties": false}}),
        json!({"name": EDITOR, "_meta": app,
            "description": "For the home view: the editor link and state of the canvas open \
                in this window; with `binding`, only if that is still the one open.",
            "inputSchema": {"type": "object", "properties": {"binding": binding, "panel": panel},
                "additionalProperties": false}}),
        json!({"name": ASSET, "_meta": app,
            "description": "For the home view's own editor: one part of this build's packaged \
                renderer (WebAssembly), base64. Parts are 262144 bytes, numbered from 0. Reads \
                no canvas and no file.",
            // The last part of the largest renderer sent (71: 18 MiB in 256 KiB parts).
            "inputSchema": doc(json!({"part": {"type": "integer", "minimum": 0,
                "maximum": MAX_ASSET_BYTES.div_ceil(PART_BYTES) - 1}}), &["part"])}),
        json!({"name": PAGE, "_meta": app,
            "description": "For the home view's own editor: the page of the canvas this view \
                shows. Without page_id: its active page (else its first), and the result names \
                this editor's mount; then with page_id and mount: that exact page again.",
            "inputSchema": doc(json!({"binding": binding, "page_id": page_id, "mount": mount}), &["binding"])}),
        json!({"name": REVISION, "_meta": app,
            "description": "For the home view's own editor: the current revision of its page.",
            "inputSchema": doc(json!({"binding": binding, "page_id": page_id, "mount": mount}), &["binding", "page_id", "mount"])}),
        json!({"name": SAVE, "_meta": app,
            "description": "For the home view's own editor: save its page (an upstream page \
                file), only if if_match is the page's revision now. A stale one is refused \
                and nothing is written.",
            "inputSchema": doc(json!({"binding": binding, "page_id": page_id, "mount": mount,
                "if_match": {"type": "string", "maxLength": 64}, "text": {"type": "string"}}),
                &["binding", "page_id", "mount", "if_match", "text"])}),
        json!({"name": READY, "_meta": app,
            "description": "For the home view's own editor: report that it loaded its page and \
                its renderer is Ready. Changes no file.",
            "inputSchema": doc(json!({"binding": binding, "mount": mount}), &["binding", "mount"])}),
        json!({"name": RELEASE, "_meta": app,
            "description": "For the home view's own editor: it was left (Back, Close or the \
                view going away). The canvas stays open; nothing is saved or changed.",
            "inputSchema": doc(json!({"binding": binding, "mount": mount}), &["binding", "mount"])}),
        json!({"name": CANVAS, "_meta": app,
            "description": "For the home view's own editor: the whole canvas this view shows \
                (every page in list order as an upstream page file, the active page, and the \
                canvas revision). Without mount the result names this editor's new mount, the \
                newest view, which takes the canvas's editing (`editing`); then with mount.",
            "inputSchema": doc(json!({"binding": binding, "mount": mount}), &["binding"])}),
        json!({"name": CANVAS_REVISION, "_meta": app,
            "description": "For the home view's own editor: the current canvas revision (any \
                page, the page list or the active page), whether this mount has the canvas's \
                editing (another view opened later, another mount of this editor too, takes \
                it; this one is then read-only), and the handover record (`handover`: the \
                offers views handed over and the views still waited for, see \
                openpaperdesign_app_draft, or null).",
            "inputSchema": doc(json!({"binding": binding, "mount": mount}), &["binding", "mount"])}),
        json!({"name": CLAIM, "_meta": app,
            "description": "For the home view's own editor: Edit here. Take the canvas's \
                editing back for this view from the view that took it. Nothing is saved by \
                this; the editor's next save is still checked against the canvas's revision, \
                so a change saved elsewhere meanwhile is a conflict, never overwritten.",
            "inputSchema": doc(json!({"binding": binding, "mount": mount}), &["binding", "mount"])}),
        json!({"name": DRAFT, "_meta": app,
            "description": "For the home view's own editor: the handover between views of one \
                canvas. `answer` (by a view that lost the editing): what it has unsaved, its \
                whole canvas as `text` (a canvas save with every page, on revision \
                `if_match`; `changed` when it differs from it) and its `unfinished` edits (the \
                editor's continuations, a JSON list as text), or nothing; it then no longer \
                keeps the editing view waiting. The editing view, by the handed-over change's \
                `id`: `apply` saves it on that revision (a change saved since is a conflict, \
                never overwritten), `keep` keeps it as a new library canvas, `dismiss` drops \
                it; `stop` (a pending view's `id`) stops waiting for that view. Answering \
                changes no canvas file.",
            "inputSchema": doc(json!({"binding": binding, "mount": mount,
                "action": {"type": "string",
                    "enum": ["answer", "apply", "keep", "dismiss", "stop"]},
                "id": {"type": "string", "maxLength": 64},
                "if_match": {"type": "string", "maxLength": 64}, "text": {"type": "string"},
                "changed": {"type": "boolean"}, "unfinished": {"type": "string"},
                // The kept recoveries an apply, keep or dismiss resolves (run 194, R83): their
                // ids, a JSON list as text (as `unfinished`).
                "resolved": {"type": "string", "maxLength": 2048}}),
                &["binding", "mount", "action"])}),
        json!({"name": CANVAS_SAVE, "_meta": app,
            "description": "For the home view's own editor: save the canvas (the page list, \
                the active page and each changed page), only if if_match is the canvas \
                revision now. A stale one is refused and nothing is written.",
            "inputSchema": doc(json!({"binding": binding, "mount": mount,
                "if_match": {"type": "string", "maxLength": 64}, "text": {"type": "string"}}),
                &["binding", "mount", "if_match", "text"])}),
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
                "replace": replace,
                "panel": panel
            }, "required": ["payload"], "additionalProperties": false}}),
        json!({"name": EXPORT, "_meta": app,
            "description": "For the home view: a library canvas, as last saved, as a canvas \
                file (canvas only: no design direction or tokens). Either the canvas this view \
                shows (`binding`) or a canvas from the list (`id`).",
            "inputSchema": {"type": "object", "properties": {"binding": binding, "id": id},
                "additionalProperties": false}}),
        json!({"name": RENAME, "_meta": app,
            "description": "For the home view: rename a library canvas (its name only; its \
                pages, content and identity stay). A canvas another OpenPaperDesign view is \
                editing is refused, with nothing changed: open it here first.",
            "inputSchema": {"type": "object", "properties": {
                "id": id,
                "name": {"type": "string", "minLength": 1, "maxLength": MAX_NAME_CHARS}
            }, "required": ["id", "name"], "additionalProperties": false}}),
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
                    "panel_editors": state.bound.as_ref().map_or(0, |b| b.mounts.len()),
                    "browser_editor": if bundle_complete() { "built in" } else { "not built" },
                    "panel_editor": match panel_editor() {
                        Ok(_) => "built in".to_string(),
                        Err(why) => format!("not available: {why}"),
                    },
                    "binding": state.bound.as_ref().and_then(|b| b.binding.clone()),
                    "agent_tools": "openpaperdesign_library_* and the paper-rs design tools, \
                        each naming the open canvas's binding",
                    "workspace": null,
                    "workspace_usable": false,
                    "workspace_problem": "This server runs the app library and has no workspace \
                        or workspace project tools; workspace projects need a separately started \
                        openpaperdesign-mcp --codex-context server.",
                }))
            }
            n if PANEL_TOOLS.contains(&n) => {
                let args = Value::Object(args.unwrap_or_default());
                // rmcp cancels `ct` on `notifications/cancelled` but keeps running this handler.
                let ct = context.ct.clone();
                self.panel_call(n, &args, async move { ct.cancelled().await })
                    .await
            }
            n if TOOLS.contains(&n) => {
                self.app(lib, n, Value::Object(args.unwrap_or_default()))
                    .await
            }
            n if AGENT_TOOLS.contains(&n) => {
                self.agent_library(lib, n, Value::Object(args.unwrap_or_default()))
                    .await
            }
            n if agent_api::is_design_tool(n) => self.agent_design(n, args).await,
            _ => Err(LIBRARY_ONLY.into()),
        }
    }

    pub(super) async fn app(&self, lib: &Arc<Library>, name: &str, args: Value) -> ToolResult {
        let (strings, flags): (&[&str], &[&str]) = match name {
            CREATE => (&["name", "replace"], &["force", "panel"]),
            OPEN => (&["id", "replace"], &["force", "panel"]),
            EDITOR => (&["binding"], &["panel"]),
            EXPORT => (&["binding", "id"], &[]),
            RENAME => (&["id", "name"], &[]),
            APPEARANCE => (&["appearance"], &[]),
            VIEW => (&["home_layout", "background"], &[]),
            CLOSE => (&["binding"], &["force"]),
            IMPORT => (&["payload", "replace"], &["force", "panel"]),
            _ => (&[], &[]),
        };
        let map = direct::strict(&args, strings, flags)?;
        let get = |k: &str| map.get(k).and_then(Value::as_str);
        let force = map.get("force").and_then(Value::as_bool).unwrap_or(false);
        // The view edits in its own panel: no browser link is handed out by this result.
        let panel = map.get("panel").and_then(Value::as_bool).unwrap_or(false);
        // A request for the view's own editor is refused before anything is locked, created,
        // imported, opened or switched when this build can't serve that editor.
        if panel {
            Self::panel_available(lib)?;
        }
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
                // What a binding that ended here still owes is recorded first (R92-01), and
                // the result says so (`cleanup`, R93-02).
                let mut listed = lib.list_settling(open.as_deref())?;
                // Read-only too: a missing or unknown preference is System, and nothing is made.
                listed["appearance"] = json!(lib.appearance());
                // Likewise the other view preferences (their defaults: grid, no background).
                listed["view"] = lib.view().to_json();
                text(listed)
            }
            VIEW => {
                // Off the async executor: a keep may wait (bounded) for another process's.
                let (lib, layout, background) = (
                    lib.clone(),
                    get("home_layout").map(String::from),
                    get("background").map(String::from),
                );
                let kept = tokio::task::spawn_blocking(move || {
                    lib.set_view(layout.as_deref(), background.as_deref())
                })
                .await
                .map_err(|e| format!("The view preferences couldn't be kept ({e})."))??;
                text(json!({
                    "view": kept.to_json(),
                    "note": "Kept as this library's view preferences. No canvas was read or \
                        changed.",
                }))
            }
            APPEARANCE => {
                let kept = lib.set_appearance(get("appearance").unwrap_or(""))?;
                text(json!({
                    "appearance": kept,
                    "note": "Kept as this library's appearance (a view preference). No canvas \
                        was read or changed.",
                }))
            }
            CREATE => {
                let name = canvas_name(get("name"))?;
                let doc = NewDocument::blank(&name);
                self.app_create(lib, doc, force, replace, "created", panel)
                    .await
            }
            OPEN => {
                let id = get("id")
                    .filter(|s| valid_id(s))
                    .ok_or("A canvas id from the library list is required. Nothing was opened.")?;
                self.app_open(lib, id, force, replace, panel).await
            }
            EDITOR => {
                let state = self.state.clone().lock_owned().await;
                let mut state = Self::fence_library(state).await?;
                // Without `binding`: whichever canvas is open (the menu's "Open current canvas").
                if binding.is_some() {
                    expect_binding(&state, binding)?;
                }
                text(Self::library_editor(&mut state, lib, "reopened", panel)?)
            }
            CLOSE => self.app_close(binding, force).await,
            IMPORT => {
                let file =
                    get("payload").ok_or("A canvas file is required. Nothing was imported.")?;
                // Checked completely before anything is locked or written.
                let doc = payload::validate(file)?;
                self.app_create(lib, doc, force, replace, "imported", panel)
                    .await
            }
            EXPORT => match (binding, get("id")) {
                (Some(_), Some(_)) => Err(
                    "Name either the shown canvas (binding) or a library canvas (id), not \
                         both. Nothing was exported."
                        .into(),
                ),
                (None, Some(id)) => self.app_export_id(lib, id).await,
                _ => self.app_export(binding).await,
            },
            RENAME => {
                let id = get("id")
                    .filter(|s| valid_id(s))
                    .ok_or("A canvas id from the library list is required. Nothing was renamed.")?;
                let name = canvas_name(get("name")).map_err(|_| {
                    format!(
                        "A canvas name of 1 to {MAX_NAME_CHARS} characters, without control \
                         characters, is required. Nothing was renamed."
                    )
                })?;
                self.app_rename(lib, id, &name).await
            }
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
    /// manifest first re-checks the canvas, the library and the lease. With `panel`, the view
    /// edits it in its own panel: the result carries no link and gives none out (Close and
    /// Switch then count the panel's mount instead, see `editors_in_use`).
    fn library_editor(
        state: &mut LauncherState,
        lib: &Library,
        action: &str,
        panel: bool,
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
        let (loaded, saves) = b.server.editor_state();
        let mut out = json!({
            "action": action,
            "storage": "app-library",
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
            // Issue #23: whether this binding has the canvas's editing ("here") or another,
            // later-opened view does ("elsewhere": this one is read-only until it takes it back
            // by opening the canvas again or the editor's Edit here).
            "editing": if b.server.editing_here(None) { "here" } else { "elsewhere" },
        });
        if panel {
            out["panel"] = json!(true);
            out["panel_editor"] = json!(match panel_editor() {
                Ok(_) => "built in".to_string(),
                Err(why) => format!("not available: {why}"),
            });
            out["open_in"] = json!("this view");
            out["note"] = json!("Saved edits are kept in OpenPaperDesign's app library.");
        } else {
            b.editor_opened = true;
            out["url"] = json!(format!("http://127.0.0.1:{}/#token={}", b.port, b.token));
            out["open_in"] = json!("the host's in-app browser (loopback only)");
            out["note"] = json!(
                "Saved edits are kept in OpenPaperDesign's app library. The token in the URL \
                 is for this window only; don't write it into files."
            );
        }
        Ok(out)
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
        panel: bool,
    ) -> ToolResult {
        let state = self.state.clone().lock_owned().await;
        refuse_if_shut_down(&state)?;
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
            let install = Self::prepare_library_install(&lib, dir, &id, listening, lease, guard)
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
        text(Self::library_editor(&mut state, lib, action, panel)?)
    }

    /// Open a library canvas. Opening the canvas that is already open here is a checked no-op;
    /// a busy, invalid or failing target leaves the open canvas, its URL and lease as they were.
    async fn app_open(
        &self,
        lib: &Arc<Library>,
        id: &str,
        force: bool,
        replace: Option<&str>,
        panel: bool,
    ) -> ToolResult {
        let state = self.state.clone().lock_owned().await;
        refuse_if_shut_down(&state)?;
        let mut state = Self::fence_library(state).await?;
        if replace.is_some_and(|r| current_binding(&state) != Some(r)) {
            // A confirmation shown for another binding never acts on this one.
            return Err(STALE.into());
        }
        if state.bound.as_ref().and_then(|b| b.doc.as_deref()) == Some(id) {
            // Opening it again here is asking to edit it here (issue #23: the newest open
            // edits): this binding takes the editing back if another view had taken it. The same
            // binding, URL and lease; a failed claim leaves it read-only, and the result says so.
            if let Some(b) = state.bound.as_ref()
                && !b.server.editing_here(None)
                && let Err(e) = b.server.claim(None, None).await
            {
                return Err(format!(
                    "This canvas is open here, but editing couldn't be taken back from the \
                     other view ({}). It stays read-only here; nothing was changed.",
                    e.msg
                ));
            }
            return text(Self::library_editor(
                &mut state,
                lib,
                "already-open",
                panel,
            )?);
        }
        // Checked before a lease file is made for it: only an existing library canvas.
        lib.read_document(id)
            .map_err(|e| format!("This canvas can't be opened: {e}. Nothing was changed."))?;
        // A canvas whose earlier write here couldn't be undone is never opened writable again in
        // this process (its lease is kept, so other processes get "busy"); nor one a binding that
        // ended here still keeps locked, once what it owes is tried again (R92-01).
        lib.settle_owed();
        lib.refuse_if_broken(id)
            .map_err(|e| format!("This canvas can't be opened: {e}. Nothing was changed."))?;
        library_switch(&state, force, replace)?;
        let listening = Self::listen(lib.root())
            .await
            .map_err(|e| format!("{e}. Nothing was opened; the open canvas is unchanged."))?;
        // Another view may have this canvas open (issue #23): that is no reason to refuse.
        // Only another process's hold of its lease (a write under way, or one it keeps) is,
        // after a bounded wait.
        let lease = match Lease::acquire_within(lib, id).await {
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
            Self::prepare_library_install(lib, lib.document_dir(id), id, listening, lease, guard)?;
        Self::install(state, install, None, Owner::Library).await?;
        let mut state = self.state.clone().lock_owned().await;
        if state.bound.as_ref().and_then(|b| b.doc.as_deref()) != Some(id) {
            return Err(
                "The canvas was opened, but this window's open canvas changed before it could be \
                 shown. Open it again from the library."
                    .into(),
            );
        }
        text(Self::library_editor(&mut state, lib, "opened", panel)?)
    }

    /// A library binding prepared under the lease the open (or creation) holds: its server, and
    /// its own editing of the canvas (`library::Editing`), claimed once it is installed.
    fn prepare_library_install(
        lib: &Arc<Library>,
        dir: PathBuf,
        doc: &str,
        (listener, port, token): (Listener, u16, String),
        lease: Arc<Lease>,
        guard: crate::Guard,
    ) -> Result<Install, String> {
        let editing = Arc::new(crate::library::Editing::new(lib.clone(), &lease)?);
        let prepared =
            Server::prepare_library(dir, BUNDLE_FILES, port, token.clone(), guard, editing)
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
        if editors_in_use(b) && !force {
            return Err(CLOSE_NEEDS_FORCE.into());
        }
        // The binding's end records that its views ended (waiting for another window's lease if
        // it must: `Server::retire`); what of that didn't happen goes with the answer (`cleanup`,
        // run 202, R91-01). Closed either way: its admission ended for good.
        let cleanup = tokio::spawn(async move {
            let mut state = state;
            let mut cleanup = None;
            if let Some(b) = state.bound.as_ref() {
                b.server.retire().await;
                cleanup = b.server.retirement_outcome();
            }
            state.bound = None;
            cleanup
        })
        .await
        .map_err(|e| format!("closing stopped unexpectedly: {e}"))?;
        let mut out = json!({
            "action": "closed",
            "canvas": {"id": doc},
            "note": "The canvas's editor server was stopped. Everything saved before this is in \
                the library; an edit still unsaved in a browser tab was not saved by closing.",
        });
        if let Some(c) = cleanup {
            out["cleanup"] = c;
        }
        text(out)
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
        text(export_result(&doc, current.as_deref(), e))
    }

    /// A library canvas named by its id, as last saved, as a canvas file (the home's file
    /// menu). The canvas open here is read through its binding's writer fence, as `app_export`
    /// reads it; any other canvas under its own lease, taken for the read and released after
    /// (one another window holds is refused as busy). Nothing is opened, bound or written.
    async fn app_export_id(&self, lib: &Arc<Library>, id: &str) -> ToolResult {
        if !valid_id(id) {
            return Err(
                "A canvas id from the library list is required. Nothing was exported.".into(),
            );
        }
        let state = self.state.clone().lock_owned().await;
        refuse_if_shut_down(&state)?;
        let state = Self::fence_library(state).await?;
        let open = state
            .bound
            .as_ref()
            .filter(|b| b.doc.as_deref() == Some(id))
            .map(|b| (b.server.clone(), b.binding.clone()));
        let (e, binding) = match open {
            Some((server, binding)) => {
                drop(state);
                let e = server
                    .saved_snapshot()
                    .await
                    .map_err(|e| format!("Not exported: {e}"))?;
                (e, binding)
            }
            None => {
                lib.read_document(id).map_err(|e| {
                    format!("This canvas can't be exported: {e}. Nothing was exported.")
                })?;
                // Its files may be inconsistent: not offered as a canvas file. (What a binding
                // that ended here still owes is recorded first: R92-01.)
                lib.settle_owed();
                lib.refuse_if_broken(id)
                    .map_err(|e| format!("Not exported: {e}. Nothing was exported."))?;
                // A canvas open in other views is read as last saved, as here (issue #23).
                let lease = match Lease::acquire_within(lib, id).await {
                    Ok(l) => l,
                    Err(LeaseError::Busy) => return Err(BUSY.into()),
                    Err(LeaseError::Failed(e)) => {
                        return Err(format!("{e}. Nothing was exported."));
                    }
                };
                let e = lib
                    .export_document(&lease)
                    .map_err(|e| format!("Not exported: {e}"))?;
                drop(lease);
                drop(state);
                (e, None)
            }
        };
        text(export_result(id, binding.as_deref(), e))
    }

    /// Rename a library canvas (its name only). The canvas open here is renamed through its
    /// binding's server (writer fence, commit, rollback, `Broken`); any other one under its own
    /// lease, taken for the write and released after. A canvas another OpenPaperDesign window
    /// holds is refused as busy; any refusal leaves its name, content and the open canvas as
    /// they were. The name is data in `project.json`, never a path.
    async fn app_rename(&self, lib: &Arc<Library>, id: &str, name: &str) -> ToolResult {
        let state = self.state.clone().lock_owned().await;
        refuse_if_shut_down(&state)?;
        let state = Self::fence_library(state).await?;
        let open = state
            .bound
            .as_ref()
            .filter(|b| b.doc.as_deref() == Some(id))
            .map(|b| b.server.clone());
        match open {
            Some(server) => {
                drop(state);
                server
                    .rename_canvas(name)
                    .await
                    .map_err(|e| format!("Not renamed: {}", e.msg))?;
            }
            None => {
                lib.read_document(id).map_err(|e| {
                    format!("This canvas can't be renamed: {e}. Nothing was changed.")
                })?;
                // A canvas fenced here keeps its lease, so this comes first: its reason, not
                // "busy". (What a binding that ended here still owes is recorded first: R92-01.)
                lib.settle_owed();
                lib.refuse_if_broken(id)
                    .map_err(|e| format!("Not renamed: {e}. Nothing was changed."))?;
                let lease = match Lease::acquire_within(lib, id).await {
                    Ok(l) => l,
                    Err(LeaseError::Busy) => return Err(BUSY.into()),
                    Err(LeaseError::Failed(e)) => {
                        return Err(format!("{e}. Nothing was renamed."));
                    }
                };
                // A view editing it (issue #23) would see its next save refused as a conflict:
                // its editing record is read under the lease, and any editor refuses the rename.
                match lib.editor_of(id) {
                    Ok(None) => {}
                    Ok(Some(_)) => return Err(RENAME_ELSEWHERE.into()),
                    Err(e) => return Err(format!("{e}. Nothing was renamed.")),
                }
                // The lease goes with the rename: released after it, or kept for good if its
                // write couldn't be undone. Its message is shown as it is.
                lib.rename_document(lease, name)?;
                drop(state);
            }
        }
        text(json!({"action": "renamed", "canvas": {"id": id, "name": name}}))
    }
}

/// A canvas file export's result: the file's suggested name (from the canvas name, letters,
/// digits, `-` and `_` only), its text and the notes about what it holds.
fn export_result(id: &str, binding: Option<&str>, e: payload::Exported) -> Value {
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
    json!({
        "action": "exported",
        "binding": binding,
        "canvas": {"id": id, "name": e.name},
        "content": payload::CANVAS_ONLY,
        "file_name": format!("{}.opdcanvas.json", if stem.is_empty() { "canvas" } else { stem }),
        "bytes": e.text.len(),
        "payload": e.text,
        "note": payload::NOTE,
    })
}

/// Only the named arguments, each of its type: `strings`, and `ints` (whole numbers from 0 to
/// 2^53 - 1; a fraction, a sign, a float such as `1.0` or a numeric string is refused). For
/// the in-panel editor's tools, whose numbers `direct::strict` (strings and booleans) can't
/// check.
fn panel_args<'a>(
    args: &'a Value,
    strings: &[&str],
    ints: &[&str],
    bools: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, String> {
    let map = args
        .as_object()
        .ok_or("arguments must be an object; nothing was read or changed")?;
    for (k, v) in map {
        let ok = (strings.contains(&k.as_str()) && v.is_string())
            || (ints.contains(&k.as_str()) && v.as_u64().is_some_and(|n| n <= MAX_SAFE_INTEGER))
            || (bools.contains(&k.as_str()) && v.is_boolean());
        if !ok {
            return Err(format!(
                "unexpected argument `{k}`, or not of its type; nothing was read or changed"
            ));
        }
    }
    Ok(map)
}

/// Part `part` of the packaged renderer: exactly `PART_BYTES` bytes from `part * PART_BYTES`
/// (the last part shorter), base64, with the renderer's identity, total size and part count
/// so the editor can check each answer belongs to the same renderer and comes in order. No
/// other file is ever sent, and no argument names one.
pub(super) fn asset_part(bytes: &[u8], identity: &str, part: u64) -> Result<Value, String> {
    if bytes.is_empty() || bytes.len() > MAX_ASSET_BYTES {
        return Err(format!(
            "this build's renderer is {} bytes, outside what the in-panel editor accepts (1 to \
             {MAX_ASSET_BYTES}); nothing was sent",
            bytes.len()
        ));
    }
    let parts = bytes.len().div_ceil(PART_BYTES);
    let i = usize::try_from(part)
        .ok()
        .filter(|i| *i < parts)
        .ok_or_else(|| {
            format!(
                "there is no renderer part {part}: it has {parts} parts, 0 to {}; nothing was sent",
                parts - 1
            )
        })?;
    let start = i * PART_BYTES;
    let end = (start + PART_BYTES).min(bytes.len());
    Ok(json!({
        "asset": WASM_FILE,
        "identity": identity,
        "bytes": bytes.len(),
        "parts": parts,
        "part": i,
        "offset": start,
        "part_bytes": end - start,
        "data": base64(&bytes[start..end]),
    }))
}

/// An admitted in-panel call: the open canvas's server and, for a call naming a mount, that
/// mount's lifetime flag.
type Admitted = (Arc<Server>, Option<Arc<std::sync::atomic::AtomicBool>>);

fn panel_unavailable(why: &str) -> String {
    format!(
        "The in-panel editor isn't available in this OpenPaperDesign build ({why}), so nothing \
         was created, imported, opened or changed. Rebuild OpenPaperDesign with its editor \
         (./scripts/build-web.sh), then reinstall it."
    )
}

/// An in-panel page call's refusal, as JSON the editor reads: the message, an HTTP-like status
/// (409 with the page's current revision for a conflict, 410 when the binding or mount isn't
/// current any more, 413 when it is too large for the panel, 400 for bad arguments, 503 when
/// it couldn't run) and nothing else. Every refusal means nothing was written.
fn panel_refusal(status: u16, error: &str, revision: Option<&str>) -> String {
    json!({"error": error, "status": status, "revision": revision}).to_string()
}

fn api_refusal(e: crate::ApiError) -> String {
    panel_refusal(e.status.as_u16(), &e.msg, e.etag.as_deref())
}

/// A panel's handover answer (`openpaperdesign_app_draft` `answer`): `if_match` the revision
/// its canvas (`text`, a whole canvas save) was made on, `changed`, and `unfinished` (a JSON
/// list of the editor's continuations, as text); none of them when it has nothing to hand
/// over. Checked as the browser's (`crate::parse_offer`).
fn panel_offer(
    map: &serde_json::Map<String, Value>,
) -> Result<Option<crate::library::NewOffer>, crate::ApiError> {
    if !map.contains_key("text") && !map.contains_key("unfinished") {
        return Ok(None);
    }
    let unfinished = match map.get("unfinished").and_then(Value::as_str) {
        Some(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        None => json!([]),
    };
    let body = json!({
        "base": map.get("if_match").cloned().unwrap_or(Value::Null),
        "canvas": map.get("text").cloned().unwrap_or(json!("")),
        "changed": map.get("changed").cloned().unwrap_or(json!(false)),
        "unfinished": unfinished,
    });
    crate::parse_offer(&body.to_string())
}

impl Launcher {
    /// Whether this build serves the in-panel editor (its module and renderer, see
    /// `panel_editor`).
    fn panel_available(lib: &Library) -> Result<(), String> {
        #[cfg(test)]
        if crate::test_support::fault(lib.root(), "panel_unavailable") {
            return Err(panel_unavailable("injected test failure"));
        }
        let _ = lib;
        panel_editor().map(|_| ()).map_err(panel_unavailable)
    }

    /// The home view's own editor (the in-panel editor): the renderer's parts, and the canvas
    /// (every page, the page list and the active page), its revision and save, the per-page
    /// read, revision and save (kept as they were), Ready and release calls of the canvas the
    /// view shows. The renderer part is static packaged data. Every page or canvas call names
    /// the view's `binding` and, after its first read, the `mount` that read returned; both are
    /// compared under the state lock before anything is read or written (`panel_server`, which
    /// also re-checks the canvas, library and lease). A `page_id` is checked as a page of the
    /// canvas's manifest; a mount is not tied to one page. The calls run through the same
    /// `Server` operations as the browser's HTTP API (`Server::canvas`, `canvas_revision`,
    /// `save_canvas_while`, `page`, `page_revision`, `save_page_while`, `mark_ready`), with the
    /// same conflict, commit and rollback rules, never through a loopback request.
    ///
    /// A save is tied to its mount's lifetime: a release revokes it, and a save not started by
    /// then never runs; a save whose request is cancelled before it starts never runs either
    /// (`cancelled` is observed before admission, while waiting for it and before the writer
    /// fence is taken). A started save always finishes its commit or rollback.
    pub(super) async fn panel_call(
        &self,
        name: &str,
        args: &Value,
        cancelled: impl std::future::Future<Output = ()>,
    ) -> ToolResult {
        tokio::pin!(cancelled);
        if name == ASSET {
            let map = panel_args(args, &[], &["part"], &[])?;
            let part = map
                .get("part")
                .and_then(Value::as_u64)
                .ok_or("`part` (a whole number) is required; nothing was sent")?;
            let (bytes, id) = wasm().ok_or(
                "This OpenPaperDesign command was built without the browser editor, so it has \
                 no renderer to send.",
            )?;
            return text(asset_part(bytes, id, part)?);
        }
        let (strings, ints, required): (&[&str], &[&str], &[&str]) = match name {
            PAGE => (&["binding", "page_id"], &["mount"], &["binding"]),
            REVISION => (
                &["binding", "page_id"],
                &["mount"],
                &["binding", "page_id", "mount"],
            ),
            SAVE => (
                &["binding", "page_id", "if_match", "text"],
                &["mount"],
                &["binding", "page_id", "mount", "if_match", "text"],
            ),
            READY | RELEASE => (&["binding"], &["mount"], &["binding", "mount"]),
            CANVAS => (&["binding"], &["mount"], &["binding"]),
            CANVAS_REVISION | CLAIM => (&["binding"], &["mount"], &["binding", "mount"]),
            DRAFT => (
                &[
                    "binding",
                    "action",
                    "id",
                    "if_match",
                    "text",
                    "unfinished",
                    "resolved",
                ],
                &["mount"],
                &["binding", "mount", "action"],
            ),
            CANVAS_SAVE => (
                &["binding", "if_match", "text"],
                &["mount"],
                &["binding", "mount", "if_match", "text"],
            ),
            _ => return Err(format!("unknown tool {name}")),
        };
        let bad = |status: u16, msg: &str| panel_refusal(status, msg, None);
        let bools: &[&str] = if name == DRAFT { &["changed"] } else { &[] };
        let map = panel_args(args, strings, ints, bools).map_err(|e| bad(400, &e))?;
        if let Some(k) = required.iter().find(|k| !map.contains_key(**k)) {
            return Err(bad(
                400,
                &format!("`{k}` is required; nothing was read or changed"),
            ));
        }
        let get = |k: &str| map.get(k).and_then(Value::as_str);
        let binding = get("binding").filter(|b| valid_binding(b)).ok_or_else(|| {
            bad(
                400,
                "`binding` isn't a binding from an editor result; nothing was read or changed",
            )
        })?;
        let mount = map.get("mount").and_then(Value::as_u64);
        let page_id = get("page_id");
        if page_id.is_some_and(|p| !crate::valid_page_id(p)) {
            return Err(bad(
                400,
                "`page_id` isn't a page id; nothing was read or changed",
            ));
        }
        if name == PAGE && page_id.is_some() != mount.is_some() {
            return Err(bad(
                400,
                "the first page read names neither page_id nor mount, and every later one names \
                 both; nothing was read or changed",
            ));
        }
        // No editor is mounted from a build that can't serve one.
        if (name == PAGE || name == CANVAS) && mount.is_none() {
            panel_editor().map_err(|why| bad(503, &panel_unavailable(why)))?;
        }
        let save = name == SAVE || name == CANVAS_SAVE;
        // A save too large for the panel is refused before anything else, as the request it
        // came in (see `request_bytes`).
        if save {
            let size = request_bytes(name, args);
            if size > MAX_PANEL_MESSAGE {
                return Err(bad(
                    413,
                    &format!(
                        "this save is {size} bytes as a message, more than the in-panel editor's \
                         {MAX_PANEL_MESSAGE}; nothing was written. Open the canvas in the browser \
                         editor to save a page this large."
                    ),
                ));
            }
        }
        if name == RELEASE {
            let released = {
                let mut state = self.state.lock().await;
                expect_binding(&state, Some(binding)).map_err(|e| bad(410, &e))?;
                let b = state.bound.as_mut().ok_or_else(|| bad(410, NO_CANVAS))?;
                // Revoked under the state lock: no later call is admitted for this mount, and
                // a save admitted earlier finds it revoked under the writer fence.
                mount.and_then(|m| b.mounts.remove(&m)).map(|live| {
                    live.store(false, std::sync::atomic::Ordering::Release);
                    b.server.clone()
                })
            };
            // Without the state lock: a save of this editor that had already started finishes
            // its commit or rollback before the release is acknowledged. The binding stays. Its
            // record cleanup follows (it never edits or is waited for again); what of that
            // didn't complete is said (`note`, `cleanup_status`: R89-01), never a refusal.
            if let Some(server) = &released {
                server.drain_writer().await;
                if let Some(m) = mount
                    && let Err(e) = server.release_view(&format!("m{m}")).await
                {
                    let mut out = crate::release_outcome(&e);
                    out["binding"] = json!(binding);
                    return text(out);
                }
            }
            return text(json!({"binding": binding, "released": released.is_some()}));
        }
        const CANCELLED: &str = "the save was cancelled before it started; nothing was written";
        const CANCELLED_CHANGE: &str = "this was cancelled before it started; nothing was changed";
        // Every call that changes something (a save, a claim, a handover) is admitted the same
        // way: cancellation first, its mount's lifetime under the writer fence (R78-05).
        let mutating = save || name == CLAIM || name == DRAFT;
        let admitted = if mutating {
            // An already cancelled save, or one cancelled while waiting for admission, never
            // starts.
            tokio::select! {
                biased;
                () = &mut cancelled => return Err(bad(503, CANCELLED)),
                r = self.panel_server(binding, mount) => r,
            }
        } else {
            self.panel_server(binding, mount).await
        };
        let (server, live) = admitted.map_err(|e| bad(410, &e))?;
        // Issue #23: each mount is a view of its own (`m<n>`), so of two mounts of this binding
        // only the newer one edits (see `Server::claim`).
        let view = mount.map(|m| format!("m{m}"));
        match name {
            PAGE => {
                let page = server.page(page_id).map_err(api_refusal)?;
                let body = String::from_utf8(page.bytes).map_err(|_| {
                    bad(
                        422,
                        "the page file isn't UTF-8 text, so it can't be edited here; nothing was \
                         changed",
                    )
                })?;
                let mut out = json!({
                    "binding": binding,
                    "canvas": {"name": page.project},
                    "page": {"id": page.id, "name": page.name},
                    "revision": page.revision,
                    "text": body,
                    // The widest mount number, for the measurement below.
                    "mount": MAX_SAFE_INTEGER,
                });
                // Measured as it would be sent: the tool result and its JSON-RPC response,
                // every level of escaping included, and the host allowance. Refused before a
                // mount is registered.
                let size = response_bytes(&text(out.clone()));
                if size > MAX_PANEL_MESSAGE {
                    return Err(bad(
                        413,
                        &format!(
                            "this page is {size} bytes as a message, more than the in-panel \
                             editor's {MAX_PANEL_MESSAGE}, so it wasn't opened here; nothing was \
                             changed. Open it in the browser editor instead."
                        ),
                    ));
                }
                let m = self.mount_for(binding, mount, &server).await?;
                out["mount"] = json!(m);
                text(out)
            }
            CANVAS => {
                let canvas = server.canvas().await.map_err(api_refusal)?;
                let mut out = canvas.to_json().map_err(api_refusal)?;
                out["binding"] = json!(binding);
                // Issue #23: whether this view may save it (see `CANVAS_REVISION`); the longer
                // value for the measurement below, the actual one once the mount is known.
                out["editing"] = json!(false);
                // The widest mount number, for the measurement below.
                out["mount"] = json!(MAX_SAFE_INTEGER);
                // Measured as it would be sent, as a page read is; refused before a mount is
                // registered.
                let size = response_bytes(&text(out.clone()));
                if size > MAX_PANEL_MESSAGE {
                    return Err(bad(
                        413,
                        &format!(
                            "this canvas is {size} bytes as a message, more than the in-panel \
                             editor's {MAX_PANEL_MESSAGE}, so it wasn't opened here; nothing was \
                             changed. Open it in the browser editor instead."
                        ),
                    ));
                }
                let m = self.mount_for(binding, mount, &server).await?;
                out["mount"] = json!(m);
                out["editing"] = json!(server.editing_here(Some(&format!("m{m}"))));
                text(out)
            }
            CANVAS_REVISION => {
                let revision = server.canvas_revision().map_err(api_refusal)?;
                // Issue #23: whether this view still has the canvas's editing (another view
                // opened later takes it; this one then shows the canvas read-only, and keeps
                // following what is saved there), and the handover record. Hints:
                // every save checks the editing again.
                text(json!({
                    "binding": binding,
                    "revision": revision,
                    "editing": server.editing_here(view.as_deref()),
                    "handover": server.handover_json(view.as_deref()),
                }))
            }
            CLAIM => {
                // Under the writer fence (a save of this view under way finishes first), with
                // this mount's lifetime checked again there: a claim of a mount released while
                // it waited never runs (R78-05). Cancellation first on every poll, as a save's;
                // once it holds the fence it runs to its end within that poll. A refusal
                // changes nothing and leaves the view read-only.
                // Its refusal keeps its condition and status (`Server::claim`); a claim that took
                // the editing says what of its bookkeeping after didn't happen (`note`).
                let live = live.ok_or_else(|| bad(410, UNMOUNTED))?;
                let claimed = tokio::select! {
                    biased;
                    () = &mut cancelled => return Err(bad(503, CANCELLED_CHANGE)),
                    r = server.claim(view.as_deref(), Some(&live)) => r.map_err(api_refusal)?,
                };
                text(json!({"binding": binding, "editing": true, "note": claimed.note}))
            }
            DRAFT => {
                // Issue #23's handover, through this mount's own view and lifetime, admitted
                // as a save is (see `Server::answer`, `Server::apply_offer`).
                let live = live.ok_or_else(|| bad(410, UNMOUNTED))?;
                let view = view.as_deref().unwrap_or_default();
                let id = get("id").unwrap_or_default();
                let action = get("action").unwrap_or_default();
                let offer = if action == "answer" {
                    panel_offer(map).map_err(api_refusal)?
                } else {
                    None
                };
                // `resolved` comes as a JSON list in text, as `unfinished` does.
                let listed = match map.get("resolved").and_then(Value::as_str) {
                    None => None,
                    Some(t) => Some(serde_json::from_str::<Value>(t).map_err(|_| {
                        bad(400, "`resolved` is a JSON list of ids; nothing was changed")
                    })?),
                };
                let resolved = crate::resolved_of(listed.as_ref()).map_err(api_refusal)?;
                let done = async {
                    match action {
                        "answer" => server
                            .answer(view, Some(&live), offer)
                            .await
                            .map(|id| json!({"binding": binding, "offer": id})),
                        "apply" => server
                            .apply_offer(Some(view), Some(&live), id, &resolved)
                            .await
                            .map(|a| {
                                json!({"binding": binding, "applied": true,
                                "revision": a.revision, "left": a.left})
                            }),
                        "keep" => server
                            .keep_offer(Some(view), Some(&live), id, &resolved)
                            .await
                            .map(|k| {
                                json!({"binding": binding,
                                "kept": {"id": k.id, "name": k.name}, "left": k.left})
                            }),
                        "dismiss" => server
                            .dismiss_offer(Some(view), Some(&live), id, &resolved)
                            .await
                            .map(|()| json!({"binding": binding, "dismissed": true})),
                        _ => server
                            .stop_waiting(Some(view), Some(&live), id)
                            .await
                            .map(|()| json!({"binding": binding, "stopped": true})),
                    }
                };
                if !["answer", "apply", "keep", "dismiss", "stop"].contains(&action) {
                    return Err(bad(
                        400,
                        "`action` is answer, apply, keep, dismiss or stop; nothing was read or \
                         changed",
                    ));
                }
                let out = tokio::select! {
                    biased;
                    () = &mut cancelled => return Err(bad(503, CANCELLED_CHANGE)),
                    r = done => r.map_err(api_refusal)?,
                };
                text(out)
            }
            CANVAS_SAVE => {
                let if_match = get("if_match").unwrap_or_default();
                let body = get("text").unwrap_or_default();
                let live = live.ok_or_else(|| bad(410, UNMOUNTED))?;
                // As for a page save: cancellation first on every poll; once the save holds the
                // fence it runs to its commit or rollback within that poll.
                let revision = tokio::select! {
                    biased;
                    () = &mut cancelled => return Err(bad(503, CANCELLED)),
                    r = server.save_canvas_while(&live, view.as_deref().unwrap_or_default(), if_match, body) => {
                        r.map_err(api_refusal)?
                    }
                };
                text(json!({"binding": binding, "saved": true, "revision": revision}))
            }
            REVISION => {
                let page_id = page_id.unwrap_or_default();
                let revision = server.page_revision(page_id).map_err(api_refusal)?;
                text(json!({"binding": binding, "page_id": page_id, "revision": revision}))
            }
            SAVE => {
                let page_id = page_id.unwrap_or_default();
                let if_match = get("if_match").unwrap_or_default();
                let body = get("text").unwrap_or_default();
                let live = live.ok_or_else(|| bad(410, UNMOUNTED))?;
                // Cancellation is checked first on every poll, so a cancelled save never takes
                // even a free writer fence. Once the save holds the fence it runs to its commit
                // or rollback within that same poll, without waiting again
                // (`Server::save_page_while`), so it can't be stopped halfway.
                let revision = tokio::select! {
                    biased;
                    () = &mut cancelled => return Err(bad(503, CANCELLED)),
                    r = server.save_page_while(&live, view.as_deref().unwrap_or_default(), page_id, if_match, body) => {
                        r.map_err(api_refusal)?
                    }
                };
                text(json!({
                    "binding": binding,
                    "page_id": page_id,
                    "saved": true,
                    "revision": revision,
                }))
            }
            READY => {
                server.mark_ready().map_err(api_refusal)?;
                text(json!({"binding": binding, "ready": true}))
            }
            _ => Err(format!("unknown tool {name}")),
        }
    }

    /// The open canvas's server for an in-panel call naming `binding` (and `mount`, after the
    /// first page read), compared under the state lock, with that mount's lifetime flag. The
    /// lock is released before the server is used: a save then waits for the binding's writer
    /// fence without it. Close holds the state lock while it drains that fence, so this order is
    /// never reversed.
    async fn panel_server(&self, binding: &str, mount: Option<u64>) -> Result<Admitted, String> {
        let state = self.state.clone().lock_owned().await;
        refuse_if_shut_down(&state)?;
        let state = Self::fence_library(state).await?;
        expect_binding(&state, Some(binding))?;
        let b = state.bound.as_ref().ok_or(NO_CANVAS)?;
        let live = match mount {
            Some(m) => Some(b.mounts.get(&m).cloned().ok_or(UNMOUNTED)?),
            None => None,
        };
        Ok((b.server.clone(), live))
    }

    /// The mount of an in-panel read: the one it names, or for a first read (none) a new one,
    /// registered only while `binding` is still the open one. A new mount is the newest view of
    /// the canvas, so it takes the editing (issue #23; see `Server::claim`): every earlier mount
    /// and tab of this binding, and every other process's view, is read-only from then on. If
    /// it can't be taken now (the canvas busy), the mount opens read-only and says so.
    async fn mount_for(
        &self,
        binding: &str,
        mount: Option<u64>,
        server: &Server,
    ) -> Result<u64, String> {
        if let Some(m) = mount {
            return Ok(m);
        }
        let m = {
            let mut state = self.state.lock().await;
            expect_binding(&state, Some(binding)).map_err(|e| panel_refusal(410, &e, None))?;
            let b = state
                .bound
                .as_mut()
                .ok_or_else(|| panel_refusal(410, NO_CANVAS, None))?;
            b.next_mount += 1;
            let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
            b.mounts.insert(b.next_mount, live.clone());
            (b.next_mount, live)
        };
        let (m, live) = m;
        // Without the state lock, as every use of the writer fence by a panel call; never for
        // a mount released meanwhile.
        let _ = server.claim(Some(&format!("m{m}")), Some(&live)).await;
        Ok(m)
    }
}
