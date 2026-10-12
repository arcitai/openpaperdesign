---
name: setup
description: Set up openpaperdesign on this machine (build the browser editor and install the openpaperdesign-mcp command), and check it.
metadata:
  version: "0.1.0"
---

# Set up openpaperdesign

The plugin starts a local command, `openpaperdesign-mcp`, that must be on the `PATH` the
host app sees. It is built from the openpaperdesign repository. Nothing is downloaded at
run time; there is no cloud service and no telemetry.

Check first: run `openpaperdesign-mcp --version` and `openpaperdesign-mcp doctor`. `doctor`
prints JSON and exits 0 when the command, the built-in browser editor and the platform are
fine. Report its output as is.

To install, in a checkout of https://github.com/onlinesourdough/openpaperdesign (macOS or
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

Restart the app so it starts the plugin's MCP server. Build the web
bundle before installing: it is compiled into the command, and without it `open_editor`
refuses ("build the web bundle first") while the editing tools still work.

Ask before installing anything; these commands change the user's machine. Say plainly what
was checked and what wasn't (for example, whether the app actually started the server).
