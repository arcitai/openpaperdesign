# Product page

Static HTML/CSS/modules, locally served Geist, no framework, analytics or build step. Matches the Arc’IT / gustavonline / onlinesourdough family with its own blue canvas mark.

```sh
python3 -m http.server 4388 --bind 127.0.0.1 --directory site
node --test scripts/tests/site.test.mjs
python3 scripts/refresh-site-versions.py --check
python3 scripts/sync-site-assets.py --check
```

Serve `site/` at a domain root. The approved public address is **https://openpaperdesign.arcitai.com/**, hosted as the `openpaperdesign` Cloudflare Pages project in the same account as Arc’IT. The production page has canonical/social metadata and allows indexing. This folder never serves the owner's editor or project library.

## Product direction

The page describes the lightweight Codex / Claude Code plugin at https://github.com/gustavonline/openpaperdesign ([successor issue #4](https://github.com/gustavonline/openpaperdesign/issues/4)). The standalone desktop app is retired (see `docs/openpaperdesign/lightweight-transition.md`). Claims come from the successor README and skills, not from this repository's Rust app:

- install commands are copied from the successor README, with **Node.js 22 or later** stated next to them;
- Papers are local folders outside the plugin cache; Notion is a manual snapshot and sharing is optional (local network, or a temporary Internet link that needs the optional cloudflared tool);
- exports are PNG and HTML for selected artwork, JSON layers and the complete `.paper.zip`. There is no SVG, PDF, Excalidraw or draw.io import/export claim;
- documents saved by the earlier standalone app are preserved, use a different format and are **not converted automatically**.

There is no public plugin archive. The page links to the successor README install guide and its releases page, and says a packaged archive is not offered yet. Update that wording only when verified public artifacts exist (successor issue #3).

## Installation, not downloads

The page has no download buttons, download counter or call to the GitHub release API; `site.js` makes no network request. `releases.json` is no longer read and only records that standalone downloads are retired. `downloads.js` and `scripts/tests/downloads.test.mjs` remain as historical code and tests for the retired desktop track. The page does not import `downloads.js`.

## Product images

The gallery and feature rows use only three images, supplied from the lightweight plugin running in a browser:

- `/assets/product-plugin-design.png`
- `/assets/product-plugin-diagram.png`
- `/assets/product-plugin-home.png`

They are labelled actual browser/plugin development previews. They are neither native legacy-app captures nor release-qualified. Image bytes and their provenance entries in `assets-sources.json` are added together by the project owner; never invent a hash or capture record. The older `product-*.png` captures of the desktop app stay in the repository for history but are no longer referenced by the page.

The shared SVG identity lives in `assets/brand/`. Site and plugin assets match the canonical source. Font and upstream licences are retained.

## Review and publication

Verify compact/intermediate/wide views, both themes, mobile overlay/focus/scroll-lock, Offers, the three preview tabs, the copyable install commands and every destination. Check that the three images exist and match `assets-sources.json` before publishing. Stable publication waits for the owner's acceptance of the lightweight candidate and for the public artifacts from successor issue #3. Preparing source does not deploy. No hosting credentials or installed plugins are changed by this page.

Export deployments with `python3 scripts/export-product-site.py --output /new/output/path`. This exports committed files from Git and refuses preview entries. Do not upload the working `site/` directory directly.

Brand marks and owner artwork are credited separately and are not relicensed by the product’s MIT/Apache code licences.

## Production deployment and recovery

The owner approved publication of the product page on 29 October 2026 (issues #19 and #20). Use the existing authenticated Cloudflare account; no editor/library service is exposed. From a clean reviewed commit:

```sh
python3 scripts/export-product-site.py --output /new/absolute/export
wrangler pages deploy /new/absolute/export --project-name openpaperdesign --branch main --commit-hash REVIEWED_COMMIT
```

The export uses tracked files only. Read back the custom domain, HTTPS, HTML/asset hashes, theme/navigation/gallery and the install and release links. Keep the deployment ID and source commit in the successor #4 delivery comment. Roll back to a previous successful production deployment in Cloudflare Pages if needed; no new hosting is created.
