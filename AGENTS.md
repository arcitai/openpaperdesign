# AGENTS.md — openpaperdesign

Revision: 0.90 · Updated: 2026-10-23

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
      - **Curve re-edit** (R15; natively too; the lead's synthetic-host finding on 869: after
        a first bend no bend handle was left, and a drag on the apex was the box resize,
        ignoring x), `src/editor/whiteboard.rs` `connector_handles`: a three-point `Curved`
        arrow's bend handle (index 2) is its bend point, so the existing `ConnectorMid`
        gesture and `drag_connector_mid` move that point again (past the drag threshold, one
        undo step per drag; endpoints rerouted to their bindings). It is offered even under
        the label, since a double-click (checked before handles) and Enter still edit the
        label. Elbows (routed) still have none. (Curves with more points first had none
        either; see the multi-bend entry below.) A connector locked itself or through an
        ancestor now has no handles at all (the box resize already skipped locked layers).
        Same document, gesture and history; no format change. Upstreamable as a fix.
      - **A connector drag held through Lock** (R51-F1; natively too), `src/editor/mod.rs`
        `pointer_move`/`pointer_up` with `whiteboard.rs` `connector_gesture_stale`: Lock
        (⌘⇧L) while a bend or end drag is held commits what the drag had recorded and locks
        the arrow, but the gesture went on and the next move changed the locked arrow in a
        step of its own (a press on the new curve handle, locked before the drag threshold,
        then bent it; the same held after a first movement, on the earlier engine too). Now
        a `ConnectorMid` or `ConnectorEnd` gesture whose arrow is locked (itself or through
        an ancestor) or gone ends at the next move or release, inert: nothing more changes or
        is recorded, and what was recorded before, the Lock included, stays. No restore of a
        press-time state, no other gesture changed. Upstreamable as a fix.
      - **Multi-bend re-edit** (R15; natively too; the lead's synthetic-host finding on 27a:
        a four-point curve had no bend handles, so a drag on its first bend moved the whole
        arrow, both bends at once, and let go of its shapes), `src/editor/whiteboard.rs`
        `connector_handles`/`connector_handle_down`/`drag_connector_mid`, `src/editor/mod.rs`
        `Gesture::ConnectorMid { point, count }`, `src/render/scene.rs` (the bend glyph for
        every handle from 2 up): a curve has one bend handle per interior point (handle
        2 + k is point k + 1, so a three-point curve's is still 2), and the existing
        `ConnectorMid` gesture carries the point it was pressed on and the point count it
        began with. A drag moves that point alone; every other point keeps its world place
        (the ends rerouted to their bindings by `update_connector`), in one undo step. It
        moves nothing (and records nothing) if the arrow is no longer a curve with that many
        points, or the point isn't an interior one, so a changed arrow never gets a wrong
        point moved. The threshold, Escape, label editing, end handles, Undo and the R51
        Lock fence are as before. Not built: adding or removing bends, straight polylines'
        corners and elbows (routed). Where two handles overlap, the first listed is taken.
        Same document and format. Upstreamable as a fix.
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
        - The clipboard refusal of this correction (Copy and Cut of labelled layers) is
          replaced by the labelled clipboard payload below.
        - While a label is edited, Copy and Cut take its selected characters as plain text.
        - Duplicate (internal) is unchanged.
        - `src/library.rs`: `LibItem::has_labels`; `Library::save` refuses any labelled item
          before writing (`LABELS_REFUSED`), and `src/ui/library.rs` Add refuses before the
          library changes.
        - `src/io/file.rs`: `to_string_checked` (`save` uses it). `to_string` stays
          unchecked and documented.
        - For the Library and the single-file format this stays a refusal, not label
          transfer support.
    - **Labelled clipboard payload** (R15; natively too), `src/io/clipboard.rs`,
      `src/editor/mod.rs` `copy`/`paste_from_clipboard`: Copy and Cut of layers holding an
      arrow label write them in `LABELLED_TYPE` (`openpaperdesign/clipboard`) with `version`
      1: the same `Payload` fields and `Node` serialization, a type the unversioned
      `paper-rs/clipboard` readers (upstream paper-rs and every earlier OpenPaperDesign; both
      pinned readers' `decode` take layers only for exactly that type) don't take for layers.
      Such a reader falls through its Excalidraw check (a type of its own), and its SVG check,
      to plain text: it pastes the whole JSON as one text layer, never the arrows without
      their labels. That is the actual compatibility, not an explicit refusal in the old UI
      (lead check 2026-10-19: the literal pinned `0b1b91da` and `8da0f9f` clipboard modules,
      compiled against this library, decode a labelled payload as the exact text).
      Layers without labels are written exactly as before (no `version` key), so upstream and
      earlier readers keep pasting them. Reading a `LABELLED_TYPE` payload checks it whole
      before anything is registered or inserted: exactly version 1, only its known fields,
      readable `Payload`, and one consistent layer forest (every id once, roots present with
      their positions, children present and naming their parent, nothing left over; walked
      iteratively), and whole images (R47-F1, `check_images`): every image a layer's fill
      uses is in the payload, and every image it holds has base64 exactly as
      `assets::base64` writes it, bytes that decode as an image, and an `assets::hash` equal
      to its claimed id. All are checked before any is registered, so a payload brings all
      its images or none, in any order; an image already registered under a claimed id
      doesn't stand in for a missing or bad body. (The unversioned legacy payload keeps its
      earlier permissive image handling.) Before any of that, the raw text is read once with
      serde (`Unambiguous`, R48-F1): a member named twice in any object (names compared as
      decoded, so escaped spellings are one name), such as `assets` or an image id, is
      refused, whatever its order or values, because serde_json's `Value` keeps only the
      last and what was checked could differ from what was sent. A payload is recognized by
      its top-level `type` member, read key by key with serde (`Header`) as far as the text is JSON, so a cut-off one, one followed by
      more text, and any formatting or key order are recognized; a second top-level `type`
      or `version` is ambiguous. Text that only mentions the type stays ordinary text.
      Anything else of that type is `Content::Refused`: the editor says why and changes
      nothing (no layer, no text layer, no undo step, no image).
      An accepted payload pastes through the existing `insert_subtrees` (one undo step;
      bindings to copied shapes remapped, others let go of outside the same page, as for
      every paste), so labels, groups and geometry are kept, and says "Pasted N layer(s)" in
      the editor status as every paste does (R47-F2: an earlier refusal's note no longer
      stays after a successful paste; a pasted text says "Pasted text"). Cut is the existing Cut: the
      layers stay on the internal clipboard and Undo brings them back; the system clipboard
      write is the Provider's (not acknowledged, as for every Cut). Upstreamable as a feature
      with its format guard: upstream would have to adopt the labelled type and its label
      field together, which is upstream's decision.
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
    - **Paste into a label being edited** (R15 gap; natively too), `src/editor/mod.rs`
      `paste_from_clipboard`: while an arrow's label is edited, Paste puts the clipboard's
      text into that label through the text editor's own `insert_text` (over the selected
      characters, or at the caret), inside the label's edit and so its one undo step.
      `label_paste_text` makes line breaks `\n`, a tab a space and drops other control
      characters. The text is never decoded: JSON, SVG or anything else is not read as
      layers, foreign content or a new text layer, the internal layer clipboard isn't used,
      and nothing happens without text. Nor while the edited arrow is locked, itself or
      through an ancestor (`Document::is_effectively_locked` on the edit's own target, since
      Lock empties the selection but can leave the label's edit open; R43-F1): the clipboard
      isn't read and nothing is pasted anywhere else (now through the shared guard below).
      Before, the browser host's `paste` event (a label is not an egui field) and the native
      Paste decoded the text, so a label's Paste added
      a text layer or pasted layers beside the arrow. Plain Paste outside a label edit, and
      a focused egui field's paste, are unchanged. Same Editor, text edit, history, clipboard
      provider and its permission and failure paths. Upstreamable as a fix.
    - **A text edit left open on a locked layer** (R18/R15 safety; natively too),
      `src/editor/text_edit.rs` `Editor::text_edit_writable`: Lock (⌘⇧L) while a text layer,
      note or arrow label is edited empties the selection but leaves the edit open, and its
      typing, Enter, Backspace and Delete still changed the locked text. Every change an open
      edit makes to its own text now asks this one predicate about the edit's own target
      (it exists, has text, and `Document::is_effectively_locked` is false, ancestors
      included; never the selection): `insert_text` (typing, Enter, label Paste),
      `delete_text` (Backspace, Delete, Cut's deletion), `style_text_with` for the text being
      edited (Bold, Italic, Underline, Strikethrough, size and colour of a range or of the
      layer while it is edited), Cut and Paste in `run`/`paste_from_clipboard` (for any open
      edit on a locked layer, before anything is copied or the clipboard is read, so neither
      falls back to cutting the selected layers or pasting a new one; R43-F1's label Paste
      refusal is now this one), and `end_text_edit` (asked before the edit is taken: leaving a
      locked layer's edit neither removes an emptied text layer nor clears a blank label; an
      unlocked one is cleaned up as before). A refused change leaves the document, history,
      document stamp, selection, the edit with its caret and selected characters, and the
      clipboard as they were; keys stay the edit's. The edit stays open until it is left:
      caret moves, Copy, Escape, Undo and unlocking work as before, and unlocked it types
      again. Not covered: changes that don't go through the open edit (inspector edits of the
      selected layers, and layer commands such as Delete or Duplicate from a menu, keep their
      existing rules). Upstreamable as a fix.
    - Not changed: Excalidraw import (its arrow-bound text stays a separate text layer) and
      the PNG/SVG/PDF formats themselves. Upstreamable as a feature with its format guard.
  - Owner UX revision of 2026-10-20 (root `DESIGN.md` revision 18; owner-authorized; source
    only, not built, rendered or natively qualified here). Native windows change too.
    - `src/ui/mod.rs`:
      - **No header.** `Panel::top("toolbar")` and `top_bar` are gone. `left_panel` and
        `right_panel` run the full height, each starting with `top_row` (`BAR_H` 38, or
        `theme::TOOLBAR_H` 40 with native chrome, the traffic lights' row). The frames have no
        top margin; the sheet's "← Canvas" keeps its own. The header's hairline is gone; the
        panels' own edges are the only dividers. Reason: owner point 1.
      - **Floating top rows.** While a panel is closed, `float_left` (`nav`: All projects,
        `app_menu`, the Layers toggle; then `compact_identity`) and `float_right` (the Design
        toggle) are egui Areas over the canvas's top corners, each control where it is in the
        open panel's row. They are floating UI (`over_floating_ui`), so presses stay theirs.
        The rail and the zoom get the canvas below the left row as their room; `draw` still
        returns the one canvas rectangle (rendering, clipping, hit tests unchanged). A sheet's
        top row carries both toggles. `Chrome` adds `canvas`, `left_float`, `right_float` and
        `hover_row` (records, no behaviour).
      - **Identity.** `identity`/`compact_identity`, `canvas_name` and `paper_line` in fixed
        slots (`NAME_H`, `PAPER_H`; the compact width from the kept name), so the Canvas name
        field (`file_title_field`, the whole slot) opens in place. `in_slot` draws a slot's
        label or field in a child ui (`Ui::new_child`) that allocates nothing outside the slot:
        a single-line field is at least egui's 24 px interact height, and placed in a shrunk
        slot it had pushed the Paper line 2 px down (run132 repair). `paper_menu` (every
        Paper, then `identity_menu`: Rename Canvas…, Rename Paper…, `paper_actions`), the
        name's `context_menu`, and the compact menu's top share them. `start_paper_rename`
        opens the Papers list's own rename (one field, one path); `start_canvas_rename` the
        name's. `finish_title_draft` finishes a draft whose identity isn't drawn (Hide UI, the
        Design sheet), as losing focus does. The title's invalid-name note says Canvas.
        R57-P2: `draw`'s finish of hidden renames asks `Panels::wanted` again after the
        panels' widgets ran, so the inspector's Rename Paper… with Papers closed keeps its new
        draft until its row is shown and focused (before, the frame's earlier `want` finished
        it at once, and typing reached tool shortcuts). The browser host's `key_down` keeps
        keys from the editor while a page or layer rename is pending (`renaming` set) and its
        field doesn't have the keyboard yet; `draw` still finishes any rename whose rows
        aren't coming. R58-P2, `src/ui/pages.rs` `page_row`: while a page rename field asks
        for the focus it also scrolls its row into the Papers list's capped window
        (`Response::scroll_to_me_animation`, as little as needed, at once), so a rename asked
        for elsewhere (the tenth of ten Papers, say) is visible and hittable. Only until it has
        the focus: typing and the person's own scrolling are never pinned back.
      - **Toggles.** `sidebar_toggle` (the pair, with an accessibility name through
        `widget_info`): "Papers and layers" (`Icon::Sidebar`) and "Properties"
        (`Icon::SidebarRight`); `pill` and the "Design" text pill are gone. The `···` menu is
        a bare glyph at rest (`app_menu`: no inactive fill or outline).
      - **Linked hover.** `canvas_hover_row`: `Editor::hover`, only while the pointer is in
        the last laid-out canvas and over no floating UI, mapped to the shown row (the
        nearest collapsed group); `layer_row` paints it with the hover fill. No selection,
        document, scroll or expansion change; the row's own `ed.hover` is unchanged.
      - Layer symbols are secondary grey (components and instances keep the accent); the
        canvas menu and the zoom use the floating radius 10.
    - `src/ui/rail.rs`: `CHEVRON_W` 24 → 12 (`RAIL_W` 64 → 52). The arrow's target is always
      laid out; it is painted while its row is hovered or focused, its flyout is open, or its
      family is the current tool's (`RailRects::arrows` records which). Reason: owner point 5.
      Short rooms (owner correction after run133): `row_height` gives the rows the whole
      pixels that fit the tools' room (32 down to `MIN_ROW_H` 24; `rail_rect` and `draw` use
      it). At 320 × 240 the 210 px rail had run past the 240 px canvas and egui showed it 12 px
      higher, its first button (y 33) under the floating row (y 1–37); the rows are now 27
      px and every target lies in the requested rail. Taller rooms are unchanged.
    - `src/editor/pages.rs` `add_page` (upstream-owned default-name literal, owner correction):
      a page a person adds is named "Paper N" instead of "Page N". Only this creation path:
      `add_page_in_background` (agents) and `insert_empty_page`'s own default stay
      upstream's, existing names, migrations and formats are untouched, and
      `unique_page_name` still makes it unique.
    - `src/ui/inspector.rs`: `nothing_selected` takes `UiState`; `paper_context` replaces the
      generic hint and the always-editable page name (which renamed on every keystroke):
      the Paper's name read-only with Rename Paper…, its content and place, then View ▸ Grid
      (`UiState::set_background`). Creation tools' "For new …" defaults come first. The
      "Design" title moved to the panel's top row. Export's scope texts say Paper.
    - `src/ui/pages.rs`: "Papers", Add Paper, and the row menu's Rename/Duplicate/Delete Paper;
      the current Paper's icon is text-coloured, not accent.
    - `src/theme.rs`: `RADIUS` (6, widgets too, was 7), `focus_ring` (keyboard focus on
      `quiet_icon_button`, `icon_button` and the rail's arrow and flyout rows),
      `Icon::SidebarRight`; `quiet_icon_button` lights on focus too.
    - `src/render/scene.rs`: `Background::Lines.label()` is "Grid"; `name()` stays `lines`.
      `src/io/export.rs`: scope labels "This Paper", "All Papers". `src/ui/palette.rs`:
      "Go to Paper: …". `src/editor/shortcuts.rs`: Next/Previous Paper, "Show / hide
      background" (descriptions and menu labels only; actions and bindings unchanged).
    - Same Editor, Session, transactions, save, binding, lease and guards; no renderer, format
      or dependency change. Upstreamable as presentation, except the Paper/Canvas vocabulary,
      which is an OPD choice.
  - Paper-inspired polish of 2026-10-20 evening (root `DESIGN.md` revision 19; owner-authorized;
    source only, not built, rendered or natively qualified here). Presentation only; native
    windows change too. Reason: the owner asked for a materially more Paper-like chrome
    (type, density, surfaces, states, icons, Light/Dark) through one token change propagated
    to the existing controls.
    - `src/theme.rs`:
      - Neutral palette, Light and Dark: no blue cast; the panel #fafafa / #1e1e1f.
      - Type scale: `BODY` 13 → 12, a new `TITLE` 13 for the Canvas name, and `Heading`.
      - New tokens: `FIELD_H` 24 and `ICON` 16; `RADIUS` 6 → 5.
      - Widgets are filled `field` with a faint edge (not white outlined boxes), hover and
        press darken the fill, and the `active` (pressed, keyboard-focused, open) state has a
        1 px accent-ink edge.
      - egui's selection is a translucent accent tint whose text and focused-field edge are
        `accent_ink`, not a solid accent with white text.
      - Shadows are softer and closer.
      - `icon_rect` gives every icon button a 16 pt glyph (smaller only on buttons under
        24 pt). `paint_icon` uses one stroke weight, 1.2 pt clamped to 1–1.4.
      - `icon_button`'s selected state (the current tool, alignment and layout toggles) is
        the `selected_fill` tint with an accent-ink glyph, not solid accent with white.
      - `quiet_icon_button`'s on state (an open panel's toggle) is a neutral pressed fill.
      - `disclosure` draws a line chevron for folded sections.
    - `src/ui/mod.rs`: `section` and the Layers/Library tabs use Inter Medium 11 (were
      SemiBold); `title_text` (inspector section titles) is Medium 12 in the text colour (was
      SemiBold 13); the Canvas name keeps SemiBold at `TITLE` (`name_font`, used by the label,
      the compact width and the rename field, so the field still opens in the name's slot);
      layer rows' trailing eye/lock use `RADIUS` and light up on hover.
    - `src/ui/inspector.rs`:
      - Rows and fields are `FIELD_H` (24, were 20) with 12 pt labels.
      - Paired fields fill the row in two columns (`pair_w`). That covers X/Y, W/H,
        Points/Ratio, Radius, the arrow heads, Constraints, blend and opacity, and the export
        format and scale. Single-choice rows fill the rest of the row.
      - Mixed is drawn as the field it stands for, with the same placeholder text as before.
      - Sections sit 6 px apart with one hairline and even room around it.
      - Sketch and Export use the line chevron (`folded`).
      - Field prefixes ("X  "…), texts, structure and edit semantics are unchanged.
    - `src/ui/rail.rs` (flyout glyphs at `ICON`), `src/ui/pages.rs` (14 pt page icon) and
      `src/ui/palette.rs` (the chosen row is the selection tint with text-coloured labels, 28
      px rows) follow the same tokens. The rail's geometry (`RAIL_W` 52, `MAIN_W` 32,
      `CHEVRON_W` 12, `ROW_H` 32, `row_height`) and every panel inset are unchanged.
    - Same Editor, Session, transactions, save, binding, lease, guards, renderer and canvas
      display; no format or dependency change. Upstreamable as presentation.
  - Whole-app workbench, first slice (2026-10-21; root `DESIGN.md` revision 20; owner feedback
    in #10 and #14; source only, not built, rendered or natively qualified here). Native windows
    change too. Reason: the owner's Mac use asked for these exact placement, navigation and
    Theme changes, and they can only be made in the shared upstream UI.
    - `src/ui/mod.rs`:
      - **Contextual reveal** (responsive after the lead's run144 render finding that it was
        wide-only). `Panels::arranged(tier, selected, changed, width)` is what `draw` settles:
        `wanted` plus the Design panel for a new selection (`UiState::seen_selection` tells a
        new one) where the person closed it: beside the open panels on a wide surface; docked
        beside the canvas, or beside the left panel from `BOTH_DOCKED` (800 px), else in its
        place (`swapped`); on a sheet in an open left sheet's place, or (run145, after the
        lead's finding that 144 still excepted it without the owner's approval) over a
        canvas-only sheet once `Panels::hold_cover` is off: `draw` holds it for `COVER_WAIT`
        (0.4 s, the editor's double-click window) from `UiState::selected_at` and while
        `Editor::text_edit`, `pen` or `path_edit` is open, asking egui for the frame that
        ends the wait. It never writes the choices; `toggle` on a revealed Design panel
        dismisses it until the selection clears, and any toggle ends a reveal not shown yet
        (so a pinned panel closed with something selected stays closed); the left toggle,
        Rename and `start_paper_rename` (`open`) take precedence and give the left panel back;
        `draw` notes a new selection (`Panels::selection`) before the frame's `UiEvent`s, so a
        Rename arriving with the click that selected (run145's failing R25 test) ends the new
        reveal instead of being swapped for it later in the frame.
        `Panels::settled(shown)` makes `revealed()`/`swapped()` what the frame laid out, so
        toggles act on the reveal on screen. `revealed()`, `swapped()` and `design_choice()`
        are read-only records. `settle` also holds every arrangement change while
        `Editor::is_busy()` (a pointer gesture owns the canvas). The R57 check asks `wanted`
        and treats rows a swap hides as not coming. Docked, both panels draw with their
        single-panel ids; a swapped Design sheet's way back says "← Layers"/"← Theme", one
        over the canvas "← Canvas" (the selection kept).
      - **One Canvas title.** The left top row is `home_menu`, the title (`canvas_name` in a
        slot that fills the row, recorded as `UiState::left_name_w`) and `layers_toggle` at the
        right end. `float_left` gives the floating title the same slot, narrowed only to clear
        the right bar (`right_edge`). `identity`, `compact_identity`, `paper_line`,
        `paper_text` and `paper_menu` are gone; `go_to_paper` keeps the Paper navigation in
        the menu; `save_hint` keeps a quiet "Edited"/"Saving…" after the name.
      - **All Projects menu.** `home_menu` (the square control; `Popup::menu`) opens
        `all_projects_menu`: Back to All Projects first (the same `AppRequest::Home`), then
        `identity_menu`, Go to Paper and (without a native menu bar) every upstream menu,
        Appearance, Background, About & notices. `app_menu` (`···`) is gone;
        `Chrome::menu` records the same control as `Chrome::home`. The menu and every
        submenu lay out in `scrolled` (an egui `ScrollArea` capped by `menu_height` from
        `Chrome::surface`), after the run144 render showed the 320 × 240 menu cut off below Go
        to Paper; no item was removed. `menu_height` is also its `min_scrolled_height`: the
        run145 tests showed a 1280 × 800 menu stopping at about 430 px (the popup offers less
        room than the surface), so a menu that fits is now shown whole.
      - **An open popup owns the input** (run147; the lead's probe on frozen 145: with the
        All Projects menu open and nothing focused, ArrowDown nudged the selected rectangle
        from y 100 to 101, an undoable, saved edit). `ui::popup_open` (egui's public
        `Popup::is_any_open`) is what both hosts now check, beside `captures_input`: the
        browser host's `key_down` (after a focused field's own keys), `copy`/`paste` (no Cut
        or Paste behind it) and `pointer_down`; native `App`'s pressed keys (releases still
        reach the editor, so a held Space's pan ends), IME commits, mouse presses and touch
        starts. Reason for the upstream-owned `src/app.rs` and `src/ui/mod.rs` change: the
        same shared UI's menus, and the same hole, in both hosts; a fix. `home_menu` focuses
        Back to All Projects when the menu is opened from the keyboard
        (`clicked && !clicked_by(Primary)`, as egui's own submenus tell it), so egui's focus
        moves through it with the arrow keys.
      - **Releases always reach the editor** (R61-F1, P2; the R61 review's reproduction:
        Space held on the canvas, a pointer-opened menu given the focus with Tab, Space
        released there, Escape, then a Select drag panned the camera by (-80, -40) instead of
        moving the rectangle). The browser host's `key_up` and native `App`'s released keys
        no longer go through the focused-field/popup guard: upstream acts on a release only to
        let go of a key it holds (`Editor::space_down`), so presses keep their owners and a
        held key still ends. The native side is the same source change, not a Mac reproduction.
      - **Cursors.** A layer or Paper row carried to a new place sets egui's `Grabbing` cursor.
      - **Right bar.** `right_tools` (in the Design panel's row) and `float_right` hold
        `export_button` (the existing `AppRequest::ExportFormat` and Copy as PNG/SVG
        actions; labelled Export, no sharing service) and `zoom_group` (−, `text_button`
        level with Fit, Selection, 100%, +; compact where there is no room) before
        `design_toggle`, whose tooltip is now "Design". `zoom_controls` (lower left) is gone;
        `Chrome::export` is new.
      - **Design/Theme.** `UiState::left_tab` (`LeftTab`) and `left_switch` under the top row;
        Theme hides the Paper and layer rows and finishes their renames first, as the Library
        tab does; `start_paper_rename` and the layer Rename event switch back to Design.
    - `src/ui/design_system.rs` (new): the Theme view, useful part first: the shown Paper's
      text styles and colours (`text_styles`, `colours`, read through
      `inspector::char_styles` and the node styles); choosing one only selects its layers
      (`Editor::selection`), so edits go through the existing Design panel. Then one status
      row each (`status_row`) for the font (the bundled Inter faces; the renderer draws no
      other family), named tokens (no token store in `Document`) and `DESIGN.md` (no admitted
      project route, and the guide reading/editing isn't built), reasons in tooltips and a
      folded Details. Nothing is invented. (Run144 replaced its first, paragraph-heavy layout,
      which filled a 320 × 240 sheet.)
      **A colour in use changes in place** (revision 21, 2026-10-22, #14/#10 group 6;
      implemented; lead-checked with real renders, independent review, production and native
      pending; see root `DESIGN.md`): a colour row's swatch is the Design panel's colour button (egui's
      `color_edit_button_srgba` in a child `Ui` with its own explicit id, taking no room; the
      row around it still selects). Reason: selecting a colour's layers couldn't change the
      colour, since layers sharing it seldom share whole fills and strokes, so the Design
      panel showed Mixed and offered only "Use one fill/stroke" (replacing them). `Recolour`
      (in egui's temp memory) records, at the first change, the exact solid fills and strokes
      with that colour on unlocked layers (`places`; `Document::is_effectively_locked`), and
      every change of that picker use sets those places only (`recolour`: one
      `begin_coalescing` transaction keyed by the colour and layers, `mark_overrides` for
      `fills`/`stroke` as `inspector::apply` does, then `commit`), so passing another colour
      on the way never spreads it; the history is sealed when that picker closes, so one use
      is one undo step. The picker keeps its identity (the colour it began with) while its row
      moves; a row of locked layers only shows its swatch. Gradients, text-run colours and
      other pages are untouched; no named value, store, format or engine change, and the
      canvas save, conflict, Undo and lease are the existing ones. Run158 correction (found by
      reading, not by a failing run): `Recolour` also keeps `Editor::doc_stamp` after its own
      change; any other change of the document while its picker is open (a reload or the
      conflict discard, another Paper, Undo, an agent's edit) ends it and closes the picker
      (`Popup::close_id`), since egui keeps a popup open across a reload and the recorded
      places could otherwise reach another document's layers reusing those ids. Tests
      (`crates/openpaperdesign-web/src/state_tests.rs`, `rev21_*`, headless egui layout through
      the real Theme view and egui's own picker fields): targets kept across colours already
      in use (rows merging and moving), locked layers and ancestors, gradients, text runs and
      the other Paper exact, one Undo/Redo step per picker use, save and reopen, a reload
      under an open picker, and hiding Theme or switching Papers mid-use. Run159 correction
      (a rendered failure on the real WASM UI and Linux backend: R typed 1, 5, 0 showed 150 and
      Saved, and one Undo gave 15, not 50): each autosave's acknowledgement
      (`WebState::save_succeeded` → `Editor::mark_page_saved` → `mark_saved`) seals the
      history, which is right in general (the saved state stays a history boundary) but split
      the open picker use. `design_system::recolouring` (a fixed egui memory id, the picker's
      popup id and the document stamp) tells the host a changed picker use is still open, and
      `WebState::save_snapshot` holds the save while it is and the page has the focus; it
      saves once the picker closes or isn't shown, the document changes otherwise, or the page
      blurs (a departure). `mark_saved`, in-flight and conflict handling are unchanged. Test
      `rev21_autosaves_while_typing_into_the_picker_keep_its_use_one_undo_step`.
      **A text style in use changes in place** (revision 21, run162; implemented; lead-checked
      with real renders, independent review, production and native pending):
      each text style row's "Aa" button (an explicit-id child `Ui` in the swatch slot, the row
      around it still selecting) opens a `Popup::menu` editor with the style's size (a
      `DragValue`, 1–1000) and Inter's four faces; no family. Reason: selecting a style's layers
      edits their whole text through `style_text_with`, not only that style's characters.
      `Restyle` (egui temp memory, fixed id) freezes `style_targets` at the first change: per
      unlocked text layer, every character resolved (`TextStyle::char_style`, the runs between
      span edges, as `text_styles` lists them) to that style. A layer wholly in it changes its
      own `font_size` or `bold`/`italic` with its ranges' values for it dropped (as
      `style_text_with`'s whole-layer path); otherwise `TextStyle::apply_range` over exactly
      those char ranges sets only the changing value, keeping each range's colour, underline
      and strike. Then `mark_overrides` (`font size`/`font`), `fit_text` (fixed widths kept)
      and one coalesced transaction per editor use, sealed when it closes; the same identity,
      stale-stamp and `Popup::close_id` handling as `Recolour`, and `design_system::editing`
      (colour or style under way) is what `WebState::save_snapshot` now holds the autosave on.
      No new primitive: existing `TextStyle`, `Editor` transaction, override and layout code.
      Tests: `rev21_a_theme_text_style_changes_only_its_characters_in_one_step_and_saves_exactly`
      and `rev21_a_text_style_editor_never_outlives_its_document_or_paper`. An agent's
      `get_jsx` reports a text's base style only, so a range's proof is the page file and the
      render, never inferred rich-run JSX.
      **R67 corrections** (run165; the independent R67 review's two P2s, source only):
      - **Locked instances.** Theme's targets skip locked layers, but committing a change inside
        a main component syncs every instance of it (`Editor::fixup` → `sync_node` →
        `copy_props`) whatever its lock, so a locked instance's inherited red or 24 changed
        anyway. Component sync is unchanged; Theme now refuses such a change before anything
        changes: `reaches_locked` follows `InstanceLink::main` from the change's layers and
        override groups (`fills`/`stroke`, `font`), through instances of instances, while the
        layer is inside a main component and not overriding that group, and refuses when it
        reaches an effectively locked layer that isn't a target. Such a row's swatch or "Aa"
        (`refused_slot`) opens nothing; its tooltip and, when chosen, the status line say
        `INHERITED_COLOUR` / `INHERITED_STYLE` (unlock that instance). The check runs again when
        a change begins, so a refusal never applies anything. Rows no locked instance inherits,
        and unlocked instances (which take the change as targets, with the inspector's
        overrides), are unchanged.
      - **Native autosave.** `Session::autosave_due_ui` (upstream `src/session.rs`, called by
        `App::tick_autosave` in `src/app.rs` with the app's egui context) holds the native
        autosave while `design_system::editing` is true, as `WebState::save_snapshot` does,
        moving its deadline on so the event loop's timer doesn't spin; it is due again an
        autosave delay after the use ends, and closing the project (`save_all`) saves
        regardless. `Session::autosave_due`, `save`, `mark_saved` and the history seal are
        unchanged. Reason for the upstream-owned files: the shared Theme UI is drawn by the
        native app too, and only its own host can hold its own save.
      - Tests (`rev21`'s `state_tests.rs`, `r67_*`): the reviewer's three passing probes (late
        acknowledgement during colour and style, failed save and discard) kept unchanged but for
        formatting; refusal for colour and style with the whole page, history, stamp, unsaved
        state and selection unchanged and the reason shown, plus successful unlocked-instance
        controls with one-step Undo/Redo; and the native session through `autosave_due_ui` and
        the real `Session` save (adapted: a fresh temporary store, and the normal save after
        the picker closes).
      **R68 correction** (run167; the independent R68 review's one P2, source only): a locked
      instance whose text overrides only `font` kept its own 18 but still took the main's new
      text box, since `restyle` lays the main out again (`fit_text`) and `copy_props` copies a
      box unless `size` is overridden. `style_groups` now names `font` and `size` for each
      target, and `size`/`position` for its group and auto-layout ancestors up to the
      component, so `reaches_locked` refuses such a row up front; and every text style change,
      the first and each later one, is first applied to a throwaway `Editor::new` over a copy
      of the shown Paper through the same `restyle` (range edits, `fit_text`, overrides, the
      commit's fixup with component sync, groups, auto layout and arrows), refusing it if any
      effectively locked layer would differ (`restyle_reaches_locked`; a first change refused
      leaves everything as it was, a later one ends the use with its earlier changes as one
      step). `INHERITED_STYLE` now names the text box too. Component sync itself is unchanged.
      Tests: the reviewer's fixture and its three passing probes kept (the font-and-size
      override control, the native style hold and timer, native close and the generic seal),
      unchanged but for formatting; its failing probe's case as a refusal test (whole Paper,
      history, stamp, unsaved state, selection, child and instance root unchanged, reason
      shown), since the probe's changed-main expectation isn't the refusal contract.
      **R69 correction** (run168; the independent R69 review's one P2, source only): the first
      check above tried `restyle` on a document-only copy, then ended the live text edit
      before its final check, so a refused first change could already have committed the open
      draft (an emptied text removed, its wrapping auto-layout row reflowed, a locked next row
      moved, history 0 → 1, a changed page offered to save). Now `fresh_restyle` admits the
      whole first step on `Editor::preview` (new in upstream `src/editor/mod.rs`: a throwaway
      editor with the shown page's document, open transaction and coalescing key, text edit,
      selection and suspended layouts; `src/history.rs` `Recording` gains a derive-only
      `Clone` for it): ending the text edit, the targets after it, the inheritance foresight,
      and the change with all its follow-ups; refused if any effectively locked layer of the
      live document would differ. Only an admitted use then ends the live text edit and finds
      its targets again; a refusal leaves the live editor exactly as it was (document, open
      draft with caret and range, selection, history, open transaction, stamp, unsaved state).
      Later changes of a use are tried on a preview too. Text completion, component sync,
      transactions and saving are unchanged elsewhere. Reason for the upstream-owned files: a
      faithful preview needs the open transaction, which only the editor holds. Tests: the
      reviewer's five passing probes (three actual-widget, two helper-level beside
      `design_system`) unchanged but for formatting; its helper-level failing probe unchanged
      (its assertions are the corrected contract); its actual-widget failing probe with the
      same fixture, trigger, visible refusal and requested-font checks, and its middle
      assertions (which recorded the defect: a changed page saved, then reversed and replayed)
      adapted to the exact-state contract.
    - `src/ui/rail.rs`: seven families (`FAMILIES`: Pointer = Select, Laser; Hand on its own),
      `Rail::members` `[Tool; 7]`; `ROW_GAP` 2 → 1 so 320 × 240 still fits 24 px rows; the
      arrow glyph is drawn at its target's near edge (targets unchanged).
    - `src/ui/inspector.rs`: `paper_context` no longer repeats the Paper's name or a Rename
      Paper… (its count reads "1 layer" / "n layers"); `char_styles` is `pub(crate)` for
      Theme.
    - `src/ui/pages.rs`: no shown-Paper name beside the folded Papers header; a row being
      dragged to a new place sets the `Grabbing` cursor.
    - Same Editor, Session, transactions, save, binding, lease, guards, renderer and document
      formats; no new dependency, engine, token store or host route. Upstreamable as
      presentation, except the Canvas/Paper vocabulary and the panel defaults (OPD choices).
- Upstream changes in the browser host (not upstream-owned, for reference):
  - `WebState::pointer_down` keeps a press on a panel's resize handle from the editor
    (`Chrome::on_panel_edge`), as `App` does.
  - `WebState` reports `HostRequest::Background` and applies a kept one
    (`set_background`).
  - It saves a finished title rename in the canvas save (`saved_name`, the in-flight
    `name`). A draft is guarded as the renames are, and a reload closes it.
  - The page's accessible name of its editor follows the editor's own (R61-F2, P2: Home's
    `#editor` region, labelled by `#ed-title`, kept the name it was mounted with after a saved
    rename). `WebState::canvas_name` / `WebEditor::canvas_name` (upstream's
    `UiState::project_name`: a finished rename, a reload's) is read after each frame by the
    shared mount (`editor.js` `onName`, once per change, never after disposal); `panel.js`
    passes it on only while that editor isn't left, and Home sets `#ed-title` as text only for
    the shown editor (`panelHandle === h`); the browser page sets its tab title. No remount,
    server call, save or new timer.
- Project design guide codec (R19–R21 precursor, not upstream-owned; written in source with
  its unit tests. History: the independent R63 and R64 reviews requested changes, corrected
  below. Which exact candidate passed which checks or reviews is recorded
  in its source handoff, not here; nothing in this entry is an independent, native, Theme or
  source-link acceptance): `crates/openpaperdesign-serve/src/guide.rs` (tests in
  `guide_tests.rs`), `pub mod guide` of the serve library. Pure: bytes in, a `Guide` snapshot,
  `Tokens` inspection or a checked `Proposal` out; no file system, path, metadata, MCP, browser,
  host, timer, grant or canvas access, and no caller yet. `Guide::read` keeps the exact bytes
  (≤ 1 MiB, UTF-8, no NUL; BOM and every line's LF/CRLF/lone CR); `plan_text` plans a prose
  edit against an expected `Revision` (FNV-1a and length: tells versions apart, not a
  security check) as Myers line hunks, untouched lines byte-exact, new or changed lines taking
  the nearest preceding original line's terminator, bounded (`MAX_CHANGED_LINES`,
  `MAX_DIFF_STEPS`) and refused beyond, and refused (`PlanError::TooLarge`, nothing proposed)
  when the guide it would make exceeds `MAX_BYTES` (run149 correction after the lead's probe:
  a `MAX_BYTES` CRLF guide and `MAX_BYTES` draft made a `MAX_BYTES + 1` proposal its own
  reader refused); `tokens` reads exactly one fenced `json` block marked
  `"opdDesignTokens": 1` with its own span-recording JSON reader (escape-equivalent duplicate
  names, depth, lone surrogates and invalid JSON refused; numbers kept as lexemes), version 1
  groups `colors`, `fontSizes`, `fontWeights` (400/700), `spacing`, `radii`, `frames`
  (direction, gap, padding); `plan_value` replaces one known supported value's span only and
  re-reads the result. R63 correction (four P2s and a P3 of the independent review):
  - a CR and LF never meet across two written lines (an LF there read as one CRLF and the
    line was lost: `"a\rb\n"` made `"a\n\n"` proposed `"a\r\n"`): a new empty line right
    after a lone CR takes a lone CR, and a new line that would end in a lone CR before an
    unchanged empty LF line takes CRLF. An unchanged line's bytes are never rewritten for
    this (the first R63 correction rewrote such an empty line's LF to CR; the lead's probe
    showed that broke the untouched-bytes contract): where an edit would bring an unchanged
    empty LF line right after a lone CR, it is refused (`PlanError::LineEndMerge`, nothing
    proposed). Every prose proposal is read back against the draft (`PlanError::SelfCheck`);
  - a `json` block that doesn't parse counts as marked when the marker's name appears in it
    (`mentions_marker`), so an escaped-marker broken block makes the values read-only (alone,
    or `Multiple` beside a valid block) instead of passing as unmarked data. R64-1: it no
    longer pairs quotes (a broken string before the name, `{"x":"broken, "<name>":1,}`, hid
    an escaped name); it matches the name at every byte, each character literal or a `\u`
    escape, at most six bytes a character, nothing repaired or rewritten. Its tests build the
    escaped name at run time, so their inputs really hold the backslash escape and not the
    plain name (an earlier version of those inputs held the plain name);
  - `fences` skips HTML blocks (`<pre>`/`<script>`/`<style>`/`<textarea>` to the closing tag,
    other tag lines to a blank line) and marks fences indented inside a list `contained`
    (ignored), so documentation examples are not editable values. R64-2: the enclosed forms
    (`<!--`, `<![CDATA[`, `<?`, a declaration `<!` and a letter) run to their own `-->`,
    `]]>`, `?>` or `>`, blank lines inside included, or unclosed to the guide's end; before,
    all but the comment ended at their first blank line;
  - `number` uses a lexeme only when an f64 holds it exactly as written (`Decimal`: the f64's
    shortest decimal equals the lexeme's exact value), so `400.00000000000000001`,
    `1000.00000000000000001` or `-1e-99999` stay unsupported while `4e2` and `7.00e2` stay
    weights 400 and 700;
  - a marked block's info string must be exactly `json` (any case): `json custom-v2` is
    `ReadOnly::Format`.
  Reason: the admitted guide/value codec any later source reader/editor needs (rev2 §8–9).
  It is not a source link: reading, creating or saving a project's guide, Theme's connection, named tokens, font families and Apply stay gated on the real model-call
  admission and enforceable revocation proof; no Theme, MCP or file route uses it. Same Editor,
  Session, save, binding and lease; no new dependency or store.
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
- The home view resource (`HOME_URI`, currently `ui://openpaperdesign/home-21.html`).
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

`home-10` (2026-10-19 source candidate) is `home-9`'s view (HTML and layout unchanged) with the
corrected in-panel editor: Paste while an arrow's label is edited edits that label as plain
text (the shared Editor change above). The editor's WASM changed, so it gets a new immutable
key: a cached `home-9` view and its editor can't pass for this one. Not installed or natively
accepted.

`home-11` (2026-10-19 source candidate) is the same view (HTML and layout unchanged) with the
editor whose open text edit changes nothing once its layer is locked (the shared Editor
change above). The editor's WASM changed again, so `home-10` is refused and a cached `home-10`
view can't pass for this one.

`home-12` (2026-10-19 source candidate) is the same view (HTML and layout unchanged) with the
editor that copies, cuts and pastes labelled arrows through the clipboard (the labelled
payload above). The editor's WASM changed again, so `home-11` is refused and a cached
`home-11` view can't pass for this one.

`home-13` (2026-10-19 source candidate) is the same view (HTML and layout unchanged) with the
R47 corrections of that payload (whole images before any registration; a successful paste
said in the status). The editor's WASM changed again, so `home-12` is refused and a cached
`home-12` view can't pass for this one.

`home-14` (2026-10-19 source candidate) is the same view (HTML and layout unchanged) with the
R48 correction of that payload (a member named twice refused before anything is checked or
registered). The editor's WASM changed again, so `home-13` is refused and a cached `home-13`
view can't pass for this one.

`home-15` (2026-10-19 source candidate) is the same view (HTML and layout unchanged) with the
editor whose bent curve keeps its bend handle (the curve re-edit above). The editor's WASM
changed again, so `home-14` is refused and a cached `home-14` view can't pass for this one.

`home-16` (2026-10-19 source candidate) is the same view (HTML and layout unchanged) with the
editor whose connector drag stops once its arrow is locked (R51-F1 above). The editor's WASM
changed again, so `home-15` is refused and a cached `home-15` view can't pass for this one.

`home-17` (2026-10-20 source candidate) is the same view (HTML and layout unchanged) with the
editor whose curves have a bend handle per bend (the multi-bend re-edit above). The editor's
WASM changed again, so `home-16` is refused and a cached `home-16` view can't pass for this
one. Not installed or natively accepted.

`home-18` (2026-10-20 source candidate) is the owner's UX revision (root `DESIGN.md` revision
18, the upstream entry above): the editor without a full-width header (full-height panels,
the Canvas identity in the left panel, mirrored toggles, the slim rail, the Paper context,
linked hover, Grid naming), Paper counts on Home ("1 Paper", `papers`; the server's `pages`
count unchanged) with a new library file's first Paper named "Paper 1"
(`library::FIRST_PAPER` in `NewDocument::blank`; existing and imported files keep their names),
and the view's own search field drawn as one at rest (a wrapper
with a magnifier and the stronger hairline around the same `#search` input; filtering,
Escape, the empty and no-match states, the kept grid/list layout and the narrow layout are
unchanged). Both the view's HTML and the editor's WASM changed, so `home-17` is refused and a
cached `home-17` view can't pass for this one. Not built, rendered, installed or natively
accepted here.

`home-19` (2026-10-20 evening source candidate) is the Paper-inspired polish (root `DESIGN.md`
revision 19, the upstream entry above): the editor's neutral palette, 12 pt type scale, filled
fields in paired columns, tinted current-tool and selection states, one icon size and stroke,
and the view's matching CSS tokens (neutral fallbacks, 13 px body, 18 px title, 6 px control
radius, softer menu shadow; the in-view editor's failure note as danger text and edge on the
view's surface rather than white on a danger fill, which measured about 2:1 in Dark). Home's structure (Files, Search files, layout toggles, New file),
the search field's resting look, filtering, Escape, empty and no-match states, the kept
layout and the narrow layout are unchanged; no element, id or script changed. Both the view's
HTML and the editor's WASM changed, so `home-18` is refused and a cached `home-18` view can't
pass for this one. Not built, rendered, installed or natively accepted here.

`home-20` (2026-10-21 source candidate) is the whole-app workbench's first slice (root
`DESIGN.md` revision 20, the upstream entry above): the in-panel editor's contextual Design
reveal, one Canvas title, the All Projects menu with Back to All Projects first, Export and the
zoom in the right bar, separate Select and Hand, and the Design/Theme switch. The view's HTML,
layout, search, kept preferences and Home freshness are unchanged; the editor's WASM changed,
so `home-19` is refused and a cached `home-19` view can't pass for this one. Later installed
as part of `6630ec4` (root `DESIGN.md`, "Where things stand"); its loaded view and native use
are unproven.

`home-21` (2026-10-22 source candidate, root `DESIGN.md` revision 21) is the same view (HTML
and layout unchanged) with the editor whose Theme changes a colour in use wherever the Paper
uses it (the upstream entry above). The editor's WASM changed after installed `home-20`, so
`home-20` is refused and a cached `home-20` view can't pass for this one. It also carries
Theme's text style change (run162). Built and rendered by the lead at bounded scope; not
independently reviewed, installed or natively accepted.

A view a host already cached under an earlier key is not evicted or refreshed by this.
The plugin's icon assets are static SVGs in `plugin/openpaperdesign/assets/` (no script,
`foreignObject`, style, text or external reference; at least 48×48 intrinsic). The launcher
embeds `icon.svg`/`icon-dark.svg` at build time, so rebuild after changing them. Revision 21
(issue #8, the owner's 2026-10-19 architecture direction; root `DESIGN.md`, "The plugin's
mark") replaced the earlier sheet-and-handle mark in all four, same paths, sizes and wiring;
the package and icon-wiring checks passed, but its display in Codex isn't shown and it isn't
installed.
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
