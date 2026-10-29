# Product page

Static HTML/CSS/modules, locally served Geist, no framework, analytics or build step. Matches the Arc’IT / gustavonline / onlinesourdough family with its own blue canvas mark.

```sh
python3 -m http.server 4388 --bind 127.0.0.1 --directory site
node --test scripts/tests/downloads.test.mjs
python3 scripts/sync-site-assets.py --check
```

Serve `site/` at a domain root. The approved public address is **https://openpaperdesign.arcitai.com/**, hosted as the `openpaperdesign` Cloudflare Pages project in the same account as Arc’IT. The production page has canonical/social metadata and allows indexing. Standalone app publication remains separate (#18). This folder never serves the owner's editor or project library.

## Downloads

`releases.json` contains only reviewed actual release assets, with explicit platform/architecture and qualification. No executable means a source-build link, not a fake download. The primary action detects the OS and leads to explicitly labelled architecture choices. Mobile/iPad visitors see the desktop availability message. An optional ignored `releases.preview.json` is fetched **only on localhost** for actual files copied into ignored `site/downloads/`. It must never be deployed. The local preview can therefore demonstrate a built package without advertising it as a public release.

The counter reads measured GitHub release-asset totals; a successfully fetched source-only release list reports zero app downloads. Checksums and source archives are not app downloads. An unavailable API does not become a fabricated zero. A network failure keeps an explicit GitHub release fallback. Publication is separate from preparing artifacts.

## Product images

New UI/UX development-preview captures from `design/product/fixtures/` (`.excalidraw` and `.svg`). `scripts/make-product-fixtures.py` generates the editable public examples. Import them with `PAPER_RS_HOME` pointed at a disposable library, then render with the build's `--screenshot` command. See `assets-sources.json` for source revision, hashes and exact limitations. Use the new UI/UX source snapshot requested by the owner, with its revision and unreleased status explicit. Never present it as a publicly available desktop release.

The shared SVG identity lives in `assets/brand/`. Site and plugin assets match the canonical source. Font and upstream licences are retained. The original product-page screenshot remains in Git history.

## Review and publication

Verify compact/intermediate/wide views, both themes, mobile overlay/focus/scroll-lock, Offers, preview tabs and real file downloads. Validate release and source destinations, current screenshot provenance, canonical/social metadata, HTTPS and indexing when publishing. Do not deploy the ignored local package directory or preview manifest. No hosting credentials or installed plugins are changed by this page.

Export deployments with `python3 scripts/export-product-site.py --output /new/output/path`. This exports committed files from Git, refuses preview entries, and never uploads ignored local binaries. Do not upload the working `site/` directory directly.

The download counter contacts `api.github.com` in the visitor’s browser; it sends no canvas data. Brand marks and owner artwork are credited separately and are not relicensed by the product’s MIT/Apache code licences.

The review page links to the immutable integrated source revision `3286238`, which contains the actual new Home/workspace and native packaging, so guides work before merge and after branch deletion. When publishing a qualified release, update these guide URLs to that release tag.

## Production deployment and recovery

The owner approved publication on 29 October 2026. Use the existing authenticated Cloudflare account; no editor/library service is exposed. From a clean reviewed commit:

```sh
python3 scripts/export-product-site.py --output /new/absolute/export
wrangler pages deploy /new/absolute/export --project-name openpaperdesign --branch main --commit-hash REVIEWED_COMMIT
```

The export uses tracked files only. Confirm that neither `downloads/` nor `releases.preview.json` is present. Read back the custom domain, HTTPS, HTML/asset hashes, theme/navigation/gallery and source-build fallback. Keep the deployment ID and source commit in the issue #20 delivery comment. Subsequent releases can roll back to a previous successful production deployment in Cloudflare Pages; for the first publication, recovery is detaching only the new product domain if necessary, leaving Arc’IT’s existing site untouched.

Until reviewed public app assets exist, the published download area links to the source-build guide and labels the images as development previews. Localhost previews may still demonstrate the separately qualified local archives.
