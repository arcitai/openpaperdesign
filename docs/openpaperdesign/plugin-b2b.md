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
  - **One project per process:** binding again switches; changes are already saved.
  - **Serving:** the bound project is served by the B2a server, in the same process, on
    127.0.0.1 with a fresh 32-byte token. Tool calls go to that server's single host, with the
    same write lock, policy, atomic saves and revisions as its HTTP `/mcp`, so the browser and
    the agent edit the same document.
  - **`openpaperdesign-mcp doctor`** prints the setup as JSON (exit 0 only when the editor is
    built in and the platform is supported). `--version` prints the version.
- **The web bundle is compiled into the command.** `build.rs` embeds
  `target/openpaperdesign-web` (or `$OPD_WEB_BUNDLE_DIR`) when all six files exist. Otherwise
  the list is empty and `open_editor` says "build the web bundle first", without guessing a
  path at run time. Editing through the tools still works without it.
- **`plugin/openpaperdesign/`:**
  - `plugin.json` (`$schema` agent-plugins 1.0.0, MIT OR Apache-2.0, repository
    onlinesourdough/openpaperdesign, version 0.1.0, `extensions.com.openai.interface` and
    `onboardingSkill`), and an identical `.codex-plugin/plugin.json` overlay;
  - `mcp.json`: stdio `{"command": "openpaperdesign-mcp", "args": [], "cwd": "."}`;
  - `NOTICE.md`;
  - skills `openpaperdesign` (bind, edit, `open_editor`, in-app browser, limits), `setup`
    (build, install, `doctor`), and the ported `design` and `review-design`.
- **`.agents/plugins/marketplace.json`**: the local marketplace `openpaperdesign-local`,
  pointing at `./plugin/openpaperdesign`.

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

```sh
# 1. Build the browser editor and install the command (Rust 1.95 + wasm32 target,
#    wasm-bindgen CLI 0.2.129), from a checkout of this repository:
./scripts/build-web.sh
cargo install --locked --path crates/openpaperdesign-serve --bin openpaperdesign-mcp
openpaperdesign-mcp --version && openpaperdesign-mcp doctor    # "browser_editor": "built in"
```

2. The command lands in `~/.cargo/bin`. The Codex app must see it on its `PATH`.
3. Add the repository's local marketplace in Codex (it reads
   `<repo>/.agents/plugins/marketplace.json`), install `openpaperdesign`, and enable it
   (`[plugins."openpaperdesign@openpaperdesign-local"] enabled = true` in
   `.codex/config.toml`).
4. Restart the Codex app.
5. Check that the plugin and its four skills are listed, and that the `openpaperdesign` MCP
   server started.
6. In a workspace with a fixture project under `design/`, ask the agent to:
   - bind it (`bind_project design/<name>`);
   - edit it (for example `create_shape`);
   - call `open_editor` and open the URL in the in-app browser.

   Edit in the browser, then let the agent edit again. Both changes must persist across a
   reload.

## Tests and results (2026-10-12, implementation worker sandbox, offline, Rust 1.95.0, Node 26.8.1, Python 3)

| Check | Result |
|---|---|
| `tests/launcher.rs` (the real binary over stdio, rmcp child-process client) | 3 passed (details below) |
| `node --test plugin/tests/plugin.test.mjs` | 6 passed: plugin.json fields and `./` paths, overlay equal, `mcp.json` command on PATH and no token, skills' frontmatter and size, marketplace → plugin, notices and licenses |
| `plugin/tests/design` (`python3 -m unittest`) | 12 passed |
| `cargo install --locked --offline --path crates/openpaperdesign-serve --bin openpaperdesign-mcp --root <tmp>` (the setup skill's command, offline here) | installed; release binary 39,032,168 B; `--version` 0.1.0; `doctor`: `"browser_editor": "built in"`, no missing files |
| Full check list | `cargo fmt --all --check`; clippy 0 warnings on the new crates (host) and only upstream `atomic.rs:30` on wasm32; serve 10 unit + 5 agent + 3 launcher + 17, web 111, shader 2; Node 18 (`session.test.mjs`) + 6 (`plugin.test.mjs`); Python 12; wasm32 check; native build; headless 9/0/0; serial 162/0/0 (= baseline, no quota errors); root clippy 1 upstream warning; `cargo tree -p paper-rs -e features` unchanged; `git diff --check` clean |

The three launcher tests:

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
- `home_folder_workspace_and_escaping_design_folder_are_refused`.
- `embedded_bundle_list_matches_the_server_allowlist`.

## Not proven

- **Codex itself:** discovering and installing the plugin from the local marketplace, the
  exact `plugin.json`/marketplace field names (from the lead's reading of the docs; not
  checked against the schemas, which weren't available offline), and enabling it are
  untested. So is the app starting the stdio server: `command` on PATH and `cwd` ".", whose
  resolution relative to the workspace (openai/codex#22842) is unverified.
- **Opening the editor:** opening the `open_editor` URL in the in-app browser from a skill
  hasn't been tried.
- **Embedding:** proven here on Linux only; the Mac build and install are the operator's to
  run.
- **Browser behaviour:** everything listed as unproven in the B1 and B2a records still
  applies (for example agent + browser editing in a real browser).
