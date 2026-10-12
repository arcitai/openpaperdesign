//! The plugin's stdio MCP launcher (`openpaperdesign-mcp`), driven as a child process with the
//! rmcp client, in temporary workspaces.

use paper_rs::project::Store;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ContentBlock};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp() -> Tmp {
    use std::hash::{BuildHasher, RandomState};
    let d = std::env::temp_dir().join(format!(
        "opd-launcher-{:016x}",
        RandomState::new().hash_one(std::time::SystemTime::now())
    ));
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d.canonicalize().unwrap())
}

type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;

/// Start the launcher in `workspace` with HOME set to `home`.
async fn launcher(workspace: &Path, home: &Path) -> Client {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_openpaperdesign-mcp"));
    cmd.current_dir(workspace).env("HOME", home);
    ().serve(rmcp::transport::TokioChildProcess::new(cmd).unwrap())
        .await
        .unwrap()
}

async fn call(c: &Client, name: &str, args: Value) -> (bool, String) {
    let mut p = CallToolRequestParams::new(name.to_string());
    p.arguments = args.as_object().cloned();
    let r = c.call_tool(p).await.unwrap();
    let text = r
        .content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (r.is_error != Some(true), text)
}

/// What an agent does before its first change to a page: read it (here the whole page tree).
async fn read_page(c: &Client) {
    let (ok, out) = call(c, "get_tree_summary", json!({"nodeId": "root"})).await;
    assert!(ok, "{out}");
}

/// A minimal HTTP/1.1 request on loopback with the token; returns (status, ETag, body).
async fn http(
    port: u16,
    method: &str,
    path: &str,
    token: &str,
    if_match: Option<&str>,
    body: &str,
) -> (u16, Option<String>, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let im = if_match.map_or(String::new(), |m| format!("If-Match: {m}\r\n"));
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n{im}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let etag = head.lines().find_map(|l| {
        l.to_ascii_lowercase()
            .starts_with("etag:")
            .then(|| l[5..].trim().to_string())
    });
    // Chunked bodies aren't expected for these small JSON answers.
    (status, etag, body.to_string())
}

/// B2b review round 1, finding 4: the stdio client is one session of the host's
/// read-before-write tracking: a change after someone else's save is refused until it reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_stdio_session_must_read_before_it_writes() {
    let t = tmp();
    let ws = t.0.join("workspace");
    let home = t.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (p, _) = Store::open(ws.join("design")).unwrap().create("A").unwrap();
    let page = p.manifest.pages[0].id.clone();
    let c = launcher(&ws, &home).await;
    let (ok, out) = call(&c, "bind_project", json!({"path": "design/A"})).await;
    assert!(ok, "{out}");
    read_page(&c).await;
    let shape = json!({"type": "rectangle", "x": 1, "y": 1, "width": 9, "height": 9});
    let (ok, out) = call(&c, "create_shape", shape.clone()).await;
    assert!(ok, "{out}");
    let (ok, out) = call(&c, "open_editor", json!({})).await;
    if !ok {
        // Built without the editor: no browser to save through.
        assert!(out.contains("build the web bundle"), "{out}");
        return;
    }
    let (port, token) = editor(&out);
    // The person saves the page in the browser (a conditional save on its revision).
    let path = format!("/api/pages/{page}");
    let (st, tag, body) = http(port, "GET", &path, &token, None, "").await;
    assert_eq!(st, 200, "{body}");
    let mut v: Value = serde_json::from_str(&body).unwrap();
    v["next_id"] = json!(v["next_id"].as_u64().unwrap() + 1);
    let (st, _, out) = http(port, "PUT", &path, &token, tag.as_deref(), &v.to_string()).await;
    assert_eq!(st, 200, "{out}");
    let saved = std::fs::read(p.page_path(&page)).unwrap();
    // The agent's next change wasn't decided on that: refused, nothing written.
    let (ok, out) = call(&c, "create_shape", shape.clone()).await;
    assert!(!ok && out.contains("conflict"), "{out}");
    assert_eq!(std::fs::read(p.page_path(&page)).unwrap(), saved);
    // After reading the page again it goes through.
    read_page(&c).await;
    let (ok, out) = call(&c, "create_shape", shape).await;
    assert!(ok, "{out}");
    c.cancel().await.unwrap();
}

/// B2b review round 1, finding 1 (reviewer's sequence): the workspace is moved and its old
/// path replaced by a symlink to another folder; create_project writes nothing there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replaced_workspace_is_refused_not_followed() {
    let t = tmp();
    let ws = t.0.join("replaceable");
    let target = t.0.join("replacement-target");
    let home = t.0.join("home");
    for d in [&ws, &target, &home] {
        std::fs::create_dir_all(d).unwrap();
    }
    let c = launcher(&ws, &home).await;
    std::fs::rename(&ws, t.0.join("original-workspace-retained")).unwrap();
    std::os::unix::fs::symlink(&target, &ws).unwrap();
    let (ok, out) = call(&c, "create_project", json!({"name": "Escaped"})).await;
    assert!(!ok && out.contains("moved or replaced"), "{out}");
    let (ok, out) = call(&c, "bind_project", json!({"path": "."})).await;
    assert!(!ok && out.contains("moved or replaced"), "{out}");
    assert_eq!(
        std::fs::read_dir(&target).unwrap().count(),
        0,
        "nothing written there"
    );
    assert_eq!(
        std::fs::read_dir(t.0.join("original-workspace-retained"))
            .unwrap()
            .count(),
        0
    );
    // Also a plain directory put back at the same path is a different workspace.
    std::fs::remove_file(&ws).unwrap();
    std::fs::create_dir(&ws).unwrap();
    let (ok, out) = call(&c, "create_project", json!({"name": "Escaped"})).await;
    assert!(!ok && out.contains("moved or replaced"), "{out}");
    assert_eq!(std::fs::read_dir(&ws).unwrap().count(), 0);
    c.cancel().await.unwrap();
}

/// A minimal HTTP/1.1 GET on loopback; returns the status code.
async fn http_status(port: u16, path: &str, token: &str) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let head = String::from_utf8_lossy(&buf);
    head.split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn page_files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<_> = std::fs::read_dir(dir.join("pages"))
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bind_edit_and_open_the_editor_inside_the_workspace_only() {
    let t = tmp();
    let ws = t.0.join("workspace");
    let home = t.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (landing, _) = Store::open(ws.join("design"))
        .unwrap()
        .create("Landing")
        .unwrap();
    let (outside, _) = Store::open(t.0.join("elsewhere"))
        .unwrap()
        .create("Outside")
        .unwrap();
    std::os::unix::fs::symlink(&outside.dir, ws.join("escape")).unwrap();
    std::fs::create_dir_all(ws.join("notes")).unwrap();
    let outside_before = page_files(&outside.dir);

    let c = launcher(&ws, &home).await;
    let tools: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for want in [
        "bind_project",
        "create_project",
        "open_editor",
        "openpaperdesign_status",
        "write_html",
        "create_shape",
    ] {
        assert!(tools.iter().any(|t| t == want), "{want} missing");
    }
    for refused in ["export", "export_combined_pdf", "create_file"] {
        assert!(!tools.iter().any(|t| t == refused), "{refused} offered");
    }

    // Nothing works before a deliberate bind.
    for (tool, args) in [
        ("get_basic_info", json!({})),
        ("create_shape", json!({"type": "rectangle"})),
        ("open_editor", json!({})),
    ] {
        let (ok, out) = call(&c, tool, args).await;
        assert!(!ok && out.contains("bind_project"), "{tool}: {out}");
    }

    // Refused binds.
    for (path, why) in [
        (
            outside.dir.to_string_lossy().into_owned(),
            "outside the workspace",
        ),
        ("../elsewhere/Outside".into(), "outside the workspace"),
        ("escape".into(), "outside the workspace"),
        ("notes".into(), "not a paper-rs project"),
        ("missing".into(), "No such file"),
    ] {
        let (ok, out) = call(&c, "bind_project", json!({"path": path})).await;
        assert!(!ok && out.contains(why), "{path}: {out}");
    }
    assert_eq!(
        page_files(&outside.dir),
        outside_before,
        "outside project untouched"
    );

    // Bind a real project and edit it.
    let before = page_files(&landing.dir);
    let (ok, out) = call(&c, "bind_project", json!({"path": "design/Landing"})).await;
    assert!(ok, "{out}");
    read_page(&c).await;
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "ellipse", "x": 10, "y": 10, "width": 40, "height": 30}),
    )
    .await;
    assert!(ok, "{out}");
    assert_ne!(
        page_files(&landing.dir),
        before,
        "the edit was saved to the project"
    );

    // The editor URL, when the browser bundle is built in.
    let (_, status) = call(&c, "openpaperdesign_status", json!({})).await;
    let status: Value = serde_json::from_str(&status).unwrap();
    assert_eq!(
        status["bound_project"],
        json!(landing.dir.canonicalize().unwrap())
    );
    let (ok, out) = call(&c, "open_editor", json!({})).await;
    if status["browser_editor"] == "built in" {
        assert!(ok, "{out}");
        let v: Value = serde_json::from_str(&out).unwrap();
        let url = v["url"].as_str().unwrap();
        let rest = url.strip_prefix("http://127.0.0.1:").expect("loopback URL");
        let (port, frag) = rest.split_once("/#token=").unwrap();
        let port: u16 = port.parse().unwrap();
        assert_eq!(frag.len(), 64);
        assert_eq!(http_status(port, "/api/project", frag).await, 200);
        assert_eq!(http_status(port, "/api/project", "wrong").await, 401);
        assert_eq!(http_status(port, "/", "").await, 200, "the page itself");
    } else {
        assert!(!ok && out.contains("build the web bundle"), "{out}");
    }
    // The token is not written anywhere in the workspace.
    let token_files: Vec<_> = walk(&ws)
        .into_iter()
        .filter(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains("#token=")))
        .collect();
    assert!(token_files.is_empty(), "{token_files:?}");

    // create_project makes design/<name> and switches to it. Once the browser editor was
    // opened, switching needs force (unsaved edits in that tab would be lost).
    let editor_opened = status["browser_editor"] == "built in";
    if editor_opened {
        let (ok, out) = call(&c, "create_project", json!({"name": "New board"})).await;
        assert!(!ok && out.contains("force: true"), "{out}");
        assert!(!ws.join("design/New board").exists(), "nothing created");
    }
    let (ok, out) = call(
        &c,
        "create_project",
        json!({"name": "New board", "force": editor_opened}),
    )
    .await;
    assert!(ok, "{out}");
    assert!(ws.join("design/New board/project.json").is_file());
    let (_, status) = call(&c, "openpaperdesign_status", json!({})).await;
    assert!(status.contains("New board"), "{status}");
    let (ok, out) = call(&c, "create_project", json!({"name": "New board"})).await;
    assert!(!ok && out.contains("already exists"), "{out}");
    c.cancel().await.unwrap();
}

/// Every file under `dir` with its bytes (links not followed).
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut v: Vec<_> = walk(dir)
        .into_iter()
        .map(|p| {
            let b = std::fs::read(&p).unwrap();
            (p, b)
        })
        .collect();
    v.sort();
    v
}

/// The editor URL's port and token.
fn editor(out: &str) -> (u16, String) {
    let v: Value = serde_json::from_str(out).unwrap();
    let url = v["url"].as_str().unwrap();
    let rest = url.strip_prefix("http://127.0.0.1:").unwrap();
    let (port, token) = rest.split_once("/#token=").unwrap();
    (port.parse().unwrap(), token.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_rebind_keeps_the_current_project_and_its_editor() {
    let t = tmp();
    let ws = t.0.join("workspace");
    let home = t.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(ws.join("design")).unwrap();
    let (a, _) = store.create("A").unwrap();
    let (b, _) = store.create("B").unwrap();
    // Invalid targets: an unsafe page id, a linked page file, no project.json, a link out.
    let (bad_id, _) = store.create("Bad id").unwrap();
    let mpath = bad_id.dir.join("project.json");
    let mut m: Value = serde_json::from_slice(&std::fs::read(&mpath).unwrap()).unwrap();
    m["pages"][0]["id"] = json!("../../../outside/x");
    std::fs::write(&mpath, serde_json::to_vec(&m).unwrap()).unwrap();
    let (linked, _) = store.create("Linked").unwrap();
    let outside = t.0.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("page.json"), "not json").unwrap();
    let page_id = linked.manifest.pages[0].id.clone();
    std::fs::create_dir_all(linked.dir.join("pages")).unwrap();
    let _ = std::fs::remove_file(linked.page_path(&page_id));
    std::os::unix::fs::symlink(outside.join("page.json"), linked.page_path(&page_id)).unwrap();
    std::fs::create_dir_all(ws.join("notes")).unwrap();
    let (far, _) = Store::open(t.0.join("elsewhere"))
        .unwrap()
        .create("Far")
        .unwrap();
    std::os::unix::fs::symlink(&far.dir, ws.join("escape")).unwrap();

    let c = launcher(&ws, &home).await;
    let (ok, out) = call(&c, "bind_project", json!({"path": "design/A"})).await;
    assert!(ok, "{out}");
    read_page(&c).await;
    let (ok, out) = call(
        &c,
        "create_shape",
        json!({"type": "rectangle", "x": 1, "y": 1, "width": 9, "height": 9}),
    )
    .await;
    assert!(ok, "{out}");
    let (ok, out) = call(&c, "open_editor", json!({})).await;
    let url = ok.then(|| editor(&out));
    let before = (snapshot(&ws), snapshot(&outside), snapshot(&far.dir));
    let a_dir = json!(a.dir.canonicalize().unwrap());

    for (path, why) in [
        ("design/Bad id", "page id"),
        ("design/Linked", "symbolic link"),
        ("notes", "not a paper-rs project"),
        ("escape", "outside the workspace"),
        ("missing", "No such file"),
    ] {
        // Even forced, a failed bind changes nothing.
        let (ok, out) = call(&c, "bind_project", json!({"path": path, "force": true})).await;
        assert!(!ok && out.contains(why), "{path}: {out}");
        let (_, st) = call(&c, "openpaperdesign_status", json!({})).await;
        let st: Value = serde_json::from_str(&st).unwrap();
        assert_eq!(st["bound_project"], a_dir, "{path}: A is still bound");
        if let Some((port, token)) = &url {
            assert_eq!(
                http_status(*port, "/api/project", token).await,
                200,
                "{path}: A's editor works"
            );
        }
        let (ok, out) = call(&c, "get_basic_info", json!({})).await;
        assert!(ok && out.contains("\"A\""), "{path}: {out}");
    }
    assert_eq!(
        (snapshot(&ws), snapshot(&outside), snapshot(&far.dir)),
        before,
        "no files changed"
    );

    // Binding the bound project again changes nothing and needs no force.
    let (ok, out) = call(&c, "bind_project", json!({"path": "design/A"})).await;
    assert!(ok && out.contains("already bound"), "{out}");
    if let Some((port, token)) = &url {
        assert_eq!(http_status(*port, "/api/project", token).await, 200);
        // Switching to a valid project: refused without force, A untouched.
        let (ok, out) = call(&c, "bind_project", json!({"path": "design/B"})).await;
        assert!(
            !ok && out.contains("force: true") && out.contains("lost"),
            "{out}"
        );
        assert_eq!(http_status(*port, "/api/project", token).await, 200);
        // Forced: B is bound and A's editor server is stopped.
        let (ok, out) = call(
            &c,
            "bind_project",
            json!({"path": "design/B", "force": true}),
        )
        .await;
        assert!(ok, "{out}");
        let mut stopped = false;
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(("127.0.0.1", *port))
                .await
                .is_err()
            {
                stopped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
        assert!(stopped, "A's server still listens");
    } else {
        let (ok, out) = call(&c, "bind_project", json!({"path": "design/B"})).await;
        assert!(ok, "{out}");
    }
    let (_, st) = call(&c, "openpaperdesign_status", json!({})).await;
    assert!(
        st.contains(&b.dir.canonicalize().unwrap().display().to_string()),
        "{st}"
    );
    c.cancel().await.unwrap();
}

/// A host that starts the server in the installed plugin's folder: nothing is created there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_plugin_folder_is_not_a_workspace() {
    let t = tmp();
    let plugin = t.0.join("plugin");
    std::fs::create_dir_all(plugin.join("skills/openpaperdesign")).unwrap();
    std::fs::write(plugin.join("mcp.json"), "{}").unwrap();
    std::fs::write(plugin.join("skills/openpaperdesign/SKILL.md"), "x").unwrap();
    let home = t.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let c = launcher(&plugin, &home).await;
    let (ok, out) = call(&c, "create_project", json!({"name": "X"})).await;
    assert!(!ok && out.contains("plugin folder"), "{out}");
    assert!(!plugin.join("design").exists());
    let (_, st) = call(&c, "openpaperdesign_status", json!({})).await;
    assert!(st.contains("\"workspace_usable\": false"), "{st}");
    c.cancel().await.unwrap();
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        let m = std::fs::symlink_metadata(&p).unwrap();
        if m.is_dir() {
            out.extend(walk(&p));
        } else if m.is_file() {
            out.push(p);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn home_folder_workspace_and_escaping_design_folder_are_refused() {
    let t = tmp();
    // The workspace is the home folder: nothing may be bound or created.
    let home = t.0.join("home");
    let (p, _) = Store::open(home.join("design"))
        .unwrap()
        .create("Mine")
        .unwrap();
    let c = launcher(&home, &home).await;
    let (ok, out) = call(&c, "bind_project", json!({"path": "design/Mine"})).await;
    assert!(!ok && out.contains("home folder"), "{out}");
    let (ok, out) = call(&c, "create_project", json!({"name": "X"})).await;
    assert!(!ok && out.contains("home folder"), "{out}");
    assert!(p.dir.is_dir());
    c.cancel().await.unwrap();

    // design/ is a symlink leading out of the workspace.
    let ws = t.0.join("ws2");
    std::fs::create_dir_all(&ws).unwrap();
    let outside = t.0.join("outside-design");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, ws.join("design")).unwrap();
    let other_home = t.0.join("other-home");
    std::fs::create_dir_all(&other_home).unwrap();
    let c = launcher(&ws, &other_home).await;
    let (ok, out) = call(&c, "create_project", json!({"name": "Leak"})).await;
    assert!(!ok && out.contains("not a real folder"), "{out}");
    assert_eq!(
        std::fs::read_dir(&outside).unwrap().count(),
        0,
        "nothing created outside"
    );
    c.cancel().await.unwrap();
}

/// build.rs embeds exactly the files the server's bundle allowlist serves.
#[test]
fn embedded_bundle_list_matches_the_server_allowlist() {
    let build = include_str!("../build.rs");
    let start = build.find("const FILES").unwrap();
    let list = &build[start..build[start..].find("];").unwrap() + start];
    let mut embedded: Vec<&str> = list.split('"').skip(1).step_by(2).collect();
    let mut served = openpaperdesign_serve::bundle_file_names();
    embedded.sort();
    served.sort();
    assert_eq!(embedded, served);
}
