# B2b: the standalone `openpaperdesign` Codex plugin — 2026-10-12

Scope: [issue #1](https://github.com/onlinesourdough/openpaperdesign/issues/1), slice B2b, on top
of B1 (`browser-edit-b1.md`) and B2a (`agent-edit-b2a.md`). It is a small local plugin. The
agent deliberately binds one design project in the user's workspace, edits it over MCP, and
opens the editable canvas in Codex's in-app browser on a loopback URL. No AIOS dependency, no
cloud, no telemetry. Not affiliated with Paper (paper.design).

Out of scope: MCP Apps / file entrypoints (the in-app browser at loopback is the editing
surface), plugin publishing, other hosts.

## What is in it

- **`openpaperdesign-mcp`** (`crates/openpaperdesign-serve/src/bin/openpaperdesign-mcp.rs`)
  is the plugin's MCP server, over stdio.
  - **Workspace:** the working directory the host starts it in (or `--workspace`).
  - **Tools:** upstream's design tools, behind B2a's bound-project host and policy, plus:
    - `bind_project {path}`: an existing paper-rs project (has `project.json`) inside the
      workspace. Symlinks are resolved before the inside-check, so links leading out are
      refused. A workspace that is the home folder or `/` is refused.
    - `create_project {name}`: makes `<workspace>/design/<name>/`. It refuses an existing
      folder and a `design/` that leads outside.
    - `open_editor`: returns `http://127.0.0.1:<port>/#token=<token>` for the in-app browser.
      The URL is never written to disk.
    - `openpaperdesign_status`: version, workspace, bound project, whether the editor is
      built in.
  - **Before a bind:** every editing tool answers "bind a project first".
  - **One project per process:** binding again switches projects.
    - **Failed rebind:** the new project is opened and fully checked before the current one
      is touched (containment, `project.json`, page ids, page links). A failed bind leaves
      the current binding, its server, and its editor URL and token working.
    - **Rebinding the same project** changes nothing.
    - **Unsaved browser edits:** once `open_editor` has handed out the URL, switching stops
      the server that browser tab saves to. Edits made through the tools are saved by each
      call, but the tab may hold an unsaved edit, which could no longer be saved. So the
      switch (`bind_project` or `create_project`) is refused with that explanation until it is
      called again with `force: true`. The skill tells the agent to ask the person first.
      The server doesn't know whether the tab actually has unsaved edits; the earlier claim
      that "changes are already saved" was an assumption and was removed.
    - **Exit:** when the launcher exits (the session ends), an open tab loses its server
      the same way, without a prompt.
  - **Serving:** the bound project is served by the B2a server, in the same process, on
    127.0.0.1 with a fresh 32-byte token. Tool calls go to that server's single host, with the
    same write lock, policy, atomic saves and revisions as its HTTP `/mcp`, so the browser and
    the agent edit the same document. The stdio client is one MCP session of that host, so
    B2a's stale-write check applies: a change to a page the person saved since the agent last
    read it is refused as a `conflict`, and nothing is written.
  - **`openpaperdesign-mcp doctor`** prints the setup as JSON (exit 0 only when the editor is
    built in and the platform is supported). `--version` prints the version.
- **The web bundle is compiled into the command.** `build.rs` embeds
  `target/openpaperdesign-web` (or `$OPD_WEB_BUNDLE_DIR`) when all six files exist. Otherwise
  the list is empty and `open_editor` says "build the web bundle first", without guessing a
  path at run time. Editing through the tools still works without it.
- **`plugin/openpaperdesign/`:**
  - `plugin.json`, the only manifest (see "Packaging" below): `$schema` agent-plugins 1.0.0,
    MIT OR Apache-2.0, repository onlinesourdough/openpaperdesign, version 0.1.0, and
    `extensions.com.openai` with `interface` and `onboardingSkill`;
  - `mcp.json`: stdio `{"command": "openpaperdesign-mcp", "args": [], "cwd": "."}`;
  - `NOTICE.md`;
  - skills `openpaperdesign` (bind, edit, `open_editor`, in-app browser, limits), `setup`
    (build, install, `doctor`), and the ported `design` and `review-design`.
- **`.agents/plugins/marketplace.json`**: the local marketplace `openpaperdesign-local`,
  pointing at `./plugin/openpaperdesign`.

### Packaging (corrected 2026-10-12)

The operator checked draft `1714a58` against the installed Codex plugin-creator 0.1.26
(`references/plugin-packages.md`). The corrections:
- **Fixed discovery:** skills come from the `skills/` folder and the MCP server from the root
  `mcp.json`. That file names the server explicitly as `type: "stdio"`, with
  `command: "openpaperdesign-mcp"` and `args: []`. It sets no `cwd`: whether `"."` means the
  plugin folder or the user's workspace is unverified (openai/codex#22842). So the server uses
  the directory the host starts it in, and refuses that directory as a workspace if it is
  the plugin folder itself (as it refuses `/` and the home folder).
  `openpaperdesign_status` shows the workspace actually used. So the portable `plugin.json` has no top-level `skills`, `mcpServers`, `apps` or
  `interface`.
- **Short description:** `extensions.com.openai.interface.shortDescription` is at most 30
  characters. It is now "Agent + human design canvas" (27).
- **One manifest:** the `.codex-plugin/plugin.json` overlay was removed. Nothing showed that
  the local Codex needs it, and an `extensions.com.openai` object replaces the overlay
  entirely rather than merging with it. Two copies could only contradict each other. **Codex
  reads `plugin/openpaperdesign/plugin.json`**: the common fields at the top level and the
  Codex presentation in `extensions.com.openai`. If the operator's Codex doesn't list the
  plugin without the overlay, that is a finding to report; don't add the overlay back
  without a decision.
- **Limits not checked here:** the reference wasn't available in this sandbox. Only the
  30-character cap (from the operator) is enforced. The other fields set are
  `interface.displayName`, `longDescription` (233 characters), `developerName`, `category`,
  and `onboardingSkill`. Their limits still need checking against the reference.

### Ported design methods (AIOS #54)

The source-only handoff (AIOS `a14f2857`, v0.28.1, archive SHA-256 `c07f5553…`, MIT 2026
onlinesourdough) was verified: all 28 files matched their manifest hashes.

- **Ported:** `design` (skill, references, `create-handoff.mjs`, `paths.mjs`, `serve.mjs`)
  and `review-design` (skill, `audit.md`, `audit.py`), adapted host-neutral. AIOS names and
  links to AIOS-only skills became neutral wording, and the editable companion is now the
  openpaperdesign editor.
- **Left out:** the OpenPencil adapter (`openpencil-workbench`, the OpenPencil route of
  `create-handoff.mjs` and `test_openpencil.py`). It drives an external editor this plugin
  doesn't ship or test, and openpaperdesign is its own editable companion here.
- **Provenance:** each ported `SKILL.md` ends with a note, keeps its `LICENSE`, and is listed
  in `plugin/openpaperdesign/NOTICE.md`.
- **Tests:** ported to `plugin/tests/design/` (Python 3, standard library). They point at the
  plugin and drop the OpenPencil cases; the proof.json case is kept, and a preview-copy case
  replaces the mixed OpenPencil one. CI now runs them, which AIOS did not.

## Install on a Mac (operator steps)

These follow the operator's installation contract for the local Codex CLI (operator notes,
2026-10-12): `codex plugin marketplace add <local-repo> --json` and
`codex plugin add openpaperdesign@openpaperdesign-local --json`. Installing hasn't been
attempted yet, and what happens at install time is for the operator to verify: discovery,
the stdio start, `PATH` and the working directory.

```sh
# 1. Build the browser editor and install the command (Rust 1.95 + wasm32 target,
#    wasm-bindgen CLI 0.2.129), from a checkout of this repository at the reviewed commit:
cd <checkout>
./scripts/build-web.sh
# With a shared CARGO_TARGET_DIR, point the build at the bundle just verified:
OPD_WEB_BUNDLE_DIR="$PWD/target/openpaperdesign-web" \
  cargo install --locked --path crates/openpaperdesign-serve --bin openpaperdesign-mcp
openpaperdesign-mcp --version       # openpaperdesign-mcp 0.1.0
openpaperdesign-mcp doctor          # exit 0, "browser_editor": "built in", no missing files
command -v openpaperdesign-mcp      # ~/.cargo/bin/openpaperdesign-mcp

# 2. Add the local marketplace and the plugin (existing plugins stay untouched):
codex plugin marketplace add <checkout> --json
codex plugin add openpaperdesign@openpaperdesign-local --json
```

3. Restart Codex.
4. Read back and record:
   - the plugin `openpaperdesign@openpaperdesign-local`, installed and enabled ("Agent + human
     design canvas");
   - its four skills: `openpaperdesign`, `setup`, `design`, `review-design`;
   - the MCP server `openpaperdesign`, started, with `bind_project`, `create_project`,
     `open_editor` and `openpaperdesign_status` among its tools;
   - `openpaperdesign_status`: `version` 0.1.0, `workspace` (the directory Codex actually
     started the server in; it must be your project folder, not the plugin folder, see
     openai/codex#22842), `workspace_usable` true, `"browser_editor": "built in"`;
   - that the `PATH` Codex used found `~/.cargo/bin`;
   - that no file containing the token appears in the workspace or the Codex config.
5. Fixture (a task-owned synthetic project only), in that workspace:
   - `create_project` with the name "Fixture", giving `design/Fixture/`. Or bind an existing
     synthetic project with `bind_project design/<name>`;
   - a `create_shape` call; `design/<name>/pages/*.json` changes;
   - `open_editor`; open the URL in the in-app browser. The canvas shows the shape;
   - move it in the browser and wait for "Saved", then let the agent edit again (it reads
     first). Reload: both changes are there;
   - conflict: the agent reads, you move the shape and it saves, then the agent edits that
     page. The agent gets a `conflict` and nothing is written;
   - rebind: `bind_project design/<missing>` fails and the open tab still saves;
     `bind_project` to another project is refused (needs `force: true`, explains the loss)
     until forced;
   - shutdown: quitting the session stops the editor server (the tab can't save any more).

   Don't paste the token URL into reports.

## Tests and results (2026-10-12, implementation worker sandbox, offline, Rust 1.95.0, Node 26.8.1, Python 3)

| Check | Result |
|---|---|
| `tests/launcher.rs` (the real binary over stdio, rmcp child-process client) | 5 passed (details below) |
| `node --test plugin/tests/plugin.test.mjs` | 7 passed (after the packaging correction): plugin.json fields, no top-level `skills`/`mcpServers`/`apps`/`interface`, shortDescription ≤ 30; every `./` path exists inside its root; no `.codex-plugin` overlay; `mcp.json` has exactly one entry, explicit `type: "stdio"`, the command on PATH, no `cwd` and no token; skills' frontmatter and size; the marketplace path resolves (after links) to the plugin folder inside the repository; notices and licenses. On `1714a58`'s files, 2 fail (top-level `skills`, the overlay) |
| `plugin/tests/design` (`python3 -m unittest`) | 12 passed |
| `cargo install --locked --offline --path crates/openpaperdesign-serve --bin openpaperdesign-mcp --root <tmp>` (the setup skill's command, offline here) | installed; release binary 39,032,168 B; `--version` 0.1.0; `doctor`: `"browser_editor": "built in"`, no missing files |
| Full check list | `cargo fmt --all --check`; clippy 0 warnings on the new crates (host) and only upstream `atomic.rs:30` on wasm32; serve 10 unit + 5 agent + 3 launcher + 17, web 111, shader 2; Node 18 (`session.test.mjs`) + 6 (`plugin.test.mjs`); Python 12; wasm32 check; native build; headless 9/0/0; serial 162/0/0 (= baseline, no quota errors); root clippy 1 upstream warning; `cargo tree -p paper-rs -e features` unchanged; `git diff --check` clean |

The launcher tests:

- `bind_edit_and_open_the_editor_inside_the_workspace_only`:
  - the tools are listed and the refused ones aren't;
  - before a bind, editing and `open_editor` are refused;
  - an absolute outside path, `../`, a symlink out, a non-project folder and a missing folder
    are all refused, with the outside project untouched;
  - binding `design/Landing` works and `create_shape` changes its page file;
  - `open_editor` returns a 127.0.0.1 URL with a 64-hex token. On that port `/api/project`
    gives 200 with the token and 401 with a wrong one, and `/` gives 200 (the editor is built
    in here). No token-bearing file appears in the workspace;
  - `create_project` creates `design/New board` and switches to it, and refuses a duplicate.
    After `open_editor`, the switch is first refused without `force` (and nothing is
    created), then succeeds with it.
- `home_folder_workspace_and_escaping_design_folder_are_refused`.
- `embedded_bundle_list_matches_the_server_allowlist`.
- `a_failed_rebind_keeps_the_current_project_and_its_editor` (B2b additions): it binds A,
  edits it and opens the editor. Then forced binds to a project with an unsafe page id, a
  project with a symlinked page file, a folder without `project.json`, a symlink out of the
  workspace and a missing path all fail. After each one, A is still bound, its editor URL
  answers 200 with its token, and its tools work. No file in the workspace, the outside
  folder or the outside project changed. Rebinding A says "already bound". A switch to a
  valid B is refused without `force` (the message says the edits would be lost, and A still
  answers). With `force`, B is bound and A's server stops listening.
- `the_plugin_folder_is_not_a_workspace`: started in a folder that looks like the installed
  plugin, `create_project` is refused, nothing is created, and `workspace_usable` is false.

Rebased on the B2a review round 1 fixes (`a96b32c`, 2026-10-12). The launcher now passes its
session to the host (`Server::call_tool(session, …)`) and the skill tells the agent to read
again after a `conflict`. Re-run after the rebase (proportionate: serve crate, plugin files):
`cargo fmt --all --check`; clippy for serve and web, 0 warnings; serve 4 unit + 9 agent + 3
launcher + 17; `node --test plugin/tests/plugin.test.mjs`; Python design tests; native build;
`git diff --check`. The rows above are from the pre-rebase run; serial and headless are
inherited from it (no root-package file changed).

B2b additions (rebind safety, the `force` switch, `mcp.json` stdio without `cwd`, install
steps), 2026-10-12. Run: `cargo fmt --all --check`; serve clippy, 0 warnings; serve 4 unit +
9 agent + 5 launcher + 17; plugin tests plus the Node CI command, 27 passed; `git diff
--check`. On `aea6397`'s launcher (with the new tests copied in) 3 fail: after a failed
rebind nothing is bound, a switch after `open_editor` needs no force, and a project is
created inside the plugin folder.

## Not proven

- **Codex itself:** the plugin tests are source-only checks. They cover the forbidden
  top-level keys, the 30-character cap, `mcp.json` with its one stdio entry, every `./` path
  existing, and the marketplace path resolving to this folder inside the repository. Nothing
  here proves that Codex discovers, installs or enables the plugin, or reads only
  `plugin.json`. The other interface limits weren't checked against the reference. So is the app starting the stdio server: `command` on PATH and `cwd` ".", whose
  resolution relative to the workspace (openai/codex#22842) is unverified.
- **Opening the editor:** opening the `open_editor` URL in the in-app browser from a skill
  hasn't been tried.
- **Embedding:** proven here on Linux only; the Mac build and install are the operator's to
  run.
- **Browser behaviour:** everything listed as unproven in the B1 and B2a records still
  applies (for example agent + browser editing in a real browser).
