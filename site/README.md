# Product page

Static HTML/CSS/modules, locally served Geist, no framework, analytics or build step. Matches the Arc’IT / gustavonline / onlinesourdough family with its own blue canvas mark.

```sh
python3 -m http.server 4388 --bind 127.0.0.1 --directory site
node --test scripts/tests/downloads.test.mjs
python3 scripts/sync-site-assets.py --check
```

Serve `site/` at a domain root. The intended public address remains **openpaperdesign.arcitai.com**, tracked by issue #20. Production currently remains noindex until a real release/domain is reviewed. This folder never serves the owner's editor or project library.

## Downloads

`releases.json` contains only reviewed actual release assets, with explicit platform/architecture and qualification. No executable means a source-build link, not a fake download. Mobile/iPad detection never hands out an inappropriate desktop installer. An optional ignored `releases.preview.json` is fetched **only on localhost** for actual files copied into ignored `site/downloads/`. It must never be deployed. The local preview can therefore demonstrate a built package without advertising it as a public release.

A null download count is hidden. Only measured GitHub executable-asset download totals belong in the manifest; checksums and source archives are not app downloads. A network failure keeps an explicit GitHub release fallback. Publication is separate from preparing artifacts.

## Product images

Actual native editor captures from `design/product/fixtures/*.excalidraw`. `scripts/make-product-fixtures.py` generates the editable public examples. Import them with `PAPER_RS_HOME` pointed at a disposable library, then render with the build's `--screenshot` command. See `assets-sources.json` for source revision, hashes and exact limitations. Do not replace these with generated mock UI or present Factory's unreleased plugin UI as desktop.

The shared SVG identity lives in `assets/brand/`. Site and plugin assets match the canonical source. Font and upstream licences are retained. The original product-page screenshot remains in Git history.

## Review and publication

Verify compact/intermediate/wide views, both themes, mobile overlay/focus/scroll-lock, Offers, preview tabs and real file downloads. Validate release and source destinations, current screenshot provenance, canonical/social metadata, HTTPS and indexing when publishing. Do not deploy the ignored local package directory or preview manifest. No hosting credentials or installed plugins are changed by this page.
