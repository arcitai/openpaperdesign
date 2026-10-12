# AGENTS.md — openpaperdesign

Revision: 0.12 · Updated: 2026-10-12

openpaperdesign is a fork of the
[michaelshimeles/paper-rs](https://github.com/michaelshimeles/paper-rs) repository. The goal is an editable paper-rs canvas inside an agent host, such as Codex's
built-in browser or an MCP App panel ([issue #1](https://github.com/onlinesourdough/openpaperdesign/issues/1)).
It is not affiliated with or endorsed by Paper (paper.design). Do not suggest that it is.

## Upstream and credits

- Keep upstream licenses, notices and history: `LICENSE-MIT`, `LICENSE-APACHE`,
  `THIRD_PARTY.md`, `assets/fonts/OFL.txt` (Inter, SIL OFL 1.1). Any distributed build must
  carry the applicable notices.
- Keep diffs to upstream-owned paths (`src/`, `tests/`, root `Cargo.toml`/`Cargo.lock`,
  `README.md`, `PROGRESS.md`) minimal. Put new code in separate adapter crates under
  `crates/`, which reuse upstream modules via `#[path]`. If an upstream file has to change,
  give the reason in the commit.
- Upstream source changes so far (keep this list current):
  - `src/render/gpu.rs`: the image texture is sampled in uniform control flow (B1, WebGPU
    validation in browsers).
  - `src/history.rs`, `src/editor/pages.rs`: `#[derive(Clone)]` on `History`, `PageStash`
    and `PageSlot` (B2a review round 5). Derive-only, no behaviour change. It lets the agent
    host restore the exact editor state after a failed MCP call.
  - `src/mcp/exec.rs`: `pub fn with_no_stretch` gives access to the process-wide record of
    containers that don't stretch their children (B2a final review). Accessor only, no
    behaviour change. The host copies and restores its own pages' entries with the editor. A
    newly opened project drops stale entries for its page ids, and a server that stops for
    good releases its own (`Server::release_layout_state`).
- Upstream sync: `git fetch upstream`, merge the upstream default branch into a branch cut
  from `main`, run the checks below and land it with a PR into `main`. Do not rewrite upstream
  history.

## Working rules

- Stage explicit paths only. Never use `git add -A` or `git add .`, because sandboxed sessions
  can show untracked placeholder dotfiles (`.bashrc`, `.gitconfig`, `.claude/`, ...) at the
  repo root that must not be committed.
- Every GitHub write (issues, PRs, comments) targets `onlinesourdough/openpaperdesign`
  explicitly (`gh ... --repo onlinesourdough/openpaperdesign`), never the upstream repo.
- The tests regenerate the tracked screenshots in `docs/screenshots/`. Restore them
  (`git checkout -- docs/screenshots`) unless a change is meant to update them.
- Commits use Conventional Commits and keep the actual contributors' co-author trailers.

## Toolchain and checks

Rust 1.95.0. A task-local toolchain may be staged in `.toolchain/` (gitignored):

```sh
export RUSTUP_HOME=$PWD/.toolchain/rustup CARGO_HOME=$PWD/.toolchain/cargo PATH=$PWD/.toolchain/cargo/bin:$PATH
```

```sh
cargo build --locked --all-targets
cargo test --locked -- --test-threads=1        # parallel run: see the known SIGSEGV below
PAPER_RS_REQUIRE_GPU=1 cargo test --locked --test headless
cargo check --locked -p openpaperdesign-core -p openpaperdesign-web --target wasm32-unknown-unknown
cargo test --locked -p openpaperdesign-serve -p openpaperdesign-web
node --test crates/openpaperdesign-web/tests/session.test.mjs   # page lifecycle; Node >= 22
node --test plugin/tests/plugin.test.mjs                        # plugin manifests and skills
(cd plugin/tests/design && python3 -m unittest)                 # ported design-method tests
./scripts/build-web.sh                         # browser bundle; needs wasm-bindgen CLI 0.2.129
cargo clippy --locked --all-targets
cargo fmt --all --check
```

Add `--offline` when the cargo cache is pre-fetched and the network is off. Known baseline:
the default parallel `cargo test` can SIGSEGV in `tests/mcp.rs` during Vulkan instance setup
when two screenshot tests initialize a GPU at the same time. Serial runs pass. See
`docs/openpaperdesign/feasibility-2026-10-12.md`. When `PAPER_RS_REQUIRE_GPU` is unset, the
GPU tests skip themselves if there is no adapter. The MCP screenshot tests return early without
that guard, so a pass is not proof that rendering happened.

## Browser editing (issue #1)

`openpaperdesign-serve --project <dir>` serves one project to a browser on loopback
(`docs/openpaperdesign/browser-edit-b1.md`). Its revision checks protect only writers that go
through it. Never describe it as protecting a concurrently running native app or other
external writer.

Agents edit the same project over MCP at `/mcp` on that server, with the per-launch token as
`Authorization: Bearer <token>`. Only the bound project is reachable: export and project
creation are refused, and images must be `data:` URIs
(`docs/openpaperdesign/agent-edit-b2a.md`). Never write the token into repository files or commit it.
Conflicts are strict, and nothing merges. A browser save on a stale revision is refused (409).
An MCP session must read a page before it changes it (no first-call exception). Reads are
per page: only a successful call that returned that page counts. Failed calls, and calls
not attributable to one page, acknowledge nothing. The page list (names, order) is read only
via `get_basic_info`. A change is refused as a `conflict` unless every page it writes (and
the page list, if it changes it) is read at its current revision. The browser polls only
while its editor is clean. Page ids come only from the manifest, must be safe file names,
must differ in more than ASCII case (macOS file systems alias them), and must not share
their first six characters, ignoring case (node ids are `<prefix>:<n>`). A call that returns
an error changes nothing, in memory (the editor state, undo history and selection included)
or on disk. Each save (agent call or browser) is one commit that is rolled back on any error,
directory syncs included. If the rollback fails, the answer names the files that may be
inconsistent, and the server refuses every further write. That state is in memory only and
clears on restart, so check the named files first. Commits are not crash-atomic: a crash
mid-commit can leave a mix of old and new files (each one complete). A call that couldn't start within the timeout is never run; a
started call is always waited for; reads count only once their result was delivered. Inline SVG may not
contain `<image>`/`<feImage>`.
The dropped three-way-merge experiment is at the local tag `experiment/b2a-three-way-merge`;
don't bring it back without an explicit decision (collaboration is issue #2).

## Codex plugin

`plugin/openpaperdesign/` is the local Codex plugin (`docs/openpaperdesign/plugin-b2b.md`). Its
MCP server, `openpaperdesign-mcp`, must be on PATH. It binds only projects inside the
workspace and embeds the web bundle at build time, so run `./scripts/build-web.sh` before
`cargo install`. `plugin/openpaperdesign/plugin.json` is the only manifest. Codex
discovers `skills/` and `mcp.json` itself, so the manifest has no top-level `skills`,
`mcpServers`, `apps` or `interface`. Codex presentation goes in `extensions.com.openai`
(`interface.shortDescription` ≤ 30 characters). Don't add a `.codex-plugin/` overlay. Keep the ported `design` and `review-design` skills' provenance notes and
LICENSE files, and the plugin's `NOTICE.md`, intact.

## Model profile

- Implementation: `claudeAgent` `claude-opus-5-5`, effort `medium`, fastMode `false`.
- Independent review: `codex` `gpt-6-astra`, reasoningEffort `xhigh`, serviceTier `default`.
