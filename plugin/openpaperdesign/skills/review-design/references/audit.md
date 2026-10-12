# Read-only accumulated design audit

Select the actual folders and evidence relevant to the requested audit. Do not
discover a personal collection or take the first alphabetical design as a proxy.
Inspect every selected scope, including malformed or incomplete entries. Missing
required evidence is BLOCKED; contradictory/stale evidence is FAIL. PASS means
the stated audit scope agrees, not that a new visual review occurred.

The Python 3.9+ standard-library helper checks explicitly selected design folders
and frozen snapshot folders, from any cwd:

```sh
python3 <review-design-skill>/scripts/audit.py \
  --design-dir /absolute/project/design \
  --snapshot /absolute/delivery/revision
```

Repeat either option for a larger explicit scope. It checks review identity,
current brief/direction and companion hashes, required snapshot metadata,
included file integrity and acceptance completeness. It outputs JSON with
`PASS`, `FAIL` or `BLOCKED`, evidence, gaps and the smallest next action; exit
codes are 0, 1 and 2 respectively. It never repairs, writes a run, exports,
promotes, creates an issue, copies a file or invokes another workflow.

Complete the judgment the helper cannot supply: compare canonical `DESIGN.md`
against selected previews/native exports, source/reuse decisions and brief;
check actual proof locators and ownership. In existing history, follow failed
attempts to the related recovery, inspect predecessor references and required
input/output/proof, and distinguish recovered/curated historical evidence from
new acceptance. Preserve older formats and missing evidence as such; do not
manufacture failures or a new ledger to satisfy an audit. Generic repository
health remains with shared Review, not this domain script.

Old snapshot identities are never regenerated. A historical snapshot with no
bound brief hash remains unchanged and is reported with that proof gap; it is
not silently upgraded to current review. Pending receiver acceptance is visible
but does not itself fail snapshot integrity. A technical PASS cannot establish
reviewer authenticity, source rights, visual usability or a receiving decision.

Return status, exact scope, observations, gaps and the smallest corrective
action. Corrections route to the existing writer and Review under their own
authority; the audit remains read-only.
