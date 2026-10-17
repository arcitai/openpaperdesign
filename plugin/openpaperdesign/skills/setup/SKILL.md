---
name: setup
description: Set up openpaperdesign on this machine (build the browser editor and install the openpaperdesign-mcp command), and check it.
metadata:
  version: "0.1.0"
---

# Set up openpaperdesign

Workspace contract updated 2026-10-13 (Codex 0.162.1 experimental support).

The plugin starts a local command, `openpaperdesign-mcp`, that must be on the `PATH` the
host app sees. It is built from the openpaperdesign repository. Nothing is downloaded at
run time; there is no cloud service and no telemetry.

Check first: run `openpaperdesign-mcp --version` and `openpaperdesign-mcp doctor`. `doctor`
prints JSON and exits 0 when the command, the built-in browser editor and the platform are
fine. Report its output as is.

The package starts `openpaperdesign-mcp --codex-context`. Codex starts it in the plugin
folder; that folder cannot authorize edits. Workspace and permissions come from Codex
0.162.1's experimental `codex/sandbox-state-meta` on each real **model tool call**. Direct
app-server tool calls do not supply it. A successful standalone `doctor` proves neither
host-context delivery nor installed editing. Call `openpaperdesign_status` through the
model and require the intended workspace, `workspace_source: codex/sandbox-state-meta`,
and `workspace_usable: true` before binding.

Supported permission profiles are the observed managed/restricted workspace-write shape
with network `restricted` or `enabled`, and exact `{"type":"disabled"}`. Neither network
value changes adapter networking. The latter profile describes the host sandbox, not
plugin scope. A Git worktree's regular `.git` file stays opaque; its pointer is never
read or followed. Protected paths and symlinked metadata remain refused.
Read-only, missing and unknown profiles refuse binding/editing/browser access. Keep the
reported failure visible; do not change grants, inject metadata or edit cached config.
The source checkout's `docs/openpaperdesign/plugin-b2b.md` records the precise supported
shape and protected-path restrictions; repository docs are not bundled with the plugin.

Standalone use remains `openpaperdesign-mcp --workspace <project-workspace>` (or deliberate
process cwd); it is separate from packaged host mode. The two flags cannot be combined.

To install, in a checkout of https://github.com/arcitai/openpaperdesign (macOS or
Linux; Rust 1.95 with the `wasm32-unknown-unknown` target, and `wasm-bindgen` CLI 0.2.129):

```sh
./scripts/build-web.sh                       # the browser editor (wasm + page)
cargo install --locked --path crates/openpaperdesign-serve --bin openpaperdesign-mcp
openpaperdesign-mcp doctor                    # "browser_editor": "built in"
```

`cargo install` puts the command in `~/.cargo/bin`; make sure the host app's `PATH`
includes it. With a shared `CARGO_TARGET_DIR`, set `OPD_WEB_BUNDLE_DIR` to the built bundle
(`<checkout>/target/openpaperdesign-web`) for `cargo install`, and check `doctor` afterwards.

Then add the plugin in Codex, from the same checkout:

```sh
codex plugin marketplace add <checkout> --json
codex plugin add openpaperdesign@openpaperdesign-local --json
```

When replacing an already installed 0.1.0 package after review, refresh through the normal
CLI: `codex plugin remove openpaperdesign@openpaperdesign-local --json`, then the `plugin add`
command above. Verify the refreshed MCP args include `--codex-context`; never edit the cache.
This refresh is an operator step, not evidence that the replacement was installed.

Restart the app so it starts the plugin's MCP server. Build the web
bundle before installing: it is compiled into the command, and without it `open_editor`
refuses ("build the web bundle first") while the editing tools still work.

Ask before installing anything; these commands change the user's machine. Say plainly what
was checked and what wasn't (for example, whether the app actually started the server).
