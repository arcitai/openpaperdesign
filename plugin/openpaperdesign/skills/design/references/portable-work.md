# Portable work and optional snapshots

Resolve the installed skill's location through the harness. Run its scripts by
absolute path from any cwd. Node 20+ is required; preview and basic handoff use
only Node built-ins. The helpers never install dependencies. Select an actual
absolute work directory outside the installed package. Work data, temporary
editor state and exports belong outside the package; task-selected symlinks and
escaping paths are rejected. No collection slug or registry is required.

Serve only the selected preview folder on loopback:

```sh
node <design-skill>/scripts/serve.mjs --design-dir /absolute/project/design --port 0
```

Use the printed URL in an available browser. Stop the process after inspection
unless the requested review needs it left running. A running server proves
neither visual quality nor accessibility. Keep only intended preview data in
that served folder.

## Exact review evidence

For ordinary work, retain the accepted brief and review in the current task's
evidence. The optional snapshot helper consumes these small files under the
selected directory. It does not create a second lifecycle record.

`BRIEF.md` includes the accepted job and constraints plus these fields:

```md
- **Receiving outcome:** <bounded result for the receiving owner>
- **Source/reference rights, provenance, and licensing:** <selected sources, asset licenses and reuse limits>
- **Review mode:** independent
- **Review owner:** <exact selected reviewer identity>
- **Receiver acceptance:** <separate receiving decision for this snapshot>
```

`DESIGN.md` declares non-empty `name` and `version` frontmatter and includes
`- **Known limitations:** <actual limitations or an explicit none>`. Preserve
existing design identity/version conventions; these are artifact metadata,
independent of the skill's version.

The selected reviewer supplies `REVIEW.md` on the exact final subject:

```text
Reviewer: <identity matching Review owner>
Result: PASS
Reviewed BRIEF.md SHA-256: `<64 lowercase hexadecimal characters>`
Reviewed DESIGN.md SHA-256: `<64 lowercase hexadecimal characters>`
Reviewed source companion: `<selected-relative-path>` — SHA-256 `<hash>`
Checks: <actual scope, observations, proof locators and limitations>
```

Repeat the companion line for each selected pre-existing preview, asset,
editable source and native export. `proof.json` is an alternative using `review`,
`reviewer`, `reviewed_brief_sha256`, `reviewed_design_sha256` and a
`reviewed_source_companions` path-to-hash object. These files record evidence;
the helper cannot authenticate a person's identity or supply missing review.
Do not fabricate a named reviewer decision.

In `independent` mode the matching PASS is sufficient. In `owner` mode the
helper reports `waiting-owner` (exit 2) until that exact owner's complete bound
PASS exists. The receiving owner is a separate role, never a substitute reviewer.
Changes to the brief, direction or selected companions invalidate affected review.

## Snapshot only for a real delivery boundary

Same-workspace work continues with its selected files and needs no binder.
For an authorized cross-owner delivery or explicitly requested frozen copy:

```sh
node <design-skill>/scripts/create-handoff.mjs \
  --design-dir /absolute/project/design \
  --output /absolute/delivery/new-revision \
  --receiving-owner "Named receiving owner"
```

The output's parent must exist and the output must be absent. Every existing
output is preserved, including pending, failed and accepted snapshots. The
minimal snapshot contains `BRIEF.md`, `DESIGN.md`, the review evidence and
`HANDOFF.md`. It needs no assets, preview or editor. The binder retains the
`ADS-HANDOFF/1` identifier for compatibility with existing snapshots; that label
does not require ADS. Its stable identity binds name, receiver and outcome.
New revisions bind the complete included manifest, so companion-only changes
also receive a different revision. Existing snapshots keep their original IDs.

Select `--preview` for `index.html` and repeat `--asset assets/<file>` for
individual reviewed assets and license notices. The helper never copies a
whole media directory. Review every dependency needed to make a preview usable.
For selected CSS, token or Tailwind derivatives, use `--export css`, `tokens`
or `tailwind` with `--designmd-cli /absolute/external/install/dist/index.js`.
The compatible source tool is `@google/design.md@0.3.0`, separately installed
under its own license. Run its lint/export only when needed and available;
record tool version and actual result. The minimal route requires neither it
nor its package files. Derivatives bind the reviewed DESIGN hash and their own
hash, rather than pretending they were reviewed before generation.

For the optional editor binding, see the
[openpaperdesign editor](../../openpaperdesign/SKILL.md). Missing optional tooling is an
explicit companion limitation; it cannot silently satisfy a required native
deliverable. Keep independently usable direction work moving.

Inspect the generated manifest, provenance/licenses, review, limitations and
receiver/outcome. Acceptance starts `PENDING`; generation and copying are not
acceptance. The receiving owner records its decision, identity, date and exact
accepted revision/use in the binder or a bound acceptance record. Preserve
accepted bytes; later changes need a new revision and its applicable acceptance.
The receiver owns its implementation/production copy, with no live sync back to
the design workspace. Revalidate behavior, accessibility and rights on that
receiving surface. Retain failed output for recovery; retry at a new path.
