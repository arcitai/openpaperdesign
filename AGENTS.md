# AGENTS.md — openpaperdesign

Revision: 0.1 · Updated: 2026-10-12

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
cargo check --locked -p openpaperdesign-core --target wasm32-unknown-unknown
cargo clippy --locked --all-targets
cargo fmt --all --check
```

Add `--offline` when the cargo cache is pre-fetched and the network is off. Known baseline:
the default parallel `cargo test` can SIGSEGV in `tests/mcp.rs` during Vulkan instance setup
when two screenshot tests initialize a GPU at the same time. Serial runs pass. See
`docs/openpaperdesign/feasibility-2026-10-12.md`. When `PAPER_RS_REQUIRE_GPU` is unset, the
GPU tests skip themselves if there is no adapter. The MCP screenshot tests return early without
that guard, so a pass is not proof that rendering happened.

## Model profile

- Implementation: `claudeAgent` `claude-opus-5-5`, effort `medium`, fastMode `false`.
- Independent review: `codex` `gpt-6-astra`, reasoningEffort `xhigh`, serviceTier `default`.
