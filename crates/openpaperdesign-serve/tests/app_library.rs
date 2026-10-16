//! The app library (`openpaperdesign-mcp --app-library <dir>`) as real child processes sharing
//! one library folder, driven with the rmcp client and real loopback HTTP (the editor's API).
//! No request carries Codex metadata. This is a synthetic host: no browser renderer, no Codex
//! navigation, no native plugin data root, and not a Mac receipt.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ContentBlock};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp() -> Tmp {
    use std::hash::{BuildHasher, RandomState};
    let d = std::env::temp_dir().join(format!(
        "opd-app-library-it-{:016x}",
        RandomState::new().hash_one(std::time::SystemTime::now())
    ));
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d.canonicalize().unwrap())
}

type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;

fn command(root: &Path, home: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_openpaperdesign-mcp"));
    // A working directory that isn't the library: it must not matter.
    cmd.current_dir(home)
        .env("HOME", home)
        .env_remove("PLUGIN_ROOT")
        .arg("--app-library")
        .arg(root);
    cmd
}

async fn library(root: &Path, home: &Path) -> Client {
    ().serve(rmcp::transport::TokioChildProcess::new(command(root, home)).unwrap())
        .await
        .unwrap()
}

async fn call(c: &Client, name: &str, args: Value) -> Result<Value, String> {
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
    if r.is_error == Some(true) {
        Err(text)
    } else {
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
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
    (status, etag, body.to_string())
}

/// The browser editor's load-change-save of the active page; returns the page id.
async fn edit(port: u16, token: &str) -> String {
    let (_, _, project) = http(port, "GET", "/api/project", token, None, "").await;
    let page = serde_json::from_str::<Value>(&project).unwrap()["active_page"]
        .as_str()
        .unwrap()
        .to_string();
    let path = format!("/api/pages/{page}");
    let (st, tag, body) = http(port, "GET", &path, token, None, "").await;
    assert_eq!(st, 200, "{body}");
    let mut data = paper_rs::project::format::page_from_json(&body).unwrap();
    let id = data.doc.alloc_id();
    data.doc.nodes.insert(
        id,
        paper_rs::model::Node::new(
            id,
            paper_rs::model::NodeKind::Rect,
            paper_rs::geom::Rect::new(1.0, 1.0, 20.0, 20.0),
        ),
    );
    data.doc.root.push(id);
    let body = paper_rs::project::format::page_to_json(&data);
    let (st, _, answer) = http(port, "PUT", &path, token, tag.as_deref(), &body).await;
    assert_eq!(st, 200, "{answer}");
    page
}

async fn nodes(port: u16, token: &str, page: &str) -> usize {
    let (st, _, body) = http(port, "GET", &format!("/api/pages/{page}"), token, None, "").await;
    assert_eq!(st, 200, "{body}");
    paper_rs::project::format::page_from_json(&body)
        .unwrap()
        .doc
        .nodes
        .len()
}

/// Open `id`, waiting (bounded) while another process still holds it.
async fn open_when_free(c: &Client, id: &str) -> Value {
    for _ in 0..100 {
        match call(c, "openpaperdesign_app_open", json!({"id": id})).await {
            Ok(v) => return v,
            Err(e) if e.contains("another OpenPaperDesign window") => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => panic!("{e}"),
        }
    }
    panic!("the canvas stayed busy");
}

/// Two processes on one library: create and edit in one, busy in the other until closed,
/// a different canvas meanwhile, then the saved edit in a fresh process; an ended process
/// releases its canvas. No request carries metadata.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_processes_share_the_library_one_canvas_at_a_time() {
    let t = tmp();
    let (root, home) = (t.0.join("data"), t.0.join("home"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let a = library(&root, &home).await;
    let b = library(&root, &home).await;
    let created = call(&a, "openpaperdesign_app_create", json!({"name": "Shared"}))
        .await
        .unwrap();
    let id = created["canvas"]["id"].as_str().unwrap().to_string();
    let (port, token) = url(&created);
    let page = edit(port, &token).await;
    let e = call(&b, "openpaperdesign_app_open", json!({"id": id}))
        .await
        .unwrap_err();
    assert!(e.contains("another OpenPaperDesign window"), "{e}");
    let other = call(&b, "openpaperdesign_app_create", json!({"name": "Other"}))
        .await
        .unwrap();
    let other_id = other["canvas"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        nodes(port, &token, &page).await,
        1,
        "A's canvas is unaffected"
    );
    call(&a, "openpaperdesign_app_close", json!({"force": true}))
        .await
        .unwrap();
    let in_b = call(
        &b,
        "openpaperdesign_app_open",
        json!({"id": id, "force": true}),
    )
    .await
    .unwrap();
    let (pb, tb) = url(&in_b);
    assert_eq!(
        nodes(pb, &tb, &page).await,
        1,
        "the saved edit, in another process"
    );
    // B ends without closing: its canvas becomes free for a fresh process.
    b.cancel().await.unwrap();
    let c = library(&root, &home).await;
    let reopened = open_when_free(&c, &id).await;
    let (pc, tc) = url(&reopened);
    assert_eq!(nodes(pc, &tc, &page).await, 1);
    let listed = call(&c, "openpaperdesign_app_canvases", json!({}))
        .await
        .unwrap();
    let ids: Vec<&str> = listed["canvases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&id.as_str()) && ids.contains(&other_id.as_str()));
    // Project routes don't exist here, metadata or not.
    let e = call(&c, "bind_project", json!({"path": root}))
        .await
        .unwrap_err();
    assert!(e.contains("runs the app library"), "{e}");
    a.cancel().await.unwrap();
    c.cancel().await.unwrap();
}

/// Launch refusals: conflicting flags, an unexpanded placeholder, a relative or dotted path,
/// a missing folder, a link, the package root and the package cache. Each exits non-zero
/// before serving anything and creates nothing.
#[tokio::test]
async fn unusable_library_folders_are_refused_at_launch() {
    let t = tmp();
    let home = t.0.join("home");
    let real = t.0.join("data");
    let package = t.0.join("package");
    let cache = t.0.join("codex/plugins/cache/x");
    let fallback = package.join(".plugin-data");
    for d in [&home, &real, &fallback, &cache] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::os::unix::fs::symlink(&real, t.0.join("link")).unwrap();
    let bin = env!("CARGO_BIN_EXE_openpaperdesign-mcp");
    let run = |args: &[&str], plugin_root: Option<&Path>| {
        let mut cmd = std::process::Command::new(bin);
        cmd.current_dir(&home)
            .env("HOME", &home)
            .env_remove("PLUGIN_ROOT");
        if let Some(p) = plugin_root {
            cmd.env("PLUGIN_ROOT", p);
        }
        let out = cmd.args(args).arg("doctor").output().unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let real_s = real.to_str().unwrap();
    for (args, plugin_root, want) in [
        (
            vec!["--app-library", real_s, "--codex-context"],
            None,
            "cannot be repeated or combined",
        ),
        (
            vec!["--app-library", real_s, "--app-library", real_s],
            None,
            "cannot be repeated",
        ),
        (vec!["--app-library", "${PLUGIN_DATA}"], None, "placeholder"),
        (vec!["--app-library", "data"], None, "plain absolute path"),
        (
            vec!["--app-library", "/tmp/../tmp"],
            None,
            "plain absolute path",
        ),
        (vec!["--app-library", "/"], None, "filesystem root"),
        (
            vec!["--app-library", "--workspace", real_s],
            None,
            "cannot be repeated or combined",
        ),
    ] {
        let (code, err) = run(&args, plugin_root);
        assert_eq!(code, Some(2), "{args:?}: {err}");
        assert!(err.contains(want), "{args:?}: {err}");
    }
    let missing = t.0.join("missing");
    let link = t.0.join("link");
    for (dir, plugin_root, want) in [
        (&missing, None, "No such file"),
        (&link, None, "not a link"),
        (&home, None, "home folder"),
        (&fallback, Some(package.as_path()), "PLUGIN_ROOT"),
        (&fallback, None, ".plugin-data"),
        (&cache, None, "plugins/cache"),
    ] {
        let (code, err) = run(&["--app-library", dir.to_str().unwrap()], plugin_root);
        assert_eq!(code, Some(2), "{dir:?}: {err}");
        assert!(err.contains(want), "{dir:?}: {err}");
    }
    assert!(!missing.exists());
    assert!(
        std::fs::read_dir(&real).unwrap().next().is_none(),
        "nothing created"
    );
    // The usable folder passes doctor's library check (the editor may not be built here).
    let mut cmd = std::process::Command::new(bin);
    let out = cmd
        .current_dir(&home)
        .env("HOME", &home)
        .env_remove("PLUGIN_ROOT")
        .args(["--app-library", real_s, "doctor"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["mode"], "app-library");
    assert_eq!(report["library_usable"], true);
    assert!(
        std::fs::read_dir(&real).unwrap().next().is_none(),
        "doctor creates nothing"
    );
}
