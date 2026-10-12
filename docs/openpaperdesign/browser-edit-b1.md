# B1: editing a paper-rs project in a browser — 2026-10-12

Scope: [issue #1](https://github.com/onlinesourdough/openpaperdesign/issues/1), slice B1. A
loopback server opens one existing paper-rs project folder. A browser page runs the upstream
editor and canvas renderer as wasm and saves through the server with revision checks.
Agent/MCP writes are B2.

## Run it

```sh
./scripts/build-web.sh --offline          # cargo build (wasm32, release) + wasm-bindgen 0.2.129
cargo run --locked -p openpaperdesign-serve -- --project "<project folder>" [--port 29981]
# open the printed http://127.0.0.1:<port>/#token=… URL in a browser on the same machine
```

The page shows the project's active page. It supports select, drag-move, the Rectangle and
Ellipse tools, Delete, and undo/redo (toolbar or ⌘/Ctrl+Z, ⌘/Ctrl+Shift+Z). All of upstream's
other canvas shortcuts work as well. Each change is saved about 300 ms after the gesture ends.

## Pieces

- `crates/openpaperdesign-web` (wasm32 cdylib). Compiles upstream `editor`, `project`, `io`,
  `render::{scene, gpu, …}` and the rest of the non-UI modules from `src/` through `#[path]`,
  plus `web.rs` (wasm-bindgen API) and `keys.rs` (DOM key and modifier mapping).
  `static/{index.html, app.js}` holds the page. It has no egui panels.
  - **Rendering:** upstream `scene::canvas` → `gpu::CanvasRenderer` on a wgpu surface for the
    `<canvas>`. WebGPU is tried first, and WebGL2 is compiled in as a fallback (glow, offline).
    `?backend=webgl` forces WebGL2. The page shows which backend and adapter actually ran.
  - **Clipboard:** `openpaperdesign-clipboard-stub` is imported as `arboard`, so upstream
    `io/clipboard.rs` compiles unchanged. There is no system clipboard, so copy and paste work
    within the page only.
  - **Clocks:** times come from `performance.now()` / `event.timeStamp`. No upstream code path
    used here calls `Instant`/`SystemTime` (`project::now_ms` is only reached when creating a
    project, which the web side never does).
  - **Randomness:** `fastrand` is seeded from `crypto.getRandomValues` at startup. On
    wasm32-unknown-unknown it otherwise starts from a fixed seed, so page and project ids made
    by `project::new_id` could repeat. Node ids are per-document counters and never random.
  - **Input platform:** upstream picks the command modifier at compile time (Cmd on macOS,
    Ctrl otherwise), and a wasm build counts as "otherwise". When `navigator` reports a Mac,
    the page reports Meta (⌘) to the editor as `ctrl` and Control as `logo`. Shortcuts then
    use ⌘ on a Mac and Ctrl elsewhere. Upstream is unchanged.
- `crates/openpaperdesign-serve` (native bin, axum/tokio as in the root lockfile). It depends
  on the `paper-rs` library itself, so saves use upstream `Project::save_page` (format and
  atomic write).

## Server security and revision model

- **Binding and project:** binds `127.0.0.1` only. `--project` must already be a paper-rs
  project (`project.json`, checked with upstream `Project::open`); nothing is created.
- **Token:** every launch makes a random token (32 bytes from the OS RNG via `getrandom`). The
  printed URL carries it in the fragment, which never reaches the server, logs or `Referer`.
  The page moves it into `sessionStorage` for this tab, removes it from the address bar and
  sends `Authorization: Bearer …` on `/api/*`. A restart makes a new token, and the old one
  then gets 401.
- **Request checks:** the server refuses a `Host` other than `127.0.0.1:<port>` /
  `localhost:<port>` (DNS rebinding), an `Origin` other than those, and
  `Sec-Fetch-Site` values other than `same-origin`/`none`. Responses carry `no-store`,
  `nosniff`, `no-referrer` and a same-origin CSP with `frame-ancestors 'none'`. Only the four
  bundle files are served.
- **Pages:** pages are addressed only by ids listed in the manifest, which must also be plain
  `[A-Za-z0-9_-]`, max 64 characters. The server builds every path. The body's page id must
  match the URL. The body must parse and validate as an upstream page. The manifest's page
  name wins, as in upstream `load_page`.
- **Revisions:** a page's revision (ETag) is a 64-bit SipHash (std `DefaultHasher`, fixed
  keys) of its file bytes as they are on disk now, or `"absent"` for a listed page with no
  file. It detects changes; it isn't a cryptographic digest. `PUT` needs `If-Match`; without
  it the server answers 428, and with a stale value it answers 409 plus the current ETag,
  writing nothing. A process-wide lock serializes check and write. A successful save also
  bumps `manifest.modified`, like the native session.
- **Browser on 409:** the page stops saving and keeps the local edits in the tab without
  writing them. It shows a banner asking to reload from disk, and never overwrites.
- **B2 hook:** an MCP/agent writer added to this same server takes the same lock and the same
  `If-Match` revision, making the server the single authority for writers that go through
  it.

### Boundary: other writers are not protected

The server guarantees revision-checked, serialized writes only for writers that go through
it. Because the revision is computed from the file on disk, a change by any other writer (a
running native paper-rs, an editor, a sync tool) makes the browser's next save fail with 409
instead of overwriting it. The reverse is not protected: a running native app or other writer
can overwrite the browser's saves. A write landing between the server's revision check and
its atomic rename is also not detected. Reopening the project in native paper-rs after the
server stops is supported, and was checked below. Editing it in both at the same time is not
supported.

## Results (2026-10-12, implementation worker sandbox, offline, Rust 1.95.0)

| Check | Result |
|---|---|
| Native `cargo build --all-targets` | pass |
| `cargo test -- --test-threads=1` | 162 passed, 0 failed, 0 ignored (= baseline) |
| `cargo test` (parallel) | `tests/mcp.rs` SIGSEGV, others pass (= baseline) |
| `PAPER_RS_REQUIRE_GPU=1 cargo test --test headless` | 9 passed |
| `cargo clippy --all-targets` (root) | 1 upstream warning, `tests/perf.rs:105` (= baseline) |
| `cargo clippy` on the new crates, host and wasm32 | clean, apart from upstream `src/io/atomic.rs:30` (`drop_non_drop`, wasm32 only) |
| `cargo fmt --all --check` | clean |
| `cargo test -p openpaperdesign-serve` | 8 passed: stale `If-Match` → 409, file unchanged; outside change → 409; ids/traversal/body-id rejected, no stray files; Host/Origin/fetch-site/token; non-projects refused; restart serves the same bytes and revision; native `Session::open` sees the saved move; absent page file |
| `cargo test -p openpaperdesign-web` (host) | 97 passed (upstream unit tests compiled via `#[path]` + 3 key-mapping tests) |
| `cargo check -p openpaperdesign-web --target wasm32-unknown-unknown` | pass, 0 warnings |
| Bundle (`--release`, wasm-bindgen 0.2.129) | `openpaperdesign_web_bg.wasm` 11,648,700 B (4,228,063 B gzip -9), `openpaperdesign_web.js` 124,585 B, `app.js` 6,508 B, `index.html` 2,139 B |
| Real loopback HTTP, release bundle, demo project | bundle files 200 with the right content types; API without token 401, foreign Host 403, cross Origin 403, traversal 400; PUT with current revision 200 (legacy v1 page saved as page v3); PUT with stale revision 409, file unchanged; restart: new token, old token 401, same ETag and bytes |
| Native `paper-rs --export-svg` / `--screenshot` on that project | both open it. The screenshot (llvmpipe) shows all 23 layers with the moved text |

The demo project was built from upstream's `docs/screenshots/m6_launch_plan.paper-rs` (a
legacy single-file page) in a temporary folder.

### Browser runtime: not run here

Headless Chromium 152 could not start in the implementation sandbox:
`FATAL:chrome/browser/process_singleton_posix.cc:297 Check failed: . socket() failed:
Operation not permitted (1)`. The sandbox wasn't widened and Chromium's own sandbox was not
disabled. So in-browser loading, the drag/create/delete/undo/redo sequence, the backend
actually used and screenshots are **not yet proven**. Run them on the host, where the server
and browser share loopback:

```sh
node scripts/browser-check.mjs '<printed URL>' target/evidence/b1-browser edit
```

The script drives the real page through CDP. It drags a node, draws a rectangle with the
toolbar tool, deletes it, undoes with Ctrl+Z, then redoes and undoes from the toolbar. It
waits for each save, takes screenshots, reloads, and records the backend, adapter and node
positions to `result-edit.json`. Afterwards, check the page file on disk, restart the server
and run `… verify`, then open the folder in native `paper-rs`.

## Not proven / open

- The whole browser runtime (above): WebGPU or WebGL2 in a real browser, the renderer's WGSL
  pipelines, 4× MSAA and blend modes on those backends, and pointer/keyboard behavior.
- Image fills: the browser registers no project assets, so image fills don't draw. Saving
  keeps their references.
- Text editing UI, inspector, layers, multiple pages (only the active page loads), export,
  thumbnails, async GPU readback, IME and touch/pen pressure.
- Mac browser ⌘ mapping is implemented and unit-tested but not tried on a Mac.
- ~4.2 MB gzip download: the fonts alone are 2.5 MB uncompressed. Size work is open.
- Codex built-in browser, MCP App panel, plugin packaging and B2 agent writes.
