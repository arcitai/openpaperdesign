# Foundation and wasm32 compile feasibility — 2026-10-12

Scope: [issue #1](https://github.com/onlinesourdough/openpaperdesign/issues/1), first slice.
This record proves compile feasibility only. It does not show that anything runs in a
browser.

Source: upstream paper-rs `0b1b91da7caa10abae7cbf75d2ce003822c85fe9`, branch
`foundation/wasm-core-feasibility`. Run by the Claude implementation worker in its sandboxed
Linux worktree, using the task-local toolchain in `.toolchain/`. Everything ran with
`--locked --offline`. Logs are in `target/evidence/` (gitignored, not retained by git).

## Toolchain

```
rustc 1.95.0 (59807616e 2026-04-14)
cargo 1.95.0 (f2d3ce0bd 2026-03-21)
installed targets: wasm32-unknown-unknown, x86_64-unknown-linux-gnu
cargo, rustc -> <worktree>/.toolchain/cargo/bin/
```

## Native baseline (unmodified source) and after this change

| Check | Baseline | After |
|---|---|---|
| `cargo build --all-targets` | pass | pass |
| `cargo test` (default, parallel) | exit 101: `tests/mcp.rs` SIGSEGV; all other binaries pass | same |
| `cargo test -- --test-threads=1` | 162 passed, 0 failed, 0 ignored | 162 passed, 0 failed, 0 ignored |
| per binary (serial) | lib 108, excalidraw 7, figma 9, headless 9, io 5, mcp 12, perf 1, projects 6, workflow 5 | same |
| `PAPER_RS_REQUIRE_GPU=1 cargo test --test headless` | 9 passed; adapter `llvmpipe (LLVM 22.1.8, 256 bits) (Gl)` | 9 passed |
| GPU skips | none: no `no GPU adapter; skipping` in a `--nocapture` MCP run | — |
| `cargo clippy --all-targets` | 1 warning (`clippy::drop_non_drop`, `tests/perf.rs:105`), 0 errors | same |
| `cargo fmt --check` | clean | clean (`--all`, including the new crate) |

The parallel SIGSEGV shows the same symptom as the operator's host baseline for the same commit. This sandbox
renders with llvmpipe through GL, not the operator's Vulkan driver. The root cause is still not
established. Serial runs are a workaround, not a fix. The tests rewrite tracked
`docs/screenshots/*.png`. Those files were restored to HEAD after every run.

## wasm32 result

Approach (a): workspace member `crates/openpaperdesign-core`. Its `lib.rs` includes the
upstream files through `#[path]`, with module paths that mirror the upstream crate. None of
the upstream source files under `src/` were edited.

```
cargo check --locked --offline -p openpaperdesign-core --target wasm32-unknown-unknown
=> Finished, 0 warnings, 0 errors
```

Included: `geom`, `vector`, `model` (document, node, paint, rough), `history`, `layout`,
`ink`, `text` (bundled Inter fonts via `include_bytes!`), `theme` (egui styling; `text` takes
its Regular font from it), `connector`, `assets`, `io::atomic`, `render::{draw, mesh, tess,
rough}`. Also passes as a host `cargo check`. `cargo clippy` on wasm32 reports one upstream
warning, `drop_non_drop` at `src/io/atomic.rs:30`. It appears because `std::fs::File` is an
inert stub on this target.

### Renderer boundary (temporary probes, not committed)

1. **wgpu 30.0.1 itself compiles for wasm32** with the root's feature set (wgpu defaults).
   That set includes the `webgpu` backend but not `webgl`, so there is no WebGL2 fallback as
   configured. `pollster` compiles too.
2. **`render/gpu.rs` stops at `render/scene.rs:6`** (`use crate::editor::{Editor, hit,
   snap::Guide}`). `gpu.rs` needs `scene::{CanvasFrame, Item}`. `scene.rs` builds both the
   document frame and the editor overlays (`overlay` at :288, `whiteboard_overlay` at :422,
   `vector_edit_overlay` at :504, `canvas(ed: &Editor, …)` at :589) in the same file.
3. Adding the editor's closure (`editor`, all of `io`, `library`, `project`, `demo`) hits the
   **first hard dependency stop: `arboard` 3.6.1 does not compile for wasm32** (no `platform`
   backend). Its only user is `src/io/clipboard.rs:49,53,69`.
4. With a throwaway arboard API stub (outside the repo), **the rest type-checks on wasm32 with
   0 errors and 0 warnings**: `editor`, `io`, `project`, `library`, `demo`, `render::scene`
   and `render::gpu`. winit, usvg, i_overlay, miniz_oxide and dirs all compile for this
   target. The excluded modules are `app`, `ui`, `render::frame`, `session`, `mcp` and
   `macos`. They need egui-winit/egui-wgpu windowing, rfd and Tokio/rmcp.

Compile success here says little about runtime. The items below compile but panic or do
nothing on `wasm32-unknown-unknown`.

## Upstream diff

Against `0b1b91da`: no upstream source file under `src/` or `tests/` was edited.

| Path | Change | Why |
|---|---|---|
| `Cargo.toml` | +3 | `[workspace] members = [".", "crates/openpaperdesign-core"]`. Root stays the default member, so plain `cargo build/test` behave as before |
| `Cargo.lock` | +15 | one added `[[package]] openpaperdesign-core` entry. No version changes (`cargo update --workspace --offline`: "Locking 1 package") |
| `.gitignore` | +1 | `.toolchain/` |

New files: `crates/openpaperdesign-core/{Cargo.toml,src/lib.rs}`, `AGENTS.md`,
`.github/workflows/ci.yml`, this record. `PROGRESS.md` is left unchanged: it is upstream's
log, and a pointer there would only add sync conflicts.

## Native-only couplings blocking a browser editor

- **Clocks** (`Instant::now` panics on wasm32-unknown-unknown; `SystemTime::now` too):
  `src/session.rs:9,20,22,67,78,82,141,156,219`, `src/app.rs:15,85,203,214,719,1206,1218`,
  `src/project/format.rs:240`, `src/ui/home.rs:8,34`.
- **Blocking GPU calls**: `pollster::block_on` at `src/render/gpu.rs:736,744` and
  `src/app.rs:856,863`. Readback `map_async` followed by `poll(wait_indefinitely)` at
  `src/render/gpu.rs:809,811` needs async handling for export, screenshot and thumbnail
  (used by `session.rs`).
- **Threads**: `src/mcp/server.rs:206` (Tokio thread), `src/project/mod.rs:412`
  (`thread::sleep`).
- **Filesystem** (std::fs is an inert stub on wasm32): `src/project/mod.rs` (21 uses),
  `src/io/atomic.rs`, `src/io/file.rs:82-87`, `src/assets.rs:93,97,111`,
  `src/library.rs`, `src/session.rs:162,234,265`, `src/mcp/exec.rs:1054,1128,1295`;
  `dirs` at `src/project/mod.rs:81` and `src/mcp/exec.rs:1028`.
- **Clipboard**: `arboard` at `src/io/clipboard.rs:49-69` (compile blocker).
- **Dialogs/windowing**: `rfd` at `src/app.rs:129-144`. winit/egui-winit/egui-wgpu in
  `src/app.rs`, `src/render/frame.rs`. `ui/` depends on `app` state.
- **Input platform**: `cfg!(target_os = "macos")` decides Cmd vs Ctrl at
  `src/editor/input.rs:29,36` and `src/editor/shortcuts.rs:337,357,364,623`. A wasm build
  picks Ctrl even in a Mac browser.
- **Renderer/editor coupling**: `src/render/scene.rs:6` (above).
- **Randomness**: fastrand 2.5 without its `js` feature has no seed source on
  wasm32-unknown-unknown and falls back to a fixed seed (`io::atomic` temp names).
- **Dependency gating**: `Cargo.toml` declares desktop and Tokio/rmcp deps unconditionally,
  and `src/lib.rs` declares `app` and `mcp` unconditionally. The adapter crate avoids this by
  selecting modules. A browser build of more than the core needs target-specific
  dependencies, not just skipping calls at startup.

## Architecture-review corrections carried as follow-ups

From `.toolchain/operator-evidence/architecture-review.md` (SOUND WITH CORRECTIONS):

1. Preserve font and upstream notices (`assets/fonts/OFL.txt`, `THIRD_PARTY.md`, licenses)
   in any distributed artifact.
2. GPU readback for export, screenshot and thumbnail must become async in a browser
   (`gpu.rs:809`).
3. `session.rs` is reusable but not browser-ready: clocks, headless GPU and native storage.
4. Decide the Cmd/Ctrl mapping for the browser input platform.
5. Gate native dependencies unconditionally per target, not only at startup.
6. `mcp/exec` keeps filesystem authority even without a transport (`exec.rs:1295` reads
   arbitrary paths). Adapt it explicitly if agent commands are kept.
7. Later probes must exercise a delivered module and the real renderer, not API presence.
   Keep the Codex-browser and MCP-App gates independent.

## Not proven

- Any browser runtime: no wasm module was built with wasm-bindgen, delivered or run.
- The renderer in a browser: no WebGPU or WebGL adapter, device, pipeline or readback was
  created on wasm32. MSAA, WGSL pipelines and blend modes are unvalidated there.
- A Codex built-in browser surface, an MCP App panel, or any plugin.
- Payload size and memory: fonts alone are 2,511,796 bytes before compression.
- The CI workflow has not run. Its action is not yet pinned to a commit SHA, and lavapipe
  availability on the runner is unverified.
