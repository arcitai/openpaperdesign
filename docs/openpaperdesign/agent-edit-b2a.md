# B2a: an agent and a person editing one project — 2026-10-12

Scope: [issue #1](https://github.com/onlinesourdough/openpaperdesign/issues/1), slice B2a, on top
of B1 (`browser-edit-b1.md`). While `openpaperdesign-serve --project <dir>` runs and the page
is open in a browser, an AI agent connected over MCP edits the **same** project, and the
browser shows the agent's change without a manual reload. Edits in the browser are kept, and
overlapping edits are refused as a conflict, never overwritten either way.

Out of scope: Codex
plugin packaging, skills, discovery, the MCP App panel (B2b), several projects, remote use.

Updated 2026-10-12: the automatic three-way merge was dropped in favour of strict revisions
(see "Decision: strict revisions, no merge").

## Run it

```sh
./scripts/build-web.sh --offline
cargo run --locked -p openpaperdesign-serve -- --project "<project folder>"
```

The server prints the browser URL and the MCP settings, and writes them nowhere:

```
open http://127.0.0.1:<port>/#token=<token>
MCP (streamable HTTP): http://127.0.0.1:<port>/mcp  with header  Authorization: Bearer <token>
  Claude Code: claude mcp add --transport http openpaperdesign http://127.0.0.1:<port>/mcp --header "Authorization: Bearer <token>"
  Codex: export OPD_MCP_TOKEN=<token>; in ~/.codex/config.toml: [mcp_servers.openpaperdesign] url = "http://127.0.0.1:<port>/mcp" bearer_token_env_var = "OPD_MCP_TOKEN"
```

The MCP endpoint takes the per-launch token as `Authorization: Bearer <token>`, the same token
the page uses. Any MCP client that can send that header works. The Claude Code and Codex lines
follow those clients' documented settings, but **neither was run against this server here**;
check your client's docs if one is refused. The token changes on every start.

## Design

### One authoritative host for the bound project (`crates/openpaperdesign-serve/src/agent.rs`)

- **Reused upstream:** the tool catalogue (`mcp::server::tool_list`), result mapping
  (`to_result`) and every tool's implementation (`mcp::exec::call`). The upstream app's own MCP
  server is not used or changed.
- **Host thread:** one thread owns an upstream `Editor` holding the bound project's pages (an
  `Editor` isn't `Send`, the same reason upstream's `mcp::serve_host` uses a thread). Each tool
  call takes **the same write lock as browser saves**, then:
  1. re-checks the project root and entries, exactly like every browser request;
  2. reloads the project if any page file or the manifest changed on disk since the host last
     read or wrote it. A browser save is the usual cause. The agent's undo history then starts
     over;
  3. applies the policy below and runs the tool;
  4. saves each changed page through upstream `Project::save_page` (atomic write), updates the
     manifest like the native session does, and records the new revision.
- **One transaction per write:** each agent write is one upstream transaction, saved before the
  call returns, so it changes that page's revision (the hash of its file on disk).
- **Endpoint:** `/mcp` is on the same loopback listener. It gets the B1 guard (`Host` must be
  `127.0.0.1:<port>`/`localhost:<port>`, any `Origin` must be ours, fetch-site checks), requires
  the token, and gets the B1 security headers. rmcp's own Origin validation is also on, allowing
  only our two origins.

### Policy (fail closed, before any upstream code runs)

- **Refused tools:** `export` and `export_combined_pdf` (they write files where the agent says),
  and `create_file`. These aren't offered in `tools/list` either.
- **Bound project only:** `list_resources` lists only the bound project. `open_file` and any
  `fileId` argument accept only its id, name or path, and the adapter hands upstream only the
  bound path. Upstream's `find_project` would otherwise accept any absolute path containing a
  `project.json`.
- **Images:** `<img src>`/`srcset` and every CSS `url(...)` must be a `data:` URI, an http(s)
  URL (upstream never fetches those) or the id of an asset already loaded. Loaded assets are the
  bound project's own `assets/` and images an agent sent as `data:` URIs. HTML is parsed with
  upstream `mcp::html::parse`, so attributes are checked after entity decoding, case-insensitive.
  Every other string argument (styles maps) is scanned for `url(`. Anything else is refused, so
  no local path, relative or absolute, is ever read.
- **Inline `<svg>`:** usvg's default resolver reads `<image href>` files from disk, so inline
  SVG must be well-formed XML, parsed with roxmltree, the same parser usvg uses. Every `href`
  and `url(...)` in it must be `#id` or `data:`, and there may be no DTD and no `@import`.
- **Project assets:** loaded by the adapter, not by upstream `assets::load_dir`, which follows
  symlinks. Only plain single-link files in a real `assets/` folder are registered. `assets/`
  joins the B1 entry checks (not a symlink), and agent-saved images are written there only
  after that check.
- `get_screenshot` stays. It renders only the bound project and needs a GPU (headless wgpu);
  without one it reports that.

### Revisions: strict (`lib.rs`)

- **Revision endpoint:** `GET /api/pages/{id}/revision` returns the page's current revision
  (the hash of its file on disk) cheaply.
- **Every browser save** needs `If-Match` equal to that current revision, as in B1. A stale or
  unknown revision gets `409` with the current revision, and nothing is written. Nothing is
  merged, and no revision history is kept.
- **Agent writes** go through the single host thread under the **same write lock** as browser
  saves. The host reloads from disk before each call when anything changed, and saves
  atomically, which gives a new revision.
  - So an agent write can't overwrite a browser save: the host has reloaded it first.
  - And a browser save can't overwrite an agent write: its `If-Match` names the older revision
    and is refused.

### Browser (`static/session.js`, `static/page.js`, `src/state.rs`)

- **Polling:** about once a second the page asks for the revision. Polling waits for Ready and
  pauses during a fallback, a save or a conflict, under the same gating as B1.
- **Nothing unsaved:** the new revision is loaded in place (`WebState::reload`), keeping the
  camera, tool and the selection where it still exists. "Nothing unsaved" means no unsaved
  change, no save in flight, no gesture and no text edit.
- **Unsaved work:** the page is never reloaded over it. Its next save is based on the older
  revision, so the server answers 409. The banner appears, and the unsaved state stays on
  screen.
- **Discard:** only the person's explicit click on "Reload (discard my unsaved change)"
  replaces the page. It loads the server's version **in place** (`Session.discard` via
  `page.js` `discardStep`), with no navigation. That fixes the operator's Codex finding on
  ba93173: `location.reload()` together with the always-on leave-page guard gave
  `net::ERR_ABORTED` and left the banner up. Ordinary navigation with unsaved edits stays
  guarded.
- **Stale answers:** a save, poll or discard answer only applies to the editor that asked,
  with the state it had. Anything else is dropped.

### Decision: strict revisions, no merge (2026-10-12)

The lead decided this under the owner's thin-wrapper, minimum-maintenance mandate. Issue #1
needs:
- an editable browser canvas;
- real MCP edits;
- preserved edits and safety against stale revisions;
- explicit boundaries.

It doesn't need a collaboration engine; that is issue #2. The independent review was finding
defects in the node-level merge (children lost, duplicated or deleted), which confirmed the
maintenance risk. So the three-way merge, its id renumbering and the retained-revision history
(64 entries, 256 MiB) were removed. Their code and tests stay retrievable at the local tag
`experiment/b2a-three-way-merge` (commit `2da2f52`, the pre-decision B2a head).

### Boundary (unchanged from B1)

Only writers that go through this server (the page and MCP agents) are serialized and
revision-checked. A concurrently running native paper-rs, or any other external writer, is not
protected. Its changes are detected through the on-disk revision, and the agent host reloads
them, but it can still overwrite this server's writes.

## Tests and results (2026-10-12, implementation worker sandbox, offline, Rust 1.95.0, Node 26.8.1, `TMPDIR=<worktree>/target/tmp`)

- **`tests/agent.rs` (5 passed):** real loopback listener, upstream-compatible rmcp
  streamable-HTTP client.
  - `mcp_needs_the_token_and_a_loopback_host_and_origin`: 401 without or with a wrong token,
    403 for a foreign Host or Origin, 200 with the token, and a client without the token can't
    connect.
  - `agent_edits_land_on_disk_and_change_the_revision`: create_shape/update_styles/
    delete_nodes reach the page file. The revision endpoint changes. Refused tools aren't listed.
  - `sequential_human_and_agent_edits_are_both_kept`: a browser save then an agent write, and
    an agent write then a browser save on the new revision. Both changes end up on disk each
    time, and the agent sees the browser's save on its next call.
  - `a_stale_browser_save_after_an_agent_write_is_refused_and_writes_nothing`: after an agent
    write to a different layer, to the same layer, or deleting the layer, the browser's save on
    the older revision gets 409 with the current revision, and the disk keeps the agent's
    version. An unknown revision is refused too. After a discard (GET the current version) the
    browser can save again.
  - `refused_tools_projects_and_local_paths`: export, export_combined_pdf, create_file, a
    sibling project by id or path, and a foreign `fileId` are refused. So are `<img src>` with
    absolute, `file://` and `../` paths, CSS `url()`, SVG `<image href>` and
    `create_shape` styles `url()`. Meanwhile an `assets/` symlink to an outside image is
    present. The outside image (a real PNG) is never registered (so never read), nothing is
    written outside, refused calls write nothing, and only the bound project is listed. A
    `data:` image still works. With the policy disabled, this test fails (`create_file`
    succeeds).
- **`agent::tests` (3 passed):** refused tools and other projects; local image paths in
  `<img>`, CSS and styles, including entity-encoded, uppercase-tag and `DATA:` forms; inline SVG
  `href`, `url()`, `@import`, DTD and malformed XML.
- **`state::tests` (2 new, 10 total passed):** `reload_only_when_clean_and_it_keeps_the_view`,
  `never_reload_over_unsaved_or_in_flight_edits`.
- **`node --test crates/openpaperdesign-web/tests/session.test.mjs` (18 passed; mocked
  editor/server/page, no browser).** The B2a tests:
  - `a change made elsewhere is loaded in place when nothing is unsaved`
  - `never reloads over unsaved edits; the stale save is a conflict and the edit stays`
  - `explicit discard loads the version on disk in place and resumes saving` (through
    `discardStep`: banner hidden, status shown, saving resumes on the new revision)
  - `a discard answer is dropped when the editor changed meanwhile, and a failed one changes
    nothing`
  - `no polling before Ready or while switching`
- **B1 serve tests:** all pass (17).
- **Live smoke test:** the server binary on a demo project, driven by `scripts/mcp-client.mjs`
  (a dependency-free JSON-RPC client) over a real socket. `get_basic_info` answered,
  `create_shape` wrote node 24 to the page file (format v3), `export` was refused with the
  policy message, and a wrong token got HTTP 401.

Results of the strict-model change (2026-10-12, proportionate checks per the owner). This
change touches only the serve crate and the page JavaScript; no root-package file, web Rust,
manifest or lockfile changed.
- **Run now:**
  - `cargo fmt --all --check`;
  - clippy for serve and web (0 warnings);
  - serve tests: 3 unit, 5 agent, 17 serve;
  - the CI Node command: 18 passed, Node 26.8.1;
  - `node --check` on all page JavaScript and scripts;
  - wasm32 check, native build, `git diff --check`;
  - the bundle rebuild. The wasm is unchanged; `app.js`, `page.js` and `session.js` changed.
- **Inherited from the 2da2f52 run (not re-run):** serial 162, headless 9, root clippy and
  `cargo tree -p paper-rs`.

## Not proven

- **Real browser:** no run with a person in a real browser while an agent edits, so the live
  update, the in-place reload and the conflict banner are untested in a browser. This includes
  Codex's in-app browser and any real MCP client app (Codex, Claude Code). Chromium can't start
  in this sandbox. `scripts/browser-check.mjs … agent` is written for the operator: an agent
  shape must appear without a reload, then a saved page drag followed by an agent move must
  both survive a reload. The discard fix still needs the operator's Codex re-check.
- **Client config syntax:** the printed Claude Code/Codex settings haven't been tried with those
  clients.
- **`get_screenshot` over this server:** not exercised.
- **Undo:** agent undo/redo across a browser save is limited, because the host's history
  restarts when the page changes on disk.
- **External writers:** a concurrently running native app is still unprotected (see Boundary).
