# Third-party code and assets

paper-rs is MIT OR Apache-2.0. It ports a few algorithms from MIT-licensed projects (listed
below with their copyright notices and license text) and bundles the Inter font (SIL OFL 1.1,
`assets/fonts/OFL.txt`) and, in OpenPaperDesign, the Geist Mono font (SIL OFL 1.1,
`assets/fonts/OFL-GeistMono.txt`). Figma and Paper are proprietary: only their *behavior* was studied
(`analysis/FIGMA_NOTES.md`, `analysis/PAPER_ARCHITECTURE.md`), and no code, shaders, icons or
assets from them are used.

## Ported code

| Project | Copyright | What was ported | Where |
|---|---|---|---|
| [rough.js](https://github.com/rough-stuff/rough) 4.6.4 | Copyright (c) 2019 Preet Shihn | Seeded RNG, `_line` / `_doubleLine`, curves, ellipses, `svgPath`, solid / hachure / cross-hatch / zigzag / dots fills, generator option handling. Verified against rough.js output (`tests/fixtures/rough/`). | `src/render/rough.rs` |
| [hachure-fill](https://github.com/pshihn/hachure-fill) 0.5.2 | Copyright (c) 2023 Preet Shihn | Scan-line hachure algorithm | `src/render/rough.rs` |
| [points-on-curve](https://github.com/pshihn/bezier-points) 0.2.0, [points-on-path](https://github.com/pshihn/points-on-path) 0.2.1 | Copyright (c) 2020 Preet Shihn | Bezier flattening by flatness, Ramer–Douglas–Peucker, `curveToBezier`, path point sets | `src/render/rough.rs` |
| [Excalidraw](https://github.com/excalidraw/excalidraw) | Copyright (c) 2020 Excalidraw | Element → rough options (`adjustRoughness`, dash patterns, fill weights), binding gap and fixed-point rules, elbow-arrow grid/A* router with its segment-count heuristic, elbow corner rounding, arrowhead geometry, laser trail decay constants, sticky-note sizing, `.excalidraw` / `.excalidrawlib` formats | `src/render/rough.rs`, `src/connector.rs`, `src/editor/whiteboard.rs`, `src/io/excalidraw.rs` |

Each ported module starts with a comment naming its source.

## Dependencies

Rust crates are pulled from crates.io under their own licenses (see `Cargo.lock`); notable ones
are wgpu, winit, egui, lyon, ttf-parser, image, i_overlay (MIT), serde and rfd.

## MIT License (applies to each project listed above, with its copyright line)

```
MIT License

Copyright (c) 2019 Preet Shihn            (rough.js)
Copyright (c) 2020 Preet Shihn            (points-on-curve, points-on-path, path-data-parser)
Copyright (c) 2023 Preet Shihn            (hachure-fill)
Copyright (c) 2020 Excalidraw             (Excalidraw)
Copyright (c) 2023 Excalidraw             (@excalidraw/laser-pointer)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Fonts

Inter 4.1 by Rasmus Andersson — SIL Open Font License 1.1 (`assets/fonts/OFL.txt`). Regular,
Medium, SemiBold, Bold, Italic and Bold Italic are bundled unmodified.

Geist Pixel Square by The Geist Project Authors (https://github.com/vercel/geist-font) — SIL
Open Font License 1.1 (`crates/openpaperdesign-serve/assets/geist-OFL.txt`). Bundled
unmodified (`crates/openpaperdesign-serve/assets/geist-pixel-square.woff2`) and served only
inside the plugin's home view, for its `onlinesourdough` wordmark.

Geist Mono by The Geist Project Authors (https://github.com/vercel/geist-font) — SIL Open Font
License 1.1 (`assets/fonts/OFL-GeistMono.txt`, that repository's `OFL.txt`). Four static faces
are bundled unmodified and embedded in the canvas renderer (`src/text.rs`), where a text layer
can choose them as its font family. Their name records say "Geist Mono" with the subfamilies
below. Taken from the repository's immutable commit
`d1d9ce6e929a35f0e24b7cd10573f17db58196a0`, `fonts/GeistMono/ttf/` (and `OFL.txt` from its
root), 613,604 font bytes in all:

| Bundled file | Subfamily | Bytes | SHA-256 |
|---|---|---|---|
| `assets/fonts/GeistMono-Regular.ttf` | Regular | 149,284 | `5a0de4b3d54ab272f76a1d8c84b7fb24c67bbec6591d5300e61c7bc10094b6c8` |
| `assets/fonts/GeistMono-Bold.ttf` | Bold | 151,260 | `325de0913317e63c9b0084d3f571c6d1c9279e776e9260c62be19e1c12d1be3c` |
| `assets/fonts/GeistMono-Italic.ttf` | Italic | 155,276 | `c249093c0f30720afc801c9e8ca1e8c718809cf56860875a590ad69bc803dfbb` |
| `assets/fonts/GeistMono-BoldItalic.ttf` | Bold Italic | 157,784 | `b7c96991c157b37a59d0d44304dadd1beb75f1b053acaf6fd42567e1415ed2e3` |
| `assets/fonts/OFL-GeistMono.txt` | (license) | 4,383 | `c683bfbcc7e087f5d37a54ef628f10387c451a83ddc459b151403a164ac46c90` |

The faces have no U+2713 CHECK MARK (and other characters outside their coverage); in a Geist
Mono layer such a character is drawn from the matching bundled Inter face, and one neither
font has is drawn as Geist Mono's `?`. No other font is loaded.
