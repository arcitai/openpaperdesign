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

    // create_project makes design/<name> and switches to it.
    let (ok, out) = call(&c, "create_project", json!({"name": "New board"})).await;
    assert!(ok, "{out}");
    assert!(ws.join("design/New board/project.json").is_file());
    let (_, status) = call(&c, "openpaperdesign_status", json!({})).await;
    assert!(status.contains("New board"), "{status}");
    let (ok, out) = call(&c, "create_project", json!({"name": "New board"})).await;
    assert!(!ok && out.contains("already exists"), "{out}");
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
    assert!(!ok && out.contains("outside the workspace"), "{out}");
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
