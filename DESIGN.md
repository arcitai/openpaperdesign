# OpenPaperDesign — product design direction

Revision: 20 · 2026-10-21 · The one canonical visual and interaction direction for the OPD app
(Home and the workspace). The engine and architecture are described in
[analysis/DESIGN.md](analysis/DESIGN.md) (paper-rs's historical design: stack, crate layout,
rendering, document model), which stays valid and is not repeated here. This file is the
**product's** UI direction. It isn't a `design/<name>/` project, and it isn't the design
system of a person's canvas or project: a person's own `DESIGN.md` or `design.md` belongs to
their repository, and OPD's Theme view (principle 4) never reads this file as theirs.

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

## Whole-app workbench (revision 20, 2026-10-21)

The owner's 21 October feedback from ordinary Mac use (issue #10, and #14 for Theme) releases
the earlier wait and supersedes the older placement choices it names. These decisions are
**intended and written in source, not yet rendered, reviewed or qualified**; the before state
was captured by the lead on the frozen d0 candidate (a user-closed Design panel stayed closed
when a rectangle was selected). The operative principles below are updated to match; the
revision 18 and 19 sections stay as history.
- **Contextual Design reveal** (responsive since the run144 correction, and over a
  canvas-only sheet since run145, with no width excepted; principle 6 has the rules). Where
  the person closed the Design panel, a new selection or creation shows it at every width, and
  clearing the selection hides it again, giving back exactly the arrangement the person had
  (their left panel and its Design or Theme view included). The person's choices stay as they were (a reveal is never stored), an open
  (pinned) panel stays open, closing a revealed panel keeps it closed until the selection
  clears, and Collapse Panels still hides both. Like every arrangement change it waits while
  a field has the keyboard and while a pointer gesture owns the canvas, so no draft, focus or
  drag is lost or moved; nothing in the document changes.
- **One Canvas title.** The left bar shows All Projects, the Canvas's title and its toggle;
  there is no Paper line under the title and no shown-Paper name beside the folded Papers
  header. The Papers list (Design view) is where Papers are named, switched and renamed; the
  title's menu and the All Projects menu keep Rename Paper… (the same one field) and Go to
  Paper. The inspector no longer repeats the Paper's name or a Rename Paper… button. The
  page's own name for the open editor (Home's editor region and its heading, the browser
  tab's title) follows the title the editor has now: a finished rename at once, a rename made
  elsewhere with its reload (R61-F2: the region kept the name it was opened with), in the
  same mounted editor.
- **All Projects menu.** The square All Projects control opens one menu whose first item is
  Back to All Projects (the same guarded way home); the separate `···` button is gone, its
  items now in this menu. Where the surface is tall enough the menu and its submenus are
  shown whole; on a shorter one (320 × 240) they scroll within its height, so every item stays
  reachable, none left out to fit.
- **An open menu owns the input** (run147, after the lead's probe: with the menu open,
  ArrowDown nudged the selected rectangle behind it). While any menu, submenu, context menu
  or choice list is open, its keys and a press on the canvas are its, as an open flyout's
  and the canvas menu's already are: no key, Cut or Paste reaches the drawing, and the press
  that closes the menu never draws or selects. Opened from the keyboard (Tab to All
  Projects, then Enter or Space), the menu takes the focus at Back to All Projects; the arrow
  keys move through it, Enter or Space chooses, Escape closes it, and the selection stays.
  Opened with the pointer, nothing in it has the focus, so the arrow keys do nothing (keyboard
  use starts from the focused All Projects control). Other menus (Export, the zoom level, a Paper row's) get the same
  protection but not that first focus. A key's release always reaches the canvas, whoever has
  the keyboard then (R61-F1): Space held on the canvas and released while a menu item or a
  field has the focus ends the pan, so the next Select drag moves, not pans. All of it awaits
  the lead's rendered keyboard check.
- **Balanced corner bars.** Each bar's toggle is at its right end. The right bar holds Export
  (the existing exports and copies; labelled Export because there is no sharing service, issue
  #2) and the zoom (−, level, +), in the open Design panel's top row or floating at the
  canvas's top right, so neither moves when the panel opens or closes. Zoom is no longer at
  the lower left.
- **Tools.** Select and Hand are separate rail buttons (Pointer keeps Laser pointer in its
  flyout); all sixteen tools and shortcuts stay; the family arrow is drawn closer to its button
  without moving any target. Returning to Select after drawing a Frame or Rectangle stays
  (accepted by the owner).
- **Design and Theme.** The left panel's switch: Design is Papers, Layers and Library; Theme
  is the design system. Theme is not the appearance (System, Light, Dark stay in the menu).
  What Theme can and can't do yet is mapped below; nothing in it is decorative. It shows the
  useful part first (the Paper's text styles and colours), then one short status row each for
  the font, named tokens and the design source, the reasons in tooltips and a folded Details,
  so a short sheet still opens on something useful.
- **Naming.** The right panel and its toggle are called Design (no longer "Properties").
- **Corners.** A layer or Paper row being dragged to a new place shows the grabbing hand;
  panel edges keep egui's resize cursors. Choices (combo boxes) and the colour picker stay
  egui's own widgets in the shared tokens: their keyboard and pointer behaviour is the point,
  and restyling them further wasn't judged worth a custom widget.

### Theme: supported and blocked (revision 20)

| Capability | State | Notes |
|---|---|---|
| Fonts | Supported, read-only | The renderer draws one bundled family, Inter (SIL OFL 1.1), in Regular, Bold, Italic and Bold Italic. Theme shows it as one status row ("Inter · 4 faces"); no other family can be chosen. |
| Text styles on this Paper | Supported (inspect; edit through selection) | Every distinct size and weight/italic the Paper's text layers use (ranges included), with how many layers use it. Choosing one selects those layers, so the Design panel's Text section edits them together through the existing coalesced, undoable edits. |
| Colours on this Paper | Supported (inspect; edit through selection) | Every solid fill and stroke colour in use, with its layers; choosing one selects them for the Fill or Stroke section. |
| Named tokens (colour, radius, spacing, type…) | Blocked | The consumed upstream editor has no token store: `Document` holds its nodes and order only, a node's `Style` holds literal values, and `src/theme.rs` holds the app chrome's own values, not a canvas's. OPD will take named values from the project's design source rather than invent a second store (R21). |
| Project design source (`DESIGN.md` / `design.md`) | Blocked: admission and implementation | The app library and the in-view editor have no admitted project folder; OPD never infers one from the current folder, a picked folder, the project list or a canvas. Both the bounded source contract of R19–R20 (an explicit, admitted, revocable link) and the guide reading and editing itself are still to be built. Theme shows "Not connected", with the reason. |
| Apply to selection / new objects | Partly | Editing the selected text or colours applies values through the existing Design panel; an explicit Apply of a named value waits on named tokens. |
| Guide and value codec (precursor) | Written in source, no I/O, nothing shown | `openpaperdesign-serve`'s `guide` module, with its unit tests. The independent R63 review of an earlier version requested changes (a blank line lost after a lone CR, a broken escaped-marker block taken for unmarked data, fenced examples in lists and HTML read as values, numbers rounded into support, an info string beyond `json` accepted); those are corrected below. Which exact candidate passed which checks or reviews is recorded in its source handoff; this row is no native, Theme or source-link acceptance. Given a guide's bytes (UTF-8, at most 1 MiB, no NUL), it keeps them exactly (BOM, LF/CRLF/lone CR, front matter, unknown sections) and plans, against an expected revision, a line-hunk prose edit that leaves every untouched line's bytes (refused, not shortened, when the result would exceed 1 MiB; refused too in the rare case where an edit would bring an unchanged empty line right after a lone CR, which would merge into it), or one value's own byte span in one fenced block whose info string is exactly `json`, marked `"opdDesignTokens": 1` (also spelled with escapes, and a block that doesn't parse but names the marker, even escaped, makes the values read-only), outside lists and HTML blocks; a number counts only as written exactly (no rounding into support). Version 1 values: `colors` ("#rrggbb"/"#rrggbbaa"), `fontSizes` (points), `fontWeights` (400/700 only), `spacing`, `radii`, `frames` (direction, gap, padding). Other formats, several blocks, invalid JSON or repeated names stay read-only with the reason. It reads no file, links no source and changes nothing in Theme: the source stays "Not connected", named tokens and Apply stay blocked. |

### Accepted 22-point plan (revision 2) → current state

"Built" means written in source at that revision and checked by the lead at that
candidate's own scope; "revision 20" items are intended and unchecked.

| ID | Requirement | State now |
|---|---|---|
| R01 | Calm, consistent Home/editor/plugin | Built (revisions 2, 18, 19); revision 20 extends the same tokens to the new bars, switch and Theme |
| R02 | No permanently open left panel | Built; unchanged |
| R03 | One Collapse/Restore for both panels | Built; unchanged (the contextual reveal never applies while collapsed) |
| R04 | Panels and corner controls placed logically at every width | Revised in revision 20: toggles at each bar's right end, Export and zoom in the right bar |
| R05 | Compact left actions: All Projects and the menu | Revised in revision 20: one All Projects control with the menu, Back to All Projects first |
| R06 | Real file and page identity and rename | Built; revision 20: one Canvas title, Papers named in their list |
| R07 | Zoom − / % / + reachable with fit and selection | Revised in revision 20: upper right, in the right bar (was lower left) |
| R08 | Grouped tools with an explicit chevron | Built; revision 20: Select and Hand separate, seven families |
| R09 | Quiet contextual Design inspector | Built; revision 20: contextual reveal, no Paper rename duplicate |
| R10 | Explained optional Sketch style | Built |
| R11 | None / Dots / Grid from the canvas menu and View | Built |
| R12 | Home Grid default, kept List, equal cards | Built |
| R13 | Quiet Home without footer, notices kept | Built |
| R14 | Obvious New file destination | Built (storage cue) |
| R15 | Diagrams as an explicit workflow | Built in part (labels, label groups, bends); the rest of the workflow stays open |
| R16 | No stale canvas band when resizing panels | Kept as a reproduction item; no defect established |
| R17 | Light / Dark / System coherent | Built; re-check needed for the revision 20 surfaces |
| R18 | Input, focus, save, conflict, Home and reopen correct | Built; revision 20 must keep them (re-check needed) |
| R19 | Design system tab reads, creates and edits the project's DESIGN.md | Not built: needs the admitted-source contract and the guide reading/editing itself (Theme shows "Not connected"); not complete. A precursor is written: the no-I/O lossless guide codec (above); it reads and saves nothing |
| R20 | Resolve the actual project, reuse DESIGN.md / design.md | Not built: the same admission contract and its implementation; not complete |
| R21 | Useful tokens visible and editable alongside prose | Partly: fonts, text styles and colours in use; named tokens blocked (the codec reads and plans version 1 values from bytes, but no source reaches it) |
| R22 | Reference pack, audit and critique before implementation | Done (2026-10-18), extended by the 2026-10-20 and 2026-10-21 references |

Native, package and installation acceptance stay separate for every row.

## Owner UX revision (revision 18, 2026-10-20)

The owner's 20 October revision, as built in source (not yet rendered, reviewed or qualified
natively; see AGENTS.md for what was checked):
- **No full-width editor header.** Both panels run the surface's full height, each with its
  own top row in the safe area (principle 2). A closed panel's top row floats over the
  canvas's top corner instead.
- **Vocabulary.** The **Canvas** is the saved file; its **Papers** are its pages. Visible
  text, tooltips and accessibility names say Rename Canvas, Rename Paper, Duplicate Paper, Add
  Paper, Delete Paper, Papers, Next/Previous Paper, "Go to Paper: …", and the export scopes
  This Paper / All Papers; Home counts a file's Papers ("1 Paper", "3 Papers"). New names
  follow it: a Paper a person adds is "Paper N", and a new library file's first Paper is
  "Paper 1". Existing names stay exactly as stored ("Page 1" included); agents' unnamed pages
  keep upstream's "Page N", and upstream's own new projects keep theirs. Ids, JSON keys,
  file extensions, upstream's `page` code and schemas, the server's `pages` count, and
  Home's Files, Search files and New file are unchanged.
- **The Paper web app, as observed live** (organization and state evidence only, no hover
  proof and no Paper semantics copied): a full-height left panel with the file's identity at
  its top; no global top bar; with nothing selected the right panel shows the page's context,
  and a click on empty canvas returns to it.

## Paper-inspired polish (revision 19, 2026-10-20 evening)

The owner authorized a materially more Paper-like look on top of revision 18. These choices
are built in source but not yet rendered, reviewed or natively qualified (see AGENTS.md).
References were the before renders of the b1ca4ac candidate (Light and the actual Dark) and
the official paper.design homepage's editor illustration. Paper's own website was looked at
only; no code, asset, font or colour value was taken from it. Everything below is one token
change in `src/theme.rs` that carries through to the existing controls:
- **Type hierarchy from weight, not size.** Panel text is 12 pt Inter Regular. Inspector
  section titles (Paper, View, Fill, Stroke, Layer…) are 12 pt Medium in the text colour, list
  headers (Papers, Layers, Library, Design) 11 pt Medium in secondary grey, and the Canvas's
  name the one 13 pt SemiBold title. Before, every title was 13 pt SemiBold and the headers
  11 pt SemiBold, so the inspector read as a wall of bold.
- **Neutral, integrated surfaces.** The greys are neutral, without the earlier blue cast:
  panel #fafafa / #1e1e1f. Buttons, number fields, choices and text fields are one family,
  filled a step darker than the panel with a faint edge, instead of white boxes with a grey
  outline. Hover and press darken that fill. Floating surfaces keep a hairline and one
  softer, closer shadow.
- **Density.** The lists (layer and Paper rows, 24 px) stay compact. The inspector is the
  airy part: 24 px fields, 6 px apart, with one hairline and even room between sections.
  Paired values (X Y, W H, Points Ratio, the arrow heads, Constraints, blend and opacity,
  export format and scale) fill the row in two equal columns, and a single choice fills the
  rest of its row, so values line up at any panel width. Mixed is drawn as the field it
  stands for.
- **States.** The accent is kept for what is current or focused:
  - the current tool, a selected layer row, the chosen menu or palette row and an on
    alignment or layout toggle are the accent tint, with the glyph or text in the accent's
    "ink" (a step darker in Light, lighter in Dark);
  - an open panel toggle is pushed in (neutral grey), not blue;
  - keyboard focus is the 1.5 px accent ring on custom controls, or a 1 px accent-ink edge on
    egui's own fields, buttons and choices (their `active` look, shared with press and open);
  - disabled is tertiary grey.
  Nothing is a solid accent block with white text any more.
- **Icons.** Every icon button draws its glyph at the 16 pt grid size (smaller only on
  buttons under 24 pt) with one 1.2 pt line weight, centred. Folded sections (Sketch,
  Export) use the same line chevron as the lists, not egui's filled triangle.
- **Home** follows the same tokens: neutral fallbacks, 13 px body, an 18 px title, 6 px
  controls and softer menus. Its structure (Files, Search files, the layout toggles, New file),
  its resting search field and every behaviour are unchanged.
- **Errors stay readable** (a lead render finding on this candidate): the in-view editor's
  failure note (save failed, retrying; a refusal) is danger-coloured text with a 1 px
  danger edge on the view's own surface, and its buttons follow that colour. Before, it was
  white text on a danger fill, about 2:1 in Dark (white on #ff9a90). The fallbacks now
  measure about 6.3:1 in Light (#b42318 on #fafafa) and 8.2:1 in Dark (#ff9a90 on
  #1e1e1f). Under a host's style variables it is the host's own text-danger on its
  background-primary, the pairing those variables are meant for. The note's wording, live
  region, actions and timing are unchanged; healthy notes keep their quiet inverted look.

Unchanged: every revision 18 behaviour and protection, the rail's geometry (52 px, 32 px main
buttons, 12 × 32 arrows, short-room rows), the panel insets and top rows, the canvas
rendering and its Dark display, and the document. The combo boxes still use egui's own small
filled arrow (restyling it was left out for API safety in this source-only pass).

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
   and the closed panels' bars float over the canvas's edges, never over panels.
2. **Full-height panels, two balanced bars** (revision 18's full-height panels, with revision
   20's bars). There is no header across the surface: both panels start at its top, each with
   its own top row (38 px, 40 with native chrome, which holds the traffic lights at the left),
   separated from the canvas by one vertical hairline each. Each row's panel toggle is at its
   **right end**.
   - **The left bar:** All Projects (the square grid control, the only way back to Home), the
     **Canvas's title**, and the left panel's toggle at the end. All Projects opens one menu:
     **Back to All Projects** first (the same guarded way home: the host saves and drains
     first, and refuses while work is unsaved), then Rename Canvas…, Rename Paper…, the shown
     Paper's Duplicate, Add and Delete Paper, Go to Paper, and, where the window has no native
     menu bar, every upstream menu, Appearance, Background and About & notices. There is no
     separate `···` control.
   - **The right bar:** Export (PNG, SVG and PDF exports; Copy as PNG or SVG; an honest label,
     with no sharing service behind it), the zoom (−, the level, +; the level opens Fit,
     selection and 100%) and the Design panel's toggle at the end.
   - **A closed panel's bar floats** over the canvas's top corner instead, with the same
     controls in the same order, each where it is in the open panel's row (the title keeps the
     slot it has there), so nothing jumps when a panel opens or closes. Where the canvas is
     narrow the right bar shows the zoom's level only (its menu adds Zoom In and Out) and the
     left bar's title narrows, or is left out below 60 px, so the bars never overlap. The rail
     keeps to the room under the left bar; the canvas rectangle itself never changes for them.
   - **A sheet** (narrow) covers the canvas, so its top row carries All Projects and both
     toggles, under its "← Canvas".
   - **The Canvas's title** (revision 20: one title, no Paper line under it):
     - the file's name in one fixed slot, truncated, never overlapped, with a quiet save state
       after it only while there is one worth saying ("Edited", "Saving…"); nothing once saved.
       A rename field opens in the title's own slot: nothing moves or resizes;
     - in the web editor (browser page and Home's in-view editor) the name renames inline: a
       click (or Enter once Tab reached it), Rename Canvas… in the All Projects menu, or the
       title's secondary-click menu opens it; Enter or a click elsewhere keeps it, Escape drops
       it, an empty or over-long name is refused with a note. Until then it is an unfinished
       draft, so a clean reload or leave waits for it, and the arrangement waits while it has
       the keyboard (a hidden title finishes it as a click elsewhere does);
     - a kept name is the file's name in the library and Home. It is saved in the same canvas
       save as the drawing (one commit, one acknowledgement). A rename made elsewhere while
       this editor has unsaved changes is a conflict, as for any change;
     - native windows have no such save, so their name stays a label;
     - **Papers** are named, shown and renamed in the Papers list (the left panel's Design
       view). **Rename Paper…** from the title's menu or the All Projects menu opens that
       list's own rename field for the shown Paper, showing the list if needed: one field, one
       path, one page-list change, always on the Paper it was opened for. Go to Paper and
       Next/Previous Paper reach every Paper while the list is closed.
   - **The toggles** are one mirrored pair: the sidebar glyph and its mirror image, the same
     26 px size, quiet colours, tooltip style, focus ring and pushed-in (open) neutral fill,
     named for assistive technology. The left one says "Papers and layers", the right one
     "Design". Collapse Panels and Hide UI stay separate modes.

   Undo/redo, help and every other command stay in the All Projects menu, the command palette
   and their shortcuts. There is no outer title, stage or action bar.
3. **One restrained accent** (revision 19; before, the selected tool was a solid accent). The
   accent marks what is current, selected or focused, always as a tint. That covers the
   current tool, selected layer rows, the chosen menu or palette row, an on inspector toggle
   and Home's pressed layout buttons. Labels keep the text colour; glyphs use the accent's
   ink. Open panel toggles are pushed in, neutral grey. The shown Paper's row is the current
   one, not a selection: a grey (pressed) fill. Layer symbols are secondary grey; only
   components and instances keep the accent. Keyboard focus on a custom control is a 1.5 px
   accent ring inside it, and on egui's own controls a 1 px accent-ink edge.
4. **Contextual properties stay in the Design panel** (the right one); the Canvas's title,
   Papers, Layers, Library and the Theme in the left panel. Under the left bar a **Design /
   Theme switch** (revision 20) chooses what the left panel shows:
   - **Design:** the Papers list, then Layers and Library. Arrange (bring forward and send
     backward) shows in the layers header only while something is selected.
   - **Theme:** the canvas's design system, honestly scoped (see "Theme: supported and blocked"
     above): first the text styles and colours this Paper actually uses, each choosing its
     layers for the Design panel to edit; then one short status row each for the font
     ("Inter · 4 faces"), named tokens ("Not available") and `DESIGN.md` ("Not connected"),
     with the reasons as tooltips and in a folded Details. It is not the appearance, which
     stays System / Light / Dark in the All Projects menu.
     Switching to Theme hides the Paper and layer rows, so an open rename there is finished on
     its own Paper first, as the Library tab does.
   Every command keeps its menu entry and keyboard binding wherever a button moves or hides.
   - **Contextual reveal** (revision 20): see principle 6.
   - **Linked hover** (revision 18): the pointer over a layer on the canvas lights that layer's
     row with the ordinary hover fill, and its row's own hover still outlines the layer on the
     canvas. It is only a highlight: no selection, document, history or scroll change. It
     follows the canvas's own hit, only while the pointer is in the canvas and over no floating
     control, so it clears the moment the pointer leaves. Inside a collapsed group the group's
     row (the one shown) is lit; the tree is never expanded for a hover.
5. **One tool rail** (revision 2, built; revision 20: Select and Hand separate). A slim
   vertical rail at the canvas's left edge holds all sixteen tools in seven families:

   | Family | Tools (first = initial member) |
   |---|---|
   | Pointer | Select, Laser pointer |
   | Hand | Hand (pan) |
   | Frame | Frame |
   | Shapes | Rectangle, Ellipse, Diamond, Polygon, Star |
   | Lines | Arrow (binds to shapes), Line (free) |
   | Text | Text, Sticky note (a note, not a neutral box) |
   | Draw | Pencil, Pen (vector path), Eraser |

   - A family's main button runs its current member. Its own small arrow (a separate 12 × 32 px
     target beside it, revision 18) opens a flyout of whole-row choices: icon, name and the
     real shortcut. The arrow's target is always laid out, so nothing moves when it shows; it
     is drawn while its row is hovered or has the keyboard focus, while its flyout is open, and
     always for the current tool's family (not permanently for every family). Revision 20 draws
     the arrow at the near edge of its target, closer to the button, without moving either
     target. A surface without hover (touch) chooses the family first, then its arrow. Opening
     a flyout never chooses a tool. Hand and Frame have one tool each, so no arrow. Space and
     the middle button still pan with any tool. (TradingView's 52 px toolbar with a hover-only
     arrow was a reference for the size only.)
   - After drawing a Frame or Rectangle the tool returns to Select (owner-accepted, kept).
   - Choosing a tool, by row or shortcut, makes it the family's member.
   - Escape closes an open flyout first; Escape outside it returns to Select, and every family
     keeps its member.
   - A press outside closes the flyout and never reaches the canvas.
   - The rail's controls and rows take keyboard focus with Tab; Enter or Space chooses the
     focused one and never reaches the canvas.
   - Tool keys typed into a field stay the field's.
   - There is no bottom tool palette.
6. **A quiet, contextual Design panel** (revision 2, built; its states audited in revision 18;
   revision 20's reveal). Under the panel's own top row (Export, the zoom and its toggle):
   - **Contextual reveal** (responsive): where the person closed this panel, a new selection or
     creation shows it, and clearing the selection hides it again, giving back the arrangement
     they had. Where it goes:
     - **wide:** beside whatever is open;
     - **docked:** beside the canvas when no panel is open; beside the open left panel when
       the surface fits both and a 300 px canvas (`BOTH_DOCKED`, 800 px); narrower, in the
       left panel's place, which comes back as it was (its Design or Theme view too) when the
       selection clears, when this panel is closed, or from the left panel's own toggle;
     - **sheet:** in the open left sheet's place (its way back says "← Layers" or "← Theme"
       and returns to it); over the canvas alone (run145: the owner approved no exception
       here), the same Design sheet, once the canvas is done with the selection. That is
       after the gesture's release and the editor's 400 ms double-click window (so a
       double-click still edits a text or a path), and not while a text or path draft is
       open on the canvas. Its "← Canvas" goes back to the canvas with the selection kept
       (closed until the selection clears), and its top row's Papers and layers toggle opens
       that sheet instead; clearing the selection (a command, an agent's change) gives the
       canvas back. The narrow default stays the canvas alone.
     The person's choices are never rewritten; an open panel stays open; the toggle on a
     revealed panel closes it until the selection clears; any toggle ends a reveal that isn't
     shown yet (so closing a pinned panel with something selected keeps it closed); an
     explicit request for the left panel (its toggle, Rename, Rename Paper…) takes precedence
     over the reveal; Collapse Panels never reveals. The arrangement waits while any field has
     the keyboard and while a pointer gesture (a drag, a creation) owns the canvas, so drafts,
     focus, undo, the document and the canvas under the pointer are untouched; the change
     follows the release.
   - **Nothing selected** (also after a click on empty canvas), any tool: the shown Paper's
     context, in two kinds that are never mixed up:
     - **Paper**, the document's: what it holds ("n layers"). Its name lives in the Papers
       list, not repeated here (revision 20), and a Paper has no fill or size of its own, so
       none is shown or invented;
     - **View**, the viewer's: Grid (None, Dots, Grid), the same view preference as the
       canvas's menu, never part of the Paper, its undo or its exports.
   - **A creation tool:** its defaults for the NEXT object first, titled "For new …"; they
     never edit the selection or existing layers. The Paper and View sections follow.
   - **A selection:** the sections every selected layer has. A value they share is shown and
     editable for all. One interaction with a field (typing a value key by key, or dragging
     it) is one undo step, sealed when the field is released or loses focus; a later
     interaction is its own step. A value that differs shows Mixed and is never the first
     layer's value; Mixed always means the values differ. A widget that can't show Mixed is
     left out, with a short reason.
     - Typography is what the characters have, text ranges included (the styles the layout
       draws): equal base sizes with a range of another size are Mixed, with a word on why,
       never a shared size.
     - Independent corners are compared corner by corner: equal corners show as the four
       corner fields for every selected layer.
     - One layer, several of one kind and several of different kinds are the three selection
       states: the second and third say Mixed where values differ, and the third shows only
       what every kind has, with a word on why.
   - **Folded by default:** Sketch (the optional hand-drawn style, with a plain explanation;
     clean geometry unless turned on, and existing sketchy drawings keep it) and Export, which
     stays available in every state but secondary.
7. **Quiet surfaces, clear state.** Both panels share one colour and run the surface's full
   height, each separated from the canvas by one vertical hairline; there is no horizontal
   rule under a top row and no rounded window. Fields, buttons and choices are softly filled
   with a faint edge, never white outlined boxes.
8. **Problems stay visible.** Errors, conflicts and unfinished saves are never hidden for
   neatness (see the status rules in AGENTS.md). The status line sits at the canvas's lower
   edge, right of each control laid out there that reaches the canvas's lower half: the zoom,
   and on a short canvas the rail, which then runs down the left side. It never covers them,
   at any height, and is never hidden or shortened to make room.
9. **Light and dark are one design** (see Appearance below): Home, the chrome, the rail and
   zoom, menus, the inspector and the canvas itself share one resolved appearance.
10. **A plain canvas by default** (revision 2 slice 4, built). The background is None, Dots
    or Grid (revision 18's name for the line pattern, the same in every menu, the inspector's
    View and the palette; its stored name stays `lines`, so kept preferences still load). It
    is the viewer's grid over the canvas, never content on a Paper.
    - **Where to choose it:**
      - the canvas's own menu: a secondary click, without a drag, on blank canvas, never over
        an object, a panel or a popup. It holds Background, Zoom to Fit and
        Collapse / Restore Panels;
      - Menu ▸ Background, and the palette's "Background: …";
      - the grid shortcut (Ctrl/⌘+Shift+') and View ▸ Background On/Off, which toggle None
        against the last pattern.
    - **The menu takes its own input:** while it is open, Escape and presses (mouse or touch)
      close it and never reach the canvas. Hand, Space and middle-button panning are unchanged.
    - **A view preference, never the drawing.** Switching changes nothing in the document,
      its undo or its saves, and nothing in snapping or exports.
    - **Where it is kept:** the app library keeps it with Home's layout; the standalone
      browser page and native windows keep it for that window only.
    - **Grid** uses the dot grid's zoom-adaptive spacing, as 1 px lines.
11. **A quiet Home** (revision 2 slice 5, built).
    - **Layout.** Files show as a grid of equal cards by default, and a list once List is
      chosen; the library keeps that choice. A choice it couldn't keep still applies, and Home
      says so while that choice is the latest and its Home is shown.
    - **Cards.** Each card has its glyph, two reserved title lines (a longer name is clamped;
      its full name is the card's tooltip and accessible name) and one line each for pages
      and the last edit.
    - **No footer or wordmark.** The licences and notices are the More menu's About & notices
      (the editor's All Projects menu has the same).
    - **Storage cue:** "New files are kept in the OpenPaperDesign library, this app's own
      data folder, not in a Codex project." No project association is shown or implied.
    - **Search** looks like a search field at rest (revision 18): the field's soft surface
      inside a visible hairline, with a magnifier before the text; focus shows the ring. It is
      the same one input: it filters as before, Escape clears it, and it says when nothing
      matches and how to clear it. The grid/list choice and the narrow layout are unchanged.

12. **Diagrams from existing objects** (revision 2 slice 7, partly built). A box with its
    label is a group of one shape and one Text layer over it.
    - Selected, it has the "+" quick-connect knobs: a knob copies the whole group beside it
      (label included, still ordinary text) and joins the two shapes with one arrow, one undo
      step. Arrows always bind the shape, never the label or the group.
    - Resizing it never scales the label's characters or font. The label stays on one line
      while it fits, wraps at spaces within the shape otherwise, and stays centred on the
      shape where it was. A label too tall for its shape even so overflows it; it is never
      clipped.
    - Other groups (several shapes, nested groups) keep ordinary group behaviour.
    - **Arrow labels** (source candidate, not yet independently reviewed or qualified natively).
      An arrow can carry its own label, part of the arrow rather than a separate text layer.
      - A double-click on the arrow, or Enter with it selected, edits the label in place.
        Escape or a click elsewhere keeps it; emptying it removes it. Each edit is one undo
        step.
      - It sits at the middle of the line as drawn (curves and elbows included), on a small
        backing in the canvas colour, in the arrow's colour. It moves whenever the arrow
        reroutes. Light and Dark show it as they show the canvas; the saved colours don't
        change.
      - Where that would hide the arrow's ends or heads (a short arrow, a long label), it sits
        just beside the middle instead, above the line (left of a vertical one). The arrow's
        direction and the whole label stay visible.
      - It is duplicated, saved, reopened and exported (SVG, PDF) with its arrow. Agents set
        and read it through the same design tools.
      - **Copy, Cut and Paste keep labels.** Layers holding a label go on the clipboard in
        a labelled, versioned layer format, and paste with their labels into another
        canvas or window of this OpenPaperDesign (one undo step, as any paste). An older
        OpenPaperDesign doesn't read that format as layers: it can only paste it as plain
        text, never as arrows without their labels. A labelled clipboard it can't read (a
        newer version, damaged) is refused with a note and nothing is pasted. Layers without
        labels copy exactly as before. Duplicate works, and a label's selected characters
        copy as plain text.
      - **Not yet transferable through the Library.** The native Library file has no version,
        so an older OpenPaperDesign would take labelled items without their labels: adding
        them to the Library is refused with a note before anything changes.
      - **Paste while editing a label** puts the clipboard's text into that label, at the
        caret or over the selected characters, in the same one-step edit. It is always plain
        text (line breaks kept, tabs as spaces): whatever it looks like, it is never pasted
        as layers or as a new text layer beside the arrow.
      - Labels are plain text: no character styles. Where a label covers a straight arrow's
        middle, there is no bend handle under it (its Type still makes it curved). A label
        beside a short arrow leaves the handle free.
      - **A bend stays editable.** A curve keeps a bend handle on each of its bend points,
        even under its label (a double-click there still edits the label, and Enter does
        too), so dragging one bends the curve again there in any direction, every other bend
        staying where it is, one undo step per drag. That holds for a curve bent once and for
        one with several bends (made by an agent). Elbows are routed automatically and have
        none; neither do straight lines with corners. Bends can't be added or removed this
        way, and where two handles overlap the first one is taken. A locked arrow has no
        handles, and a bend or end drag held while it gets locked stops there: what it did
        before the Lock stays, nothing more changes.
      - **Compatibility.** A page with a label is saved in a newer page version, which an
        older OpenPaperDesign refuses to open rather than silently dropping the label. Pages
        without labels are unchanged. A single `.paper-rs` file can't hold labels: saving one
        is refused.

Still open from the revision 2 plan (not built yet, and not to be described as built):
- the rest of the diagram workflow (and any native qualification of the labels);
- the project design source (R19–R20: shown in Theme as blocked on admission);
- named tokens, the guide and value editor and their explicit Apply (R21: Theme shows the
  values in use and the bundled font; named values are blocked).

## Appearance (System, Light, Dark)

- **The choice.** The All Projects menu's Appearance in the editor: System (default), Light or
  Dark. It is compact and lives in the existing menu, so there is no extra toolbar. It is not
  the left panel's Theme (principle 4), which is the canvas's design system.
- **System** follows the OS appearance, live: the browser's colour-scheme preference (in the
  native app, egui-winit's system theme). The host's theme stands in only where the browser
  can't report one. With no signal at all, it is light.
- **Light and Dark** override the OS.
- **Where it is kept.** It is a view preference, never part of a canvas:
  - the app library keeps it in its own data folder (`library/v1/appearance`), so it survives
    closing and reopening the view. Home's layout and the canvas background are kept beside
    it, in one small record of two keys (`library/v1/view`). Each key follows the same
    freshness rule: a list read that overlapped an unresolved choice never replaces it.
    Windows keeping a key at once take turns through an OS lock file (`library/v1/view.lock`),
    so neither undoes the other's kept key; a keep that can't take its turn within about a
    second, or finds the library moved or replaced, keeps nothing and says so;
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
| ≥ 1024 | Wide: no header; layers/pages left (240) and inspector right (260), full height, each optional and resizable, beside the canvas; a closed one's top row floats over the canvas; a closed Design panel is revealed while something is selected. | Layers closed; the quiet Design inspector open |
| 600–1023 | Docked: no header; the canvas plus ONE chosen full-height panel at its usual width (Layers left, Design right); the other's top row floats. A revealed Design panel joins it where both fit (from 800 px), else stands in for the left panel until the selection clears. | Canvas only |
| < 600 | Sheet: no header; a chosen panel is an opaque sheet filling the surface, with "← Canvas" back and both toggles and the way home in its top row (no canvas sliver, no input behind it). A revealed Design sheet stands in for an open left sheet ("← Layers" / "← Theme" back), or covers the canvas once the canvas is done with the selection ("← Canvas" back, the selection kept). | Canvas only |

Rules that apply to every tier:
- **Layers and Design toggles** mean the same panel at every width. On a wide surface the two
  are independent. In a single-panel tier, opening one closes the other, and closing it
  returns to the canvas. Widening shows what the person chose; a side never chosen follows the
  fresh default.
- **Collapse panels** (menu View ▸ Collapse / Restore Panels, and the palette) hides both and
  keeps the arrangement, so Restore brings it back. Opening a panel while collapsed ends the
  collapse, with that panel only. Both floating bars (All Projects with its menu, the Canvas's
  title, Export, the zoom and the toggles) and the rail stay; no contextual reveal happens
  while collapsed. **Hide UI** (Ctrl/⌘+\) is a separate mode that hides everything. Collapse
  has no shortcut yet: one is added only after the hosts' key handling has been checked.
- **The contextual reveal** is never a stored choice, at any width: see principle 6.
- **Arrangement changes wait for fields and gestures.** A change waits while an egui text
  field has the keyboard, then lags one frame; it also waits while an editor pointer gesture
  (a drag, a creation, a marquee) owns the canvas, so the canvas never moves or resizes under
  the pointer. A reveal that would cover the canvas (a sheet) also waits out the double-click
  window and an open text or path draft on the canvas. A hidden field finishes as a click
  elsewhere finishes it. Docked ↔ sheet is the same panel, so its fields keep focus.
- **One canvas rectangle** (what the panels leave) is used for rendering, clipping and pointer
  input. The floating bars and the rail are floating UI over it: a press there is theirs,
  never the canvas's, and they never change the rectangle.
- **A panel's edge is the panel's.** A wide panel's resize handle reaches a few pixels into
  the canvas. A press egui gives that handle (its own rect widened by egui's interact radius)
  only resizes the panel: both hosts keep it from the editor, so nothing behind the edge is
  selected, moved or drawn. Elsewhere the canvas takes presses as before.
- **Native windows** use the wide tier only, unless a host opts in (`UiState::narrow`).

### Rail, bars and zoom geometry

These are the values in `src/ui/rail.rs` and `src/ui/mod.rs`. They are design calculations,
not proof of how they render (revision 20's values are unchecked).

- **Rail:** 52 px wide (32 px main button plus the arrow's 12 px target plus 2 × 4 padding;
  revision 2's 24 px chevron column made it 64). Seven 32 px rows (revision 20: Hand has its
  own) with 1 px gaps, 238 px tall, 8 px from the canvas's edges.
- **Placement:** in the tools' room (the canvas under the floating left bar while the left
  panel is closed, else the whole canvas), vertically centred above a 48 px reservation for
  the status line. Below a 270 px room height it is anchored at the room's top.
- **A short room** (too low for seven 32 px rows between its 8 px insets) gets rows of the
  whole pixels that fit, down to 24 px (main buttons square, arrows as high), so the rail
  never runs past its room. 320 × 240 gets 24 px rows; the canonical 270 px surface 28 px;
  about 293 px of canvas and taller keep 32 px rows; below about 237 px even 24 px rows don't
  fit.
- **Bars:** each a row of 26 px controls 2 px apart. Floating, a 4 px padded surface (radius
  10) whose controls sit where they are in the open panel's top row (10 px from the left edge,
  after the traffic lights with native chrome; 14 px from the right edge). The left bar's
  title slot is the open panel's (164 px in a 240 px panel), narrowed only to clear the right
  bar; below 60 px it isn't drawn (a draft then finishes).
- **Flyout:** 220 px wide, opening inward at rail.right + 4. Where that leaves less than
  180 px it opens over the rail instead. Its height is clamped to the canvas (it scrolls).
- **Zoom** (revision 20, in the right bar): −, the level (a menu with Zoom to Fit, Zoom to
  Selection and Zoom to 100%) and +, between Export and the Design toggle. Where the bar has no
  room for all of it (a canvas under 480 px for the floating bar, or a narrow panel row), only
  the level is shown and its menu holds Zoom In and Zoom Out too. The menus and shortcuts
  always zoom.

## Tokens

These are the **product UI's** values, from `src/theme.rs` (the app chrome, not any canvas's
design system). Home uses the same values as fallbacks under the host's MCP Apps style
variables.

| Token | Light | Dark |
|---|---|---|
| surface (panels, top rows) | #fafafa | #1e1e1f |
| field (buttons, fields, choices; Home's hover surface) | #f0f0f1 | #2a2a2c |
| control (floating surfaces use popover: #ffffff / #262628) | #ffffff | #2c2c2e |
| field edge | black 5.5% | white 5.5% |
| text | #1c1c1e | #ececed |
| secondary text | #6c6c70 | #a2a2a6 |
| tertiary / disabled | #aeaeb2 | #626266 |
| hairline | #e8e8ea | #303033 |
| accent | system accent, else #007aff | system accent, else #007aff |
| accent ink (text, glyph or 1 px edge on its tint; ≥ 4.5:1 there for system blue) | the accent 24% toward black (Home: #0062cc) | the accent 35% toward white (Home: #5ea8ff) |
| selected (tool, layer row, chosen row, on toggle) | accent tint over surface (≈15% light, 28% dark) | |
| egui selection (text selection, chosen menu item) | accent at 20% / 35% | |
| current (the shown Paper's row, an open panel toggle) | the pressed fill (black 5.5%) | the pressed fill (white 8%) |
| hover | black 3% | white 5% |
| focus | accent ring, 1.5 px inside (custom controls); accent-ink 1 px edge (egui controls) | |
| canvas work surface | #eeeef0 | #18181b |
| canvas grid dot | #cfcfd6 | #34343a |
| canvas grid line (Grid) | the grid dot at 60% opacity | the grid dot at 60% opacity |

The revision 18 audit (toolbar, sidebar and inspector together, Light and Dark), as revised in
revision 19: hover is one fill everywhere (rows, icon buttons, the rail's arrows, flyout rows);
the accent tint marks what is current, selected or focused (the current tool included); open
panel toggles and the current Paper are grey; layer symbols are grey but for components and
instances. Home's search uses the stronger hairline (#d6d6d9 / #48484c) at rest.

- **Type:** Inter. 12 body; 12 Medium inspector section titles in the text colour; 11
  secondary text and 11 Medium list headers in secondary grey; the Canvas's name 13
  SemiBold (`theme::TITLE`). Home: 13 px body, 18 px title.
- **Shape:**
  - controls, fields, rows, icon buttons and toggles 5 (one radius, `theme::RADIUS`);
  - floating surfaces (the rail, the floating bars, menus, the canvas's menu) 10, windows and
    popovers 12; the Design/Theme switch 6 outside, 5 for its raised half;
  - Home: controls 6, list rows 8, cards 10.
- **Spacing:**
  - a panel's top row 38 (40 with native chrome);
  - 24 interactive height (`theme::FIELD_H`): inspector fields, labels and toggles share it,
    6 apart; sections 8 + hairline + 8 apart;
  - 26 icon buttons in the bars (All Projects, Export, zoom −/+, the toggles), 2 apart; the
    Design/Theme switch 28 high; rail rows 32 × 32 plus a 12 × 32 arrow, 1 apart;
  - 16-unit icon grid, glyphs 16 on 24 pt and larger buttons (`theme::ICON`), 1.2 stroke
    (clamped 1–1.4).
- **Shadow:** only floating things (the rail, the floating bars, menus, popovers):
  one soft close shadow (popups 0/2/10 at 8% light, 31% dark). Panels are flat.
- **Borders:** hairlines between regions only (one vertical one per open panel). Icon
  controls in top rows and rows are borderless until hovered or focused; fields and buttons
  carry only their faint edge.

## Not in scope

- Multiplayer, sharing or cloud surfaces.
- Copied competitor themes, icons, fonts or code, including OpenPencil's.
- A second toolbar, an imitation DOM editor, another engine or a second theme system.
- New document formats.
- Decorative mockups counted as implemented work.
