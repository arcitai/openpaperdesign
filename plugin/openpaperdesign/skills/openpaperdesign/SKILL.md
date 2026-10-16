---
name: openpaperdesign
description: Explain and support the OpenPaperDesign canvas library in the Codex sidebar, and, with a separately started workspace server, bind and edit a design project in this workspace over MCP.
metadata:
  version: "0.1.0"
---

# openpaperdesign editor

Contract updated 2026-10-16: the packaged plugin serves the app library.

An editable design canvas (paper-rs). The packaged plugin starts
`openpaperdesign-mcp --app-library ${PLUGIN_DATA}`: an app-library server.

**The app library (packaged).** The person uses the OpenPaperDesign home in the Codex sidebar.
Its New, open, close, import and export actions call the server directly; they send you no
message and need no turn from you. Canvases live in OpenPaperDesign's own data folder
(`library/v1/canvases/<id>/`, one standard paper-rs project each) and open in the loopback
browser editor, where the person edits and saves them.

- This server has no workspace and no project or design tools. `openpaperdesign_status`
  reports `mode: app-library`, the library folder and the open canvas. Don't claim you can
  bind, read or edit a library canvas, or a project, through it.
- A library canvas is app-owned until the person exports it. Export gives a canvas file:
  the canvas as last saved, without design direction (`DESIGN.md`) or tokens. Unsaved edits
  in a browser tab aren't in it. Import makes a new canvas from such a file and never
  overwrites one.
- A canvas open in another Codex chat or panel is busy: close it there first. Closing stops
  its editor; anything still unsaved in that tab is lost, so the view asks first.
- A new blank canvas applies no design system and creates no `DESIGN.md`, tokens or assets.

**Workspace projects (separate server).** Editing `design/<name>/` in a repository with the
tools below needs a server started for it: `openpaperdesign-mcp --codex-context` (Codex's
per-call workspace context) or `openpaperdesign-mcp --workspace <dir>` (standalone). The
packaged plugin doesn't start one. When such a server is available:

1. **Check setup.** Call `openpaperdesign_status`. Require `workspace_usable: true` and the
   intended `workspace`; otherwise report `workspace_problem` and stop. Don't invent metadata,
   change permissions or route around a refusal.
2. **Bind deliberately.** `openpaperdesign_canvases` lists the projects directly under
   `design/` (read-only). Use `bind_project` with one of them or another existing project
   inside the workspace, or `create_project` for a new `design/<name>/`. Ask if several could
   be meant. A failed bind keeps the current one. Once `open_editor` was called, switching
   needs `force: true`, because the browser tab's unsaved edits would be lost: ask the person
   first and never pass `force` on your own.
3. **Read the page, then edit.** Before your first change to a page, read it
   (`get_tree_summary` with `nodeId` `"root"`, and `pageId` for another page). Reads count per
   page and only when they succeed; call `get_basic_info` before renaming, adding or removing
   pages. A change to an unread or since-changed page is refused as a `conflict` and nothing
   is written. Each change is saved at once.
4. **Open the canvas.** `open_editor` returns `http://127.0.0.1:<port>/#token=…` for the
   in-app browser, with the workspace, project and canvas it names; check they're the ones
   meant, and say whether the canvas actually loaded. Never write the token into files.
5. **Work together.** If the person has an unsaved edit while you change the same page, their
   save is refused and the page shows a conflict; nothing is overwritten. Let them decide.

Workspace limits (state them when relevant):

- One bound project per session, inside this workspace only. Export, project creation outside
  `design/` and other projects are refused.
- Only the observed managed workspace-write profile (network `restricted` or `enabled`) and
  the exact host `disabled` profile are supported; read-only or unknown profiles can't bind or
  open a writable browser. Missing or changed context after pinning retires the session.
  Checks happen on calls; this adapter provides no OS sandboxing. Protected metadata paths
  stay off limits, and a Git worktree's regular `.git` file is opaque.
- Images must be `data:` URIs; local file paths are refused, and inline `<svg>` may not
  contain `<image>`.
- A native paper-rs app editing the same project at the same time is not protected. Image
  fills aren't drawn in the browser canvas yet; browser editing is limited to the active page.

An existing root `DESIGN.md`/`design.md`, its tokens and assets, and canonical
`design/<name>/` canvases belong to the project: nothing here creates a second direction file,
copies a project into the library or migrates it. Design work follows the
[design](../design/SKILL.md) method; review uses [review design](../review-design/SKILL.md).

`openpaperdesign_projects` lists the Codex projects saved on this computer, with every root:
discovery only. A listed root is never permission, and no library canvas comes from it.
