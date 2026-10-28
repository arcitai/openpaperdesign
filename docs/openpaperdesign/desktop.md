# OpenPaperDesign desktop preview

The desktop package runs the existing native Rust editor, without Codex or a plugin. The main-branch packaging prototypes do not yet include the new UI/UX shown on the product page. Those images were rendered with the frozen PR #9 source at `1b04c31`, where the shared native workspace also has the new UI. Public desktop release is held until packages include that qualified UI and pass the normal app journey. Both hosts reuse the editor and project format; their access contracts differ.

## Current status

- **macOS / Apple Silicon:** local release builds and ad-hoc-signed bundles on an Apple M4. The newer UI integration successfully opens Home, opens a real project, accepts an edit, saves it and reopens that saved change after quitting. Native MCP was observed on this exact app process. During an earlier blocked launch, process sampling showed synchronous project enumeration under Documents before the event loop. The new entrypoint defaults to local application data instead. This fixes the observed startup block without granting blanket Documents access or moving existing projects. The owner's earlier unspecified error dialog remains unreproduced. Actual downloaded/quarantined first launch and notarization are separate and unqualified.
- **Windows / x64:** The corrected x64 build and ZIP packaging passed on the GitHub Windows runner at `04c738e` using Bash to gate both CLI commands. The static-CRT Windows package also passed at `61d94cc`. The GUI-subsystem app does not open a console from Explorer; CLI calls retain redirected output and attach to a parent console when needed. First launch and editing on a real Windows desktop remain unqualified.
- **Linux / x64:** The corrected x64 build, CLI smoke checks and tarball packaging passed on the GitHub Ubuntu runner at `04c738e`. No physical-machine pass is claimed.

Use the release's own status and checksums. A workflow archive is not a claim that an installer was accepted by Windows/macOS security checks. The source-only v0.1.0 release is still labelled source. No automatic updater, signing credential or security setting is installed by this work.

## Library and existing files

The `openpaperdesign` entrypoint uses the platform's local application-data folder:
`~/Library/Application Support/OpenPaperDesign/projects` on macOS, `%LOCALAPPDATA%/OpenPaperDesign/projects` on Windows, and the XDG data location on Linux. An explicit `PAPER_RS_HOME` selects another library. Existing Documents projects, the upstream library and the plugin library are not moved or silently adopted. Native projects retain the upstream folder/JSON/assets format and `.paper-rs` compatibility.

Desktop launches keep private local diagnostic files under the adjacent `OpenPaperDesign/logs` directory. The ordinary log is bounded to 1 MiB plus a truncation marker; an adjacent `.error` file retains the latest error (up to 64 KiB) independently of that quota. Nothing is uploaded. CLI calls use their normal stderr instead. Logs may include project paths or error content: inspect them before sharing.


Do not edit the same project in the native app and the Codex browser plugin at the same time. The plugin's checked save/binding path does not protect against an independent native writer.

## Build from source

Use Rust 1.95 or later and the platform's native compiler/linker:

```sh
cargo build --release --locked --bin openpaperdesign --bin paper-rs-mcp
./target/release/openpaperdesign
```

On Windows, use `target/release/openpaperdesign.exe`. The workflow statically links the MSVC C runtime. A manual build without that flag may require the Microsoft Visual C++ runtime; the GUI executable does not create a console when launched from Explorer. On Linux, provide the normal GTK3, X11/Wayland, libxkbcommon and Vulkan/GL runtime libraries. The Ubuntu 24.04 package targets glibc 2.39 or newer. The upstream [build guide](../upstream-README.md#build-and-run) describes those dependencies. A source build does not require an account or an AI agent.

The old `paper-rs` binary remains available for upstream-compatible commands. The thin distribution entrypoint reuses that CLI rather than adding a second editor.

## Package an existing build

```sh
python3 scripts/package-desktop.py --platform macos --arch arm64 --bin-dir target/release --output dist
```

Use `--platform windows --arch x64` or `--platform linux --arch x64` **on the matching build host** with binaries built for that target. The script refuses missing binaries. The Mac packager uses the repository icon and macOS `iconutil`, `codesign` and `ditto`; it only applies an ad-hoc signature. Archives contain both binaries, project/font notices, BUILD.json provenance and an adjacent SHA256 file. BUILD.json marks a dirty source tree honestly.

`Desktop packages` builds a universal macOS archive (Intel and Apple Silicon slices), Windows x64 ZIP and Linux x64 tarball. It reuses a pinned, OS/target-specific Rust cache without skipping builds or tests. It tests the native library, app entrypoint and project behavior in the release profile, restores only generated screenshot outputs, then refuses dirty distribution sources. Packages contain input binary hashes and source/component provenance; adjacent checksums describe the final archives. Candidate branches and PRs retain exact archives for review. A `v*` tag reachable from main can create a draft prerelease only after every platform succeeds and all archive checksums/provenance agree. Existing releases are never replaced. Publication is deliberate; the workflow does not deploy the site. No automatic updates are included. Public release assets must come from the reviewed exact build; do not rename another platform's executable.

## Optional AI agents

The native MCP server is on by default. The running app exposes the upstream local MCP endpoint at `http://127.0.0.1:29980/mcp`. Configure a compatible MCP client to use that URL, or use the bundled `paper-rs-mcp` stdio bridge. The app must be running. `PAPER_RS_MCP_PORT` changes the port in **both** app and bridge; `PAPER_RS_MCP=0` disables the app server.

Sketch/paper-rs uses the same default port. Stop the other app or choose a unique port in both app and bridge, and verify the actual receiving library before editing. A successful connection alone does not establish which app received it.

The existing upstream [Claude Code and Codex instructions](../upstream-README.md#ai-agents-mcp) are retained. For a Mac bundle, the bridge executable is `OpenPaperDesign.app/Contents/MacOS/paper-rs-mcp`; on other platforms it is beside the app executable. No client configuration is changed by downloading or opening the app.

This is the **native app's MCP contract**, not the project-bound plugin's workspace/lease contract. The native app exposes its library and local file authoring operations; choose the client access you intend to grant. Loopback/origin checks are not an OS sandbox.

## Required qualification before a public desktop release

For each advertised platform, record the exact source revision, binary/archive hashes and actual platform/architecture. Verify normal first launch, project creation, editing, save, quit/reopen, import/export, and an upgrade with existing data retained. Qualify optional MCP on a disposable library. Test actual downloaded security handling rather than a locally copied app alone. Keep code signing/notarization and physical-device passes distinct from a CI build.

## Upstream selection

The actual merge base with `michaelshimeles/sketch` is `0b1b91da7caa10abae7cbf75d2ce003822c85fe9` after removing the shallow-clone limitation. Subsequent native changes are mostly the Sketch rename, packaging/updater integration and autosave while covered. Six-colour sticky notes, hand-drawn rendering, frames/components, exports and MCP already exist in this fork.

The upstream autosave fix `66893186714a0bb5f16f1e5b9d93e69b5b37a86d` is a relevant candidate: it starts autosave from the event loop when an occluded window does not render. It is isolated on a separate port branch based on Factory PR #9, preserving this fork's `autosave_due_ui` guard. A no-frame, timed save/reopen regression passes locally. The integration candidate records its exact port revision; PR21 itself does not silently adopt Factory's whole branch. Do not merge the whole upstream branch, rename the fork, adopt upstream's signing/update keys or copy its analytics.

macOS bundle layout was informed by Rasmic's release script. This implementation is a small packager without Sparkle, downloaded frameworks, an update feed or signing credentials. Core editor and bundled licence attribution are retained.
