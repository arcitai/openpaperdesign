//! Agent edits over MCP against a real loopback listener (rmcp streamable HTTP client), plus
//! browser-style saves through the same server.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use openpaperdesign_serve::{Config, MCP_PATH, Server, etag, router};
use paper_rs::editor::shortcuts::Action;
use paper_rs::editor::{Button, Editor, InputEvent, Tool};
use paper_rs::geom::v2;
use paper_rs::project::format::{page_from_json, page_to_json};
use paper_rs::project::{Project, Store};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt as _;

const TOKEN: &str = "agent-test-token-0123456789abcdef";

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp() -> Tmp {
    use std::hash::{BuildHasher, RandomState};
    let d = std::env::temp_dir().join(format!(
        "opd-agent-{:016x}",
        RandomState::new().hash_one(std::time::SystemTime::now())
    ));
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

/// A project with one rectangle drawn through the upstream editor.
fn project(root: &Path, name: &str) -> (Project, String) {
    let store = Store::open(root).unwrap();
    let (p, page) = store.create(name).unwrap();
    let mut ed = Editor::default();
    ed.open_pages(vec![page], None);
    ed.run(Action::Tool(Tool::Rect));
    for ev in [
        InputEvent::PointerDown {
            pos: v2(100.0, 100.0),
            button: Button::Primary,
            pressure: None,
            time_ms: 0.0,
        },
        InputEvent::PointerMove {
            pos: v2(220.0, 180.0),
            pressure: None,
            time_ms: 16.0,
        },
        InputEvent::PointerUp {
            pos: v2(220.0, 180.0),
            button: Button::Primary,
            time_ms: 32.0,
        },
    ] {
        ed.handle(ev);
    }
    p.save_page(&ed.page_data(0)).unwrap();
    let id = p.manifest.pages[0].id.clone();
    (p, id)
}

struct Running {
    s: Arc<Server>,
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(dir: &Path) -> Running {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let s = Server::open(Config {
        project: dir.to_path_buf(),
        web_dir: dir.parent().unwrap().join("no-bundle"),
        port,
        token: TOKEN.into(),
    })
    .unwrap();
    let app = router(s.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Running { s, port, task }
}

async fn client(port: u16) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let cfg =
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}{MCP_PATH}"))
            .auth_header(TOKEN);
    ().serve(StreamableHttpClientTransport::from_config(cfg))
        .await
        .unwrap()
}

fn params(name: &str, args: Value) -> CallToolRequestParams {
    let mut p = CallToolRequestParams::new(name.to_string());
    p.arguments = args.as_object().cloned();
    p
}

fn text(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn call(
    c: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: Value,
) -> (bool, String) {
    let r = c.call_tool(params(name, args)).await.unwrap();
    (r.is_error != Some(true), text(&r))
}

/// What an agent does before it changes a page: read it in its own session (`get_basic_info`
/// returns the page the user is viewing and the page list).
async fn read(c: &rmcp::service::RunningService<rmcp::RoleClient, ()>) {
    let (ok, out) = call(c, "get_basic_info", json!({})).await;
    assert!(ok, "{out}");
}

/// Read one page (its whole tree) in the session.
async fn read_page(c: &rmcp::service::RunningService<rmcp::RoleClient, ()>, page: &str) {
    let (ok, out) = call(
        c,
        "get_tree_summary",
        json!({"nodeId": "root", "pageId": page}),
    )
    .await;
    assert!(ok, "{out}");
}

fn file(p: &Project, id: &str) -> Vec<u8> {
    std::fs::read(p.page_path(id)).unwrap()
}

fn doc(p: &Project, id: &str) -> paper_rs::model::Document {
    page_from_json(&String::from_utf8(file(p, id)).unwrap())
        .unwrap()
        .doc
}

/// Browser-style request through the router (same checks as the socket).
async fn http(
    r: &Running,
    method: &str,
    path: &str,
    if_match: Option<&str>,
    body: String,
) -> (StatusCode, Option<String>, axum::http::HeaderMap, String) {
    use http_body_util::BodyExt;
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, format!("127.0.0.1:{}", r.port))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    if let Some(m) = if_match {
        b = b.header(header::IF_MATCH, m);
    }
    let res = router(r.s.clone())
        .oneshot(b.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let st = res.status();
    let tag = res
        .headers()
        .get(header::ETAG)
        .map(|v| v.to_str().unwrap().to_string());
    let h = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (st, tag, h, String::from_utf8_lossy(&body).into_owned())
}

/// The id of the only node that `before` doesn't have.
fn new_node(before: &paper_rs::model::Document, after: &paper_rs::model::Document) -> u64 {
    let ids: Vec<u64> = after
        .nodes
        .keys()
        .filter(|k| !before.nodes.contains_key(k))
        .map(|k| k.0)
        .collect();
    assert_eq!(ids.len(), 1, "{ids:?}");
    ids[0]
}

fn key(page: &str, n: u64) -> String {
    format!("{}:{n}", &page[..page.len().min(6)])
}

fn moved(page_json: &str, node: u64, dx: f64) -> String {
    let mut page = page_from_json(page_json).unwrap();
    page.doc
        .nodes
        .get_mut(&paper_rs::model::NodeId(node))
        .unwrap()
        .rect
        .x += dx;
    page_to_json(&page)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_needs_the_token_and_a_loopback_host_and_origin() {
    let t = tmp();
    let (p, _) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "t", "version": "1"}}})
    .to_string();
    let send = |host: String, token: Option<&str>, origin: Option<&str>| {
        let mut b = Request::builder()
            .method("POST")
            .uri(MCP_PATH)
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream");
        if let Some(t) = token {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        let req = b.body(Body::from(init.clone())).unwrap();
        let app = router(r.s.clone());
        async move { app.oneshot(req).await.unwrap().status() }
    };
    let host = format!("127.0.0.1:{}", r.port);
    assert_eq!(
        send(host.clone(), None, None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(host.clone(), Some("wrong"), None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send("evil.example".into(), Some(TOKEN), None).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(host.clone(), Some(TOKEN), Some("https://evil.example")).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(send(host, Some(TOKEN), None).await, StatusCode::OK);

    // And over the socket, a client without the token can't connect.
    let cfg = StreamableHttpClientTransportConfig::with_uri(format!(
        "http://127.0.0.1:{}{MCP_PATH}",
        r.port
    ));
    assert!(
        ().serve(StreamableHttpClientTransport::from_config(cfg))
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_edits_land_on_disk_and_change_the_revision() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    let info = c.peer_info().unwrap();
    assert_eq!(info.server_info.as_ref().unwrap().name, "openpaperdesign");
    let tools = c.list_all_tools().await.unwrap();
    for refused in ["export", "export_combined_pdf", "create_file"] {
        assert!(
            !tools.iter().any(|t| t.name == refused),
            "{refused} offered"
        );
    }
    let before = doc(&p, &page);
    let rev0 = etag(&file(&p, &page));
    // Review round 2, finding 2: a session must read a page before it changes it, even on
    // its first call.
    let disk = file(&p, &page);
    let manifest = std::fs::read(p.dir.join("project.json")).unwrap();
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "ellipse", "x": 400, "y": 50, "width": 80, "height": 60}),
    )
    .await;
    assert!(!ok && out.contains("hasn't been read"), "{out}");
    assert_eq!(file(&p, &page), disk);
    assert_eq!(std::fs::read(p.dir.join("project.json")).unwrap(), manifest);
    read(&c).await;
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "ellipse", "x": 400, "y": 50, "width": 80, "height": 60}),
    )
    .await;
    assert!(ok, "{out}");
    let after = doc(&p, &page);
    let added = new_node(&before, &after);
    let rev1 = etag(&file(&p, &page));
    assert_ne!(rev0, rev1, "the agent's write changed the page revision");
    let (st, tag, _, _) = http(
        &r,
        "GET",
        &format!("/api/pages/{page}/revision"),
        None,
        String::new(),
    )
    .await;
    assert_eq!((st, tag.as_deref()), (StatusCode::OK, Some(rev1.as_str())));

    // update_styles moves it; delete_nodes removes it.
    let (ok, out) = call(
        &c,
        "update_styles",
        json!({"updates": [{"nodeIds": [key(&page, added)], "styles": {"left": "500px"}}]}),
    )
    .await;
    assert!(ok, "{out}");
    let x = doc(&p, &page).nodes[&paper_rs::model::NodeId(added)].rect.x;
    assert_eq!(x, 500.0, "{out}");
    let (ok, out) = call(&c, "delete_nodes", json!({"nodeIds": [key(&page, added)]})).await;
    assert!(ok, "{out}");
    assert!(
        !doc(&p, &page)
            .nodes
            .contains_key(&paper_rs::model::NodeId(added))
    );
    c.cancel().await.unwrap();
}

/// GET the page as the browser does: (revision, body).
async fn browser_get(r: &Running, page: &str) -> (String, String) {
    let (st, tag, _, body) =
        http(r, "GET", &format!("/api/pages/{page}"), None, String::new()).await;
    assert_eq!(st, StatusCode::OK);
    (tag.unwrap(), body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sequential_human_and_agent_edits_are_both_kept() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    let rect = *doc(&p, &page).nodes.keys().next().unwrap();
    let x0 = doc(&p, &page).nodes[&rect].rect.x;

    // Human, then agent: the browser saves on the current revision, the agent then edits.
    let (tag, body) = browser_get(&r, &page).await;
    let (st, _, _, out) = http(
        &r,
        "PUT",
        &format!("/api/pages/{page}"),
        Some(&tag),
        moved(&body, rect.0, 33.0),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{out}");
    let before = doc(&p, &page);
    read(&c).await;
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "rectangle", "x": 600, "y": 0, "width": 50, "height": 50}),
    )
    .await;
    assert!(ok, "{out}");
    let agent_node = new_node(&before, &doc(&p, &page));
    let d = doc(&p, &page);
    assert_eq!(
        d.nodes[&rect].rect.x,
        x0 + 33.0,
        "the human's move survived the agent's write"
    );
    assert!(d.nodes.contains_key(&paper_rs::model::NodeId(agent_node)));

    // Agent, then human: the browser picks up the new revision (as its poll would) and saves.
    let (tag, body) = browser_get(&r, &page).await;
    assert_eq!(tag, etag(&file(&p, &page)));
    let (st, _, _, out) = http(
        &r,
        "PUT",
        &format!("/api/pages/{page}"),
        Some(&tag),
        moved(&body, rect.0, 7.0),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{out}");
    let d = doc(&p, &page);
    assert_eq!(d.nodes[&rect].rect.x, x0 + 40.0);
    assert!(
        d.nodes.contains_key(&paper_rs::model::NodeId(agent_node)),
        "the agent's shape survived the human's save"
    );
    // And the agent sees the human's last save on its next call (it reloads from disk).
    let (ok, out) = call(&c, "get_node_info", json!({"nodeId": key(&page, rect.0)})).await;
    assert!(ok && out.contains(&format!("{}", x0 + 40.0)), "{out}");
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_browser_save_after_an_agent_write_is_refused_and_writes_nothing() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    // The browser loaded revision 1 and then has an unsaved local edit.
    let (base_tag, base) = browser_get(&r, &page).await;
    let rect = doc(&p, &page).nodes.keys().next().unwrap().0;
    read(&c).await;

    for (what, agent) in [
        (
            "a different layer",
            json!(["create_shape", {"type": "rectangle", "x": 600, "y": 0, "width": 50, "height": 50}]),
        ),
        (
            "the same layer",
            json!(["update_styles", {"updates": [{"nodeIds": [key(&page, rect)], "styles": {"left": "700px"}}]}]),
        ),
        (
            "deleting the layer",
            json!(["delete_nodes", {"nodeIds": [key(&page, rect)]}]),
        ),
    ] {
        // The agent writes (a new revision).
        let (ok, out) = call(&c, agent[0].as_str().unwrap(), agent[1].clone()).await;
        assert!(ok, "{what}: {out}");
        let agent_version = file(&p, &page);
        // The browser's save is based on an older revision: 409, nothing written.
        let (st, current, _, body) = http(
            &r,
            "PUT",
            &format!("/api/pages/{page}"),
            Some(&base_tag),
            moved(&base, rect, 5.0),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT, "{what}: {body}");
        assert_eq!(
            current.unwrap(),
            etag(&agent_version),
            "{what}: the answer names the current revision"
        );
        assert_eq!(
            file(&p, &page),
            agent_version,
            "{what}: disk keeps the agent's version"
        );
    }
    // An unknown revision is refused the same way.
    let disk = file(&p, &page);
    let (st, _, _, _) = http(
        &r,
        "PUT",
        &format!("/api/pages/{page}"),
        Some("\"0000000000000000\""),
        moved(&base, rect, 5.0),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(file(&p, &page), disk);
    // After an explicit discard the browser loads the agent's version and can save again.
    let (tag, body) = browser_get(&r, &page).await;
    assert_eq!(body.as_bytes(), disk);
    let (st, _, _, out) = http(&r, "PUT", &format!("/api/pages/{page}"), Some(&tag), body).await;
    assert_eq!(st, StatusCode::OK, "{out}");
    c.cancel().await.unwrap();
}

/// Review round 1, finding 4: an agent that read a page, then a browser save, then an agent
/// write based on what it read: the write is refused and the browser's save stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_agent_write_after_a_browser_save_is_refused_and_writes_nothing() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    let other = client(r.port).await;
    let rect = doc(&p, &page).nodes.keys().next().unwrap().0;
    let x0 = doc(&p, &page).nodes[&paper_rs::model::NodeId(rect)].rect.x;

    // Both agents read the page.
    for a in [&c, &other] {
        let (ok, out) = call(a, "get_node_info", json!({"nodeId": key(&page, rect)})).await;
        assert!(ok, "{out}");
    }
    // The browser saves on the current revision.
    let (tag, body) = browser_get(&r, &page).await;
    let (st, _, _, out) = http(
        &r,
        "PUT",
        &format!("/api/pages/{page}"),
        Some(&tag),
        moved(&body, rect, 90.0),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{out}");
    let browser_version = file(&p, &page);
    let manifest = std::fs::read(p.dir.join("project.json")).unwrap();

    // Each agent's write was decided before that save: refused, nothing written.
    for (a, args) in [
        (
            &c,
            json!({"updates": [{"nodeIds": [key(&page, rect)], "styles": {"left": "200px"}}]}),
        ),
        (
            &other,
            json!({"updates": [{"nodeIds": [key(&page, rect)], "styles": {"width": "10px"}}]}),
        ),
    ] {
        let (ok, out) = call(a, "update_styles", args).await;
        assert!(!ok && out.contains("conflict"), "{out}");
        assert_eq!(
            file(&p, &page),
            browser_version,
            "disk keeps the browser's save"
        );
        assert_eq!(std::fs::read(p.dir.join("project.json")).unwrap(), manifest);
    }
    // Retrying without reading is refused again.
    let (ok, out) = call(&c, "delete_nodes", json!({"nodeIds": [key(&page, rect)]})).await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(file(&p, &page), browser_version);

    // After reading again, the agent's write goes through on top of the browser's save.
    let (ok, out) = call(&c, "get_basic_info", json!({})).await;
    assert!(ok, "{out}");
    let (ok, out) = call(
        &c,
        "update_styles",
        json!({"updates": [{"nodeIds": [key(&page, rect)], "styles": {"width": "10px"}}]}),
    )
    .await;
    assert!(ok, "{out}");
    let n = &doc(&p, &page).nodes[&paper_rs::model::NodeId(rect)];
    assert_eq!(n.rect.x, x0 + 90.0, "the browser's move is kept");
    assert_eq!(n.rect.w, 10.0, "the agent's change landed");
    // Its own write doesn't make its next write stale; the other agent's still is.
    let (ok, out) = call(
        &c,
        "update_styles",
        json!({"updates": [{"nodeIds": [key(&page, rect)], "styles": {"height": "12px"}}]}),
    )
    .await;
    assert!(ok, "{out}");
    let (ok, out) = call(
        &other,
        "update_styles",
        json!({"updates": [{"nodeIds": [key(&page, rect)], "styles": {"height": "99px"}}]}),
    )
    .await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(
        doc(&p, &page).nodes[&paper_rs::model::NodeId(rect)].rect.h,
        12.0
    );
    c.cancel().await.unwrap();
    other.cancel().await.unwrap();
}

fn manifest(p: &Project) -> Value {
    serde_json::from_slice(&std::fs::read(p.dir.join("project.json")).unwrap()).unwrap()
}

fn page_names(p: &Project) -> Vec<String> {
    manifest(p)["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect()
}

/// Review round 2, finding 2: a session whose reads were forgotten (more sessions than the
/// host remembers) must read again; its stale write is refused, never let through.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forgotten_session_must_read_again() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    let rect = doc(&p, &page).nodes.keys().next().unwrap().0;
    let x0 = doc(&p, &page).nodes[&paper_rs::model::NodeId(rect)].rect.x;
    read(&c).await;
    let (tag, body) = browser_get(&r, &page).await;
    let (st, _, _, out) = http(
        &r,
        "PUT",
        &format!("/api/pages/{page}"),
        Some(&tag),
        moved(&body, rect, 90.0),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{out}");
    let browser_version = file(&p, &page);
    // More sessions than the host remembers each read once.
    for _ in 0..257 {
        let o = client(r.port).await;
        read(&o).await;
        o.cancel().await.unwrap();
    }
    let (ok, out) = call(
        &c,
        "update_styles",
        json!({"updates": [{"nodeIds": [key(&page, rect)], "styles": {"left": "200px"}}]}),
    )
    .await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(file(&p, &page), browser_version, "the browser's save stays");
    // After reading again the write lands on top of the browser's save.
    read(&c).await;
    let (ok, out) = call(
        &c,
        "update_styles",
        json!({"updates": [{"nodeIds": [key(&page, rect)], "styles": {"width": "10px"}}]}),
    )
    .await;
    assert!(ok, "{out}");
    let n = &doc(&p, &page).nodes[&paper_rs::model::NodeId(rect)];
    assert_eq!((n.rect.x, n.rect.w), (x0 + 90.0, 10.0));
    c.cancel().await.unwrap();
}

/// Review round 2, finding 3: a host reload (here after a browser save to another page)
/// keeps the page names saved in the manifest; a later write doesn't revert a rename.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reloads_keep_saved_page_names() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    read(&c).await;
    let (ok, out) = call(
        &c,
        "rename_pages",
        json!({"updates": [{"pageId": page, "name": "Saved new name"}]}),
    )
    .await;
    assert!(ok, "{out}");
    let (ok, out) = call(&c, "create_page", json!({"name": "Second"})).await;
    assert!(ok, "{out}");
    let second = manifest(&p)["pages"][1]["id"].as_str().unwrap().to_string();
    assert_eq!(page_names(&p), ["Saved new name", "Second"]);
    // A browser save to the second page makes the host reload everything.
    let (tag, body) = browser_get(&r, &second).await;
    let mut data = page_from_json(&body).unwrap();
    data.camera = None;
    data.doc.next_id += 1;
    let (st, _, _, out) = http(
        &r,
        "PUT",
        &format!("/api/pages/{second}"),
        Some(&tag),
        page_to_json(&data),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{out}");
    // An unrelated agent edit on the second page, after reading that page.
    read_page(&c, &second).await;
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "rectangle", "x": 0, "y": 0, "width": 5, "height": 5, "pageId": second}),
    )
    .await;
    assert!(ok, "{out}");
    assert_eq!(
        page_names(&p),
        ["Saved new name", "Second"],
        "the rename is kept"
    );
    let first = page_from_json(&String::from_utf8(file(&p, &page)).unwrap()).unwrap();
    assert_ne!(
        first.name, "Saved new name",
        "renames only write the manifest"
    );
    c.cancel().await.unwrap();
}

/// Review round 2, finding 4: page renames (manifest metadata) obey the same conflict rule.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_page_renames_are_refused() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let a = client(r.port).await;
    let b = client(r.port).await;
    read(&a).await;
    let (ok, out) = call(&a, "create_page", json!({"name": "Two"})).await;
    assert!(ok, "{out}");
    let two = manifest(&p)["pages"][1]["id"].as_str().unwrap().to_string();
    read(&a).await;
    read(&b).await;
    // A renames page one; B, which read before that, renames it too: refused.
    let (ok, out) = call(
        &a,
        "rename_pages",
        json!({"updates": [{"pageId": page, "name": "First saved name"}]}),
    )
    .await;
    assert!(ok, "{out}");
    let m = std::fs::read(p.dir.join("project.json")).unwrap();
    let (ok, out) = call(
        &b,
        "rename_pages",
        json!({"updates": [{"pageId": page, "name": "Stale second name"}]}),
    )
    .await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(std::fs::read(p.dir.join("project.json")).unwrap(), m);
    // A batch with one stale entry writes nothing either.
    let (ok, out) = call(
        &b,
        "rename_pages",
        json!({"updates": [{"pageId": two, "name": "B two"}, {"pageId": page, "name": "B one"}]}),
    )
    .await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(std::fs::read(p.dir.join("project.json")).unwrap(), m);
    assert_eq!(page_names(&p), ["First saved name", "Two"]);
    // After reading, B's rename goes through.
    read(&b).await;
    let (ok, out) = call(
        &b,
        "rename_pages",
        json!({"updates": [{"pageId": page, "name": "B one"}]}),
    )
    .await;
    assert!(ok, "{out}");
    assert_eq!(page_names(&p), ["B one", "Two"]);
    a.cancel().await.unwrap();
    b.cancel().await.unwrap();
}

/// A browser-style save of `page` on its current revision (changes its bytes only).
async fn browser_save(r: &Running, page: &str) {
    let (tag, body) = browser_get(r, page).await;
    let mut data = page_from_json(&body).unwrap();
    data.doc.next_id += 1;
    let (st, _, _, out) = http(
        r,
        "PUT",
        &format!("/api/pages/{page}"),
        Some(&tag),
        page_to_json(&data),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{out}");
}

/// A served project with two pages, A (the one the user views, with a rectangle) and B
/// (with a rectangle the setup session drew).
async fn two_pages(t: &Tmp) -> (Project, String, String, Running) {
    let (p, a) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let setup = client(r.port).await;
    read(&setup).await;
    let (ok, out) = call(&setup, "create_page", json!({"name": "B"})).await;
    assert!(ok, "{out}");
    let b = manifest(&p)["pages"][1]["id"].as_str().unwrap().to_string();
    let (ok, out) = call(&setup, "create_shape", shape_on(&b)).await;
    assert!(ok, "{out}");
    setup.cancel().await.unwrap();
    (p, a, b, r)
}

/// The id of the first top-level node of `page`, as a node key.
fn first_node(p: &Project, page: &str) -> String {
    key(page, doc(p, page).root[0].0)
}

/// Review round 3: a change to several pages needs every one of them read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_batch_write_needs_every_page_it_writes_read() {
    let t = tmp();
    let (p, a, b, r) = two_pages(&t).await;
    let c = client(r.port).await;
    read_page(&c, &a).await;
    let batch = json!({"updates": [
        {"nodeIds": [first_node(&p, &a)], "styles": {"width": "11px"}},
        {"nodeIds": [first_node(&p, &b)], "styles": {"width": "12px"}},
    ]});
    let before = (
        file(&p, &a),
        file(&p, &b),
        std::fs::read(p.dir.join("project.json")).unwrap(),
    );
    let (ok, out) = call(&c, "update_styles", batch.clone()).await;
    assert!(!ok && out.contains("hasn't been read"), "{out}");
    assert_eq!(
        (
            file(&p, &a),
            file(&p, &b),
            std::fs::read(p.dir.join("project.json")).unwrap()
        ),
        before,
        "neither page nor the manifest was written"
    );
    read_page(&c, &b).await;
    let (ok, out) = call(&c, "update_styles", batch).await;
    assert!(ok, "{out}");
    c.cancel().await.unwrap();
}

fn shape_on(page: &str) -> Value {
    json!({"type": "rectangle", "x": 0, "y": 0, "width": 5, "height": 5, "pageId": page})
}

/// Review round 3: reading one page acknowledges only that page.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reading_page_a_does_not_acknowledge_page_b() {
    let t = tmp();
    let (p, a, b, r) = two_pages(&t).await;
    let c = client(r.port).await;
    // B read, then saved by the browser; the agent then reads only A and writes B: refused.
    read_page(&c, &b).await;
    browser_save(&r, &b).await;
    read_page(&c, &a).await;
    let disk = file(&p, &b);
    let (ok, out) = call(&c, "create_shape", shape_on(&b)).await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(file(&p, &b), disk);
    // A page never read in this session isn't writable after reading another one either.
    let c2 = client(r.port).await;
    read_page(&c2, &a).await;
    let (ok, out) = call(&c2, "create_shape", shape_on(&b)).await;
    assert!(!ok && out.contains("hasn't been read"), "{out}");
    // A successful read of A allows writing A.
    let (ok, out) = call(&c2, "create_shape", shape_on(&a)).await;
    assert!(ok, "{out}");
    assert_eq!(file(&p, &b), disk);
    c.cancel().await.unwrap();
    c2.cancel().await.unwrap();
}

/// Review round 3: a failed call acknowledges nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_read_acknowledges_nothing() {
    let t = tmp();
    let (p, a, _, r) = two_pages(&t).await;
    let c = client(r.port).await;
    read_page(&c, &a).await;
    browser_save(&r, &a).await;
    // Reads that fail: a node that doesn't exist, a page that doesn't exist.
    for (tool, args) in [
        ("get_node_info", json!({"nodeId": key(&a, 999_999)})),
        (
            "get_tree_summary",
            json!({"nodeId": "root", "pageId": "no-such-page"}),
        ),
    ] {
        let (ok, out) = call(&c, tool, args).await;
        assert!(!ok, "{tool}: {out}");
    }
    let disk = file(&p, &a);
    let (ok, out) = call(&c, "create_shape", shape_on(&a)).await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(file(&p, &a), disk);
    // A fresh session whose only call failed hasn't read anything.
    let c2 = client(r.port).await;
    let (ok, _) = call(&c2, "get_node_info", json!({"nodeId": key(&a, 999_999)})).await;
    assert!(!ok);
    let (ok, out) = call(&c2, "create_shape", shape_on(&a)).await;
    assert!(!ok && out.contains("hasn't been read"), "{out}");
    assert_eq!(file(&p, &a), disk);
    c.cancel().await.unwrap();
    c2.cancel().await.unwrap();
}

/// Review round 3: a session's own write keeps that page writable, and only that page.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn own_writes_acknowledge_only_the_pages_written() {
    let t = tmp();
    let (p, a, b, r) = two_pages(&t).await;
    let c = client(r.port).await;
    read_page(&c, &a).await;
    for _ in 0..2 {
        // The second write needs no re-read: the session wrote that revision itself.
        let (ok, out) = call(&c, "create_shape", shape_on(&a)).await;
        assert!(ok, "{out}");
    }
    let disk = file(&p, &b);
    let (ok, out) = call(&c, "create_shape", shape_on(&b)).await;
    assert!(!ok && out.contains("hasn't been read"), "{out}");
    assert_eq!(file(&p, &b), disk);
    // Someone else's save of A still makes the session's next write to A stale.
    browser_save(&r, &a).await;
    let (ok, out) = call(&c, "create_shape", shape_on(&a)).await;
    assert!(!ok && out.contains("conflict"), "{out}");
    c.cancel().await.unwrap();
}

/// Review round 3: calls that can't be attributed to a page acknowledge nothing, and a
/// page read doesn't acknowledge the page list (names, order), which renames change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unattributable_reads_acknowledge_nothing() {
    let t = tmp();
    let (p, a, _, r) = two_pages(&t).await;
    let c = client(r.port).await;
    // Successful calls that don't return a page's state.
    for (tool, args) in [
        ("find_nodes", json!({"textValue": "x"})),
        ("list_resources", json!({})),
        ("get_guide", json!({"topic": "html"})),
        ("get_font_family_info", json!({"familyNames": ["Inter"]})),
        ("rename_pages", json!({"updates": []})),
        ("finish_working_on_nodes", json!({})),
        ("select_nodes", json!({"nodeIds": [first_node(&p, &a)]})),
    ] {
        let (ok, out) = call(&c, tool, args).await;
        assert!(ok, "{tool}: {out}");
    }
    // And failed ones: an unknown tool, a mutation with missing arguments, a bad guide.
    for (tool, args) in [
        ("no_such_tool", json!({})),
        ("update_styles", json!({})),
        ("get_guide", json!({"topic": "no-such-topic"})),
    ] {
        let (ok, out) = call(&c, tool, args).await;
        assert!(!ok, "{tool}: {out}");
    }
    let disk = file(&p, &a);
    let (ok, out) = call(&c, "create_shape", shape_on(&a)).await;
    assert!(!ok && out.contains("hasn't been read"), "{out}");
    assert_eq!(file(&p, &a), disk);
    // A page read allows content changes, but not a rename (the page list is separate).
    read_page(&c, &a).await;
    let m = std::fs::read(p.dir.join("project.json")).unwrap();
    let (ok, out) = call(
        &c,
        "rename_pages",
        json!({"updates": [{"pageId": a, "name": "Renamed"}]}),
    )
    .await;
    assert!(!ok && out.contains("page list"), "{out}");
    assert_eq!(std::fs::read(p.dir.join("project.json")).unwrap(), m);
    // get_basic_info returns the page list, so then the rename goes through.
    read(&c).await;
    let (ok, out) = call(
        &c,
        "rename_pages",
        json!({"updates": [{"pageId": a, "name": "Renamed"}]}),
    )
    .await;
    assert!(ok, "{out}");
    assert_eq!(page_names(&p), ["Renamed", "B"]);
    c.cancel().await.unwrap();
}

fn node_id(out: &str) -> String {
    serde_json::from_str::<Value>(out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Review round 4: a call that returns an error changes nothing, in memory or on disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_call_changes_nothing() {
    let t = tmp();
    let (p, a, b, r) = two_pages(&t).await;
    let c = client(r.port).await;
    read(&c).await;
    read_page(&c, &a).await;
    read_page(&c, &b).await;
    // Page A: the rectangle, a text layer and a frame.
    let rect_a = first_node(&p, &a);
    let rect_b = first_node(&p, &b);
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "text", "x": 0, "y": 300, "width": 80, "height": 20, "text": "Original", "pageId": a}),
    )
    .await;
    assert!(ok, "{out}");
    let text_a = node_id(&out);
    let (ok, out) = call(
        &c,
        "create_artboard",
        json!({"name": "Frame", "pageId": a, "styles": {"left": "500px", "top": "0px", "width": "100px", "height": "100px"}}),
    )
    .await;
    assert!(ok, "{out}");
    let frame_a = node_id(&out);

    let cases = [
        // The first rename is valid, the second page doesn't exist.
        (
            "rename_pages",
            json!({"updates": [{"pageId": a, "name": "Partial rename"}, {"pageId": "missing-page", "name": "Never"}]}),
        ),
        // The first move is valid, the second crosses pages (refused upstream).
        (
            "move_nodes",
            json!({"moves": [{"nodeId": rect_a, "parentId": frame_a}, {"nodeId": rect_b, "parentId": frame_a}]}),
        ),
        // The first text change is valid, the second targets a rectangle.
        (
            "set_text_content",
            json!({"updates": [{"nodeId": text_a, "textContent": "PARTIAL REPLACEMENT"}, {"nodeId": rect_b, "textContent": "x"}]}),
        ),
        // Replacing the rectangle with an SVG that has nothing visible fails after the
        // target was removed.
        (
            "write_html",
            json!({"targetNodeId": rect_a, "mode": "replace", "html": "<svg width='0' height='0'><rect width='10' height='10'/></svg>"}),
        ),
    ];
    for (tool, args) in cases {
        let disk = (
            file(&p, &a),
            file(&p, &b),
            std::fs::read(p.dir.join("project.json")).unwrap(),
        );
        let (ok, out) = call(&c, tool, args).await;
        assert!(!ok, "{tool} must fail: {out}");
        assert_eq!(
            (
                file(&p, &a),
                file(&p, &b),
                std::fs::read(p.dir.join("project.json")).unwrap()
            ),
            disk,
            "{tool}: nothing written"
        );
        // And nothing changed in memory: the next calls see the state before the failed one.
        let (ok, out) = call(&c, "get_basic_info", json!({"pageId": a})).await;
        assert!(ok && !out.contains("Partial rename"), "{tool}: {out}");
        let (ok, out) = call(&c, "get_node_info", json!({"nodeId": rect_a})).await;
        assert!(ok, "{tool}: the rectangle is still there: {out}");
        let (ok, out) = call(&c, "get_node_info", json!({"nodeId": text_a})).await;
        assert!(ok && out.contains("Original"), "{tool}: {out}");
        // A later successful change saves only itself, not the failed call's leftovers.
        let (ok, out) = call(
            &c,
            "update_styles",
            json!({"updates": [{"nodeIds": [frame_a], "styles": {"opacity": "0.9"}}]}),
        )
        .await;
        assert!(ok, "{tool}: {out}");
        let d = doc(&p, &a);
        let rect =
            &d.nodes[&paper_rs::model::NodeId(rect_a.split(':').nth(1).unwrap().parse().unwrap())];
        assert_eq!(rect.parent, None, "{tool}: the rectangle wasn't moved");
        assert_eq!(page_names(&p), ["Page 1", "B"], "{tool}");
    }
    c.cancel().await.unwrap();
}

/// Another 1×1 PNG (not `PNG_B64`, which a test keeps unregistered as an "outside" image).
const OTHER_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

/// Review round 4, F1 (the reviewer's I/O failure): a save that fails part-way leaves every
/// file as it was. Page A changes and page B needs a new image, but `assets/` can't be
/// written: neither page nor the manifest may change.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_save_failing_part_way_writes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let t = tmp();
    let (p, a, b, r) = two_pages(&t).await;
    let c = client(r.port).await;
    read(&c).await;
    read_page(&c, &a).await;
    read_page(&c, &b).await;
    let assets = p.dir.join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::set_permissions(&assets, std::fs::Permissions::from_mode(0o555)).unwrap();
    let before = (
        file(&p, &a),
        file(&p, &b),
        std::fs::read(p.dir.join("project.json")).unwrap(),
    );
    let (ok, out) = call(
        &c,
        "update_styles",
        json!({"updates": [
            {"nodeIds": [first_node(&p, &a)], "styles": {"width": "33px"}},
            {"nodeIds": [first_node(&p, &b)], "styles": {"backgroundImage": format!("url(data:image/png;base64,{OTHER_PNG_B64})")}},
        ]}),
    )
    .await;
    std::fs::set_permissions(&assets, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!ok, "{out}");
    assert_eq!(
        (
            file(&p, &a),
            file(&p, &b),
            std::fs::read(p.dir.join("project.json")).unwrap()
        ),
        before,
        "nothing written: {out}"
    );
    // Once the folder is writable again, the same change goes through in full.
    let (ok, out) = call(
        &c,
        "update_styles",
        json!({"updates": [
            {"nodeIds": [first_node(&p, &a)], "styles": {"width": "33px"}},
            {"nodeIds": [first_node(&p, &b)], "styles": {"backgroundImage": format!("url(data:image/png;base64,{OTHER_PNG_B64})")}},
        ]}),
    )
    .await;
    assert!(ok, "{out}");
    assert_ne!(file(&p, &a), before.0);
    assert_ne!(file(&p, &b), before.1);
    c.cancel().await.unwrap();
}

/// Review round 5, F2: a failed call leaves the editor as it was, selection and undo history
/// included (a failed read used to reload the project and lose both).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_call_keeps_selection_and_undo_history() {
    let t = tmp();
    let (p, a, b, r) = two_pages(&t).await;
    let c = client(r.port).await;
    read(&c).await;
    read_page(&c, &a).await;
    read_page(&c, &b).await;
    let rect = first_node(&p, &a);
    let x = || {
        doc(&p, &a)
            .nodes
            .values()
            .find(|n| n.parent.is_none())
            .unwrap()
            .rect
            .x
    };
    let x0 = x();
    let (ok, out) = call(
        &c,
        "update_styles",
        json!({"updates": [{"nodeIds": [rect], "styles": {"left": "111px"}}]}),
    )
    .await;
    assert!(ok, "{out}");
    let (ok, out) = call(&c, "select_nodes", json!({"nodeIds": [rect]})).await;
    assert!(ok, "{out}");
    // A failed read, then a failed change (a move across pages, refused upstream).
    for (tool, args) in [
        ("get_node_info", json!({"nodeId": key(&a, 999_999)})),
        (
            "move_nodes",
            json!({"moves": [{"nodeId": rect, "parentId": first_node(&p, &b)}]}),
        ),
    ] {
        let (ok, out) = call(&c, tool, args).await;
        assert!(!ok, "{tool}: {out}");
        let (ok, out) = call(&c, "get_selection", json!({})).await;
        assert!(ok && out.contains(&rect), "{tool}: selection kept: {out}");
    }
    // The undo history survived: undo reverts the earlier successful change.
    let (ok, out) = call(&c, "undo", json!({"pageId": a})).await;
    assert!(ok && out.contains("true"), "{out}");
    assert_eq!(x(), x0, "undone on disk");
    c.cancel().await.unwrap();
}

/// Review round 4: the page list's revision can't collide whatever the names contain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_list_revisions_are_unambiguous() {
    let t = tmp();
    let (p, a, b, r) = two_pages(&t).await;
    // The reviewer's names: under the old encoding both lists below hashed the same.
    let path = p.dir.join("project.json");
    let mut m = manifest(&p);
    m["pages"][0]["name"] = json!(format!("x\n{b}\0y"));
    m["pages"][1]["name"] = json!("z");
    std::fs::write(&path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
    let stale = client(r.port).await;
    read(&stale).await;
    // A session that never read the list can't rename to the colliding list.
    let blind = client(r.port).await;
    let before = std::fs::read(&path).unwrap();
    let (ok, out) = call(
        &blind,
        "rename_pages",
        json!({"updates": [{"pageId": a, "name": "x"}, {"pageId": b, "name": format!("y\n{b}\0z")}]}),
    )
    .await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    // Someone else renames to the colliding list; the session that read before is stale.
    read(&blind).await;
    let (ok, out) = call(
        &blind,
        "rename_pages",
        json!({"updates": [{"pageId": a, "name": "x"}, {"pageId": b, "name": format!("y\n{b}\0z")}]}),
    )
    .await;
    assert!(ok, "{out}");
    let after = std::fs::read(&path).unwrap();
    let (ok, out) = call(
        &stale,
        "rename_pages",
        json!({"updates": [{"pageId": b, "name": "stale overwrote name"}]}),
    )
    .await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(std::fs::read(&path).unwrap(), after);
    stale.cancel().await.unwrap();
    blind.cancel().await.unwrap();
}

fn set_manifest_pages(p: &Project, pages: Value) {
    let path = p.dir.join("project.json");
    let mut m: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    m["pages"] = pages;
    std::fs::write(&path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
}

fn open_error(dir: &Path) -> String {
    match Server::open(Config {
        project: dir.to_path_buf(),
        web_dir: dir.parent().unwrap().join("no-bundle"),
        port: 1,
        token: TOKEN.into(),
    }) {
        Ok(_) => panic!("the server opened"),
        Err(e) => e.to_string(),
    }
}

/// Review round 1, finding 1: page ids never become paths outside the project.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_ids_cannot_reach_outside_the_project() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let outside = t.0.join("outside");
    std::fs::create_dir_all(&outside).unwrap();

    // A page file whose own id is an outside path: the manifest's id decides where it saves.
    let victim = outside.join("outside-victim");
    let mut data = page_from_json(&String::from_utf8(file(&p, &page)).unwrap()).unwrap();
    data.id = victim.to_string_lossy().into_owned();
    std::fs::write(p.page_path(&page), page_to_json(&data)).unwrap();
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    read(&c).await;
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "rectangle", "x": 600, "y": 0, "width": 50, "height": 50}),
    )
    .await;
    assert!(ok, "{out}");
    assert_eq!(
        std::fs::read_dir(&outside).unwrap().count(),
        0,
        "nothing written outside"
    );
    let saved = page_from_json(&String::from_utf8(file(&p, &page)).unwrap()).unwrap();
    assert_eq!(saved.id, page, "saved in place, under the manifest's id");
    assert_eq!(saved.doc.nodes.len(), 2);
    c.cancel().await.unwrap();
    drop(r);

    // A manifest with an unsafe or duplicate id is refused before any page is read.
    std::fs::write(outside.join("x.json"), page_to_json(&data)).unwrap();
    let abs = outside.join("x").to_string_lossy().into_owned();
    for (what, pages) in [
        ("absolute", json!([{"id": abs, "name": "A"}])),
        ("dot-dot", json!([{"id": "../../outside/x", "name": "A"}])),
        ("slash", json!([{"id": "a/b", "name": "A"}])),
        ("empty", json!([{"id": "", "name": "A"}])),
        (
            "duplicate",
            json!([{"id": page, "name": "A"}, {"id": page, "name": "B"}]),
        ),
    ] {
        set_manifest_pages(&p, pages);
        let e = open_error(&p.dir);
        assert!(e.contains("page id"), "{what}: {e}");
    }
}

/// Operator Mac finding on a96b32c: on case-insensitive APFS, page ids `CasePage` and
/// `casepage` are one file. Refused on every file system, before any page is read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_ids_differing_only_in_case_are_refused() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let mut data = page_from_json(&String::from_utf8(file(&p, &page)).unwrap()).unwrap();
    for id in ["CasePage", "casepage"] {
        data.id = id.into();
        std::fs::write(p.page_path(id), page_to_json(&data)).unwrap();
    }
    set_manifest_pages(
        &p,
        json!([{"id": "CasePage", "name": "A"}, {"id": "casepage", "name": "B"}]),
    );
    let e = open_error(&p.dir);
    assert!(e.contains("differ in more than case"), "{e}");
    // Mixed case alone is fine.
    set_manifest_pages(&p, json!([{"id": "CasePage", "name": "A"}]));
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    read(&c).await;
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "rectangle", "x": 0, "y": 0, "width": 5, "height": 5}),
    )
    .await;
    assert!(ok, "{out}");
    c.cancel().await.unwrap();
}

/// Review round 4: node ids name their page by its first six characters, so page ids that
/// share them (ignoring case) are refused before anything is read or written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_ids_sharing_their_first_six_characters_are_refused() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let mut data = page_from_json(&String::from_utf8(file(&p, &page)).unwrap()).unwrap();
    for (one, two) in [("abcdef-A", "abcdef-B"), ("abcdef-A", "ABCDEF-B")] {
        for id in [one, two] {
            data.id = id.into();
            std::fs::write(p.page_path(id), page_to_json(&data)).unwrap();
        }
        set_manifest_pages(
            &p,
            json!([{"id": one, "name": "A"}, {"id": two, "name": "B"}]),
        );
        let e = open_error(&p.dir);
        assert!(e.contains("first six characters"), "{one}/{two}: {e}");
    }
    // Ids that differ within their first six characters are fine.
    set_manifest_pages(
        &p,
        json!([{"id": "abcdef-A", "name": "A"}, {"id": "abcdeg-B", "name": "B"}]),
    );
    data.id = "abcdeg-B".into();
    std::fs::write(p.page_path("abcdeg-B"), page_to_json(&data)).unwrap();
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    read_page(&c, "abcdeg-B").await;
    let (ok, out) = call(&c, "create_shape", shape_on("abcdeg-B")).await;
    assert!(ok, "{out}");
    c.cancel().await.unwrap();
}

/// Review round 1, finding 3: a linked page file is refused before its bytes are read.
#[cfg(unix)]
#[test]
fn page_links_are_refused_before_reading() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let outside = t.0.join("outside.json");
    // Not JSON: if it were read, the error would be a parse error.
    std::fs::write(&outside, "not json").unwrap();
    let path = p.page_path(&page);
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    let e = open_error(&p.dir);
    assert!(e.contains("symbolic link"), "{e}");
    std::fs::remove_file(&path).unwrap();
    std::fs::hard_link(&outside, &path).unwrap();
    let e = open_error(&p.dir);
    assert!(e.contains("hard link"), "{e}");
}

/// Review round 1, finding 2, end to end: the reviewer's SVG is refused before upstream runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn svg_image_fragments_are_refused() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    let disk = file(&p, &page);
    let (ok, out) = call(
        &c,
        "write_html",
        json!({"targetNodeId": "root", "mode": "insert-children", "html": "<svg xmlns='http://www.w3.org/2000/svg' width='30' height='20'><image href='#private.png' width='30' height='20'/></svg>"}),
    )
    .await;
    assert!(!ok && out.contains("refused"), "{out}");
    // Review round 2, finding 1: a malformed data: href would be read as a file name.
    let (ok, out) = call(
        &c,
        "write_html",
        json!({"targetNodeId": "root", "mode": "insert-children", "html": "<svg xmlns='http://www.w3.org/2000/svg' width='30' height='20'><image href='data:private.png' width='30' height='20'/></svg>"}),
    )
    .await;
    assert!(!ok && out.contains("refused"), "{out}");
    assert_eq!(file(&p, &page), disk);
    c.cancel().await.unwrap();
}

/// A 1×1 PNG (would be registered as an asset if it were ever read).
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_tools_projects_and_local_paths() {
    let t = tmp();
    let (p, page) = project(&t.0, "Agent");
    // A sibling project and an outside image the agent must not reach.
    let (other, _) = project(&t.0, "Other project");
    let png = paper_rs::assets::base64_decode(PNG_B64).unwrap();
    let outside = t.0.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.png"), &png).unwrap();
    let secret_id = paper_rs::assets::hash(&png);
    // And an `assets/` entry that links outside the project.
    std::fs::create_dir_all(p.dir.join("assets")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret.png"), p.dir.join("assets/evil.png")).unwrap();

    let r = serve(&p.dir).await;
    let c = client(r.port).await;
    let disk = file(&p, &page);
    let outside_before = std::fs::read_dir(&outside).unwrap().count();

    for (tool, args) in [
        (
            "export",
            json!({"nodeIds": [], "outputDir": outside.to_string_lossy()}),
        ),
        (
            "export_combined_pdf",
            json!({"outputPath": outside.join("x.pdf").to_string_lossy()}),
        ),
        ("create_file", json!({"name": "New"})),
        ("open_file", json!({"fileId": other.manifest.id})),
        ("open_file", json!({"fileId": other.dir.to_string_lossy()})),
        (
            "get_basic_info",
            json!({"fileId": other.dir.to_string_lossy()}),
        ),
    ] {
        let (ok, out) = call(&c, tool, args.clone()).await;
        assert!(!ok, "{tool} {args}: {out}");
    }
    let secret = outside.join("secret.png").to_string_lossy().into_owned();
    for html in [
        format!("<img src=\"{secret}\">"),
        format!("<img src=\"file://{secret}\">"),
        "<img src=\"../outside/secret.png\">".to_string(),
        format!("<div style=\"width:10px;height:10px;background-image:url('{secret}')\"></div>"),
        format!(
            "<svg width=\"10\" height=\"10\"><image href=\"{secret}\" width=\"10\" height=\"10\"/></svg>"
        ),
    ] {
        let (ok, out) = call(
            &c,
            "write_html",
            json!({"html": html, "targetNodeId": "root", "mode": "insert-children"}),
        )
        .await;
        assert!(!ok, "{html}: {out}");
        assert!(out.contains("refused"), "{html}: {out}");
    }
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "rectangle", "styles": {"backgroundImage": format!("url({secret})")}}),
    )
    .await;
    assert!(!ok, "{out}");
    // Nothing outside was read (never registered as an asset), written, or changed.
    assert!(
        paper_rs::assets::get(&secret_id).is_none(),
        "outside image was read"
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), outside_before);
    assert_eq!(file(&p, &page), disk, "refused calls wrote nothing");
    // Only the bound project is listed.
    let (ok, out) = call(&c, "list_resources", json!({})).await;
    assert!(
        ok && out.contains(&p.manifest.id) && !out.contains(&other.manifest.id),
        "{out}"
    );
    // data: images still work.
    read(&c).await;
    let (ok, out) = call(
        &c,
        "create_artboard",
        json!({"name": "A", "styles": {"width": "100px", "height": "100px"}}),
    )
    .await;
    assert!(ok, "{out}");
    let board = serde_json::from_str::<Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (ok, out) = call(&c, "write_html", json!({"targetNodeId": board, "mode": "insert-children", "html": format!("<img src=\"data:image/png;base64,{PNG_B64}\" style=\"width:10px;height:10px\">")})).await;
    assert!(ok, "{out}");
    c.cancel().await.unwrap();
}
