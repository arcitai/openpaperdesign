# OpenPaperDesign — product design direction

Revision: 4 · 2026-10-18 · The one canonical visual and interaction direction for the OPD app
(Home and the workspace). The engine and architecture are described in
[analysis/DESIGN.md](analysis/DESIGN.md) (paper-rs's historical design: stack, crate layout,
rendering, document model), which stays valid and is not repeated here. This file isn't a
`design/<name>/` project or a design system for user canvases. A user's own repository
direction belongs to that repository.

## Priority: minimal, airy, low-noise (owner, 2026-10-18)

The owner's explicit priority governs every choice below, and wins where a reference is denser:
- The canvas is the visual focus. Spacing is generous and consistent, surfaces are calm, and
  there is a small, clear set of primary tools.
- Related controls are grouped. Secondary and contextual options appear when they are relevant
  (a selection, an open panel) and stay reachable through the menu, the command palette, labels
  or tooltips, and their shortcuts. Targets stay comfortable.
- Borders, shadows, accents, badges and permanent chrome are restrained. An empty canvas and
  ordinary editing feel quiet. No card per control, and not every feature at once.
- Real save, conflict, error and unsaved feedback is visible whenever it applies. Quietness
  never hides an unresolved problem or weakens safe navigation.
- The same restraint applies to Home, the editor, the tools, the inspector, type and spacing,
  in light, dark and narrow layouts. Airiness never shrinks the drawing room below usable.

## Direction: a calm workbench around the canvas

The canvas is the product. The chrome is one quiet surface around it, the same in Home and the
editor, light and dark. References studied (visual and interaction only; no code, assets or
themes copied):
- Paper: inspector rhythm and paired fields.
- tldraw: canvas-first arrangement and the floating tool dock.
- Excalidraw: readable grouped controls and visible selected state.
- OpenPencil, the public demo as the coordinator observed it on 2026-10-18 (a supporting
  structural reference): neutral side surfaces, Pages/Layers on the left, restrained
  separators, a compact floating tool group with a single blue selected tool, and a right
  inspector with a few labelled sections when nothing is selected. Its demo artwork, coloured
  cards and feature breadth are not OPD chrome.

The principles:
1. **Drawing room first.** Panels give way before the canvas does (see Layout). Floating tools
   never cover panels, and they wrap rather than run under them.
2. **One top bar, one way home.** From left to right:
   - All projects (the grid icon, the only way back to Home);
   - one Menu holding every upstream menu (on platforms without a native menu bar);
   - the file's name over its page and save state, truncated, never overlapped;
   - on the right: undo/redo, zoom, help.

   Nothing is duplicated outside it, so there is no outer title, stage or action bar. Its icons
   are quiet: secondary colour, the text colour on hover, and an accent tint only for an open
   toggle (help, or the narrow layout's Layers and Design). The zoom level is secondary text.
3. **One strong accent.** The canvas's selected tool is the only solid accent control. Selected
   rows, open toggles and Home's pressed layout buttons use the accent tint, and their labels
   keep their colour.
4. **Contextual properties stay in the inspector** (Design), pages and layers in the left
   panel. Arrange (bring forward and send backward) shows in the layers header only while
   something is selected. Every command keeps its menu entry and keyboard binding wherever a
   button moves or hides.
5. **Quiet surfaces, clear state.** Panels and bar share one colour, separated by hairlines.
   Fields are softly filled.
6. **Problems stay visible.** Errors, conflicts and unfinished saves are never hidden for
   neatness (see the status rules in AGENTS.md).
7. **Light and dark are one design** (see Appearance below): Home, the chrome, every toolbar,
   menus, the inspector and the canvas itself share one resolved appearance.

## Appearance (System, Light, Dark)

- **The choice.** Menu ▸ Appearance in the editor: System (default), Light or Dark. It is
  compact and lives in the existing menu, so there is no extra toolbar.
- **System** follows the OS appearance, live: the browser's colour-scheme preference (in the
  native app, egui-winit's system theme). The host's theme stands in only where the browser
  can't report one. With no signal at all, it is light.
- **Light and Dark** override the OS.
- **Where it is kept.** It is a view preference, never part of a canvas:
  - the app library keeps it in its own data folder (`library/v1/appearance`), so it survives
    closing and reopening the view;
  - the standalone browser page keeps it only while it is open (its origin changes per launch);
  - nothing is stored in the browser;
  - native macOS has no egui Menu, so it stays System and follows the OS.
- **Home and the editor** resolve it with the same rule, including narrow layouts.
- **The resolved appearance owns every colour.** A host's colour variables (MCP Apps style
  variables) are its palette for the theme it sent them with. They join Home's colours only
  while that theme is the resolved appearance. Under an explicit Light or Dark, or a System OS
  that differs from the host, they are removed and Home's own palette applies. A new host
  palette replaces the earlier colours entirely. The host's sanitized font and radius
  variables apply in every appearance.

### The canvas in Dark (display, not data)

- **The work surface** is dark (#18181b) with a quiet grid (#34343a). The chrome surface is
  #1f1f22, so the canvas sits a step deeper than the panels, as in light.
- **Each drawn colour** (solid fills, gradient stops, strokes, text) is adapted for display.
  Its OKLab lightness is mirrored into 0.93 (authored black) … 0.27 (authored white). Its hue,
  its chroma (reduced only as far as sRGB requires) and its alpha are kept. So black text and
  strokes read light, white shapes sit just above the surface, and red stays red. This is in
  the spirit of tldraw and Excalidraw dark modes; nothing was copied.
- **Images** are never adapted, and the composited result is never inverted.
- **Unchanged by the canvas display:** the selection accent, handles and guides are the same in
  both. Frame-name labels use a lighter grey on the dark surface.
- **Nothing in the document changes.** A theme switch writes nothing and doesn't mark the
  canvas dirty, save it, touch undo or change a revision. The document's colours are its own:
  the inspector and swatches show the authored values.
- **Exports** (SVG, PDF, PNG, thumbnails) always draw the document as authored, whatever the
  appearance.

## Layout tiers (CSS px of the granted surface)

| Width | Arrangement |
|---|---|
| ≥ 1024 | Ordinary: layers/pages left (240), canvas, inspector right (260), resizable; top bar across canvas and inspector. |
| 600–1023 | Docked: top bar across the surface; the canvas plus at most ONE panel at its usual width, chosen with the Layers / Design toggles (Layers left, Design right). Default: canvas only. |
| < 600 | Narrow: top bar across; the chosen panel fills the room under it, or the canvas with the tools wrapped into rows. |

Rules that apply to every tier:
- A change of arrangement waits while an egui text field has the keyboard, then lags one
  frame. A hidden field finishes as a click elsewhere finishes it.
- Docked ↔ filled is the same panel, so its fields keep focus.
- Native windows use the ordinary tier only, unless a host opts in (`UiState::narrow`).
- Undo/redo and zoom leave the narrow top bar first (from about 420 and 560 px of bar). They
  stay reachable through Menu and their shortcuts.

## Tokens

These are the workspace values, from `src/theme.rs`. Home uses the same values as fallbacks
under the host's MCP Apps style variables.

| Token | Light | Dark |
|---|---|---|
| surface (bar, panels) | #fbfbfc | #1f1f22 |
| field / hover surface | #f1f1f3 | #2a2a2e |
| control | #ffffff | #2c2c30 |
| text | #1d1d1f | #ececef |
| secondary text | #6e6e76 | #a1a1a8 |
| tertiary / disabled | #b4b4ba | #5f5f66 |
| hairline | #e6e6ea | #333338 |
| accent | system accent, else #007aff | system accent, else #007aff (Home: #4c9bff) |
| selected | accent tint over surface (≈13% light, 27% dark) | |
| canvas work surface | #eeeef0 | #18181b |
| canvas grid dot | #cfcfd6 | #34343a |

- **Type:** Inter. 13 body, 11 secondary and section labels (semibold), 13 semibold titles.
- **Shape:**
  - control radius 7;
  - rows and icon buttons 6;
  - toggles 7;
  - menus 10, windows and popovers 12;
  - Home cards 12.
- **Spacing:**
  - top bar 38 (40 with native chrome);
  - 24 interactive height;
  - 26 icon buttons in bars, 30 in the tool dock;
  - 16-unit icon grid, about 1.25 stroke.
- **Shadow:** only floating things (tool dock, menus, popovers). Panels are flat.
- **Borders:** hairlines between regions only. Controls in bars and rows are borderless until
  hovered.

## Not in scope

- Multiplayer, sharing or cloud surfaces.
- Copied competitor themes, icons, fonts or code, including OpenPencil's.
- A second toolbar, an imitation DOM editor, another engine or a second theme system.
- New document formats.
- Decorative mockups counted as implemented work.
