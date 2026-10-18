# OpenPaperDesign — product design direction

Revision: 6 · 2026-10-18 · The one canonical visual and interaction direction for the OPD app
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
1. **Drawing room first.** Panels give way before the canvas does (see Layout). The tool rail
   and zoom float over the canvas's edges, never over panels.
2. **One header, one way home** (revision 2, built). The header's safe area is laid out before
   the panels, across the whole surface, at every width:
   - left: All projects (the grid icon, the only way back to Home), one compact menu (`···`,
     holding every upstream menu and Appearance; native macOS keeps its menu bar instead) and
     the Layers toggle (pages and layers);
   - centre: the file's real name over its page and save state, centred on the surface and
     truncated, never overlapped;
   - right: the Design toggle.

   Undo/redo, help and every other command stay in the menu, the command palette and their
   shortcuts; zoom is at the canvas's lower left. Nothing is duplicated outside the header, so
   there is no outer title, stage or action bar. Its controls are quiet and only an open
   toggle is tinted.
3. **One strong accent.** The canvas's selected tool family is the only solid accent control.
   Selected rows, open toggles and Home's pressed layout buttons use the accent tint, and their
   labels keep their colour.
4. **Contextual properties stay in the inspector** (Design), pages and layers in the left
   panel. Arrange (bring forward and send backward) shows in the layers header only while
   something is selected. Every command keeps its menu entry and keyboard binding wherever a
   button moves or hides.
5. **One tool rail** (revision 2, built). A slim vertical rail at the canvas's left edge holds
   all sixteen tools in six families:

   | Family | Tools (first = initial member) |
   |---|---|
   | Pointer | Select, Hand, Laser pointer |
   | Frame | Frame |
   | Shapes | Rectangle, Ellipse, Diamond, Polygon, Star |
   | Lines | Arrow (binds to shapes), Line (free) |
   | Text | Text, Sticky note (a note, not a neutral box) |
   | Draw | Pencil, Pen (vector path), Eraser |

   - A family's main button runs its current member. Its own chevron (a separate 24 × 32 px
     target, always visible) opens a flyout of whole-row choices: icon, name and the real
     shortcut.
   - Choosing a tool, by row or shortcut, makes it the family's member.
   - Escape closes an open flyout first; Escape outside it returns to Select, and every family
     keeps its member.
   - A press outside closes the flyout and never reaches the canvas.
   - The rail's controls and rows take keyboard focus with Tab; Enter or Space chooses the
     focused one and never reaches the canvas.
   - Tool keys typed into a field stay the field's.
   - There is no bottom tool palette.
6. **A quiet, contextual inspector** (revision 2, built):
   - **Nothing selected, Select/Hand/Laser:** the page and a one-line hint.
   - **A creation tool:** only that tool's defaults, titled "For new …"; they never edit the
     selection.
   - **A selection:** the sections every selected layer has. A value they share is shown and
     editable for all, as one coalesced undo step. A value that differs shows Mixed and is
     never the first layer's value. A widget that can't show Mixed is left out, with a short
     reason.
   - **Folded by default:** Sketch (the optional hand-drawn style, with a plain explanation;
     clean geometry unless turned on, and existing sketchy drawings keep it) and Export.
7. **Quiet surfaces, clear state.** Panels and header share one colour, separated by hairlines.
   Fields are softly filled.
8. **Problems stay visible.** Errors, conflicts and unfinished saves are never hidden for
   neatness (see the status rules in AGENTS.md). A transient status sits right of the zoom,
   never over it or the rail.
9. **Light and dark are one design** (see Appearance below): Home, the chrome, the rail and
   zoom, menus, the inspector and the canvas itself share one resolved appearance.

Still open from the revision 2 plan (not built yet, and not to be described as built):
- None / Dots / Lines backgrounds and the blank-canvas menu;
- the Home grid default, equal cards, footer and storage cue, and view preferences;
- file rename through the fenced canvas save, and page rename from the centred identity;
- the diagram workflow and connector labels;
- the project Design system source;
- the guide and value editor and Apply.

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

| Width | Arrangement | A fresh view |
|---|---|---|
| ≥ 1024 | Wide: header across; layers/pages left (240) and inspector right (260), each optional and resizable, beside the canvas. | Layers closed; the quiet Design inspector open |
| 600–1023 | Docked: header across; the canvas plus at most ONE panel at its usual width (Layers left, Design right). | Canvas only |
| < 600 | Sheet: header across; a chosen panel is an opaque sheet filling the room under the header, with "← Canvas" back (no canvas sliver, no input behind it). | Canvas only |

Rules that apply to every tier:
- **Layers and Design toggles** mean the same panel at every width. On a wide surface the two
  are independent. In a single-panel tier, opening one closes the other, and closing it
  returns to the canvas. Widening shows what the person chose; a side never chosen follows the
  fresh default.
- **Collapse panels** (menu View ▸ Collapse / Restore Panels, and the palette) hides both and
  keeps the arrangement, so Restore brings it back. Opening a panel while collapsed ends the
  collapse, with that panel only. The header, rail and zoom stay. **Hide UI** (Ctrl/⌘+\) is a
  separate mode that hides everything. Collapse has no shortcut yet: one is added only after
  the hosts' key handling has been checked.
- **Arrangement changes wait for fields.** A change waits while an egui text field has the
  keyboard, then lags one frame. A hidden field finishes as a click elsewhere finishes it.
  Docked ↔ sheet is the same panel, so its fields keep focus.
- **One canvas rectangle** (what the panels leave) is used for rendering, clipping and pointer
  input.
- **Native windows** use the wide tier only, unless a host opts in (`UiState::narrow`).

### Rail and zoom geometry

These are the values in `src/ui/rail.rs`. They are design calculations, not proof of how they
render.

- **Rail:** 64 px wide (32 px main button plus 24 px chevron plus 2 × 4 padding; the plan's
  52 px is widened so the chevron is a real target). Six 32 px rows with 2 px gaps, 210 px tall,
  8 px from the canvas's edges.
- **Placement:** vertically centred in the room above a 48 px zoom reservation. Below a 270 px
  canvas height it is anchored at the top, with the zoom beside it.
- **Flyout:** 220 px wide, opening inward at rail.right + 4. Where that leaves less than
  180 px it opens over the rail instead. Its height is clamped to the canvas (it scrolls).
- **Zoom:** −, the level (a menu with Zoom to Fit, Zoom to Selection and Zoom to 100%) and +,
  at the canvas's lower left.

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
  - header 38 (40 with native chrome);
  - 24 interactive height;
  - 26 icon buttons in the header and zoom; rail rows 32 × 32 plus a 24 × 32 chevron;
  - 16-unit icon grid, about 1.25 stroke.
- **Shadow:** only floating things (the rail, zoom, menus, popovers). Panels are flat.
- **Borders:** hairlines between regions only. Controls in bars and rows are borderless until
  hovered.

## Not in scope

- Multiplayer, sharing or cloud surfaces.
- Copied competitor themes, icons, fonts or code, including OpenPencil's.
- A second toolbar, an imitation DOM editor, another engine or a second theme system.
- New document formats.
- Decorative mockups counted as implemented work.
