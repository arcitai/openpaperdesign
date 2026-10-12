//! Actual child-process stdio, synthetic host metadata. Real Codex model/IAB proof is
//! an operator gate; direct app-server tool calls do not inject this metadata.
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ContentBlock, RequestMetaObject};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[path = "support/codex_context.rs"]
mod context;

type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;
struct Fixture {
    root: PathBuf,
    ws: PathBuf,
    plugin: PathBuf,
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        use std::hash::{BuildHasher, RandomState};
        let root = std::env::temp_dir().join(format!(
            "opd-context-{:016x}",
            RandomState::new().hash_one(std::time::SystemTime::now())
        ));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let ws = root.join("workspace with spaces é");
        let plugin = root.join("plugin");
        let home = root.join("home");
        for dir in [&ws, &home, &plugin.join("skills/openpaperdesign")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(plugin.join("mcp.json"), "{}").unwrap();
        std::fs::write(plugin.join("skills/openpaperdesign/SKILL.md"), "fixture").unwrap();
        Self {
            root,
            ws,
            plugin,
            home,
        }
    }
    async fn launch(&self, args: &[&str]) -> Client {
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_openpaperdesign-mcp"));
        cmd.current_dir(&self.plugin)
            .env("HOME", &self.home)
            .args(args);
        ().serve(rmcp::transport::TokioChildProcess::new(cmd).unwrap())
            .await
            .unwrap()
    }
    async fn host(&self) -> Client {
        self.launch(&["--codex-context"]).await
    }
    fn no_cache_writes(&self) {
        assert!(!self.plugin.join("design").exists());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
async fn call(c: &Client, name: &str, args: Value, meta: RequestMetaObject) -> (bool, String) {
    let mut p = CallToolRequestParams::new(name.to_string());
    p.arguments = args.as_object().cloned();
    p.meta = Some(meta);
    let r = tokio::time::timeout(std::time::Duration::from_secs(20), c.call_tool(p))
        .await
        .unwrap()
        .unwrap();
    let s = r
        .content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (r.is_error != Some(true), s)
}
async fn status(c: &Client, meta: RequestMetaObject) -> Value {
    let (ok, s) = call(c, "openpaperdesign_status", json!({}), meta).await;
    assert!(ok, "{s}");
    serde_json::from_str(&s).unwrap()
}

#[tokio::test]
async fn synthetic_context_from_plugin_cwd_selects_only_the_workspace() {
    let f = Fixture::new();
    let c = f.host().await;
    assert_eq!(
        c.peer_info()
            .unwrap()
            .capabilities
            .experimental
            .as_ref()
            .unwrap()[context::KEY],
        serde_json::Map::new()
    );
    let absent = status(&c, Default::default()).await;
    assert_eq!(absent["workspace_state"], "unresolved");
    assert_eq!(absent["workspace_usable"], false);
    let meta = context::writable(&f.ws);
    let s = status(&c, meta.clone()).await;
    assert_eq!(s["workspace"], f.ws.to_str().unwrap());
    assert_eq!(s["workspace_source"], context::KEY);
    assert_eq!(s["workspace_permission_mode"], "managed-workspace-write");
    assert_eq!(s["workspace_usable"], true);
    let (ok, s) = call(&c, "create_project", json!({"name":"Canvas", "_meta":{"codex/sandbox-state-meta":{"sandboxCwd":context::uri(&f.home)}}}), meta.clone()).await;
    assert!(ok, "{s}");
    let p = f.ws.join("design/Canvas");
    assert!(p.join("project.json").is_file());
    let (ok, s) = call(
        &c,
        "get_tree_summary",
        json!({"nodeId":"root"}),
        meta.clone(),
    )
    .await;
    assert!(ok, "{s}");
    let (ok, s) = call(
        &c,
        "create_artboard",
        json!({"name":"Scoped","styles":{"width":"300px","height":"200px"}}),
        meta.clone(),
    )
    .await;
    assert!(ok, "{s}");
    let before = std::fs::read(p.join("project.json")).unwrap();
    let (ok, s) = call(
        &c,
        "bind_project",
        json!({"path":"design/Canvas"}),
        meta.clone(),
    )
    .await;
    assert!(ok && s.contains("already bound"), "{s}");
    for path in ["../home", "/", "design/missing"] {
        assert!(
            !call(&c, "bind_project", json!({"path":path}), meta.clone())
                .await
                .0
        );
        assert_eq!(
            status(&c, meta.clone()).await["bound_project"],
            p.to_str().unwrap()
        );
    }
    assert_eq!(std::fs::read(p.join("project.json")).unwrap(), before);
    f.no_cache_writes();
    c.cancel().await.unwrap();
}

#[tokio::test]
async fn synthetic_readonly_unknown_and_argument_metadata_never_authorize_writes() {
    let f = Fixture::new();
    let c = f.host().await;
    let writable = context::writable(&f.ws);
    let mut cases = vec![Default::default(), context::readonly(&f.ws)];
    for profile in [
        Value::Null,
        json!({}),
        json!({"type":"managed"}),
        json!({"type":"disabled","extra":true}),
        json!({"type":"external","network":"restricted"}),
    ] {
        let mut m = writable.clone();
        m.get_mut(context::KEY).unwrap()["permissionProfile"] = profile;
        cases.push(m);
    }
    for network in [
        None,
        Some(Value::Null),
        Some(json!(true)),
        Some(json!("custom")),
        Some(json!({"enabled":true})),
    ] {
        let mut m = writable.clone();
        let profile = m.get_mut(context::KEY).unwrap()["permissionProfile"]
            .as_object_mut()
            .unwrap();
        if let Some(network) = network {
            profile.insert("network".into(), network);
        } else {
            profile.remove("network");
        }
        cases.push(m);
    }
    for modification in 0..6 {
        let mut m = writable.clone();
        let entries =
            m.get_mut(context::KEY).unwrap()["permissionProfile"]["file_system"]["entries"]
                .as_array_mut()
                .unwrap();
        match modification {
            0 => {
                entries.remove(4);
            }
            1 => entries.push(entries[1].clone()),
            2 => entries[0]["access"] = "write".into(),
            3 => entries[1]["path"]["path"] = f.root.to_str().unwrap().into(),
            4 => entries[4]["access"] = "write".into(),
            _ => entries.push(
                json!({"path":{"type":"glob_pattern","pattern":"**/*.secret"},"access":"deny"}),
            ),
        }
        cases.push(m);
    }
    for meta in cases {
        let s = status(&c, meta.clone()).await;
        assert_eq!(s["workspace_usable"], false, "{s}");
        for tool in [
            "create_project",
            "bind_project",
            "open_editor",
            "create_artboard",
        ] {
            let args = json!({"name":"Forbidden","path":".","sandboxCwd":context::uri(&f.ws),"_meta":&*writable});
            let (ok, s) = call(&c, tool, args, meta.clone()).await;
            assert!(!ok, "{tool}: {s}");
        }
    }
    assert!(!f.ws.join("design").exists());
    f.no_cache_writes();
    // A valid context can resolve a session which never established authority.
    assert_eq!(status(&c, writable).await["workspace_usable"], true);
    c.cancel().await.unwrap();
}

#[tokio::test]
async fn synthetic_scope_rejects_uri_guards_and_retires_on_identity_or_context_loss() {
    let f = Fixture::new();
    let c = f.host().await;
    for uri in [
        "file://remote/tmp",
        "file:///tmp?query",
        "file:///tmp#fragment",
        "file:///tmp/%00",
        "file:///tmp/%2fescape",
        "file:///tmp/%2E%2E",
        "file:///tmp/%FF",
        "file:/tmp",
        "https://host/path",
        "file:///tmp/../outside",
        "file:///tmp/./path",
        "file:////tmp",
        "file://user@localhost/tmp",
        "file://localhost:12/tmp",
        "file:///tmp/%",
        "file:///tmp/%1Z",
    ] {
        let mut m = context::writable(&f.ws);
        m.get_mut(context::KEY).unwrap()["sandboxCwd"] = uri.into();
        assert_eq!(status(&c, m).await["workspace_usable"], false, "{uri}");
    }
    for path in [
        Path::new("/"),
        &f.home,
        &f.plugin,
        &f.ws.join("missing"),
        &f.plugin.join("mcp.json"),
    ] {
        assert_eq!(
            status(&c, context::disabled(path)).await["workspace_usable"],
            false
        );
    }
    c.cancel().await.unwrap();
    for case in ["missing", "changed", "readonly", "malformed", "replaced"] {
        let c = f.host().await;
        assert_eq!(
            status(&c, context::writable(&f.ws)).await["workspace_usable"],
            true
        );
        let bad = match case {
            "missing" => Default::default(),
            "changed" => context::writable(&f.root),
            "readonly" => context::readonly(&f.ws),
            "malformed" => {
                let mut m = context::writable(&f.ws);
                m.get_mut(context::KEY).unwrap()["sandboxCwd"] = json!(12);
                m
            }
            _ => {
                std::fs::rename(&f.ws, f.root.join("old")).unwrap();
                std::fs::create_dir(&f.ws).unwrap();
                context::writable(&f.ws)
            }
        };
        assert_eq!(
            status(&c, bad).await["workspace_state"],
            "retired",
            "{case}"
        );
        assert_eq!(
            status(&c, context::writable(&f.ws)).await["workspace_usable"],
            false,
            "{case}"
        );
        c.cancel().await.unwrap();
    }
    f.no_cache_writes();
}

#[tokio::test]
async fn synthetic_disabled_profile_and_protected_paths_keep_plugin_scope() {
    for disabled in [false, true] {
        let f = Fixture::new();
        let c = f.host().await;
        let meta = if disabled {
            context::disabled(&f.ws)
        } else {
            context::writable(&f.ws)
        };
        assert_eq!(
            status(&c, meta.clone()).await["workspace_permission_mode"],
            if disabled {
                "host-sandbox-disabled"
            } else {
                "managed-workspace-write"
            }
        );
        let (ok, s) = call(
            &c,
            "create_project",
            json!({"name":"Allowed"}),
            meta.clone(),
        )
        .await;
        assert!(ok, "{s}");
        for protected in [".git", ".agents", ".codex", ".aws"] {
            let (p, _) = paper_rs::project::Store::open(f.ws.join(protected))
                .unwrap()
                .create("Protected")
                .unwrap();
            let (ok, s) = call(&c, "bind_project", json!({"path":p.dir}), meta.clone()).await;
            assert!(!ok && s.contains("read-only"), "{s}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(f.ws.join(".codex"), f.ws.join("alias")).unwrap();
            assert!(
                !call(
                    &c,
                    "bind_project",
                    json!({"path":"alias/Protected"}),
                    meta.clone()
                )
                .await
                .0
            );
            symlink(&f.home, f.ws.join("escape")).unwrap();
            assert!(
                !call(&c, "bind_project", json!({"path":"escape"}), meta.clone())
                    .await
                    .0
            );
        }
        assert!(
            !call(&c, "bind_project", json!({"path":f.home}), meta)
                .await
                .0
        );
        f.no_cache_writes();
        c.cancel().await.unwrap();
    }
}

#[tokio::test]
async fn synthetic_canonical_aliases_and_standalone_mode_remain_deliberate() {
    let f = Fixture::new();
    #[cfg(unix)]
    {
        let alias = f.root.join("alias");
        std::os::unix::fs::symlink(&f.ws, &alias).unwrap();
        let c = f.host().await;
        assert_eq!(
            status(&c, context::writable(&alias)).await["workspace"],
            f.ws.to_str().unwrap()
        );
        assert_eq!(
            status(&c, context::writable(&f.ws)).await["workspace_usable"],
            true
        );
        let mut local = context::writable(&f.ws);
        local.get_mut(context::KEY).unwrap()["sandboxCwd"] = context::uri(&f.ws)
            .replacen("file://", "file://localhost", 1)
            .into();
        assert_eq!(status(&c, local).await["workspace_usable"], true);
        c.cancel().await.unwrap();
    }
    let c = f.launch(&["--workspace", f.ws.to_str().unwrap()]).await;
    let s = status(&c, Default::default()).await;
    assert_eq!(s["workspace"], f.ws.to_str().unwrap());
    assert_eq!(s["workspace_source"], "explicit --workspace");
    assert!(
        call(
            &c,
            "create_project",
            json!({"name":"Standalone"}),
            Default::default()
        )
        .await
        .0
    );
    c.cancel().await.unwrap();
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_openpaperdesign-mcp"))
        .args(["--codex-context", "--workspace", f.ws.to_str().unwrap()])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot be"));
    f.no_cache_writes();
}

#[cfg(unix)]
#[tokio::test]
async fn synthetic_context_refuses_protected_aliases_and_escaping_design() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    symlink(&f.home, f.ws.join("design")).unwrap();
    let c = f.host().await;
    let (ok, s) = call(
        &c,
        "create_project",
        json!({"name":"Escape"}),
        context::writable(&f.ws),
    )
    .await;
    assert!(!ok && s.contains("not a real folder"), "{s}");
    assert!(!f.home.join("Escape").exists());
    c.cancel().await.unwrap();
    std::fs::remove_file(f.ws.join("design")).unwrap();
    // A protected link can alias an otherwise ordinary project. Refuse this
    // unsupported scope rather than lose the host's carveout by canonicalizing it.
    symlink(&f.home, f.ws.join(".codex")).unwrap();
    let c = f.host().await;
    assert_eq!(
        status(&c, context::writable(&f.ws)).await["workspace_usable"],
        false
    );
    assert_eq!(
        status(&c, context::disabled(&f.ws)).await["workspace_usable"],
        false
    );
    c.cancel().await.unwrap();
    std::fs::remove_file(f.ws.join(".codex")).unwrap();
    let protected = f.ws.join(".codex/workspace");
    std::fs::create_dir_all(&protected).unwrap();
    let c = f.host().await;
    assert_eq!(
        status(&c, context::disabled(&protected)).await["workspace_usable"],
        false
    );
    c.cancel().await.unwrap();
    symlink(&f.home, f.ws.join(".git")).unwrap();
    let c = f.host().await;
    assert_eq!(
        status(&c, context::writable(&f.ws)).await["workspace_usable"],
        false
    );
    assert_eq!(
        status(&c, context::disabled(&f.ws)).await["workspace_usable"],
        false
    );
    c.cancel().await.unwrap();
    f.no_cache_writes();
}

#[tokio::test]
async fn synthetic_worktree_observed_network_values_keep_the_same_scope() {
    let f = Fixture::new();
    let c = f.host().await;
    // These two strings are separately live-observed. Neither expands scope or
    // changes the adapter's loopback listener / absence of outbound requests.
    for network in ["restricted", "enabled"] {
        let mut meta = context::writable(&f.ws);
        meta.get_mut(context::KEY).unwrap()["permissionProfile"]["network"] = network.into();
        let s = status(&c, meta.clone()).await;
        assert_eq!(s["workspace_usable"], true, "{network}: {s}");
        assert_eq!(s["workspace"], f.ws.to_str().unwrap());
        assert!(
            !call(&c, "bind_project", json!({"path":f.home}), meta)
                .await
                .0
        );
    }
    f.no_cache_writes();
    c.cancel().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn synthetic_worktree_pointer_is_opaque_and_edits_stay_inside_workspace() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    // Faithful worktree pointer fixture, not a Git subprocess or live Codex host.
    // Snapshot every sibling file and directory so even an added file is detected.
    fn snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
        fn visit(dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Option<Vec<u8>>>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let path = e.unwrap().path();
                if path.is_dir() {
                    out.insert(path.clone(), None);
                    visit(&path, out);
                } else {
                    out.insert(path.clone(), Some(std::fs::read(path).unwrap()));
                }
            }
        }
        let mut out = Default::default();
        visit(root, &mut out);
        out
    }
    for profile in ["restricted", "enabled", "disabled"] {
        let f = Fixture::new();
        let sibling = f.root.join("main");
        let gitdir = sibling.join(".git/worktrees/task");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("sentinel"), b"external git metadata\n").unwrap();
        let (outside, _) = paper_rs::project::Store::open(sibling.join("design"))
            .unwrap()
            .create("Outside")
            .unwrap();
        let before_sibling = snapshot(&sibling);
        let pointer = f.ws.join(".git");
        let pointer_bytes = b"gitdir: ../main/.git/worktrees/task\n";
        std::fs::write(&pointer, pointer_bytes).unwrap();
        // The launcher must not read the pointer even when its bytes are unreadable.
        std::fs::set_permissions(&pointer, std::fs::Permissions::from_mode(0o000)).unwrap();
        symlink(&sibling, f.ws.join("outside-alias")).unwrap();
        let c = f.host().await;
        let mut meta = context::writable(&f.ws);
        if profile == "disabled" {
            meta = context::disabled(&f.ws);
        } else {
            meta.get_mut(context::KEY).unwrap()["permissionProfile"]["network"] = profile.into();
        }
        let s = status(&c, meta.clone()).await;
        assert_eq!(s["workspace_usable"], true, "{profile}: {s}");
        assert_eq!(s["workspace"], f.ws.to_str().unwrap());
        let (ok, s) = call(&c, "create_project", json!({"name":"Canvas"}), meta.clone()).await;
        assert!(ok, "{s}");
        let project = f.ws.join("design/Canvas");
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(project.join("project.json")).unwrap()).unwrap();
        let page = manifest["pages"][0]["id"].as_str().unwrap();
        let (ok, s) = call(
            &c,
            "get_tree_summary",
            json!({"nodeId":"root","pageId":page}),
            meta.clone(),
        )
        .await;
        assert!(ok, "{s}");
        let (ok, s) = call(&c, "create_artboard", json!({"name":"Scoped worktree canvas","pageId":page,"styles":{"width":"300px","height":"200px"}}), meta.clone()).await;
        assert!(ok, "{s}");
        let saved = std::fs::read(project.join("pages").join(format!("{page}.json"))).unwrap();
        assert!(String::from_utf8_lossy(&saved).contains("Scoped worktree canvas"));
        let (ok, s) = call(
            &c,
            "get_tree_summary",
            json!({"nodeId":"root","pageId":page}),
            meta.clone(),
        )
        .await;
        assert!(ok && s.contains("Scoped worktree canvas"), "{s}");
        for path in [
            ".git",
            ".git/../design/Canvas",
            ".agents",
            ".codex",
            ".aws",
            "outside-alias/design/Outside",
            outside.dir.to_str().unwrap(),
            gitdir.to_str().unwrap(),
        ] {
            let (ok, s) = call(&c, "bind_project", json!({"path":path}), meta.clone()).await;
            assert!(!ok, "{path}: {s}");
            assert_eq!(
                status(&c, meta.clone()).await["bound_project"],
                project.to_str().unwrap()
            );
        }
        assert_eq!(
            std::fs::read(project.join("pages").join(format!("{page}.json"))).unwrap(),
            saved
        );
        c.cancel().await.unwrap();
        std::fs::set_permissions(&pointer, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&pointer).unwrap(), pointer_bytes);
        assert_eq!(snapshot(&sibling), before_sibling);
        f.no_cache_writes();
    }
}
