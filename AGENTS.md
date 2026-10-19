# AGENTS.md — openpaperdesign

Revision: 0.64 · Updated: 2026-10-19

openpaperdesign is a fork of the
[michaelshimeles/paper-rs](https://github.com/michaelshimeles/paper-rs) repository. The goal is an editable paper-rs canvas inside an agent host, such as Codex's
built-in browser or an MCP App panel ([issue #1](https://github.com/arcitai/openpaperdesign/issues/1)).
It is not affiliated with or endorsed by Paper (paper.design). Do not suggest that it is.
The product's visual and interaction direction is root `DESIGN.md` (one canonical file); the
engine architecture is `analysis/DESIGN.md`.

## Upstream and credits

- Keep upstream licenses, notices and history: `LICENSE-MIT`, `LICENSE-APACHE`,
  `THIRD_PARTY.md`, `assets/fonts/OFL.txt` (Inter, SIL OFL 1.1). Any distributed build must
  carry the applicable notices.
- Keep diffs to upstream-owned paths (`src/`, `tests/`, root `Cargo.toml`/`Cargo.lock`,
  `README.md`, `PROGRESS.md`) minimal. Put new code in separate adapter crates under
  `crates/`, which reuse upstream modules via `#[path]`. If an upstream file has to change,
  give the reason in the commit.
- Upstream source changes so far (keep this list current):
  - `src/render/gpu.rs`: the image texture is sampled in uniform control flow (B1, WebGPU
    validation in browsers).
  - `src/history.rs`, `src/editor/pages.rs`: `#[derive(Clone)]` on `History`, `PageStash`
    and `PageSlot` (B2a review round 5). Derive-only, no behaviour change. It lets the agent
    host restore the exact editor state after a failed MCP call.
  - `src/mcp/exec.rs`: `pub fn with_no_stretch` gives access to the process-wide record of
    containers that don't stretch their children (B2a final review). Accessor only, no
    behaviour change. The host copies and restores its own pages' entries with the editor. A
    newly activated project drops stale entries for its page ids. `Server::retire` first
    closes admission and drains the current writer/rollback, then releases every page id
    the host held. Queued and retained old handles cannot execute or save after retirement.
  - `src/ui/mod.rs` `layer_row` and `src/ui/pages.rs` `page_row`: the inline layer and page
    renames check `lost_focus` before asking for focus again (and ask only while they lack
    focus). With egui 0.36, `request_focus` takes focus back at once, so the old order hid
    Enter and click-away and the rename never committed, natively too. A plain upstream bug
    fix, the same two lines in both places. (The earlier `UiState::pages_locked` host flag is
    gone: the browser editor now stores every page action, so it had no caller left.)
  - `src/ui/pages.rs` `panel`/`run`, `src/ui/mod.rs` `commit_rename` (R22, natively too): the
    pages panel's add, switch, duplicate, delete and move run once after its rows are drawn,
    instead of inside the row loop (a delete of a non-final row indexed past the shortened
    list and panicked). Before such an action, open fields are finished on the page they were
    opened on, as a click elsewhere finishes them: the layer rename (`UiState::renaming` names
    a node by its document-local id, which another page may reuse) is committed to the shown
    page, the page rename (it names its row by position) is committed to its page, and other
    keyboard focus ends, so no draft continues on the next page. `commit_rename` is the layer
    row's existing lost-focus commit, shared. Same `Editor` page operations; upstreamable.
  - R23 (natively too): that finish is `pages::finish_fields`, also run before every command
    palette command (Go to page called `switch_page` directly). A rename whose row isn't drawn
    is finished as losing focus does, on the shown page: the layer rename when the Library tab
    or hidden UI hides the layer rows, or its row is inside a collapsed group; the page rename
    when the pages panel is collapsed or the UI hidden. Before, such a draft stayed stored with
    a document-local id or row position and could later land on another page.
    `UiState::rename_field` and `PagesUi::rename_field` record each rename field's widget id
    (no behaviour change), like `PaletteUi::search_id`: the browser host exempts only that
    focused field, never another field while a rename is merely stored.
  - `src/editor/pages.rs`: `Editor::page_gen` is `pub` (visibility only). The browser host
    records each sent page's history generation, so a save's late answer marks only what it
    sent as saved, never a newer edit (upstream saves synchronously and has no such seam).
  - `src/ui/palette.rs`: `PaletteUi::search_id`, the search field's actual widget id, recorded
    each frame the palette is drawn (no behaviour change). The browser host needs it to exempt
    only that ephemeral search from its unfinished-input guard; the id is otherwise automatic
    and can't be known without guessing. Upstreamable as a read-only host seam.
  - UI/UX pass (owner-admitted 2026-10-18, minimal/airy/low-noise direction in root
    `DESIGN.md`):
    - `src/theme.rs`: one calm palette (bar and panels share one surface, softly filled fields,
      more readable secondary text); slightly larger control padding and radii;
      `selected_fill` (an accent tint for selected rows and toggles); `quiet_icon_button`
      (secondary until hovered, tinted when on) for the top bar; Undo/Redo icons.
    - `src/ui/mod.rs`:
      - one shared `top_bar` (All projects, one Menu with every upstream menu, the file's name
        and page/save state truncated instead of overlapped, undo/redo, zoom, help), with
        quiet icons and the zoom level as secondary text;
      - the floating tool palette wraps whenever one row doesn't fit the canvas (before, it
        ran under the side panels);
      - layer selection is the accent tint; the layers header's bring forward / send backward
        show only while something is selected (Arrange menu, palette and shortcuts unchanged).
    - The native ordinary layout looks different too: the new palette and spacing, the
      quieter top bar with undo/redo buttons, and (where there is no native menu bar, i.e. not
      macOS) one Menu button in place of the inline menu titles. Shortcuts, menu entries,
      commands and editing semantics are unchanged. Upstreamable as presentation.
  - Appearance, System/Light/Dark (owner-admitted 2026-10-18; semantics in root `DESIGN.md`):
    - `src/ui/mod.rs`: `UiState::appearance` (egui's `ThemePreference`, default System),
      applied by `draw` with egui's `Context::set_theme`. Menu ▸ Appearance sets it. Native
      default unchanged: System is what `theme::apply` already set, and native macOS (no egui
      Menu) keeps it and follows the OS through egui-winit.
    - `src/render/scene.rs` / `frame.rs`: `scene::Display` (Light = the document as authored;
      Dark = a dark work surface and grid, plus display-adapted fill, gradient-stop, stroke
      and glyph colours: OKLab lightness mirrored, hue/chroma/alpha kept, images never
      adapted). It is held by `SceneCache` (a change drops cached meshes). `Painter::paint`
      picks it from the egui context's resolved theme.
    - Exports and `SceneCache::default()` stay Light. Nothing writes the document.
    - Native windows in a dark OS appearance now get the dark canvas too (before, dark chrome
      around a light canvas). Upstreamable as presentation.
    - The browser host feeds the OS appearance as egui's raw-input `system_theme`
      (`WebState::set_system_dark`), applies a kept preference (`set_appearance`) and reports
      a menu choice (`HostRequest::Appearance`). The app library keeps it through
      `openpaperdesign_app_appearance` in `library/v1/appearance`, and its list reports it.
  - `src/ui/mod.rs` (R25-F3): an opt-in narrow layout, `UiState::narrow` (`NarrowLayout`,
    `NarrowPanel`). Off by default (`below` 0), so every native window lays out exactly as
    before. The browser host turns it on below 1024 CSS px (it was 720 until the UI/UX pass),
    where the side panels (240 + 260 px, at least 180 + 220) leave the canvas too little room
    (a 320 px sidebar showed only the layers panel and a clipped inspector). From `dock_from`
    (600 in the browser host) the one chosen panel is docked beside the canvas at its usual
    width; narrower, it fills the room. Below it the top bar spans the surface (All projects, the
    file's name, every menu under one Menu, Layers and Design toggles) and at most one of the
    same side panels fills the room under it; otherwise the canvas with the same floating
    tools, which wrap there (`toolbar`'s `wrap`, false in the ordinary layout). An arrangement
    change waits while an egui text field has the keyboard and then lags one frame, so a
    field about to be hidden finishes as a click elsewhere finishes it; hidden layer/page
    renames are finished as when the UI is hidden (R23), and upstream's Rename shows the
    layer rows. Same panels, commands and canvas; no DOM copy, no global scaling.
    Upstreamable as a host opt-in for small windows.
  - Revision 2 workspace, slices 1–3 (owner plan OPD-UX-PLAN revision 2, 2026-10-18; built
    decisions in root `DESIGN.md`). These change how native windows look and lay out too.
    - `src/ui/mod.rs`:
      - **Header first.** `draw` lays the header out before the panels, across the whole
        surface. It holds All projects, a compact `···` menu (every upstream menu and
        Appearance) and the Layers toggle; the file's name and page centred; and the Design
        toggle. Undo/redo, zoom and help left the header and stay in the menu, palette and
        shortcuts.
      - **Panel model.** `UiState::panels` (`Panels`) holds the side-panel choices:
        - Layers closed by default; Design open by default only in the wide tier;
        - Collapse panels hides both and keeps the arrangement for Restore, separate from
          Hide UI.
        - `NarrowLayout` keeps its thresholds and focus-safe settling, but now settles a whole
          `Arrangement` (`Tier::{Wide, Docked, Sheet}`), and a sheet has a "← Canvas" way
          back. (Before: a narrow-only `NarrowPanel` choice, and both panels always shown in
          the ordinary layout.) Docked ↔ sheet with the same panel doesn't wait: the panel's
          body has the same widget ids in both, so a focused field keeps its draft.
      - **Removed:** the bottom tool palette (`toolbar`) is gone.
      - **Zoom** is at the canvas's lower left (`zoom_controls`), and the status line sits
        right of it.
      - **Geometry record.** `UiState::chrome` records where the header controls and zoom were
        laid out, for checks (no behaviour).
    - `src/ui/rail.rs` (new): the six-family tool rail with all 16 tools, remembered members,
      a separate chevron and a clamped, scrolling flyout. `Rail::captures_input` lets hosts
      hold Escape and a dismissing press back from the canvas while a flyout is open, and
      `Rail::takes_key` Enter and Space while a rail control has egui's keyboard focus.
    - `src/ui/inspector.rs`:
      - **Contextual.** The empty state shows the page plus a hint, or a creation tool's own
        "For new …" defaults.
      - **Mixed values.** Sections appear only when every selected layer has them. `uniform`
        shows shared values; differing ones show Mixed or are left out with a reason, never
        the first layer's value.
      - **Multi-layer edits.** W/H for several layers is one coalesced resize.
      - **Folded:** Sketch (explained) and Export.
    - `src/editor/shortcuts.rs`, `commands.rs`, `mod.rs`, `src/ui/palette.rs`:
      `Action::TogglePanels` / `UiEvent::TogglePanels`, reached from View ▸ Collapse /
      Restore Panels and the palette. It has no shortcut until the hosts' key handling is
      checked; the menu test allows it as menu-only.
    - `src/app.rs`: the native host holds Escape and a press back from the editor while a
      flyout is open, and Enter/Space while a rail control has the keyboard focus, as the
      browser host does (`crates/openpaperdesign-web/src/state.rs`).
    - Same Editor, Session, transactions, save, binding and lease; no renderer change.
      Upstreamable as presentation, except the panel defaults, which are an OPD choice.
  - Revision 2 workspace, slices 4–6 (same plan; root `DESIGN.md` principles 2, 10 and 11).
    Native windows change too.
    - **Background.**
      - `src/render/scene.rs`: `Background` (None by default, Dots or Lines) and a line grid
        beside the dot grid, built each frame in the screen-space underlay (no cache).
        `scene::canvas` and `Painter::paint` take `impl Into<Background>`, so an existing
        `bool` caller still means dot grid or none.
      - `src/ui/mod.rs`: `UiState::background` replaces `show_grid_off`, set with
        `set_background`. `toggle_background` (the grid shortcut, ToggleGrid) toggles None
        against the last pattern.
      - The canvas's own menu (`open_canvas_menu`, `canvas_menu`): Background, Zoom to Fit
        and Collapse / Restore Panels.
      - `UiState::captures_input` (flyout or canvas menu open) is what both hosts now check
        before a canvas press or Escape.
      - Menu ▸ Background and About & notices (`NOTICES`).
      - `src/ui/palette.rs`: `Command::Background`.
      - `src/editor/shortcuts.rs`: the View item "Dot Grid" is "Background On/Off".
      - `src/macos.rs`: its check item starts off, labelled Background.
      - `src/app.rs`: paints `st.background` and uses `captures_input`. The background is not
        persisted natively (one window's view).
      - Reason: plan section 4. The existing grid renderer and shortcut are reused; no
        second engine, DOM grid or document change.
    - **Header identity.**
      - `src/ui/mod.rs`: `UiState::file_rename` (a host opt-in; native off), the title's
        `file_title_draft`/`file_title_field` and `file_name`.
      - The page line's page actions (`page_actions`), through `pages::run_action` and
        `PageAction` (now `pub(super)`, no behaviour change).
      - Reason: plan section 6. Page rename stays upstream's own page transaction; the file
        name is saved by the host's canvas save, below.
  - Revision 2 corrections (independent review R33 of slices 1–3, and the first slice 4–6
    checks; root `DESIGN.md` revision 8). Native windows change too.
    - **A panel edge's press** (R33-F1; inherited from before revision 2): a wide panel's
      resize handle reaches into the canvas, and its first press also reached the editor, so a
      panel resize dragged the shape behind the edge. `src/ui/mod.rs` records each handle as
      laid out (`Chrome::panel_edges`: egui's own handle widget, read by its egui 0.36 id and
      widened by egui's interact radius, within which egui's hit test gives it the press);
      `Chrome::on_panel_edge` is what both hosts (`src/app.rs`, the browser host's
      `pointer_down`) check before a canvas press, and the canvas menu doesn't open there.
      Editor semantics are unchanged.
    - **Effective typography** (R33-F2), `src/ui/inspector.rs`: several text layers share a
      size, colour, bold or italic only when every character has it, from upstream
      `TextStyle::char_style` per run between range edges (`char_styles`, `shared_char`);
      otherwise Mixed, with a hint. Before, base values were compared, so a 48 range beside a
      plain 24 showed an editable "shared" 24.
    - **One undo step per field interaction** (R33-F3), `src/editor/text_edit.rs`:
      `Editor::style_text_coalesced` is `style_text` with the inspector's coalescing key (field
      and layers) through the existing `begin_coalescing`, sealed by the field's release or
      focus loss as before. The inspector's Size and colour use it; ⌘B/⌘I and other callers
      keep `style_text` (one step per press). A text range's change is unchanged.
    - **Status and zoom room** (R33-F4, F5), `src/ui/mod.rs`: `status` is placed right of the
      zoom and rail rects laid out this frame wherever they reach the canvas's lower half (a
      short canvas's rail did not count before). `zoom_controls` judges its room from its
      measured width (kept in egui's temp memory) plus the current percentage's text: without
      room only the level is shown, its menu adding Zoom In/Out; without room for that, none.
    - **Independent corners** (R33-F6), `src/ui/inspector.rs`: several layers' corners are
      compared corner by corner (`corner_radii`), so equal ones show the four fields, not
      Mixed.
    - **The just-closed canvas menu** (slice 4–6 checks): egui counts an area visible for one
      frame after its last, so until the next frame a press where the closed menu was found its
      stale area: a secondary click there didn't reopen it, and a press there (a middle-button
      pan) never reached the editor. One shared rule, `UiState::takes_pointer` /
      `ui::over_floating_ui` (any area above the background, except the canvas menu's own
      layer while it is closed), is now used by `open_canvas_menu`, the browser host's
      `over_canvas` and the native `App::over_canvas` (which used `is_pointer_over_egui`, the
      same `layer_id_at` test, at egui's last pointer position). An open menu still takes
      presses (`captures_input`); every other area still owns its stale frame as before.
    - **Home shows and sends library names exactly** (lead identity probes, not upstream):
      `crates/openpaperdesign-serve/src/home.html` used the workspace label rule
      (`cleanName`: quotes and backticks to spaces, spaces collapsed, cut at 60 UTF-16 units)
      for library files too, so a legal saved `A  file "quoted"` showed as `A file quoted`
      and a 60-character name ending in an emoji was cut through its surrogate pair. Library
      names now follow the server's `canvas_name` rule: `libraryName` shows the saved name
      exactly (an answer breaking the rule is still bounded: controls to spaces, 60 whole
      characters), and Rename's `libraryNameInput` trims and refuses, with a reason, a name
      with control characters or over 60 characters instead of rewriting it. The rename
      field's `maxlength` is 120 UTF-16 units (any 60 characters fit); storage and the
      server's bounds are unchanged. `cleanName` stays the workspace view's label rule.
    - Reason: review findings on the admitted slices; same Editor, transactions, renderer,
      Session, save, binding and lease. Upstreamable as fixes.
    - **A touch while the canvas menu or a flyout is open** (R34-F1), `src/app.rs`: native
      touch moved into `App::touch`, whose first contact checks `UiState::captures_input` and
      the panel edge as the mouse press does. Before, a touch that dismissed the menu also
      drew with the active tool.
  - Revision 2 slice 7, diagrams with existing objects (plan section 7, R15; root `DESIGN.md`).
    Native windows change too.
    - **One label-group rule**, `src/layout.rs` `label_group`: a group of exactly one plain
      bindable shape and one Text layer painted over it, with the text's centre on the shape.
      Anything else (free text, several shapes, a nested group, an instance or component) is
      not one.
    - **Grouped quick connect**, `src/editor/whiteboard.rs`: `quick_knobs` shows the "+"
      knobs for a selected label group too (`quick_shape`), and `quick_connect_label` copies
      the whole group beside it through the existing `insert_subtrees_tx` (Duplicate's
      transaction) and joins the two shapes with one arrow, in one undo step. The arrow
      itself is the plain quick connect's, now shared (`quick_arrow`). The label is copied
      unchanged and stays ordinary editable text.
    - **A scaled label is laid out again**, `src/layout.rs` `refit_label`, run by
      `resize_children` for a label group (so for a handle resize, `update_styles` and auto
      layout alike). Before, a half-size group left its text box scaled and wrapped through
      its words: 72.6 px of text in a 12.1 px box. Now its characters, font and style are
      never scaled; a label that grew to fit its text keeps one line while that fits the
      shape and otherwise wraps at spaces within its width; a fixed-width one keeps its width,
      narrowed to the shape's. The box is the laid-out lines, kept centred where it was and
      moved only as far as needed to stay on the shape. A label taller than its shape even so
      is centred on it and overflows: nothing is clipped. Other groups scale as before.
    - **Arrow bindings read back**, `src/mcp/exec.rs`: `get_node_info` on an arrow reports
      `startNodeId`/`endNodeId` (as `create_shape` takes them; null when free or the target
      is gone), `startAnchor`/`endAnchor` (`x`, `y` of the target's box, `inside`), `route` and
      its routed world `points`; `get_tree_summary` adds "from [id] to [id]" (or "free").
      Read-only: same ids, page reads and acknowledgement rules; other nodes read as before.
    - Reason: the plan's bounded repairs of measured gaps; same Editor, commands, document
      format, Session, save, binding and lease, no new field. Upstreamable as fixes.
  - Revision 2 slice 7, attached arrow labels (plan section 7, R15; owner-admitted). Native
    windows change too.
    - **The field.** `src/model/node.rs`: `NodeKind::Connector::label: Option<String>`,
      omitted when absent. Same node, routing, anchors and history; no separate Text.
      `NodeKind::edit_text`/`edit_text_mut` let the text editor work on a label while
      `NodeKind::text` (typography, search, export names) still means text layers and
      sticky notes only; `NodeKind::label` reads it.
    - **Page version guard.** `src/project/format.rs`: `PAGE_VERSION_LABELS` (4) and
      `page_version_for`. A page is written as v4 only when an arrow has a label, so an
      unlabelled page keeps its v3 bytes. `check_labels` refuses a label in a page that
      says an older version, on a node that isn't an arrow, or that isn't text; v3 → v4 is an
      identity step. The retained old603 reader drops an unknown v3 label on save and refuses
      v4 before editing: a labelled page deliberately doesn't open there (a downgrade limit,
      not compatibility). `src/io/file.rs`: the version-1 `.paper-rs` writer refuses a
      labelled document (nothing written).
    - **Placement and drawing.** `src/connector.rs` `midpoint` (arc length along
      `line_path`, curves and elbow corners included). `src/text.rs` `LABEL_SIZE`, and
      `layout_node`/`text_offset` lay a label out centred on that point (so text editing,
      the caret and hit-testing reuse the text code). `src/render/draw.rs` `label_rect`,
      `label_ops` (an authored-white backing, which the dark canvas display adapts, and the
      text in the arrow's stroke colour, rough arrows too) and `draw_bounds` including it, so
      the GPU scene, SVG and PDF draw it. `src/io/svg.rs`: its `aria-label`.
    - **Editing.** `src/editor/mod.rs`: a double-click on the selected arrow (before its
      handles) or Enter edits its label through `start_text_edit`. An empty label is
      created to type into, and a blank one is removed at `end_text_edit`, in the same undo
      step. `src/editor/text_edit.rs`: typing and deleting use `edit_text_mut`; labels take
      no character styles. `src/editor/whiteboard.rs`: no bend handle under a label.
      `src/editor/hit.rs`: a press on a label hits its arrow.
    - **Corrections (R37-F1 and the selected-arrow double-click).**
      - `src/layout.rs` `fits_its_text`: the width policy is explicit, never measured. A
        label that grew to fit its text and is wrapped by `refit_label` gets the existing
        width sizing `Sizing::Hug` (its box hugs its lines, as the inspector's Hug and MCP's
        `fit-content` mean for text). It counts as growing to fit its text (`auto_width` or
        Hug), so shrinking and growing a label group back, even across save and reopen,
        returns it to one line.
      - Every explicit width choice sets `Sizing::Fixed`: MCP's numeric `width`, resizing the
        text itself, and the inspector's W field (R38 drove the actual egui field: 100 is kept
        after a wrap). The inspector's Fixed/Hug combo isn't shown for a label in a group
        (`child_layout` shows it only for layout containers and auto-layout children). The
        tests that write Fixed or Hug through `inspector::apply` exercise the field's meaning,
        not that combo. Auto width off with the default Fixed sizing is a fixed width, kept
        whatever it measures.
      - Data: a wrapped label's `layout_child.sizing[0]` is `"hug"` on disk (an existing
        field, page v3). It is ignored for a text in a group outside auto layout, and
        `get_computed_styles` reports that width as `fit-content`.
      - `src/editor/{mod,whiteboard}.rs`: a press on an arrow's bend handle bends it only once
        it moves past the drag threshold (`Gesture::ConnectorMid::press`, `bend_connector`),
        as a move does. A click there, such as the first half of a double-click that edits
        the label, changes nothing. A real drag bends exactly as before.
    - **MCP.** `src/mcp/exec.rs`/`tools.rs`: `create_shape`'s `text` labels an arrow,
      `set_text_content` sets one exactly (empty removes it), `get_node_info` reports
      `label`, and `get_tree_summary` shows it. Same reads, writes, acknowledgements and
      gates.
    - **Library file.** `crates/openpaperdesign-serve/src/library_payload.rs` admits page
      v3, or v4 for a labelled page. The write-back comparison still refuses an unlabelled
      v4 page and anything else upstream wouldn't keep.
    - **R38 corrections.**
      - **Transfer refusal, F1.** The clipboard's `paper-rs/clipboard` and the native
        `paper-rs-library` 1 have no version that an older reader refuses: the R37 source
        pasted, loaded and saved them without labels, and an older Library replaces a file
        it can't read with its starter items.
        - `src/editor/mod.rs`: Copy and Cut of layers holding a label anywhere in them are
          refused before anything changes, with `io::clipboard::LABELS_REFUSED` in the
          editor status. Both clipboards and the document are untouched, with no undo step.
        - While a label is edited, Copy and Cut take its selected characters as plain text.
        - Duplicate (internal) is unchanged.
        - `src/library.rs`: `LibItem::has_labels`; `Library::save` refuses any labelled item
          before writing (`LABELS_REFUSED`), and `src/ui/library.rs` Add refuses before the
          library changes.
        - `src/io/file.rs`: `to_string_checked` (`save` uses it). `to_string` stays
          unchecked and documented.
        - This is a refusal, not label transfer support.
      - **Short arrows, F2.** `src/connector.rs` `label_centre` is the one placement
        (through `text::text_offset`, so drawing, bounds, hit testing, the caret and
        SVG/PDF follow). The label is centred on the middle unless its box would cover the
        line's ends or a head; then it moves beside the middle, across the line (up, or left
        of a vertical line), just past every end and head point. Nothing is drawn over the
        glyphs.
        - `src/editor/whiteboard.rs`: the bend handle is hidden only where a label covers it.
      - **Raw page version, F3.** `src/project/format.rs` `migrate` checks the raw version
        (a whole number, 1 to current; missing = 1) before narrowing it. 2^32 + 3 is refused
        rather than read as 3.
    - Not changed: Excalidraw import (its arrow-bound text stays a separate text layer) and
      the PNG/SVG/PDF formats themselves. Upstreamable as a feature with its format guard.
- Upstream changes in the browser host (not upstream-owned, for reference):
  - `WebState::pointer_down` keeps a press on a panel's resize handle from the editor
    (`Chrome::on_panel_edge`), as `App` does.
  - `WebState` reports `HostRequest::Background` and applies a kept one
    (`set_background`).
  - It saves a finished title rename in the canvas save (`saved_name`, the in-flight
    `name`). A draft is guarded as the renames are, and a reload closes it.
- Upstream sync: `git fetch upstream`, merge the upstream default branch into a branch cut
  from `main`, run the checks below and land it with a PR into `main`. Do not rewrite upstream
  history.

## Working rules

- Stage explicit paths only. Never use `git add -A` or `git add .`, because sandboxed sessions
  can show untracked placeholder dotfiles (`.bashrc`, `.gitconfig`, `.claude/`, ...) at the
  repo root that must not be committed.
- Every GitHub write (issues, PRs, comments) targets `arcitai/openpaperdesign`
  explicitly (`gh ... --repo arcitai/openpaperdesign`), never the upstream repo.
- The tests regenerate the tracked screenshots in `docs/screenshots/`. Restore them
  (`git checkout -- docs/screenshots`) unless a change is meant to update them.
- Commits use Conventional Commits and keep the actual contributors' co-author trailers.

## Toolchain and checks

Rust 1.95.0. A task-local toolchain may be staged in `.toolchain/` (gitignored):

```sh
export RUSTUP_HOME=$PWD/.toolchain/rustup CARGO_HOME=$PWD/.toolchain/cargo PATH=$PWD/.toolchain/cargo/bin:$PATH
```

```sh
cargo build --locked --all-targets
cargo test --locked -- --test-threads=1        # parallel run: see the known SIGSEGV below
PAPER_RS_REQUIRE_GPU=1 cargo test --locked --test headless
cargo check --locked -p openpaperdesign-core -p openpaperdesign-web --target wasm32-unknown-unknown
cargo test --locked -p openpaperdesign-serve -p openpaperdesign-web
node --test crates/openpaperdesign-web/tests/session.test.mjs   # page lifecycle; Node >= 22
node --test crates/openpaperdesign-web/tests/panel.test.mjs     # in-panel editor module; needs build-web output
node --test plugin/tests/plugin.test.mjs                        # plugin manifests and skills
node --test crates/openpaperdesign-serve/tests/home.test.mjs    # sidebar home view protocol
(cd plugin/tests/design && python3 -m unittest)                 # ported design-method tests
./scripts/build-web.sh                         # browser bundle; needs wasm-bindgen CLI 0.2.129
cargo clippy --locked --all-targets
cargo fmt --all --check
```

Add `--offline` when the cargo cache is pre-fetched and the network is off. Known baseline:
the default parallel `cargo test` can SIGSEGV in `tests/mcp.rs` during Vulkan instance setup
when two screenshot tests initialize a GPU at the same time. Serial runs pass. See
`docs/openpaperdesign/feasibility-2026-10-12.md`. When `PAPER_RS_REQUIRE_GPU` is unset, the
GPU tests skip themselves if there is no adapter. The MCP screenshot tests return early without
that guard, so a pass is not proof that rendering happened.

## Browser editing (issue #1)

`openpaperdesign-serve --project <dir>` serves one project to a browser on loopback
(`docs/openpaperdesign/browser-edit-b1.md`). Its revision checks protect only writers that go
through it. Never describe it as protecting a concurrently running native app or other
external writer.

Agents edit the same project over MCP at `/mcp` on that server, with the per-launch token as
`Authorization: Bearer <token>`. Only the bound project is reachable: export and project
creation are refused, and images must be `data:` URIs
(`docs/openpaperdesign/agent-edit-b2a.md`). Never write the token into repository files or commit it.
Conflicts are strict, and nothing merges. A browser save on a stale revision is refused (409).
An MCP session must read a page before it changes it (no first-call exception). Reads are
per page: only a successful call that returned that page counts. Failed calls, and calls
not attributable to one page, acknowledge nothing. The page list (names, order) is read only
via `get_basic_info`. A change is refused as a `conflict` unless every page it writes (and
the page list, if it changes it) is read at its current revision. The browser polls only
while its editor is clean. Page ids come only from the manifest, must be safe file names,
must differ in more than ASCII case (macOS file systems alias them), and must not share
their first six characters, ignoring case (node ids are `<prefix>:<n>`). A call that returns
an error changes nothing, in memory (the editor state, undo history and selection included)
or on disk. Each save (agent call or browser) is one commit that is rolled back on any error,
directory syncs included. If the rollback fails, the answer names the files that may be
inconsistent, and the server refuses every further write. That state is in memory only and
clears on restart, so check the named files first. Commits are not crash-atomic: a crash
mid-commit can leave a mix of old and new files (each one complete). A call that couldn't start within the timeout is never run; a
started call is always waited for; reads count only once their result was delivered. Inline SVG may not
contain `<image>`/`<feImage>`.
The dropped three-way-merge experiment is at the local tag `experiment/b2a-three-way-merge`;
don't bring it back without an explicit decision (collaboration is issue #2).

## Codex plugin

`plugin/openpaperdesign/` is the local Codex plugin (`docs/openpaperdesign/plugin-b2b.md`). Its
MCP server, `openpaperdesign-mcp`, must be on PATH. It embeds the web bundle at build time, so
run `./scripts/build-web.sh` before `cargo install`. `plugin/openpaperdesign/plugin.json` is the
only manifest. Codex discovers `skills/` and `mcp.json` itself, so the manifest has no top-level
`skills`, `mcpServers`, `apps` or `interface`. Codex presentation goes in `extensions.com.openai`
(`interface.shortDescription` ≤ 30 characters). Don't add a `.codex-plugin/` overlay.
`mcp.json` is explicit `type: "stdio"`, args `["--app-library", "${PLUGIN_DATA}"]` (the
app library, below), with no `cwd` or `env`. Workspace modes (`--codex-context`, `--workspace`)
are separately started invocations that keep every guard below; the three flags are mutually
exclusive. In `--codex-context` mode the launcher pins canonical workspace/identity from Codex 0.162.1's experimental
`codex/sandbox-state-meta` in `RequestContext.meta` on real model tool calls. Direct
app-server calls omit it; arguments are never authority. Accept only the observed managed
workspace-write shape or exact `{"type":"disabled"}` profile; never infer missing grants.
Managed network must be `restricted` or `enabled`; neither changes adapter networking.
Root `.git` may be an opaque regular file for a Git worktree: never read or follow its
pointer. Protected project paths and symlinked metadata remain refused.
Read-only/unsupported profiles cannot bind or open a writable browser. Plugin scope and
protected-path checks apply even when the host sandbox is disabled. Missing, changed or
no-longer-writable context after pinning permanently retires the binding through the same
writer fence; caller cancellation cannot interrupt retirement. Checks are per call, not
instantaneous detection of idle host permission changes, and provide no OS sandboxing.
Standalone `--workspace`/cwd mode is separate; reject conflicting flags. The launcher
refuses a moved or replaced workspace. It creates
`design/<name>/` itself without following links. A failed `bind_project` must leave the
current binding working. Replacement preparation must not change the active host's layout
registry. On a successful switch, retire the old server before activating the replacement;
HTTP sessions, stdio clones and browser saves share that fence. Same-project binding is a
no-op. Never claim browser edits are saved on a switch or exit: once
`open_editor` was called, switching needs `force: true`. The summary bind returns runs in a
one-off session and is not the agent's read: instructions must have the agent read the
target page (`get_tree_summary` root with its `pageId`) before its first change. The ported
audit rejects duplicate evidence (JSON keys, repeated records). Keep the ported `design` and `review-design` skills' provenance notes and
LICENSE files, and the plugin's `NOTICE.md`, intact.

Sidebar home (issue #6, `docs/openpaperdesign/plugin-b2b.md`): four things are answered
before workspace admission:
- `openpaperdesign_home`, which refuses arguments.
- The home view resource (`HOME_URI`, currently `ui://openpaperdesign/home-9.html`).
- The read-only native inventory `openpaperdesign_projects` (only `projectId`).
- `openpaperdesign_choose_folder` (`{}` only): the host directory picker
  (`openai/elicitation/create`, `directory` resource input) for a client that advertises
  `extensions["openai/elicitation"].form` on a protocol before 2026-07-28. Its answer is input
  only: never opened, listed, looked up or treated as a workspace or grant.

They read or change none of the session's context, binding, project, listener or files. This
named exemption covers only those four (plus the fixed refusal of the app-library tool names,
which reads and changes nothing); never route another tool around admission. The home resource
URI is versioned: hosts may cache a view by its URI, so any incompatible change to the view's
code, presentation or the responses it expects gets a new URI in `HOME_URI` (every tool and
resource registration uses it) and the URI tests. No earlier URI is served or aliased. History:
`home.html`; `home-2` added the in-panel editor; `home-3` (2026-10-17) is the browser-first
home, where New/Open/Import/Open current canvas open the web editor link and the panel is
optional; `home-4` (2026-10-17) hosts upstream's workspace UI in the editor (no tool buttons of
the view's own); `home-5` (2026-10-17) is the whole-canvas pages protocol: its renderer starts on
the whole canvas and its in-panel editor reads and saves through `_app_canvas*`; `home-6`
(2026-10-18) is the one app: a file library whose New/Open mount the editor in the view, with
the file name and the way back in upstream's own top bar, and which calls `_app_rename` and
exports by `id`; `home-7` (2026-10-18 source candidate) is the quieter workspace of DESIGN.md
(one calm surface, grouped top bar, docked panels on medium surfaces) whose in-panel editor
follows the view's light or dark appearance; `home-8` (2026-10-18 source candidate) is one
System/Light/Dark appearance across the view, the editor's chrome and its canvas display (the
renderer's `start` takes the OS appearance and the kept preference; the view keeps a menu
choice with `openpaperdesign_app_appearance`); `home-9` (2026-10-19 source candidate, revision
2 slices 4–6) is the quiet Home:
- Grid first, equal cards, no footer or wordmark, About & notices in the menu, and the
  storage cue;
- the home layout and the editor's canvas background kept with `openpaperdesign_app_view`
  and reported by the list (`view`);
- the editor's file rename, sent in its canvas save.

A view a host already cached under an earlier key is not evicted or refreshed by this.
The plugin's icon assets are static SVGs in `plugin/openpaperdesign/assets/` (no script,
`foreignObject`, style, text or external reference; at least 48×48 intrinsic). The launcher
embeds `icon.svg`/`icon-dark.svg` at build time, so rebuild after changing them.
`openpaperdesign_canvases` (read-only, bounded list of `design/` projects) and `open_editor`'s
workspace/project/canvas identity are ordinary admitted calls, not exemptions. `open_editor`
reads the bound manifest only after the bound root's identity and entries pass again
(`Server::bound_manifest`). Otherwise it refuses and marks nothing as opened. The home is
both a global and a thread entry.

The inventory starts `codex app-server` from PATH for one bounded exchange. It sends only
`initialize` (`experimentalApi`), `initialized`, and `project/list` or `project/read`, with
schema checks, a deadline and kill-on-drop. An MCP cancellation (`RequestContext.ct`) kills
and waits for its child before the inventory is free again. Never add thread, turn, settings, auth or project
writes to it. It keeps Codex's namespace and ids, and every root. A listed project or root is
discovery, never authority: never join it with other ids by name, and never pick a root for
the person.

The view's basic actions call the server directly (owner correction, 2026-10-14): never
`ui/message`, a paste prompt or a model turn. The app-only `openpaperdesign_ui_canvases`,
`_ui_create`, `_ui_open` and `_ui_editor` are admitted only from that call's own
`codex/sandbox-state-meta` (the same `Workspace::from_meta` as model calls), and a named folder
must equal the admitted workspace exactly. A refused direct action never pins; an unpinned
session is pinned only by an accepted one (a listing under its lock, or a create/open inside the
bind handoff that installs the binding). Each pin and binding records whose authority admitted
it (`Owner::Model` or `Owner::Direct`). A view call carrying the host's context key whose
context is invalid, downgraded or names another workspace is an observed revocation: when the
authority in force is the view's own, it retires the session through the same
`retire_session` fence as a model call (admission closed; started writes finish, queued and
late HTTP, stdio and browser writers refused). A view call without that key observes nothing,
and no view call retires what a model call admitted. Revocation is seen only on calls, for
both origins. They reuse the model path's create/bind/editor code and its one bound project;
keep a single binding per process (no pool or second UI binding). Project creation (both
origins) acquires the binding's listener before writing anything; a later failure before the
binding is installed removes, under the same lock, only what is still provably that
creation's: files it finished writing with the same identity and bytes, and folders it created
with the same identity at their canonical path, once empty. Anything replaced, changed, moved,
unknown or unobservable (only NotFound counts as gone) stays and is named (never existing `design/`, direction files, tokens or assets).
The check-then-remove path race, crashes and other writers remain unprotected. Observed so far: app-view calls
carry no workspace context, so these actions are refused on that host; that is the honest
result, not a reason to borrow model context, use a picked folder or fall back to messages. The
view stores nothing (no URL, token or path) and opens only the exact loopback editor URL. Host theme and style variables (MCP Apps
`HostContext.theme`/`styles.variables`) change presentation only: apply a fixed list of
standard names with plain values, never `styles.css.fonts`, and never treat them as a
workspace or permission. A listed or picked folder travels only as the action's exact `root`.
Several folders are asked, never picked. Results and busy state belong to the view that started
them: a newer result, cancellation or Back drops late replies and old editor links. Request
buttons belong to the list that rendered them; the New form sends only the destinations it
showed (copied when it opens). Result paths and the port stay under Details, cleared per
result. The wordmark font
(Geist Pixel Square, OFL, `crates/openpaperdesign-serve/assets/`) is embedded unmodified at
serve time; never load a font by URL.

App library (issue #6, owner admission 2026-10-16, `crates/openpaperdesign-serve/src/library*.rs`,
`launcher_app.rs`): `--app-library <dir>` is a separate process mode for canvases the app owns.
Codex's Agent Plugin loader expands the literal `${PLUGIN_DATA}` argument (no shell); the root
must be an existing real folder given as a plain absolute path, never `/`, `$HOME`, inside
`$PLUGIN_ROOT`, a `.plugin-data` fallback or `plugins/cache` (not durable), and is pinned by
identity. Library validation is its own code: never build a `Workspace` for it, never loosen
workspace protected-path rules, never use cwd, a picked or listed folder, model metadata or a
tool argument as a path. Layout `<root>/library/v1/canvases/<id>/` (a standard paper-rs project,
the only live copy) and `locks/<id>.lock`; ids are 16 hex digits made by the server; names are
data. Listing creates and locks nothing and is bounded (entries and manifest bytes). In that
process only the home, its resource, `openpaperdesign_status` (library report), the read-only
inventory and the app-only `openpaperdesign_app_{canvases,create,open,editor,close,import,export,rename,appearance,view}`
(the last two keep view preferences in `library/v1/appearance` and `library/v1/view`: no
canvas, lease or binding)
and in-panel editor tools (`_app_editor_asset`, `_app_canvas`, `_app_canvas_revision`,
`_app_canvas_save`, the per-page `_app_page`, `_app_page_revision`, `_app_page_save`,
`_app_ready`, `_app_page_release`), plus the agent's library controls and
binding-guarded upstream design tools (below),
are answered, with strict argument keys and types; every project route (including the
`_ui_*` actions, the picker and the upstream design tools) is refused without admission or
state change, and a workspace process refuses the `_app_*` and `_library_*` names the same way. One binding per
process (the page-id layout registry forbids a second slot). Each canvas needs a permanent
per-id lease: nonblocking `File::try_lock` on a regular single-link lock file, taken before
`Project::open` (its stale-temp cleanup) or any write, rechecked (handle and path identity, link
count, library folders) by a guard on every browser and MCP access; a failed check fences that
canvas for good, and the next action retires it. Lock files are never removed, truncated or
replaced, and never exported. The lease moves into the binding and is dropped only after
retirement drained it, also at the normal end of the service: when stdio closes or the service
fails, `main` runs `Launcher::shutdown`, which refuses any later create/bind/install and retires
the final binding (started writer and rollback drained, queued and late writers refused) in an
owned task spawned before any wait, even for the state lock, before its lease drops (both
modes); same id is a checked no-op; busy, invalid or failing targets keep the
current canvas and URL. It is advisory: no protection from other programs, no OS sandbox.
Create/import write through the checked commit with every new folder's parent synced, finish
preparation before the old binding retires, run in an owned task, remove only what is still
theirs (naming what stays or may not be durably removed) and are never retried; a lost reply is
recovered from the list. The canvas file (`library_payload.rs`, version 1, canvas-only) is
bounded and fully validated before a fresh identity is published (duplicate keys, unknown
fields, contained manifest/page exactly `paper-rs-project` 1 / `paper-rs-page` 3 (4 only for a
page with arrow labels) checked raw
before any upstream parsing or migration, each page's JSON matching what upstream writes back
from it so no nested field is silently dropped (only the writer's known omissions at their
defaults may be explicit: page `camera: null`; node `parent: null`, `children: []`,
`layout: null`, `instance: null`, `component: false`), ids, case/prefix collisions, page
correspondence, node ids at most 2^53 - 1 and
`next_id` at most 2^53 checked before upstream parsing, one rooted forest walked iteratively
from the top level with matching parents and at most 256 levels, exact image set); images
come only from the file or the canvas's own `assets/`, never the process image registry. Export
reads one saved snapshot under the lease and the writer fence and says it holds saved content
without direction or tokens; it names the shown `binding` or a listed `id` (never both): the
open canvas by id goes through its binding's fence, any other under its own lease taken for the
read (busy is refused). Rename (`_app_rename`, `id` and a 1–60 character name) changes the
manifest's name and modified time only: the open canvas through its binding's
`start_write`/`commit_fenced` (rollback, `Broken`), any other under its own lease with one
`commit_fenced` of `project.json` through that canvas's own `Broken` in the `Library`: if its
undo fails too, the canvas is fenced in this process (every later rename, open and export of it
is refused with the files named, and its lease is kept so other processes find it busy) until
the files are checked and OpenPaperDesign restarts; other canvases are unaffected. A lease that
no longer checks out after a committed rename is reported as written but unconfirmed, never as
nothing changed. Busy, retired or invalid targets change nothing. Names stay data, never paths. Companions (`DESIGN.md`, tokens) are unsupported in version 1 and
refused; project-bound export is not offered. A new blank canvas infers no design system. The
editor reports readiness itself (`POST /api/ready`); an open-link acknowledgement is never a
load. Close retires and saves nothing in browser memory; it needs `force` once the link was
given. Views share the one binding: each binding has an identity (`binding`, `<id>.<n>`) in
editor results; close, export and readiness polls send it, and a confirmed switch sends the
`replace` binding its refusal named; all are compared under the state lock before any effect,
and a stale one is refused with the current canvas, files, URL and lease untouched. It is a
stale-view check, never a credential or authority. An import's file read belongs to the view
generation and selection that started it; a superseded read sends nothing and shows nothing.
Every image of an imported or exported canvas file is fully decoded with the `image` crate
(declared in `openpaperdesign-serve` with the root's exact requirement), never through
`assets::register`, before anything is published; hash, signature, base64 and exact-set checks
stay. The browser canvas still doesn't draw image fills.

Agents on library canvases (owner direction 2026-10-17, `launcher_library_agent.rs`): the
model sees `openpaperdesign_library_{canvases,current,create,open,close,editor}` and
upstream's offered design tools under their own names, descriptions and schemas, each with a
required `binding` (the open canvas meant). The binding is compared under the state lock after
the canvas/library/lease re-check, removed, and the call goes to `Server::call_tool` with this
process's session, exactly as in workspace mode: same policy, per-page read-before-write
acknowledgements, one commit with rollback/`Broken`, lifecycle fence for queued and late calls.
A caller-supplied session field is refused. A stale or missing binding is refused with no
effect. The model library controls take no `force`/`replace`: a canvas with a mounted panel or a
given link is never switched or closed by the agent. Only `_library_editor` hands out the
loopback link (a link given); no other agent result carries the URL or token. Cross-process
ownership stays the lease's (busy is refused, never transferred). The panel and browser editor
see agent commits through their revision polls; their stale saves are 409 conflicts. Never add
a parallel editing API, drawing DSL or document format here: reuse upstream's commands and
renderer, and keep the adapter to binding, authority, transport and hosting.

In-panel editor (owner direction 2026-10-16, `static/{editor,panel}.js`, `launcher_app.rs`):
the library home can edit a canvas in its own MCP App view with the same Rust/WASM renderer,
`Session` and editor mount as the browser page (`editor.js`, import-free, shared by `app.js`).
The launcher inlines one fixed module into the view's one `<script type="module">` (glue,
`session.js`, `page.js`, `editor.js`, `panel.js`, in that order) and refuses to embed text with
`</script`, `<script`, `<!--` or an import; no eval, URL import, outbound domain, CSP, CORS or
framing change. The renderer comes only from the packaged wasm, in sequential 256 KiB parts
(integer `part`, identity/size/count/length checked, 16 MiB cap) and goes to the glue's own
`__wbg_init({module_or_path: bytes})`. The view loads and saves the whole canvas
(`_app_canvas*`, as the browser's `/api/canvas`, below); its calls name the view's `binding`,
then the `mount` of the first read. The per-page tools stay for older views (`page_id` only
checked as a manifest page; a mount isn't tied to a page). Binding and mount
are compared under the state lock, which is released before the writer fence (never
reversed). They use the same `Server::{canvas, canvas_revision, save_canvas_while, page,
page_revision, save_page_while, mark_ready}` as HTTP: one transaction, structured
409/rollback/`Broken`, bounded queue. A save carries its
mount's lifetime flag, checked again under the writer fence; a release clears it under the
state lock, then waits for the fence before it is acknowledged, so a queued save of that mount
never runs and a started one finishes first (the binding stays). A cancelled save never starts:
cancellation is observed before and during admission and before the fence is taken. A started
save always finishes, and retirement waits. Panel messages are capped at 4 MiB as sent: a read as the whole tool result in
its JSON-RPC response, a save as its whole `tools/call` request, each escaping counted, plus a
fixed 16 KiB host allowance (not a known host limit), separately from the 64 MiB page limit;
larger pages stay browser-only, untouched, and no mount is registered for them. `panel: true` on create/open/import/
editor returns no link and gives none; mounted panels and a given link both make Close/Switch
need `force`. Leaving (Home) drains (wait for the save in flight, save what is left, require
`can_reload`); conflict, failed/unanswered save or an edit under way keep the editor, resumed,
with the reason and an explicit Leave without saving. Teardown uses the same drain and closes
nothing, but an abrupt host removal or crash saves nothing. Ready is the mount's own renderer's;
Saved only a matching commit answer. The server refuses `panel` requests its build can't serve
before any lock, file, lease, switch or mount. Host CSP for WebAssembly, WebGL2/WebGPU, message
limits, input and focus in the native panel are unproven.

One OPD app (owner admission 2026-10-18, superseding the 2026-10-17 web-editor-first home):
the library view is one application in whatever surface Codex grants it. Home is a calm file
library: the app-owned context said plainly as the storage cue ("New files are kept in the
OpenPaperDesign library, this app's own data folder, not in a Codex project"; app-library
files have no native-project grouping or association, so none is shown, implied or granted),
New file (created at once as "Untitled"), search by name (Escape clears), a grid of equal
cards by default or a list (the library keeps the choice, `openpaperdesign_app_view`), no
footer (the licences and notices are the More menu's About & notices), and named files with
pages and last edit (relative, else the date), each with a restrained menu: Open, Rename… (a form;
`_app_rename`) and Export… (`_app_export {id}`), keyboard-operable (arrows, Home/End, Escape
back to its button). There is no "Open in new chat". Workspace mode keeps its genuine project
selection unchanged. With the module, New file, Open, Import and Open current file send
`panel: true` and mount the editor directly in the view: no result screen, no "Edit in this
view", no `ui/open-link`, and nothing on this path marks a browser link as given. The editor
fills the view edge to edge (no heading, frame, fixed-height stage, Close/Export/Full
screen/browser buttons, Details or footer); upstream's own top bar shows the file name
(`canvas.name`, separate from the page name) and its "All projects" is the way home. Every
status goes to a status live region for assistive technology and diagnostics; how it shows is
`opdStatusKind` (editor.js, shared with the browser page): stable healthy states (Ready, Saved,
Saving…) show nothing, loading on the way to Ready shows as a small temporary note that the
next state replaces, and failures, conflicts, refusals and notes show as one note over the
workspace. A save failure ("Save failed; retrying", unreachable), a conflict or a terminal
error stays shown, with its severity, through the retry's "Saving…", Ready, informational
notes (a refused copy, say; they reach the live region only) and every timer until it is
resolved (an acknowledged Saved or the conflict's reload for a failure or conflict; only a new
editor for a terminal error, which nothing recoverable replaces): `opdStatusDisplay` per editor
in the view, `opdStatusNote` on the page (R25-F2, R26-F2). That Saved or reload also ends a
refused leave's hold and its Leave without saving (R26-F3). A departure (Home, the explicit
discard, teardown) captures its view generation, editor handle and binding once and, after
every await (drain, the mount's release, Close), acts only while it still owns the view: a
newer result, Home or editor is never closed, hidden or let go by it (R25-F1). Departure
admission is per editor: only the editor being drained refuses a second departure, and that
mark goes with its departure or its disposal (R26-F1). Home drains, disposes (releasing the mount), then closes exactly that
binding, not forced; a refused, failed or unanswered close is said on the library and never
shown as closed, and a close answer after a newer result belongs to nothing. A start failure
offers a visible Back to files. Without the module, the library falls back to the old link
hand-off (the result's own same-binding loopback link, opened once, which counts as given), and
the agent's `_library_editor` still hands out the link. Whether a result is a library one comes
from the view's own mode and the action that asked, never from its `storage` label alone; it is
used only if its canvas id and binding are valid and match the canvas asked for, in the
presentation asked for (link or panel). The home never infers a browser load. The browser page
(`index.html`/`app.js`) is the same app's editor alone: the workspace fills the page, with the
same status rule and an About for the notices; a fixed-project standalone page has no library
Home (upstream's Home shows the "Open, create or import canvases from the OpenPaperDesign home
in Codex" note). The home owns routing and lifecycle; the web editor (WASM, upstream engine)
owns editing; the server owns the document, conflicts and authority. Don't add a second library
home or editor. Native qualification is pending.

The web editor's workspace (owner decision 2026-10-17, `crates/openpaperdesign-web/src/state.rs`):
the one web editor (browser page and optional panel alike) hosts upstream's own workspace UI,
compiled unchanged via `#[path]` (`ui::draw` with layers, inspector, pages, floating tools,
palette, menus and help; `render::frame::{layout, Painter}` on the editor's wgpu device with
upstream's `egui-wgpu`). Don't rebuild that UI in DOM, fork it or add eframe/another runtime.
DOM input goes to egui first; pointer input reaches `Editor` only over the returned canvas
rectangle and not over a floating egui area or an egui drag (as native `App::over_canvas`);
keys reach `Editor` only while no egui text field has the keyboard. egui's own keyboard zoom
is off in this host (Ctrl/⌘ +/- is the canvas zoom only), so egui paints at the page's device
pixel ratio and DOM input stays in its CSS px. Unfinished UI input is unsaved work: a changed
layer-rename draft, any other egui text field while it has the keyboard (an inspector number
field holding `-`, say; told by egui's public `TextEditState` for the focused widget, and
conservatively busy until upstream finishes or cancels it), and
typing queued for a field. It blocks a clean reload or leave (`can_reload`) and shows in the
page's leave guard (`unsaved_work`); a reload closes the rename field and drops field focus.
Only the command palette's own search field is exempt, and only while it has the keyboard
(told by the id the palette reports, `PaletteUi::search_id`): the palette stays open when a
document field is clicked. An open rename's own field is judged by its draft, told by the ids
upstream reports (`UiState::rename_field`, `PagesUi::rename_field`); a stored rename never
exempts another focused field.
Every `AppRequest` is
handled explicitly in `WebState::handle_requests`: Save is the page's checked save at once;
Home/New/Open go to the host's own navigation (the view's drained Home, which then closes its
binding, or a note in a browser tab); upstream's top-bar title (`UiState::project_name`) is the
canvas envelope's `canvas.name`, updated on reload; SVG/PDF export is upstream's in-memory export offered as a download; PNG
export/copy (GPU read-back), Duplicate project and placing images from files are refused
with a visible reason. Copy/cut/paste use the page's clipboard events through upstream's
`clipboard::Provider`; no OS clipboard. The
Library tab is session-only (never the native library file). IME composition, copying out of
egui text fields outside a copy event, DOM accessibility of the egui UI and image fills stay
unsupported. Bundle changes need a new exact production qualification.

Pages in the web editor (owner direction 2026-10-17, `state.rs`, `Server::{canvas,
save_canvas_within}`): the editor holds the whole canvas through upstream `Editor::open_pages`
(every page with its own history, selection and camera), and upstream's pages panel and
inspector add, rename, duplicate, reorder, delete (never the last page) and switch pages with
upstream's own operations. The transport is a bounded envelope around upstream's page files,
never a new format: `GET /api/canvas` (and the panel's `_app_canvas`) returns every listed page,
the active page and one canvas revision (a hash of the manifest's exact bytes and every listed
page's revision), read under the writer fence; `PUT /api/canvas` (`_app_canvas_save`) sends
`{active_page, pages: [{id, name, text?}]}` with that revision in `If-Match`. It is checked
completely before the fence (shape, at most 1000 pages, ids by the shared `check_page_ids`,
active page listed, each page file parsed and validated by upstream and naming its own id),
then against the canvas under the fence: any change since (any page, the list or the active
page) is a 409 with nothing written (conservative, no merge); a page new to the list must
carry its content (no listed id reaches an unlisted file), and a changed name must be a page
name. An optional `name` (revision 2 slice 6) renames the file in that same commit. It
follows the app's Rename rules (1–60 characters after trimming, no control characters); an
editor that doesn't send it keeps the name. One `commit_fenced` writes the sent pages and the
manifest (server-owned identity, format, version and creation time; the sent list, active
page and, when sent, name); rollback and `Broken` as for every save. A rename made elsewhere
(the library's Rename) changes the canvas revision, so a dirty editor's save is a 409, as for
any other change. A page left out of the list keeps its file, unreferenced and unreadable by id,
as the agent's saves leave it; nothing is erased. The per-page API stays as it was. Dirty is
upstream's own bookkeeping (`dirty_pages`, `meta_dirty`) plus the document stamp; a save
records each sent page's history generation (`Editor::page_gen`) and the list and active page
it sent, and its answer marks exactly those saved, never a newer edit, undo or page action. A
refused save keeps everything. A reload (only when `can_reload`, or the explicit discard)
replaces the whole canvas, shows the server's active page, keeps each remaining page's camera
and the shown page's selection, and closes rename fields (layer and page). The agent's host
reloads when the manifest changes (its revisions include the manifest's), so its default page
follows the remembered active page, and its own saves keep the manifest's active page.

## Model profile

- Implementation: `claudeAgent` `claude-opus-5-5`, effort `high`, fastMode `false`.
- AgentOps, research and independent review: `codex` `gpt-6.1-sol`, reasoningEffort `max`, serviceTier `default`.
- Owner-selected defaults and native project/AgentOps readback verified 2026-10-13.
  Native project configuration owns the
  implementation default; thread selections own AgentOps and review. Preserve the actual
  settings of an in-flight turn and apply changes at its next safe continuation.
