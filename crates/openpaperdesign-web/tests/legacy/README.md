# Legacy readers (test-only)

These files are exact copies of earlier OpenPaperDesign editor sources, kept only so the
tests can run them against the current recovery protocol (an older editor's view sharing a
canvas with a newer one). They are never bundled, served or imported by product code.

| File | What it is | Bytes | SHA-256 |
|---|---|---|---|
| `session-192.mjs` | `crates/openpaperdesign-web/static/session.js` as frozen for run 192 (uncommitted candidate tree `21b4ae64ee14e104c1b873379fc3650d7a1e5b90`), staged by the parent as `.toolchain/opd-task-inputs/issue23-r83-20261025/OLDER-192-session.mjs` | 48667 | `44a0eee1b7fb9af2657e108b2e52a266e9240f40d4bb70551db77d6098fa6ab3` |
| `session-193.mjs` | the same file as frozen for run 193 (the producer that mixes a kept recovery into its own change), staged as `.toolchain/opd-task-inputs/issue23-r84-20261026/OLDER-193-session.mjs` | 56063 | `d4276a964ecc85f912dae74b7641bf12e01ac0173cccc86d8c48decde1a34683` |

The older server they ran with is `crates/openpaperdesign-serve/src/library_192.rs`
(`library.rs` of runs 192 and 193, staged as
`.toolchain/opd-task-inputs/issue23-r84-20261026/OLDER-192-library.rs`, 92310 bytes,
`1e1c2595889e3544307ff8980fdf650d8924955126d10452630081cca5fc7ff2`), compiled only into the
serve crate's tests (`library_compat_tests.rs`); it sits beside `library.rs` because its own
`library_payload.rs` module path is relative to it.

`tests/session.test.mjs` checks these hashes before using them. Don't edit these files: a
changed copy is no longer the code it stands for.
