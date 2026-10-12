---
name: review-design
description: Review a visual direction and selected companions against their brief without editing, or audit selected accumulated design evidence.
metadata:
  version: "1.3.0"
---

# Review design

Inspect the selected brief, canonical `DESIGN.md`, companions and current proof
without editing the subject. Use the host's review method, if installed, for
lifecycle acceptance. Return PASS, REVISE with concrete corrections, or BLOCKED for a
required decision/capability that is unavailable.

Apply only the checks needed for the selected outcome:

- **Job and specificity:** the audience, value, primary action or supported
  decision are clear. Each section earns its place. Claims, names, examples and
  numbers are sourced or clearly fictional; no placeholder proof or vague promise.
- **Voice and composition:** copy is direct and particular to this task.
  Hierarchy, density, alignment, whitespace, typography, color and imagery
  support its job. Treat generic grids, pills, gradients, sidebars, symmetry and
  filler as judgment signals, not universal aesthetic bans.
  For requested polish or observed density/rhythm problems, use Design's
  [reference-led pass](../design/references/design-solution.md#reference-led-polish):
  compare preserve/borrow/change decisions with the full rendered sequence and
  current before/after proof. Check that reduced repetition preserves necessary
  labels, context and actions; a dense surface need not become sparse.
- **States and accessibility:** relevant states are useful and explicit.
  Inspect semantic structure, headings, labels, contrast, keyboard access,
  visible focus, skip navigation, alt text and reduced motion. Errors need
  adjacent, programmatically associated text; status needs more than color.
  Selected responsive surfaces retain content and actions at narrow widths.
- **Sources and consistency:** respect [source precedence and selection](../design/references/source-selection.md).
  References inform principles without copying a brand or redistributing media.
  Rights, revision, role, visual signals and limitations remain inspectable.
  Brief, canonical direction and selected companions agree; existing tokens,
  components and code remain implementation truth, linked rather than duplicated.
  Optional format
  lint/export is evidence only when actually run; new source adoption needs
  rendered-task proof.
  For reference-led work, compare the inspected source, its accepted adaptation
  in `DESIGN.md` and the rendered result at relevant viewports. Check selected
  hierarchy, proportions, alignment, density and mobile transformation; judge
  justified differences against the brief. Report an actionable discrepancy as
  expected choice, observed result, affected surface and bounded correction.
  Similarity alone is not quality, and upstream review labels are not task proof.
- **Before/after evidence:** when a comparison is selected, use Design's
  [comparison method](../design/references/before-after.md). Check that the
  baseline is genuine, the views are comparable and the claimed improvement
  serves the brief. Report meaningful regressions or evidence limits; an
  attractive after-image alone does not establish a better result.
- **Ownership and change evidence:** implementation/content responsibilities
  stay with their task owners. No automatic sibling chain, invented decision,
  weakened requirement or unsupported correction passes review. Recovered files
  retain their original provenance and acceptance status.

For direction-only work, review the instructions and disclose receiving-surface
validation as a limitation. For a selected preview/native source, inspect its
actual rendering and relevant interaction. For editable delivery, verify the
actual edit, manual-change preservation, save/reopen and selected export, with
format/path/ownership and useful implementation mapping when requested. Check
the selected route's observed capability and explicit fallback limits; a hash,
manifest or screenshot alone proves neither these operations nor accessibility.

Bind the result to the exact brief, DESIGN version/hash and selected companion
hashes, with reviewer identity, observed checks, proof locators and limitations.
Use the [portable evidence format](../design/references/portable-work.md) when a
snapshot is selected. Preserve the brief's named `independent` or `owner` review
choice. Owner mode waits for that owner's exact bound decision; matching
independent PASS needs no extra owner review. Receiver acceptance applies only
to a real delivery boundary. A binder is not a prerequisite for the review that
generates it.

Return findings to the writer for authorized repair, then inspect the changed
bytes and refreshed affected proof. For a requested audit across existing
designs, snapshots or recovery history, use [read-only audit](references/audit.md).

---

*Provenance: adapted for openpaperdesign from AIOS `skills/review-design` 1.2.1
(onlinesourdough/AIOS-plugin `a14f2857d8e229e7d3ce2c9beb15b876094e6d85`, v0.28.1), MIT
License, Copyright (c) 2026 onlinesourdough; see [LICENSE](LICENSE) and
[NOTICE](../../NOTICE.md). Changes: host-neutral wording.*
