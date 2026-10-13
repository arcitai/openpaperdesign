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
    /// The packaged launcher with only `bin` on its PATH (where a fake `codex` may live).
    async fn host_with_path(&self, bin: &Path) -> Client {
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_openpaperdesign-mcp"));
        cmd.current_dir(&self.plugin)
            .env("HOME", &self.home)
            .env("PATH", bin)
            .arg("--codex-context");
        ().serve(rmcp::transport::TokioChildProcess::new(cmd).unwrap())
            .await
            .unwrap()
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

/// Every pure home call, under each metadata variant: it succeeds, refuses extra arguments,
/// serves only the exact view, and discloses neither the workspace nor the editor token.
async fn pure_home(c: &Client, metas: &[RequestMetaObject], ws: &str, url: Option<&str>) {
    const HOME: &str = "ui://openpaperdesign/home.html";
    for meta in metas {
        let (ok, s) = call(c, "openpaperdesign_home", json!({}), meta.clone()).await;
        assert!(ok, "{s}");
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["surface"], "home");
        for (args, why) in [
            (json!({"path":"design/Canvas"}), "takes no arguments"),
            (
                json!({"_meta":{"codex/sandbox-state-meta":{}}}),
                "takes no arguments",
            ),
        ] {
            let (ok, s) = call(c, "openpaperdesign_home", args, meta.clone()).await;
            assert!(!ok && s.contains(why), "{s}");
        }
        let mut read = rmcp::model::ReadResourceRequestParams::new(HOME);
        read.meta = Some(meta.clone());
        let r = serde_json::to_value(c.read_resource(read).await.unwrap()).unwrap();
        assert_eq!(r["contents"][0]["uri"], HOME);
        assert_eq!(r["contents"][0]["mimeType"], "text/html;profile=mcp-app");
        let html = r["contents"][0]["text"].as_str().unwrap();
        assert!(html.contains("OpenPaperDesign") && html.contains("ui/initialize"));
        for other in [
            "ui://openpaperdesign/home.html?x",
            "ui://openpaperdesign/other.html",
            "file:///etc/passwd",
        ] {
            let mut read = rmcp::model::ReadResourceRequestParams::new(other);
            read.meta = Some(meta.clone());
            assert!(c.read_resource(read).await.is_err(), "{other}");
        }
        assert!(!s.contains(ws) && !html.contains(ws));
        if let Some(url) = url {
            let token = url.split("#token=").nth(1).unwrap();
            assert!(!s.contains(token) && !html.contains(token));
        }
    }
}

/// The global home tool and its view are answered without workspace admission and leave the
/// session's workspace state alone. Every other tool keeps per-call admission and retirement.
#[tokio::test]
async fn synthetic_home_and_view_are_pure_and_leave_the_pinned_binding_alone() {
    const HOME: &str = "ui://openpaperdesign/home.html";
    let f = Fixture::new();
    let c = f.host().await;

    // Registration: one global entry with a strict empty schema and the static mcp-app view.
    let tools: Vec<Value> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();
    assert_eq!(tools.len(), 35, "4 own + home + projects + 29 offered");
    let tool = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap().clone();
    let home = tool("openpaperdesign_home");
    assert_eq!(
        home["_meta"],
        json!({"ui":{"resourceUri":HOME},"openai/ui":{"entrypoints":[{"type":"global"}]}})
    );
    assert_eq!(
        home["inputSchema"],
        json!({"type":"object","properties":{},"additionalProperties":false})
    );
    for n in ["openpaperdesign_status", "open_editor"] {
        assert_eq!(
            tool(n)["_meta"],
            json!({"ui":{"resourceUri":HOME,"visibility":["model"]}}),
            "{n}"
        );
    }
    let entries = tools
        .iter()
        .filter(|t| t["_meta"].get("openai/ui").is_some())
        .count();
    assert_eq!(entries, 1, "only the home is an entrypoint");
    let resources: Vec<Value> = c
        .list_all_resources()
        .await
        .unwrap()
        .into_iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .collect();
    assert_eq!(
        resources,
        vec![
            json!({"uri":HOME,"name":"openpaperdesign-home","title":"OpenPaperDesign","mimeType":"text/html;profile=mcp-app"})
        ]
    );

    // Pure calls, with any or no metadata. None of them may disclose the workspace or token.
    let mut fake = context::writable(&f.home);
    fake.insert("thread_id".into(), json!("thread-from-another-chat"));
    let mut malformed = context::writable(&f.ws);
    malformed.get_mut(context::KEY).unwrap()["sandboxCwd"] = json!(12);
    let metas = [
        RequestMetaObject::default(),
        fake,
        malformed,
        context::readonly(&f.ws),
    ];
    // Before any context: the home pins nothing.
    pure_home(&c, &metas, f.ws.to_str().unwrap(), None).await;
    assert_eq!(
        status(&c, RequestMetaObject::default()).await["workspace_state"],
        "unresolved"
    );

    // Pinned, bound and editor opened: the home leaves all of it as it was.
    let meta = context::writable(&f.ws);
    let (ok, s) = call(&c, "create_project", json!({"name":"Canvas"}), meta.clone()).await;
    assert!(ok, "{s}");
    let before = status(&c, meta.clone()).await;
    let (ok, opened) = call(&c, "open_editor", json!({}), meta.clone()).await;
    let url = if before["browser_editor"] == "built in" {
        assert!(ok, "{opened}");
        let v: Value = serde_json::from_str(&opened).unwrap();
        Some(v["url"].as_str().unwrap().to_string())
    } else {
        assert!(!ok && opened.contains("Build the web bundle"), "{opened}");
        None
    };
    let files = |dir: &Path| {
        let mut all = vec![];
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p.clone());
                }
                all.push((p.clone(), std::fs::read(&p).ok()));
            }
        }
        all.sort();
        all
    };
    let on_disk = files(&f.root);
    pure_home(&c, &metas, f.ws.to_str().unwrap(), url.as_deref()).await;
    assert_eq!(files(&f.root), on_disk, "the home changed no file");
    let after = status(&c, meta.clone()).await;
    assert_eq!(after["workspace_state"], "pinned");
    assert_eq!(after["bound_project"], before["bound_project"]);
    assert_eq!(after["editor_opened"], url.is_some());
    if let Some(url) = &url {
        let (ok, again) = call(&c, "open_editor", json!({}), meta.clone()).await;
        assert!(ok, "{again}");
        let v: Value = serde_json::from_str(&again).unwrap();
        assert_eq!(v["url"].as_str().unwrap(), url, "same server, same session");
    }

    // An existing tool without context is still refused and still retires the session.
    let (ok, s) = call(&c, "openpaperdesign_status", json!({}), Default::default()).await;
    assert!(ok && s.contains("\"retired\""), "{s}");
    let retired = status(&c, meta.clone()).await;
    assert_eq!(retired["workspace_state"], "retired");
    assert_eq!(retired["bound_project"], Value::Null);
    let (ok, s) = call(&c, "open_editor", json!({}), meta.clone()).await;
    assert!(!ok, "{s}");
    // The pure home keeps working after retirement and revives nothing.
    pure_home(&c, &metas, f.ws.to_str().unwrap(), url.as_deref()).await;
    assert_eq!(status(&c, meta).await["workspace_state"], "retired");
    f.no_cache_writes();
    c.cancel().await.unwrap();
}

/// The native project inventory is read through `codex app-server` found on PATH, before
/// workspace admission and without touching the session: a pinned, bound session with its
/// editor open is unchanged, and listed roots never become the workspace.
#[tokio::test]
async fn synthetic_native_inventory_is_read_only_discovery_beside_the_pinned_binding() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let bin = f.root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let log = f.root.join("app-server.log");
    let acme = r#"{"id":"prj_fixture_acme","name":"Acme site","createdAt":1,"updatedAt":2,"position":0,"recencyAt":null,"metadata":{},"roots":[{"path":"/w/acme"},{"path":"/w/acme-docs"}]}"#;
    // Shell builtins only: PATH holds nothing but this directory.
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = app-server ] || exit 64\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> '{log}'\n  case \"$line\" in\n    *'\"method\":\"initialize\"'*) printf '%s\\n' '{{\"id\":1,\"result\":{{\"codexHome\":\"/fixture-home/.codex\",\"platformFamily\":\"unix\",\"platformOs\":\"linux\",\"userAgent\":\"codex_cli_rs/0.162.1\"}}}}' ;;\n    *'\"method\":\"project/list\"'*) printf '%s\\n' '{{\"id\":2,\"result\":{{\"data\":[{acme}],\"nextCursor\":null}}}}' ;;\n    *'\"method\":\"project/read\"'*) printf '%s\\n' '{{\"id\":2,\"result\":{{\"project\":{acme}}}}}' ;;\n  esac\ndone\n",
        log = log.display()
    );
    std::fs::write(bin.join("codex"), script).unwrap();
    std::fs::set_permissions(bin.join("codex"), std::fs::Permissions::from_mode(0o755)).unwrap();

    let c = f.host_with_path(&bin).await;
    let tools: Vec<Value> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();
    let inv = tools
        .iter()
        .find(|t| t["name"] == "openpaperdesign_projects")
        .unwrap();
    assert_eq!(
        inv["inputSchema"],
        json!({"type":"object","properties":{"projectId":{"type":"string","maxLength":256,"description":"Read just this project (an id from the list)."}},"additionalProperties":false})
    );
    assert!(
        inv.get("_meta").is_none(),
        "no entrypoint or view of its own"
    );

    let meta = context::writable(&f.ws);
    let (ok, s) = call(&c, "create_project", json!({"name":"Canvas"}), meta.clone()).await;
    assert!(ok, "{s}");
    let before = status(&c, meta.clone()).await;
    let (_, opened) = call(&c, "open_editor", json!({}), meta.clone()).await;
    let token = opened.split("#token=").nth(1).map(|t| t[..64].to_string());

    let want = json!({
        "available": true, "source": "codex-app-server", "host": "this computer", "complete": true,
        "projects": [{"source":"codex-app-server","host":"this computer","id":"prj_fixture_acme","name":"Acme site","position":0,"recencyAt":null,"roots":[{"path":"/w/acme"},{"path":"/w/acme-docs"}]}]
    });
    let mut fake = context::writable(&f.home);
    fake.insert("thread_id".into(), json!("thread-from-another-chat"));
    for m in [RequestMetaObject::default(), fake, context::readonly(&f.ws)] {
        let (ok, s) = call(&c, "openpaperdesign_projects", json!({}), m.clone()).await;
        assert!(ok, "{s}");
        assert_eq!(serde_json::from_str::<Value>(&s).unwrap(), want);
        assert!(!s.contains(f.ws.to_str().unwrap()) && !s.contains("fixture-home"));
        if let Some(token) = &token {
            assert!(!s.contains(token.as_str()));
        }
        let (ok, s) = call(
            &c,
            "openpaperdesign_projects",
            json!({"projectId":"prj_fixture_acme"}),
            m.clone(),
        )
        .await;
        assert!(ok, "{s}");
        assert_eq!(
            serde_json::from_str::<Value>(&s).unwrap()["projects"],
            want["projects"]
        );
        for (args, why) in [
            (
                json!({"path": f.ws.to_str().unwrap()}),
                "accepts only projectId",
            ),
            (json!({"projectId": 7}), "projectId must be"),
            (json!({"projectId": ""}), "projectId must be"),
        ] {
            let (ok, s) = call(&c, "openpaperdesign_projects", args, m.clone()).await;
            assert!(!ok && s.contains(why), "{s}");
        }
    }
    // Only initialize, initialized and read-only project calls reached the app-server.
    let methods: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| {
            serde_json::from_str::<Value>(l).unwrap()["method"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert!(!methods.is_empty());
    assert!(
        methods.iter().all(
            |m| ["initialize", "initialized", "project/list", "project/read"].contains(&m.as_str())
        ),
        "{methods:?}"
    );

    // The session is exactly as it was: same workspace, project and editor server.
    let after = status(&c, meta.clone()).await;
    assert_eq!(after["workspace_state"], "pinned");
    assert_eq!(after["workspace"], before["workspace"]);
    assert_eq!(after["bound_project"], before["bound_project"]);
    if token.is_some() {
        let (ok, again) = call(&c, "open_editor", json!({}), meta.clone()).await;
        assert!(ok && again == opened, "{again}");
    }
    // A listed root never becomes the workspace: binding outside it is still refused.
    let (ok, s) = call(&c, "bind_project", json!({"path":"/w/acme"}), meta.clone()).await;
    assert!(!ok, "{s}");
    f.no_cache_writes();
    c.cancel().await.unwrap();

    // No codex on PATH: an explicit unavailable answer, and the session is untouched.
    let empty = f.root.join("empty-bin");
    std::fs::create_dir(&empty).unwrap();
    let c = f.host_with_path(&empty).await;
    let (ok, s) = call(&c, "openpaperdesign_projects", json!({}), meta.clone()).await;
    assert!(ok, "{s}");
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["available"], false);
    assert_eq!(v["reason"], "codex_not_found");
    assert_eq!(status(&c, meta).await["workspace_usable"], true);
    c.cancel().await.unwrap();
}
