# B2a: an agent and a person editing one project — 2026-10-12

Scope: [issue #1](https://github.com/onlinesourdough/openpaperdesign/issues/1), slice B2a, on top
of B1 (`browser-edit-b1.md`). While `openpaperdesign-serve --project <dir>` runs and the page
is open in a browser, an AI agent connected over MCP edits the **same** project, and the
browser shows the agent's change without a manual reload. Edits in the browser are kept, and
overlapping edits are refused as a conflict, never overwritten either way.

Out of scope: Codex
plugin packaging, skills, discovery, the MCP App panel (B2b), several projects, remote use.

Updated 2026-10-12: the automatic three-way merge was dropped in favour of strict revisions
(see "Decision: strict revisions, no merge"). Then review round 1 findings 1–5 were fixed (see
"Review round 1"), then review round 2 findings 1–6 and the operator's Mac case-alias
finding (see "Review round 2").

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
  4. refuses the change if it touches a page whose revision moved since this MCP session last
     read it (see "Agent writes" below). The change is discarded and nothing is written;
  5. otherwise saves everything the call changed as **one commit** (see "Errors change
     nothing" below), updates the manifest like the native session does, and records the new
     revisions.
- **One transaction per write:** each agent write is one upstream transaction, saved before the
  call returns, so it changes that page's revision (the hash of its file on disk).
- **Errors change nothing** (lead contract, review round 4): a call that returns an error
  leaves the project exactly as it was, in memory and on disk.
  - **In memory:** nothing reaches the disk during a call (`Host::save` is a no-op; export
    and file creation are refused).
    - Before any tool that can change the editor, the host copies its state: every page's
      document and undo history, the page list, selection, view and active page. After a
      tool error, a conflict or a failed save, it puts that copy back, so the undo history
      and selection survive too.
    - Read tools change nothing, so a failed read needs no restore.
    - Only a caught panic, which may leave the editor mid-operation, falls back to
      reloading from disk (the undo history then restarts).
    - The copy needs `Clone` on upstream `History`, `PageSlot` and `PageStash`: three added
      derives (review round 5).
    - It also covers upstream's process-wide record of which flex containers don't stretch
      their children (`no_stretch` in `src/mcp/exec.rs`). A refused `alignItems` change used
      to stay there and change the next accepted child (0px wide instead of 300px). Only
      this project's pages' entries are copied and put back; other projects in the same
      process (another launcher binding, tests) keep theirs. Reading it needs one added
      upstream function, `exec::with_no_stretch` (final review).
    - **Ownership across bindings** (operator's Mac finding on `cffeec6`; lifecycle
      correction after final review round 3 on `b846b8a`, 2026-10-12): the record is keyed
      by page id, including ids shared by two projects opened in succession.
      - Replacement preparation validates and loads the project without changing the
        active host's layout record. A failed bind leaves its server, token and state intact;
        binding the same project again is a no-op.
      - `Server::retire` closes admission, then takes the same writer lock as MCP calls and
        browser saves. A started call completes with its own layout state, including any
        rollback and acknowledgement. Queued and late old calls are refused without
        executing; browser saves re-check admission under the lock too.
      - Only after that drain does retirement release the lifetime union of the old host's
        page ids (including removed pages). Activation then clears entries for the new
        project's ids and admits its calls. Unrelated page ids are preserved. Retained old
        routes and server handles remain retired; repeated retirement cannot clear a later
        owner's entries.
      - Initial standalone `openpaperdesign-serve` activation uses the same fresh-state
        reset. This supports the launcher's one-bound-project lifecycle, not arbitrary
        simultaneous hosts sharing page ids. It does not protect external/native writers.
  - **On disk, one commit** (`crate::commit`) per call, used by browser saves too (page plus
    manifest):
    1. The current bytes of every target are read first.
    2. Every new file (images, pages, then the manifest) is written to a temporary file and
       synced.
    3. The temporary files are renamed in that order.
    4. Each directory involved is synced.

    Every step can fail, directory syncs included. If one does, the temporary files are
    removed, every file already replaced is put back from its old bytes the same way (or
    removed if it was new), and those directories are synced again. Every project file is
    then byte-identical to before, and the answer says "nothing was written". Nothing that
    can fail comes after a successful commit: the browser's answer takes the new revision
    from the bytes it just wrote rather than reading the page back, so a completed save is
    never reported as failed.
  - **Fail closed:** if putting a file back (or syncing its directory) fails too, the
    answer says so, naming the files that may now be inconsistent, and that writes are
    disabled. From then on the server refuses every write, browser (503) and MCP alike,
    naming the same files. It never reports success or partial success. This state is kept
    **in memory only**. Restarting the server clears it without checking anything, so check
    or restore the named files before restarting.
  - **Not crash-safe:** this handles errors the running process sees; it is not a durable
    multi-file transaction. There is no journal, and the old bytes live only in memory. A
    crash or power loss in the middle of a commit can leave some page files new and others
    old, and the manifest out of step with them. Each file is itself complete, being
    replaced by an atomic rename. Nothing recovers this automatically; restore from a copy
    if it happens. Temporary files a crash leaves are removed by upstream `Project::open`
    at the next start.
- **Timeouts** (review round 4): calls are queued for the host thread. A call that hasn't
  started within 120 s is abandoned (Queued → Abandoned, decided atomically against the host
  starting it) and is never run. A call that has started (Queued → Running) is always waited
  for, and its real result is returned. What a call acknowledges is applied only after its
  result was delivered to the caller.
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
- **Inline `<svg>`:** usvg's default resolver reads `<image href>` files from disk. It treats
  `#private.png` as a file name, and it falls back to reading a file when a `data:` href
  doesn't parse (`data:private.png`). So inline SVG must be well-formed XML, parsed with
  roxmltree, the same parser usvg uses, with no DTD and no `@import`. It may contain no
  `<image>` or `<feImage>` element at all; agents put images in HTML `<img>` with a `data:`
  URI instead. Every `href` and `url(...)` must be `#id` of an element in the same SVG.
- **Page ids and files:**
  - Page ids come only from the manifest. Each must be 1–64 ASCII letters, digits, `-` or
    `_`, and no two may be equal **ignoring ASCII case**. Otherwise the project is refused
    everywhere: server start, browser requests and agent calls alike.
  - Case matters because on case-insensitive file systems (macOS APFS by default) `CasePage`
    and `casepage` are the same page file. The operator observed this on `a96b32c`: the server
    started with both ids aliasing one file (`.toolchain/operator-evidence/mac-case-alias.json`).
    The rule doesn't depend on the file system, so it is tested on Linux too.
  - No two page ids may share their first six characters (ignoring case). Upstream node ids
    are `<first six characters of the page id>:<n>`, so such pages would be ambiguous to
    every tool. The reviewer reproduced a lost update through `abcdef-A`/`abcdef-B` on
    `b730f4c`. The project is refused at start and bind, and before any read or write.
  - The host reads pages itself rather than with upstream `load_pages`. `pages/` and each page
    file are checked (no symlink, a single hard link) **before** any byte is read, and two
    pages that turn out to be the same file (same device and inode) are refused.
  - A page takes its id and name from its manifest entry, whatever its file says, as upstream
    `Project::load_page` does. So the id inside a file can't choose where it is saved, and a
    reload doesn't revert a saved rename (renames only write the manifest).
  - Before saving, every page id is checked again (format, case-folded uniqueness, distinct
    six-character prefixes), and each destination is checked not to be a link.
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
  saves. The host reloads from disk before each call when anything changed, and saves the
  call's changes as one commit (all or nothing), which gives new revisions.
  - A browser save can't overwrite an agent write: its `If-Match` names the older revision
    and is refused.
  - **An MCP session must read a page before it changes it,** and an agent write can't
    overwrite a change the session hasn't read. There is no exception for a first call.
  - **What counts as reading, per page:** a page is read at its current revision only by a
    call that **succeeded** and **returned that page's content or state**. Which page that
    is follows upstream's own resolution of the call's arguments: `pageId`, else the page
    the user is viewing; a node id `<page prefix>:<n>` names its own page.
    - `get_basic_info`: its page, plus the page list (see below).
    - `get_tree_summary`, `get_children`, `get_screenshot`: the page of `nodeId` (or of
      `"root"`).
    - `get_node_info`, `get_jsx`, `get_fill_image`: the page of the node.
    - `get_computed_styles`: the page of each node whose styles are actually in the answer
      (its node ids are keys of the returned map); a page left out isn't credited.
  - **Acknowledges nothing:** any call that failed (an error, a policy refusal, a conflict, an
    unknown tool, a call abandoned before it started), any call whose result couldn't be
    delivered, and any call that can't be attributed to specific pages or that returns
    matches or a selection rather than a page's content: `find_nodes`, `get_selection`,
    `open_file` (it returns basic info, but isn't credited; call `get_basic_info`),
    `list_resources`, `get_guide`, `get_font_family_info`, `select_nodes`,
    `finish_working_on_nodes`, no-op changes.
  - **Output really contains the page:** since page ids must differ in their first six
    characters, every node id in an answer names exactly one page. The one answer that can
    leave a requested page out, `get_computed_styles` (a map keyed by node id), credits only
    the pages whose node ids are keys in it.
  - **New pages:** `create_page` changes the page list, so it needs the page list read
    (`get_basic_info`).
  - **Page list:** the manifest's page metadata (ids, names, order) is a separate item. Only
    `get_basic_info`, which returns it, reads it. Its revision is the hash of a canonical
    JSON array of `[id, name]` pairs, so names containing newlines or NULs can't make two
    lists look the same.
  - **A call that returns an error changes nothing** (see "Errors change nothing" above) and
    acknowledges nothing, so what the session read before stays valid.
  - **What is refused:** a call that changes an existing page's content, or removes it, unless
    **every** page it writes is read at its current revision. The same applies to the page
    list if the call changes it (a rename, a new page, a removed page). Otherwise it gets a
    `conflict` error, the change is discarded, and nothing is written (no page and not the
    manifest). The agent reads what it was told and redoes the change.
  - **Own writes:** a session's own successful write counts as read, for the pages (and page
    list) it wrote and nothing else. Pages a call creates are new, so never stale.
  - **Forgotten sessions:** the host remembers the reads of the last 256 sessions. A session
    it forgot (or never saw) has read nothing, so it must read again before changing a page:
    forgetting fails closed.
  - **Granularity:** checks are per page, not per node. A browser save to one layer also
    refuses an agent edit of another layer on that page until the agent reads again.
  - **Not covered:** the active page in the manifest (set by `open_file` `pageId`) is view
    state, not checked.

### Browser (`static/session.js`, `static/page.js`, `src/state.rs`)

- **Polling:** about once a second the page asks for the revision, **only while the editor
  is clean** (nothing unsaved, no save in flight, no gesture, no text edit). No request is
  made otherwise. Polling also waits for Ready and pauses during a fallback, a save, a
  discard or a conflict.
- **Nothing unsaved:** the new revision is loaded in place (`WebState::reload`), keeping the
  camera, tool and the selection where it still exists. "Nothing unsaved" means no unsaved
  change, no save in flight, no gesture and no text edit.
- **Unsaved work:** the page is never reloaded over it. Its next save is based on the older
  revision, so the server answers 409. The banner appears, and the unsaved state stays on
  screen.
- **Discard:** only the person's explicit click on "Reload (discard my unsaved change)"
  replaces the page. While the server's version loads, editing input, toolbar actions and
  saves are suspended, so no edit can be made after the click that the load would then erase. It loads the server's version **in place** (`Session.discard` via
  `page.js` `discardStep`), with no navigation. That fixes the operator's Codex finding on
  ba93173: `location.reload()` together with the always-on leave-page guard gave
  `net::ERR_ABORTED` and left the banner up. Ordinary navigation with unsaved edits stays
  guarded.
- **Stale answers:** a save, poll or discard answer only applies to the editor that asked,
  with the state it had. Anything else is dropped. Polls never overlap, and a poll's answer
  is also dropped if a save, another poll or a discard started after it asked (a generation
  counter), so a late answer can't roll the editor back to an older revision.

### Decision: strict revisions, no merge (2026-10-12)

The lead decided this under the owner's thin-wrapper, minimum-maintenance mandate. Issue #1
needs:
- an editable browser canvas;
- real MCP edits;
- preserved edits and safety against stale revisions;
- explicit boundaries.

It doesn't need a collaboration engine; that is issue #2. The independent review was finding
defects in the node-level merge, which confirmed the maintenance risk. Round 1 (on `37cd0ed`)
reproduced two: a new child disappeared when only its colliding parent was renumbered
(finding 6), and a second save against the old base duplicated or resurrected human nodes
after a merged save (finding 7). Removing the merge closes both. So the three-way merge, its id renumbering and the retained-revision history
(64 entries, 256 MiB) were removed. Their code and tests stay retrievable at the local tag
`experiment/b2a-three-way-merge` (commit `2da2f52`, the pre-decision B2a head).

### Boundary (unchanged from B1)

Only writers that go through this server (the page and MCP agents) are serialized and
revision-checked. A concurrently running native paper-rs, or any other external writer, is not
protected. Its changes are detected through the on-disk revision, and the agent host reloads
them, but it can still overwrite this server's writes.

### Review round 1 (on `37cd0ed`), findings 1–5

| Finding | Fix | Tests (each fails on `37cd0ed`'s code) |
| --- | --- | --- |
| 1. Page ids escape the project | manifest ids validated, no duplicates; a page's id is its manifest entry's; save destinations checked | `page_ids_cannot_reach_outside_the_project` |
| 2. SVG `<image href="#private.png">` reads a file | images only `data:`; `#id` must exist in the SVG | `svg_images_take_only_data_uris_and_references_only_existing_ids`, `svg_image_fragments_are_refused` |
| 3. Start-up reads linked page files | own page reader checks links before reading | `page_links_are_refused_before_reading` |
| 4. A stale agent write overwrites a browser save | per-session seen revisions, `conflict` refusal | `a_stale_agent_write_after_a_browser_save_is_refused_and_writes_nothing` |
| 5. A late poll answer rolls the editor back | non-overlapping polls, generation counter | `a late poll answer never replaces a newer saved state (reviewer sequence)`, `polls never overlap, and a discard makes an older poll answer stale` |

Findings 6 and 7 were in the merge, which was removed (see the decision above).

### Final review (on combined `0ed4be0`, B2a `9cb0230`)

| Finding | Fix | Tests (each fails on `9cb0230`'s code) |
| --- | --- | --- |
| P2 1. A refused change survived in upstream's process-wide stretch record and altered a later accepted write | The host's copy and restore include this project's pages' entries of that record (via the new upstream `exec::with_no_stretch`), never the whole set | `a_refused_change_leaves_layout_state_unchanged` (the reviewer's sequence over MCP); unit tests `a_failed_save_leaves_layout_state_unchanged` and `restoring_layout_state_leaves_other_hosts_alone` (another host's entries, including one added between the copy and the restore, stay) |
| P2 2. Browser PUT read the page back after committing and could answer 500 for a completed save | The revision comes from the committed bytes; nothing fallible follows the commit | `tests::a_completed_browser_save_is_reported_as_saved` |

Checks (2026-10-12; serve crate plus the upstream accessor, so the full native set was
run again):
- **Serve:** `cargo fmt --all --check`; clippy for serve and web, 0 warnings; serve tests
  14 unit, 24 agent, 17 serve.
- **Root:** native build; root clippy with only the known `tests/perf.rs` warning; serial,
  162 passed; headless with `PAPER_RS_REQUIRE_GPU=1`, 9 passed; `cargo tree -p paper-rs -e
  features` unchanged.
- **Web:** wasm32 check; web tests 111 unit + 2 shader. `docs/screenshots` was restored.

For the unit tests on `9cb0230`, two scaffolds were added to that copy only, because the
tests need them to compile and inject:
- the accessor `exec::with_no_stretch`, which the old adapter doesn't call;
- a fault point at its post-commit read.

The integration test needed neither. Evidence:
`target/evidence/b2a-final-tests-on-9cb0230.txt`.

### Review round 5 (on `2fcd739`)

| Finding | Fix | Tests (each fails on `2fcd739`'s code) |
| --- | --- | --- |
| F1. Directory fsync errors were ignored (a commit returned success), and a rollback never synced its directories | Directory syncs are steps that can fail. A failure during commit rolls back, and the rollback syncs its directories too; any failure there means Broken (fail closed). | `directory_sync_failures_roll_back_or_fail_closed`; the commit step loops in `a_failed_save_leaves_every_file_unchanged` and `tests::browser_saves_are_all_or_nothing_and_fail_closed` now include the directory syncs |
| F2. A failed read reloaded the editor, losing selection and undo history | Copy and restore of the editor state before changing tools (see "In memory"); reads need none | `a_failed_call_keeps_selection_and_undo_history` (the reviewer's failed read, plus a failed cross-page move) |
| F3. A failed rollback was reported as "nothing was written" | Failed (nothing written) and Broken (files listed, writes disabled until checked and restarted) are worded separately, for MCP and the browser | `a_failed_rollback_refuses_every_later_write` (MCP), `tests::browser_saves_are_all_or_nothing_and_fail_closed` (browser) |
| Docs | The fail-closed state is in memory and clears on restart; no crash-atomic multi-file recovery | — |

### Review round 4 (on `b730f4c`)

| Finding | Fix | Tests (each fails on `b730f4c`'s code) |
| --- | --- | --- |
| F1. Failed `rename_pages`, `move_nodes` and `set_text_content` saved their partial changes, and a failed `write_html` replace deleted its target from disk. A save failing part-way (read-only `assets/`) kept page A's change. | A failed call reloads from disk and saves nothing. Each call's files are one commit with rollback, and the server fails closed if the rollback fails. Browser saves use the same commit. | `a_failed_call_changes_nothing` (the reviewer's four cases); `a_save_failing_part_way_writes_nothing` (the reviewer's I/O case). Unit tests with a test-only fault seam: `a_failed_save_leaves_every_file_unchanged` (every staging/rename step of a 1-page, a 2-page and a manifest-only commit), `a_failed_rollback_refuses_every_later_write`, `tests::browser_saves_are_all_or_nothing_and_fail_closed`. |
| F2. Lost update via colliding six-character page-id prefixes (`get_computed_styles` answered for one page and both were credited) | Such projects are refused at start/bind and before saving. `get_computed_styles` credits only pages present in its answer; `find_nodes` and `get_selection` credit nothing. | `page_ids_sharing_their_first_six_characters_are_refused` (`abcdef-A`/`abcdef-B`, and a case-folded variant) |
| F3. Newlines/NULs in page names made two page lists encode the same, permitting blind and stale renames | Page-list revision is the hash of a canonical JSON array | `page_list_revisions_are_unambiguous` (the reviewer's names, blind and stale) |
| F4. A read that timed out later ran and credited content the caller never got | Queued → Running / Queued → Abandoned; abandoned calls never run; started calls are always waited for; acknowledgments only after delivery | Unit tests: `a_call_abandoned_before_it_started_is_never_run`, `a_started_call_returns_its_real_result`, `an_undelivered_read_acknowledges_nothing` (100 ms test wait) |

The six integration tests above compile against `b730f4c` and fail there. The unit tests use
seams that don't exist there (the fault hook, `send_and_wait`), so they were checked by
mutation instead. Restoring `b730f4c`'s lifecycle (run abandoned calls, acknowledge
undelivered results, stop waiting at the timeout) makes the three F4 tests fail, and
disabling the rollback makes the three F1 unit tests fail. The reviewer's
`r4_timed_out_read_must_not_authorize_an_unseen_write` (120 s) failed on `b730f4c`.

### Review round 3 (on `9c3ba69`)

| Finding | Fix | Tests (each fails on `9c3ba69`'s code) |
| --- | --- | --- |
| P1. Reading page A acknowledged page B; failed and non-reading calls acknowledged every page; a batch writing A and B passed after reading only A | Per-page observation, set only by a successful call that returned that page (or by the session's own write to it). The page list is a separate item. Every page a change writes must be read at its current revision. | `reading_page_a_does_not_acknowledge_page_b`, `a_failed_read_acknowledges_nothing`, `own_writes_acknowledge_only_the_pages_written`, `unattributable_reads_acknowledge_nothing` (also successful non-reading and failed calls, and a rename after a page-only read), `a_batch_write_needs_every_page_it_writes_read` |
| P3. A dirty editor still started revision polls | `poll()` starts only when `can_reload()` holds | `only a clean editor polls: no revision request while dirty, in a gesture or a text edit (review round 3)`; `never reloads over unsaved edits…` now expects no poll while dirty. `an edit made while a clean poll is under way is never replaced` keeps the in-flight case covered (it passes on both). |

Two existing tests read only the page the user views before writing another page (or
nothing before writing). Under the per-page rule those writes are refused, so the tests now
read the page they write.

### Review round 2 (on `a96b32c`), findings 1–6, and the operator's Mac case-alias finding

Lead decisions: refuse SVG image elements outright, and require a read before any change
(no first-call exception).

| Finding | Fix | Tests (each fails on `a96b32c`'s code) |
| --- | --- | --- |
| 1. A malformed `data:` SVG image href (`data:private.png`) is read as a file | no `<image>`/`<feImage>` in inline SVG; references only to existing `#id` | `svg_image_elements_are_refused_and_references_stay_inside` (unit, malformed-data, `xlink:href`, `feImage` variants), `svg_image_fragments_are_refused` (over MCP) |
| 2. A session evicted from the 256 remembered let its stale write through | a session must have read a page before it changes it; an unknown or forgotten session must read again | `a_forgotten_session_must_read_again` (257 other sessions), and the first call in `agent_edits_land_on_disk_and_change_the_revision` (refused, page and manifest unchanged) |
| 3. A reload reverts a saved page name | manifest owns id and name on load | `reloads_keep_saved_page_names` |
| 4. Stale page renames aren't refused | page metadata (name, position) is part of what a session read and what a call touched | `stale_page_renames_are_refused` (including a batch with one stale entry: nothing written) |
| 5. A discard erases an edit made after the click | input, toolbar and saves are suspended while the discard loads | `editing is suspended while a discard loads, so it cannot erase a newer edit (reviewer sequence)` |
| 6. `scripts/browser-check.mjs` agent step writes without reading | the step reads (`get_basic_info`) before each write; so does the `mcp-client.mjs` command line | the same sequence run against the server binary over a socket (no Chromium): without the read `conflict`, with it `ok` |
| Mac: page ids `CasePage` and `casepage` alias one file on APFS | ids compared ASCII-case-folded at load and before saving; distinct pages must be distinct files (device, inode) | `page_ids_differing_only_in_case_are_refused` (runs on Linux) |

The reviewer also flagged two stale comments that still said "merged"
(`web/src/state.rs`, `serve/tests/serve.rs`); they are corrected.

## Tests and results (2026-10-12, implementation worker sandbox, offline, Rust 1.95.0, Node 26.8.1, `TMPDIR=<worktree>/target/tmp`)

- **`tests/agent.rs` (21 passed; see the review round tables for the newer tests):** real loopback listener, upstream-compatible rmcp
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
  - Review round 1: `a_stale_agent_write_after_a_browser_save_is_refused_and_writes_nothing`
    (two sessions read, the browser saves, both writes and a retry without reading are
    refused with page and manifest bytes unchanged; after a read the write lands on top of the
    browser's save; a session's own write doesn't make it stale),
    `page_ids_cannot_reach_outside_the_project` (a page file whose id is an outside path is
    saved in place, nothing appears outside; absolute, `..`, slash, empty and duplicate
    manifest ids refuse start-up), `page_links_are_refused_before_reading` (symlink and hard
    link to a non-JSON outside file: the error is the link refusal, not a parse error) and
    `svg_image_fragments_are_refused` (the reviewer's SVG over MCP).
- **`agent::tests` (4 passed):** refused tools and other projects; local image paths in
  `<img>`, CSS and styles, including entity-encoded, uppercase-tag and `DATA:` forms; inline SVG
  `href`, `url()`, `@import`, DTD and malformed XML; and
  `svg_images_take_only_data_uris_and_references_only_existing_ids` (the reviewer's
  `#private.png`, `feImage`, `xlink:href`, missing ids).
- **`state::tests` (2 new, 10 total passed):** `reload_only_when_clean_and_it_keeps_the_view`,
  `never_reload_over_unsaved_or_in_flight_edits`.
- **`node --test crates/openpaperdesign-web/tests/session.test.mjs` (23 passed; mocked
  editor/server/page, no browser).** The B2a tests:
  - `a change made elsewhere is loaded in place when nothing is unsaved`
  - `never reloads over unsaved edits; the stale save is a conflict and the edit stays`
  - `explicit discard loads the version on disk in place and resumes saving` (through
    `discardStep`: banner hidden, status shown, saving resumes on the new revision)
  - `a discard answer is dropped when the editor changed meanwhile, and a failed one changes
    nothing`
  - `no polling before Ready or while switching`
  - `a late poll answer never replaces a newer saved state (reviewer sequence)` and
    `polls never overlap, and a discard makes an older poll answer stale` (review round 1)
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

Results of the review round 1 fixes (2026-10-12, same proportionate rule; serve crate and
page JavaScript only):
- **Run now:** `cargo fmt --all --check`; clippy for serve and web (0 warnings); serve tests
  4 unit, 9 agent, 17 serve; the CI Node command, 20 passed (Node 26.8.1); `node --check` on
  the page JavaScript; wasm32 check, native build, `git diff --check`; the bundle rebuild
  (`session.js` changed).
- **On `37cd0ed`'s code** (a `git archive` extraction with the new tests copied in): all seven
  new tests fail. Serve: the unit SVG test accepts the reviewer's SVG; `page_ids_…` writes a
  file outside; `page_links_…` reports a parse error (outside bytes read);
  `svg_image_fragments_…` passes the policy; `a_stale_agent_write_…` gets `{"updated": 1}`.
  Node: the reviewer sequence applies the older page, and the overlap test times out.
- **Inherited (not re-run):** serial 162, headless 9, root clippy, `cargo tree -p paper-rs`
  from the 2da2f52 run.

Results of the review round 5 fixes (2026-10-12; serve crate and three upstream derives).
Because upstream `src/` changed, the full native checks were run again:
- **Serve:** `cargo fmt --all --check`; clippy for serve and web, 0 warnings; serve tests
  11 unit, 23 agent, 17 serve.
- **Root:** native build; root clippy with only the known upstream `tests/perf.rs`
  `drop_non_drop` warning; serial `cargo test --locked -- --test-threads=1`, 162 passed;
  `PAPER_RS_REQUIRE_GPU=1 cargo test --locked --test headless`, 9 passed (on a GPU, not
  skipped); `cargo tree -p paper-rs -e features` identical to `2fcd739`'s.
- **Web:** wasm32 check (core, web); web tests 111 unit + 2 shader. `docs/screenshots`
  was restored after the serial run.
- **On `2fcd739`'s code** (the new test modules copied in): all five changed or new
  regressions fail. These are `a_failed_save_leaves_every_file_unchanged` (directory-sync
  steps), `a_failed_rollback_refuses_every_later_write` (wording),
  `directory_sync_failures_roll_back_or_fail_closed`,
  `tests::browser_saves_are_all_or_nothing_and_fail_closed` (directory sync, wording) and
  `a_failed_call_keeps_selection_and_undo_history`.

Results of the review round 4 fixes (2026-10-12; serve crate only):
- **Run now:** `cargo fmt --all --check`; serve clippy (0 warnings); serve build; serve tests
  10 unit, 22 agent, 17 serve; `git diff --check`.
- **On `b730f4c`'s code** (with the new tests copied in): the four new integration tests
  fail; the unit tests by mutation, as described in the round 4 section.
- **Inherited (not re-run):** serial 162, headless 9, root clippy, `cargo tree -p paper-rs`
  from the 2da2f52 run; the Node command (no JavaScript changed).

Results of the review round 3 fixes (2026-10-12; serve crate and `session.js`):
- **Run now:** `cargo fmt --all --check`; serve clippy (0 warnings); serve tests 4 unit,
  18 agent, 17 serve; the CI Node command, 23 passed (Node 26.8.1); `node --check` on the
  page JavaScript; the bundle rebuild; `git diff --check`.
- **On `9c3ba69`'s code** (with the new tests copied in): the five new serve tests and the
  two changed or new polling tests fail. The in-flight polling test passes on both.
- **Inherited (not re-run):** serial 162, headless 9, root clippy, `cargo tree -p paper-rs`
  from the 2da2f52 run. No root-package or web Rust file changed.

Results of the review round 2 fixes (2026-10-12; serve crate, page JavaScript, the two
scripts, and one doc comment in web Rust):
- **Run now:** `cargo fmt --all --check`; clippy for serve and web (0 warnings); serve tests
  4 unit, 13 agent, 17 serve; web 111 unit + 2 shader (for the comment change); the CI Node
  command, 21 passed (Node 26.8.1); `node --check` on the page JavaScript and scripts;
  wasm32 check, native build, the bundle rebuild, `git diff --check`.
- **Live sequence:** the browser-check agent sequence was run against the
  `openpaperdesign-serve` binary over a real socket with `scripts/mcp-client.mjs`. It was
  read, `create_shape`, a conditional browser save (200), then `update_styles` without a
  read (`conflict`), then a read and `update_styles` (`ok`). Chromium wasn't run.
- **On `a96b32c`'s code** (a `git archive` extraction with the new tests copied in): all
  eight new regressions fail. Serve: the SVG unit test (the malformed-data SVG is accepted),
  `svg_image_fragments_…`, `a_forgotten_session_…`, `agent_edits_land_…` (its first-call
  write succeeds), `reloads_keep_saved_page_names` (the rename is reverted),
  `stale_page_renames_…` and `page_ids_differing_only_in_case_…` ("the server opened").
  Node: the discard sequence applies the edit during the load.
- **Inherited (not re-run):** serial 162, headless 9, root clippy, `cargo tree -p paper-rs`
  from the 2da2f52 run. No root-package file changed.

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

### Final review round 3 lifecycle correction (2026-10-12)

On exact `b846b8a`, deterministic launcher/HTTP probes showed an old refused call restoring
its layout entries after a switch (Y child 0px instead of 300px), and an accepted old call
saving with entries cleared too early (X child 450px instead of 0px). The correction above
waits for started work before transferring ownership and fences queued or late work.
The launcher regression tests exercise both outcomes through authenticated HTTP and the
concurrent stdio protocol, including queued MCP calls and browser saves. Their scheduling
points are test-only; the portable listener and inert web bundle are test fixtures, not
browser rendering or installed-plugin proof. The existing completed-save test now seeds
an old manifest timestamp so its byte-change assertion does not depend on millisecond
clock timing; its successful response and committed-byte revision checks remain.

Linux checks for this correction passed: 23 library + 24 agent + eight executable launcher
+ 17 API tests; the six lifecycle regressions also passed with real loopback. Serve
clippy all-targets, fmt and diff checks passed. The portable results and the socket checks
(after scoped sandbox approval) are separate evidence; neither is Mac or installed-plugin
proof. No upstream source, dependency, browser or plugin input changed.

The operator's 2026-10-12 Mac proof on `b846b8a` covers 65 serve tests, doctor and sequential
actual-stdio X-to-Y rebinding (Y child 300px). It did not cover overlapping calls. The new
candidate awaits independent review and fresh Mac lifecycle proof. Installed-plugin
and actual in-app-browser editing/save/conflict/reopen verification remain pending.
