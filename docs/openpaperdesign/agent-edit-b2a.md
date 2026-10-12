# B2a: an agent and a person editing one project — 2026-10-12

Scope: [issue #1](https://github.com/onlinesourdough/openpaperdesign/issues/1), slice B2a, on top
of B1 (`browser-edit-b1.md`). While `openpaperdesign-serve --project <dir>` runs and the page
is open in a browser, an AI agent connected over MCP edits the **same** project, and the
browser shows the agent's change without a manual reload. Edits in the browser are kept, and
overlapping edits are shown as a conflict, never overwritten either way. Out of scope: Codex
plugin packaging, skills, discovery, the MCP App panel (B2b), several projects, remote use.

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

### Revisions and merging (`src/merge.rs`, `lib.rs`)

- **History:** the server keeps recent page revisions' bytes in memory (64 entries, 256 MiB at
  most), recorded on every page GET and PUT and every agent save.
- **Revision endpoint:** `GET /api/pages/{id}/revision` returns the current revision cheaply.
- **Stale browser PUTs:** a PUT whose `If-Match` is stale but still retained is merged three
  ways at node level: base = the browser's revision, theirs = disk now, mine = the browser's
  page.
  - Non-overlapping adds, deletes and changes merge, and are saved. The answer is the merged
    page with its new revision and `X-Opd-Merged: 1`.
  - Both sides allocating the same new node id for different layers renumbers the browser's
    layer, with all references to it (parent, children, connector bindings, instance links).
  - Z-order: one side's reordering wins; the other side's insertions go before their next
    sibling (or on top).
  - Conflicts: the same layer changed differently, delete versus edit, a layer whose parent was
    deleted, both sides reordering the same container differently, or a cycle. The answer is
    `409` with `conflicts: [node ids | "root"]`, and nothing is written.
  - An unknown or evicted base gets a plain `409`, as in B1.
- `Document::validate` doesn't detect cycles, so the merge also checks that every node is
  reachable from the root.

### Browser (`static/session.js`, `src/state.rs`)

- **Polling:** about once a second the page asks for the revision. Polling waits for Ready and
  pauses during a fallback, a save or a conflict, under the same gating as B1 round 4.
- **Nothing unsaved:** the new revision is loaded in place (`WebState::reload`), keeping the
  camera, tool and the selection where it still exists. "Nothing unsaved" means no unsaved
  change, no save in flight, no gesture and no text edit.
- **Unsaved work:** the page is never reloaded over it. The next save carries the old base, and
  the server merges it.
  - If nothing changed locally while that save was in flight, the merged page is loaded.
  - Otherwise the old base is kept, so the following save merges again.
- **Conflict:** the overlay banner lists the affected layers, with one button,
  "Reload (discard my unsaved change)". Nothing is discarded without that click.

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
  - `a_stale_browser_save_merges_with_the_agents_change`: the agent adds a shape while the
    browser, on the older revision, moves another layer. Both end up on disk, the answer is the
    merged page, and the agent sees it on its next call.
  - `overlapping_edits_conflict_and_nothing_is_written`: the same layer changed by both, and
    delete versus edit, each give 409 with the layer id and leave the bytes on disk unchanged.
    An unknown base gives a plain 409.
  - `refused_tools_projects_and_local_paths`: export, export_combined_pdf, create_file, a
    sibling project by id or path, and a foreign `fileId` are refused. So are `<img src>` with
    absolute, `file://` and `../` paths, CSS `url()`, SVG `<image href>` and
    `create_shape` styles `url()`. Meanwhile an `assets/` symlink to an outside image is
    present. The outside image (a real PNG) is never registered (so never read), nothing is
    written outside, refused calls write nothing, and only the bound project is listed. A
    `data:` image still works. With the policy disabled, this test fails (`create_file`
    succeeds).
- **`merge::tests` (7 passed):** both sides changing different layers; the same layer (conflict
  and identical); adds on both sides with clashing ids renumbered, including a child; deletes
  and delete versus edit; a deleted parent with additions inside; z-order merge and conflict;
  reparenting, conflicting parents and cycles.
- **`agent::tests` (3 passed):** refused tools and other projects; local image paths in
  `<img>`, CSS and styles, including entity-encoded, uppercase-tag and `DATA:` forms; inline SVG
  `href`, `url()`, `@import`, DTD and malformed XML.
- **`state::tests` (2 new, 10 total passed):** `reload_only_when_clean_and_it_keeps_the_view`,
  `never_reload_over_unsaved_or_in_flight_edits`.
- **`node --test crates/openpaperdesign-web/tests/session.test.mjs` (14 passed, 5 new for B2a; mocked editor/server, no
  browser):**
  - `a change made elsewhere is loaded in place when nothing is unsaved`
  - `never reloads over unsaved edits; the next save is merged by the server`
  - `edits made while a merged save was in flight keep the old base for the next merge`
  - `a conflict is reported with its layers and stops saving and reloading`
  - `no polling before Ready or while switching`
- **B1 serve tests:** all still pass (17). One case was adapted: `security_headers_are_on_every_response`'s
  "stale write" now changes the same layer differently, because a stale save of a non-overlapping
  change now merges (200) by design.
- **Live smoke test:** the server binary on a demo project, driven by `scripts/mcp-client.mjs`
  (a dependency-free JSON-RPC client) over a real socket. `get_basic_info` answered,
  `create_shape` wrote node 24 to the page file (format v3), `export` was refused with the
  policy message, and a wrong token got HTTP 401.

The full check-list results are in the commit message of the docs commit and in the handoff.

## Not proven

- **Real browser:** no run with a person in a real browser while an agent edits, so the live
  update, the in-place reload and the conflict banner are untested in a browser. This includes
  Codex's in-app browser and any real MCP client app (Codex, Claude Code). Chromium can't start
  in this sandbox. `scripts/browser-check.mjs … agent` is written for the operator: an agent
  shape must appear without a reload, then a concurrent page drag and agent move must both
  survive a reload.
- **Client config syntax:** the printed Claude Code/Codex settings haven't been tried with those
  clients.
- **`get_screenshot` over this server:** not exercised.
- **Undo:** agent undo/redo across a browser save is limited, because the host's history
  restarts when the page changes on disk.
- **External writers:** a concurrently running native app is still unprotected (see Boundary).
