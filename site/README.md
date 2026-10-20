# OpenPaperDesign product page

A standalone static marketing page for OpenPaperDesign. This directory does
not build, start or expose the canvas editor, MCP server or Codex plugin.
It has no package install or build step, forms, analytics or backend.

## Preview

From the repository root, using Python 3.9 or newer:

```sh
python3 -m http.server 4184 --bind 127.0.0.1 --directory site
```

Open http://127.0.0.1:4184/. Only `site/` is served. In this local preview,
the Arc’IT footer link points to the companion preview on port 4183; on other
hosts it keeps its normal https://arcitai.com/ destination. Root-absolute URLs
require serving this folder at a domain root, not under a path prefix or via
`file://`.

## Files and assets

- `index.html`, `styles.css`, `site.js`: page, responsive themes and theme toggle.
- `DESIGN.md`: typography, visual direction and content boundaries.
- `assets-sources.json`: exact source revisions and SHA256 values for the two
  repository-owned logos and the existing release screenshot.
- `assets/fonts/`: local fonts, source notes and SIL OFL licences.
- `assets/licenses/`: unchanged project licences and third-party notices.

Refresh the pinned repository assets from the root with:

```sh
python3 scripts/sync-site-assets.py
```

The updater verifies each asset hash and keeps output inside `site/assets/`.
Choose a new revision and hash deliberately when replacing an asset. The logos
are pinned to the branding work in PR #9; the screenshot still shows v0.1.0.
The source download and setup link also target that existing release. No
standalone app installer is implied.

## Publication is a separate follow-up

The intended address is `openpaperdesign.arcitai.com`. This change adds the
source only: no hosting configuration, deployment workflow, DNS or release.
The page intentionally keeps `noindex, nofollow` and has no production canonical.
Before public launch, complete the appropriate application/release work,
review the real screenshot and download destinations, and verify the page in
both themes at desktop and mobile sizes. Publish only this static directory,
with production metadata and a rollback route.

- [Standalone app and downloadable release — #18](https://github.com/arcitai/openpaperdesign/issues/18)
- [Current-app screenshot — #19](https://github.com/arcitai/openpaperdesign/issues/19)
- [Domain and deployment — #20](https://github.com/arcitai/openpaperdesign/issues/20)

All three follow-ups remain open; adding this page does not complete them.
