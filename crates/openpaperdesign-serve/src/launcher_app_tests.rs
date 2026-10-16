//! The app library (`--app-library`) through the real launcher, rmcp stream protocol, axum
//! routes and HTTP API the editor uses, with no workspace or Codex metadata. Two launchers in
//! one test stand for two host processes sharing the library: the lease is a real OS file lock
//! on separate open files, so it excludes them as it excludes separate processes (that case
//! runs as real child processes in `tests/app_library.rs`). Only the listener is in memory.
//! This is not a browser or a native Codex host: the renderer and navigation are not exercised.
use super::*;
use crate::library::{Lease, NewDocument};

struct Lib {
    root: PathBuf,
    l: Launcher,
    c: Arc<Client>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Lib {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn library_root() -> PathBuf {
    use std::hash::{BuildHasher, RandomState};
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "opd-app-library-{:016x}",
        RandomState::new().hash_one(std::time::SystemTime::now())
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A library launcher with no request metadata at all.
async fn launch(root: &Path) -> (Launcher, Arc<Client>, tokio::task::JoinHandle<()>) {
    let l = Launcher::with_library(Library::open(root.to_str().unwrap()).unwrap());
    let (c, task) = connect_launcher(l.clone(), Default::default()).await;
    (l, c, task)
}

async fn library() -> Lib {
    let root = library_root();
    let (l, c, task) = launch(&root).await;
    Lib { root, l, c, task }
}

fn url(v: &Value) -> (u16, String) {
    let (port, token) = v["url"]
        .as_str()
        .unwrap()
        .strip_prefix("http://127.0.0.1:")
        .unwrap()
        .split_once("/#token=")
        .unwrap();
    (port.parse().unwrap(), token.to_string())
}

/// The binding identity an editor result names (views send it back with their actions).
fn binding(v: &Value) -> String {
    v["binding"].as_str().unwrap().to_string()
}

async fn bound_server(l: &Launcher) -> Arc<Server> {
    l.state.lock().await.bound.as_ref().unwrap().server.clone()
}

async fn open_doc(l: &Launcher) -> Option<String> {
    l.state
        .lock()
        .await
        .bound
        .as_ref()
        .and_then(|b| b.doc.clone())
}

/// One raw HTTP request; returns (status, head, body text).
async fn raw(
    port: u16,
    token: &str,
    method: &str,
    path: &str,
    extra: &str,
    body: &str,
) -> (u16, String, String) {
    let mut client = connect(port).await;
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    bounded(client.read_to_end(&mut bytes)).await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, head.to_string(), body.to_string())
}

fn etag_of(head: &str) -> String {
    head.lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("etag"))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap()
}

/// What the browser editor does for one edit: load the active page, change it, save it with
/// its revision. Returns the page id and its node count after the save.
async fn browser_edit(port: u16, token: &str) -> (String, usize) {
    let (st, _, project) = raw(port, token, "GET", "/api/project", "", "").await;
    assert_eq!(st, 200, "{project}");
    let page = serde_json::from_str::<Value>(&project).unwrap()["active_page"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, head, body) = raw(port, token, "GET", &format!("/api/pages/{page}"), "", "").await;
    assert_eq!(st, 200, "{body}");
    let mut data = paper_rs::project::format::page_from_json(&body).unwrap();
    let id = data.doc.alloc_id();
    data.doc.nodes.insert(
        id,
        paper_rs::model::Node::new(
            id,
            paper_rs::model::NodeKind::Rect,
            paper_rs::geom::Rect::new(5.0, 5.0, 40.0, 30.0),
        ),
    );
    data.doc.root.push(id);
    let (st, _, answer) = raw(
        port,
        token,
        "PUT",
        &format!("/api/pages/{page}"),
        &format!("If-Match: {}\r\n", etag_of(&head)),
        &paper_rs::project::format::page_to_json(&data),
    )
    .await;
    assert_eq!(st, 200, "{answer}");
    (page, data.doc.nodes.len())
}

async fn page_nodes(port: u16, token: &str, page: &str) -> usize {
    let (st, _, body) = raw(port, token, "GET", &format!("/api/pages/{page}"), "", "").await;
    assert_eq!(st, 200, "{body}");
    paper_rs::project::format::page_from_json(&body)
        .unwrap()
        .doc
        .nodes
        .len()
}

/// An HTTP MCP session on a canvas's loopback server that has read the active page.
async fn mcp_session(port: u16, token: &str, page: &str) -> String {
    let (status, head, v) = http(port, token, "POST", "/mcp", "", &json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":"2025-06-18", "capabilities":{}, "clientInfo":{"name":"library-test", "version":"1"}}}).to_string()).await;
    assert_eq!(status, 200, "{v}");
    let sid = head
        .lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("mcp-session-id"))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap();
    let extra = format!("Mcp-Session-Id: {sid}\r\n");
    let note = json!({"jsonrpc":"2.0", "method":"notifications/initialized"}).to_string();
    assert_eq!(
        http(port, token, "POST", "/mcp", &extra, &note).await.0,
        202
    );
    http_call(
        port,
        token,
        &sid,
        "get_tree_summary",
        json!({"nodeId":"root", "pageId": page}),
    )
    .await
    .unwrap();
    sid
}

fn shape() -> Value {
    json!({"type": "rectangle", "x": 1, "y": 1, "width": 9, "height": 9})
}

/// A late request on an already accepted connection (a retained router), as other tests do.
async fn late_get(server: &Arc<Server>, port: u16, token: &str) -> StatusCode {
    router(server.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/project")
                .header("Host", format!("127.0.0.1:{port}"))
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// The positive workflow with no metadata on any call: list (creates nothing), create, the
/// editor's own load and save, the editor's readiness report, close (needs force once the
/// editor link was given; saves nothing), then a fresh launcher on the same root lists and
/// reopens the same canvas with the edit.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_create_edit_save_close_and_reopen_without_metadata() {
    let lib = library().await;
    let listed = value(call(&lib.c, app::LIST, json!({})).await);
    assert_eq!(listed["canvases"], json!([]));
    assert_eq!(listed["complete"], true);
    assert!(
        !lib.root.join("library").exists(),
        "listing creates nothing"
    );
    let created = value(call(&lib.c, app::CREATE, json!({"name": "Plan"})).await);
    assert_eq!(created["action"], "created");
    assert_eq!(created["storage"], "app-library");
    let id = created["canvas"]["id"].as_str().unwrap().to_string();
    assert!(crate::library::valid_id(&id));
    assert_eq!(created["canvas"]["name"], "Plan");
    let dir = lib.root.join("library/v1/canvases").join(&id);
    assert_eq!(created["location"], json!(dir));
    assert!(dir.join("project.json").is_file());
    assert!(!lib.root.join("DESIGN.md").exists() && !lib.root.join("design.md").exists());
    assert!(
        !dir.join("DESIGN.md").exists(),
        "no design system is applied to a blank canvas"
    );
    assert_eq!(created["editor_loaded"], false, "a link is not a load");
    let (port, token) = url(&created);
    let (page, nodes) = browser_edit(port, &token).await;
    assert_eq!(nodes, 1);
    assert_eq!(raw(port, &token, "POST", "/api/ready", "", "").await.0, 204);
    let b = binding(&created);
    let state = value(call(&lib.c, app::EDITOR, json!({"binding": b})).await);
    assert_eq!(state["editor_loaded"], true);
    assert_eq!(state["browser_saves"], 1);
    assert_eq!(url(&state), (port, token.clone()), "the same binding");
    assert_eq!(binding(&state), b);
    let old = bound_server(&lib.l).await;
    let refusal = call(&lib.c, app::CLOSE, json!({"binding": b}))
        .await
        .unwrap_err();
    assert!(refusal.contains("force: true"), "{refusal}");
    let closed = value(call(&lib.c, app::CLOSE, json!({"binding": b, "force": true})).await);
    assert_eq!(closed["action"], "closed");
    assert!(
        closed["note"]
            .as_str()
            .unwrap()
            .contains("not saved by closing")
    );
    assert_eq!(open_doc(&lib.l).await, None);
    assert_eq!(
        late_get(&old, port, &token).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(
        lib.root
            .join(format!("library/v1/locks/{id}.lock"))
            .is_file(),
        "lease file stays"
    );
    // A fresh launcher (a new process) on the same root.
    let (l2, c2, task2) = launch(&lib.root).await;
    let listed = value(call(&c2, app::LIST, json!({})).await);
    assert_eq!(listed["canvases"][0]["id"], json!(id));
    assert_eq!(listed["canvases"][0]["name"], "Plan");
    assert_eq!(listed["canvases"][0]["open"], false);
    let reopened = value(call(&c2, app::OPEN, json!({"id": id})).await);
    assert_eq!(reopened["action"], "opened");
    let (port2, token2) = url(&reopened);
    assert_eq!(
        page_nodes(port2, &token2, &page).await,
        1,
        "the saved edit is there"
    );
    bound_server(&l2).await.retire().await;
    task2.abort();
}

/// Library mode refuses every project route without admission, whatever the arguments or
/// metadata say, and a workspace process refuses library actions without retiring anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_mode_has_no_project_routes_and_takes_no_paths() {
    let lib = library().await;
    let elsewhere = lib.root.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    call(&lib.c, app::CREATE, json!({"name": "Only"}))
        .await
        .unwrap();
    for meta in [Default::default(), context::writable(&elsewhere)] {
        for (tool, args) in [
            ("bind_project", json!({"path": elsewhere})),
            ("create_project", json!({"name": "Repo"})),
            ("open_editor", json!({})),
            ("openpaperdesign_canvases", json!({})),
            (
                "openpaperdesign_ui_open",
                json!({"root": elsewhere, "project": "design/X"}),
            ),
            ("openpaperdesign_choose_folder", json!({})),
            ("get_tree_summary", json!({"nodeId": "root"})),
        ] {
            let e = call_meta(&lib.c, tool, args, meta.clone())
                .await
                .unwrap_err();
            assert!(e.contains("runs the app library"), "{tool}: {e}");
        }
        for (tool, args, want) in [
            (app::OPEN, json!({"id": "../../elsewhere"}), "canvas id"),
            (
                app::OPEN,
                json!({"id": "0123456789abcdef", "path": "/etc"}),
                "unexpected argument",
            ),
            (
                app::CREATE,
                json!({"name": "x", "root": elsewhere}),
                "unexpected argument",
            ),
            (
                app::CREATE,
                json!({"name": "a\u{7}b"}),
                "control characters",
            ),
            (app::CREATE, json!({"name": "x".repeat(61)}), "1 to 60"),
            (
                app::OPEN,
                json!({"id": "0123456789abcdef"}),
                "no such canvas",
            ),
        ] {
            let e = call_meta(&lib.c, tool, args, meta.clone())
                .await
                .unwrap_err();
            assert!(e.contains(want), "{tool}: {e}");
        }
    }
    let listed = value(call(&lib.c, app::LIST, json!({})).await);
    assert_eq!(listed["canvases"].as_array().unwrap().len(), 1);
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "nothing reached it"
    );
    assert!(
        !lib.root
            .join("library/v1/locks/0123456789abcdef.lock")
            .exists(),
        "no lease file for a canvas that doesn't exist"
    );
    let status = value(call(&lib.c, "openpaperdesign_status", json!({})).await);
    assert_eq!(status["mode"], "app-library");
    assert_eq!(status["workspace_usable"], false);
    bound_server(&lib.l).await.retire().await;
    // A packaged workspace process without metadata: refused, nothing retired or pinned.
    let l = Launcher::new(None);
    let (c, task) = connect_launcher(l.clone(), Default::default()).await;
    let e = call(&c, app::LIST, json!({})).await.unwrap_err();
    assert!(e.contains("wasn't started for the app library"), "{e}");
    let st = l.state.lock().await;
    assert!(st.invalidated.is_none() && st.workspace.is_none() && st.bound.is_none());
    drop(st);
    task.abort();
}

/// The lease across two "processes": the same canvas is busy before anything opens it, a
/// different one is fine, the same canvas again is a checked no-op, a busy target keeps the
/// current canvas and URL, and close, switch and an ended process release it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_leases_exclude_the_same_canvas_only() {
    let a = library().await;
    let (lb, b, task_b) = launch(&a.root).await;
    let one = value(call(&a.c, app::CREATE, json!({"name": "One"})).await);
    let id1 = one["canvas"]["id"].as_str().unwrap().to_string();
    let (port1, token1) = url(&one);
    let e = call(&b, app::OPEN, json!({"id": id1})).await.unwrap_err();
    assert!(e.contains("open in another OpenPaperDesign window"), "{e}");
    assert_eq!(open_doc(&lb).await, None);
    let two = value(call(&b, app::CREATE, json!({"name": "Two"})).await);
    let id2 = two["canvas"]["id"].as_str().unwrap().to_string();
    assert_ne!(id1, id2);
    let again = value(call(&a.c, app::OPEN, json!({"id": id1})).await);
    assert_eq!(again["action"], "already-open");
    assert_eq!(url(&again), (port1, token1.clone()));
    let server1 = bound_server(&a.l).await;
    let b1 = binding(&one);
    let e = call(
        &a.c,
        app::OPEN,
        json!({"id": id2, "force": true, "replace": b1}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("open in another OpenPaperDesign window"), "{e}");
    assert!(
        Arc::ptr_eq(&bound_server(&a.l).await, &server1),
        "busy target keeps the binding"
    );
    assert_eq!(
        raw(port1, &token1, "GET", "/api/project", "", "").await.0,
        200
    );
    // B closes Two; A switches to it, which releases One for B.
    call(
        &b,
        app::CLOSE,
        json!({"binding": binding(&two), "force": true}),
    )
    .await
    .unwrap();
    let switched = value(
        call(
            &a.c,
            app::OPEN,
            json!({"id": id2, "force": true, "replace": b1}),
        )
        .await,
    );
    assert_eq!(switched["action"], "opened");
    let reopened = value(call(&b, app::OPEN, json!({"id": id1})).await);
    assert_eq!(reopened["action"], "opened");
    // An ended "process" (its binding dropped without a close) releases its lease too.
    task_b.abort();
    let _ = task_b.await;
    drop(b);
    drop(lb);
    let (l3, c3, task3) = launch(&a.root).await;
    let mut opened = None;
    for _ in 0..50 {
        match call(&c3, app::OPEN, json!({"id": id1})).await {
            Ok(v) => {
                opened = Some(v);
                break;
            }
            Err(e) if e.contains("another OpenPaperDesign window") => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert!(opened.is_some(), "the ended launcher's lease was released");
    bound_server(&l3).await.retire().await;
    bound_server(&a.l).await.retire().await;
    task3.abort();
}

/// A switch keeps the old canvas's lease until its started writer finished: another process
/// can't open it meanwhile. A caller that goes away doesn't stop the handoff.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_switch_holds_the_lease_until_the_started_writer_drained() {
    let a = library().await;
    let (lb, b, task_b) = launch(&a.root).await;
    let two = value(call(&b, app::CREATE, json!({"name": "Two"})).await);
    let id2 = two["canvas"]["id"].as_str().unwrap().to_string();
    call(
        &b,
        app::CLOSE,
        json!({"binding": binding(&two), "force": true}),
    )
    .await
    .unwrap();
    let one = value(call(&a.c, app::CREATE, json!({"name": "One"})).await);
    let id1 = one["canvas"]["id"].as_str().unwrap().to_string();
    let dir1: PathBuf = serde_json::from_value(one["location"].clone()).unwrap();
    let (port, token) = url(&one);
    let page = crate::read_manifest_checked(&dir1).unwrap().pages[0]
        .id
        .clone();
    let sid = mcp_session(port, &token, &page).await;
    let old = bound_server(&a.l).await;
    let mut snapshot = Point::arm(&dir1, "snapshot", true);
    let t = token.clone();
    let running =
        tokio::spawn(async move { http_call(port, &t, &sid, "create_shape", shape()).await });
    snapshot.reached().await;
    let l = a.l.clone();
    // The switch target, for the cancelled caller; `id2` is checked again below.
    let target = id2.clone();
    let replace = binding(&one);
    let switch = tokio::spawn(async move {
        l.app(
            l.library.as_ref().unwrap(),
            app::OPEN,
            json!({"id": target, "force": true, "replace": replace}),
        )
        .await
    });
    bounded(old.lifecycle.retiring.notified()).await;
    switch.abort();
    assert!(switch.await.unwrap_err().is_cancelled());
    let e = call(&b, app::OPEN, json!({"id": id1})).await.unwrap_err();
    assert!(
        e.contains("another OpenPaperDesign window"),
        "lease held during drain: {e}"
    );
    snapshot.resume();
    bounded(running).await.unwrap().unwrap();
    // The owned handoff finished without its caller.
    for _ in 0..100 {
        if open_doc(&a.l).await.as_deref() == Some(id2.as_str()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(open_doc(&a.l).await.as_deref(), Some(id2.as_str()));
    let reopened = value(call(&b, app::OPEN, json!({"id": id1})).await);
    let (p, t) = url(&reopened);
    assert_eq!(
        page_nodes(p, &t, &page).await,
        1,
        "the started write was kept"
    );
    bound_server(&lb).await.retire().await;
    bound_server(&a.l).await.retire().await;
    task_b.abort();
}

/// Failures before the handoff (listener, directory sync, commit, preparation) keep the open
/// canvas and its URL and remove exactly what the failed call made. The thread-local commit
/// fault needs the owned task on this same thread.
#[tokio::test(flavor = "current_thread")]
async fn library_failed_creation_keeps_the_open_canvas_and_removes_its_own_files() {
    let lib = library().await;
    let one = value(call(&lib.c, app::CREATE, json!({"name": "One"})).await);
    let (port, token) = url(&one);
    let server = bound_server(&lib.l).await;
    let canvases = lib.root.join("library/v1/canvases");
    let entries = || std::fs::read_dir(&canvases).unwrap().count();
    let check = |e: String, want: &str| {
        assert!(
            e.contains(want) && e.contains("Nothing was created"),
            "{want}: {e}"
        );
    };
    // Each switch confirms replacing exactly the open canvas.
    let create = |name: &str| json!({"name": name, "force": true, "replace": binding(&one)});
    let _listen = Point::arm(lib.root.as_path(), "listen_fails", false);
    check(
        call(&lib.c, app::CREATE, create("A")).await.unwrap_err(),
        "listen",
    );
    let _sync = Point::arm(&canvases, "sync_dir", false);
    check(
        call(&lib.c, app::CREATE, create("B")).await.unwrap_err(),
        "injected sync failure",
    );
    // Stage page, stage manifest, rename page, rename manifest (fails), then the rollback.
    crate::commit::fault::fail_at(&[3]);
    let e = call(&lib.c, app::CREATE, create("C")).await.unwrap_err();
    crate::commit::fault::fail_at(&[]);
    check(e, "nothing was written");
    let _prepare = Point::arm(lib.root.as_path(), "app_prepare_fails", false);
    check(
        call(&lib.c, app::CREATE, create("D")).await.unwrap_err(),
        "injected test failure",
    );
    assert_eq!(entries(), 1, "only the first canvas exists");
    assert!(
        Arc::ptr_eq(&bound_server(&lib.l).await, &server),
        "the open canvas is kept"
    );
    assert_eq!(
        raw(port, &token, "GET", "/api/project", "", "").await.0,
        200
    );
    // A removal that can't be made durable is named, never reported as clean.
    let lib2 = Library::open(lib.root.to_str().unwrap()).unwrap();
    let lease = Lease::fresh(&lib2).unwrap();
    let (dir, created) = lib2
        .create_document(&lease, &NewDocument::blank("E").fresh())
        .unwrap();
    let _sync = Point::arm(&canvases, "sync_dir", false);
    let left = created.undo();
    assert!(!dir.exists());
    assert!(
        left.iter().any(|l| l.contains("may not be durable")),
        "{left:?}"
    );
    server.retire().await;
}

/// The lease file or the library replaced under an open canvas fences it for browser and MCP
/// access, also after the original comes back; the next action closes it. A lease file that is
/// a link is refused before anything opens.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_replaced_lease_or_root_fences_old_handles() {
    let lib = library().await;
    let one = value(call(&lib.c, app::CREATE, json!({"name": "One"})).await);
    let id = one["canvas"]["id"].as_str().unwrap().to_string();
    let (port, token) = url(&one);
    let lock = lib.root.join(format!("library/v1/locks/{id}.lock"));
    let aside = lock.with_extension("aside");
    std::fs::rename(&lock, &aside).unwrap();
    std::fs::write(&lock, b"").unwrap();
    assert_eq!(
        raw(port, &token, "GET", "/api/project", "", "").await.0,
        403
    );
    std::fs::remove_file(&lock).unwrap();
    std::fs::rename(&aside, &lock).unwrap();
    assert_eq!(
        raw(port, &token, "GET", "/api/project", "", "").await.0,
        403,
        "stays fenced"
    );
    let old = bound_server(&lib.l).await;
    let e = call(&lib.c, app::EDITOR, json!({})).await.unwrap_err();
    assert!(e.contains("so it was closed"), "{e}");
    assert_eq!(open_doc(&lib.l).await, None);
    assert_eq!(
        late_get(&old, port, &token).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    // A second link to the lease file.
    let two = value(call(&lib.c, app::OPEN, json!({"id": id})).await);
    let (port, token) = url(&two);
    std::fs::hard_link(&lock, lib.root.join("second-link")).unwrap();
    assert_eq!(
        raw(port, &token, "GET", "/api/project", "", "").await.0,
        403
    );
    std::fs::remove_file(lib.root.join("second-link")).unwrap();
    call(
        &lib.c,
        app::CLOSE,
        json!({"binding": binding(&two), "force": true}),
    )
    .await
    .unwrap();
    // A lease path that is a link is never followed or opened through.
    let target = lib.root.join("target-file");
    std::fs::remove_file(&lock).unwrap();
    std::os::unix::fs::symlink(&target, &lock).unwrap();
    let e = call(&lib.c, app::OPEN, json!({"id": id}))
        .await
        .unwrap_err();
    assert!(
        e.contains("symbolic link") && e.contains("Nothing was opened"),
        "{e}"
    );
    assert!(!target.exists());
    // The library folder itself replaced.
    let moved = lib.root.with_extension("moved");
    let fresh = value(call(&lib.c, app::CREATE, json!({"name": "Three"})).await);
    let (port, token) = url(&fresh);
    std::fs::rename(&lib.root, &moved).unwrap();
    std::fs::create_dir(&lib.root).unwrap();
    assert_eq!(
        raw(port, &token, "GET", "/api/project", "", "").await.0,
        403
    );
    let e = call(&lib.c, app::LIST, json!({})).await.unwrap_err();
    assert!(e.contains("moved or replaced"), "{e}");
    let _ = std::fs::remove_dir_all(&moved);
}

/// Export waits for a save under way and contains it; import validates the whole file and
/// publishes a new canvas without touching the source; a bad file creates nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_export_waits_for_a_started_save_and_import_makes_a_new_canvas() {
    let lib = library().await;
    let e = call(&lib.c, app::EXPORT, json!({})).await.unwrap_err();
    assert!(e.contains("Open the canvas"), "{e}");
    let one = value(call(&lib.c, app::CREATE, json!({"name": "Export me"})).await);
    let id = one["canvas"]["id"].as_str().unwrap().to_string();
    let dir: PathBuf = serde_json::from_value(one["location"].clone()).unwrap();
    let (port, token) = url(&one);
    let page = crate::read_manifest_checked(&dir).unwrap().pages[0]
        .id
        .clone();
    let sid = mcp_session(port, &token, &page).await;
    let mut snapshot = Point::arm(&dir, "snapshot", true);
    let t = token.clone();
    let running =
        tokio::spawn(async move { http_call(port, &t, &sid, "create_shape", shape()).await });
    snapshot.reached().await;
    let c = lib.c.clone();
    let b1 = binding(&one);
    let args = json!({"binding": b1});
    let export = tokio::spawn(async move { call(&c, app::EXPORT, args).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!export.is_finished(), "export waits for the save under way");
    snapshot.resume();
    bounded(running).await.unwrap().unwrap();
    let exported = value(bounded(export).await.unwrap());
    assert_eq!(exported["content"], "canvas-only");
    assert_eq!(
        binding(&exported),
        b1,
        "the export names the binding it read"
    );
    assert!(
        exported["file_name"]
            .as_str()
            .unwrap()
            .ends_with(".opdcanvas.json")
    );
    let file = exported["payload"].as_str().unwrap().to_string();
    let parsed: Value = serde_json::from_str(&file).unwrap();
    assert_eq!(
        parsed["pages"][0]["nodes"].as_array().unwrap().len(),
        1,
        "the save is in it"
    );
    let source = crate::read_manifest_checked(&dir).unwrap();
    let source_page = std::fs::read(dir.join("pages").join(format!("{page}.json"))).unwrap();
    // A bad file creates nothing and keeps the open canvas.
    let bad = file.replacen("\"format\":", "\"format\":\"x\",\"format\":", 1);
    let e = call(
        &lib.c,
        app::IMPORT,
        json!({"payload": bad, "force": true, "replace": b1}),
    )
    .await
    .unwrap_err();
    assert!(
        e.contains("appears twice") && e.contains("Nothing was imported"),
        "{e}"
    );
    assert_eq!(open_doc(&lib.l).await.as_deref(), Some(id.as_str()));
    assert_eq!(
        std::fs::read_dir(lib.root.join("library/v1/canvases"))
            .unwrap()
            .count(),
        1
    );
    let imported = value(
        call(
            &lib.c,
            app::IMPORT,
            json!({"payload": file, "force": true, "replace": b1}),
        )
        .await,
    );
    assert_eq!(imported["action"], "imported");
    let new_id = imported["canvas"]["id"].as_str().unwrap();
    assert_ne!(new_id, id);
    assert_eq!(imported["canvas"]["name"], "Export me");
    let (p, t) = url(&imported);
    assert_eq!(page_nodes(p, &t, &page).await, 1);
    let new_dir: PathBuf = serde_json::from_value(imported["location"].clone()).unwrap();
    let copy = crate::read_manifest_checked(&new_dir).unwrap();
    assert_ne!(copy.id, source.id, "a fresh identity");
    assert_eq!(copy.pages, source.pages);
    assert_eq!(
        crate::read_manifest_checked(&dir).unwrap(),
        source,
        "source untouched"
    );
    assert_eq!(
        std::fs::read(dir.join("pages").join(format!("{page}.json"))).unwrap(),
        source_page
    );
    bound_server(&lib.l).await.retire().await;
}

/// A file with a node id the upstream parser would overflow on, or with a self-consistent
/// group no top level reaches, is an ordinary refusal: nothing is published, and the open
/// canvas, its URL and the session keep working.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_refused_imports_keep_the_open_canvas_and_the_session() {
    let lib = library().await;
    let one = value(call(&lib.c, app::CREATE, json!({"name": "Keep"})).await);
    let (port, token) = url(&one);
    let b1 = binding(&one);
    let server = bound_server(&lib.l).await;
    let exported = value(call(&lib.c, app::EXPORT, json!({"binding": b1})).await);
    let file: Value = serde_json::from_str(exported["payload"].as_str().unwrap()).unwrap();
    let rect = |id: u64| json!({"id": id, "name": "R", "type": "rect", "rect": {"x": 0, "y": 0, "w": 5, "h": 5}});
    let mut huge = file.clone();
    huge["pages"][0]["nodes"] = json!([rect(u64::MAX)]);
    huge["pages"][0]["root"] = json!([u64::MAX]);
    let mut cycle = file.clone();
    let (mut a, mut b) = (rect(900), rect(901));
    a["parent"] = json!(901);
    a["children"] = json!([901]);
    b["parent"] = json!(900);
    b["children"] = json!([900]);
    cycle["pages"][0]["nodes"] = json!([a, b]);
    cycle["pages"][0]["next_id"] = json!(902);
    for (bad, want) in [
        (huge, "above the largest allowed id"),
        (cycle, "aren't reachable"),
    ] {
        let args = json!({"payload": bad.to_string(), "force": true, "replace": b1});
        let e = call(&lib.c, app::IMPORT, args).await.unwrap_err();
        assert!(
            e.contains(want) && e.contains("Nothing was imported"),
            "{want}: {e}"
        );
        assert!(Arc::ptr_eq(&bound_server(&lib.l).await, &server));
        assert_eq!(
            raw(port, &token, "GET", "/api/project", "", "").await.0,
            200
        );
        let listed = value(call(&lib.c, app::LIST, json!({})).await);
        assert_eq!(listed["canvases"].as_array().unwrap().len(), 1);
    }
    let again = value(call(&lib.c, app::EDITOR, json!({"binding": b1})).await);
    assert_eq!(
        url(&again),
        (port, token.clone()),
        "the session still works"
    );
    server.retire().await;
}

/// Two views of one process share its one binding. After the other view switches, the first
/// view's close, export, readiness poll and switch confirmation all name a binding that is no
/// longer open: each is refused, and the current canvas, its URL, files and lease stay. A
/// switch refusal names the current binding, so a confirmation is bound to exactly that one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_stale_views_never_act_on_another_canvas() {
    let lib = library().await;
    let (view2, task2) = connect_launcher(lib.l.clone(), Default::default()).await;
    let one = value(call(&lib.c, app::CREATE, json!({"name": "One"})).await);
    let b1 = binding(&one);
    let id1 = one["canvas"]["id"].as_str().unwrap().to_string();
    // View 2 is asked to confirm replacing exactly One's binding, and does.
    let e = call(&view2, app::CREATE, json!({"name": "Two"}))
        .await
        .unwrap_err();
    assert!(
        e.contains("force: true") && e.contains(&format!("(open canvas: {b1})")),
        "{e}"
    );
    let two = value(
        call(
            &view2,
            app::CREATE,
            json!({"name": "Two", "force": true, "replace": b1}),
        )
        .await,
    );
    let b2 = binding(&two);
    assert_ne!(b1, b2);
    let (port2, token2) = url(&two);
    let server2 = bound_server(&lib.l).await;
    let dir2: PathBuf = serde_json::from_value(two["location"].clone()).unwrap();
    let files = || std::fs::read(dir2.join("project.json")).unwrap();
    let before = files();
    // View 1 still shows One.
    for (tool, args) in [
        (app::CLOSE, json!({"binding": b1, "force": true})),
        (app::EXPORT, json!({"binding": b1})),
        (app::EDITOR, json!({"binding": b1})),
        (app::OPEN, json!({"id": id1, "force": true, "replace": b1})),
        (
            app::CREATE,
            json!({"name": "Three", "force": true, "replace": b1}),
        ),
    ] {
        let e = call(&lib.c, tool, args).await.unwrap_err();
        assert!(e.contains("another view changed it"), "{tool}: {e}");
        assert!(Arc::ptr_eq(&bound_server(&lib.l).await, &server2), "{tool}");
        assert_eq!(
            raw(port2, &token2, "GET", "/api/project", "", "").await.0,
            200,
            "{tool}"
        );
    }
    assert_eq!(files(), before, "Two's files are untouched");
    let listed = value(call(&lib.c, app::LIST, json!({})).await);
    assert_eq!(
        listed["canvases"].as_array().unwrap().len(),
        2,
        "nothing created"
    );
    // A switch without the confirmation, or confirming nothing open, is refused too.
    let e = call(&lib.c, app::OPEN, json!({"id": id1, "force": true}))
        .await
        .unwrap_err();
    assert!(e.contains(&format!("(open canvas: {b2})")), "{e}");
    let e = call(&lib.c, app::CLOSE, json!({"binding": "0123456789abcdef"}))
        .await
        .unwrap_err();
    assert!(e.contains("isn't a binding"), "{e}");
    // Without a binding, "Open current canvas" shows what is open now: Two, with its binding.
    let current = value(call(&lib.c, app::EDITOR, json!({})).await);
    assert_eq!(binding(&current), b2);
    // View 2 closes Two; its old confirmation can't then open anything.
    call(&view2, app::CLOSE, json!({"binding": b2, "force": true}))
        .await
        .unwrap();
    let e = call(
        &view2,
        app::OPEN,
        json!({"id": id1, "force": true, "replace": b2}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("another view changed it"), "{e}");
    assert_eq!(open_doc(&lib.l).await, None);
    task2.abort();
}
