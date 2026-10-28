# OpenPaperDesign desktop preview

The desktop package runs the existing native Rust editor, without Codex or a plugin. It is separate from the newer Codex Home/workspace UI in PR #9. Both reuse the editor and project format; their host interfaces and access contracts differ.

## Current status

- **macOS / Apple Silicon:** local release binaries and an ad-hoc-signed `.app`/ZIP built on an Apple M4. The bundle launches to Home. Headless Metal rendering of the three public fixtures succeeds. A native Import interaction was interrupted by a computer-use timeout; new/edit/save/reopen, export and a browser-downloaded/quarantined first launch remain unqualified. Not Apple notarized.
- **Windows / x64:** The x64 build, CLI smoke check and ZIP packaging passed on the GitHub Windows runner. First launch and editing on a real Windows desktop remain unqualified.
- **Linux / x64:** CI build/package job prepared. No Linux build or physical-machine pass is claimed yet.

Use the release's own status and checksums. A workflow archive is not a claim that an installer was accepted by Windows/macOS security checks. The source-only v0.1.0 release is still labelled source. No automatic updater, signing credential or security setting is installed by this work.

## Library and existing files

The `openpaperdesign` entrypoint uses `~/Documents/OpenPaperDesign`. An explicit `PAPER_RS_HOME` selects another library. No upstream or Codex library is silently moved or adopted. Native projects retain the upstream folder/JSON/assets format and the existing `.paper-rs` compatibility.

Do not edit the same project in the native app and the Codex browser plugin at the same time. The plugin's checked save/binding path does not protect against an independent native writer.

## Build from source

Use Rust 1.95 or later and the platform's native compiler/linker:

```sh
cargo build --release --locked --bin openpaperdesign --bin paper-rs-mcp
./target/release/openpaperdesign
```

On Windows, use `target/release/openpaperdesign.exe`. On Linux, provide the normal GTK3, X11/Wayland, libxkbcommon and Vulkan/GL runtime libraries. The Ubuntu 24.04 package targets glibc 2.39 or newer. The upstream [build guide](../upstream-README.md#build-and-run) describes those dependencies. A source build does not require an account or an AI agent.

The old `paper-rs` binary remains available for upstream-compatible commands. The thin distribution entrypoint reuses that CLI rather than adding a second editor.

## Package an existing build

```sh
python3 scripts/package-desktop.py --platform macos --arch arm64 --bin-dir target/release --output dist
```

Use `--platform windows --arch x64` or `--platform linux --arch x64` **on the matching build host** with binaries built for that target. The script refuses missing binaries. The Mac packager uses the repository icon and macOS `iconutil`, `codesign` and `ditto`; it only applies an ad-hoc signature. Archives contain both binaries, project/font notices, BUILD.json provenance and an adjacent SHA256 file. BUILD.json marks a dirty source tree honestly.

`Desktop preview packages` builds each target independently and retains archives as workflow artifacts for review. It does not publish a release or deploy the site. No automatic updates are included. Public release assets must come from the reviewed exact build; do not rename another platform's executable.

## Optional AI agents

The native MCP server is on by default. The running app exposes the upstream local MCP endpoint at `http://127.0.0.1:29980/mcp`. Configure a compatible MCP client to use that URL, or use the bundled `paper-rs-mcp` stdio bridge. The app must be running. `PAPER_RS_MCP_PORT` changes the port in **both** app and bridge; `PAPER_RS_MCP=0` disables the app server.

Sketch/paper-rs uses the same default port. Stop the other app or choose a unique port in both app and bridge, and verify the actual receiving library before editing. A successful connection alone does not establish which app received it.

The existing upstream [Claude Code and Codex instructions](../upstream-README.md#ai-agents-mcp) are retained. For a Mac bundle, the bridge executable is `OpenPaperDesign.app/Contents/MacOS/paper-rs-mcp`; on other platforms it is beside the app executable. No client configuration is changed by downloading or opening the app.

This is the **native app's MCP contract**, not the project-bound plugin's workspace/lease contract. The native app exposes its library and local file authoring operations; choose the client access you intend to grant. Loopback/origin checks are not an OS sandbox.

## Required qualification before a public desktop release

For each advertised platform, record the exact source revision, binary/archive hashes and actual platform/architecture. Verify normal first launch, project creation, editing, save, quit/reopen, import/export, and an upgrade with existing data retained. Qualify optional MCP on a disposable library. Test actual downloaded security handling rather than a locally copied app alone. Keep code signing/notarization and physical-device passes distinct from a CI build.

## Upstream selection

The actual merge base with `michaelshimeles/sketch` is `0b1b91da7caa10abae7cbf75d2ce003822c85fe9` after removing the shallow-clone limitation. Subsequent native changes are mostly the Sketch rename, packaging/updater integration and autosave while covered. Six-colour sticky notes, hand-drawn rendering, frames/components, exports and MCP already exist in this fork.

The upstream autosave fix `66893186714a0bb5f16f1e5b9d93e69b5b37a86d` is a relevant candidate: it starts autosave from the event loop when an occluded window does not render. It is **not applied here**, because Factory's PR #9 also changes `src/app.rs` and owns the active save/UI work. Coordinate that bounded patch with the existing writer and regression-test it there. Do not merge the whole upstream branch, rename the fork, adopt upstream's signing/update keys or copy its analytics.

macOS bundle layout was informed by Rasmic's release script. This implementation is a small packager without Sparkle, downloaded frameworks, an update feed or signing credentials. Core editor and bundled licence attribution are retained.
