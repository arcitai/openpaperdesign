---
name: openpaperdesign
description: Bind a design project in this workspace, edit it over MCP, and open its editable canvas in the in-app browser so the person can edit it too.
metadata:
  version: "0.1.0"
---

# openpaperdesign editor

Workspace contract updated 2026-10-13 (Codex 0.162.1 experimental support).

An editable design canvas (paper-rs) for one project in this workspace. You edit through the
`openpaperdesign` MCP tools; the person edits the same project in the in-app browser. Both
see each other's changes live.

1. **Check setup.** Call `openpaperdesign_status`. If the tools are missing or the browser
   editor is "not built", use the [setup skill](../setup/SKILL.md) and say what is missing.
   Require `workspace_usable: true` and the intended `workspace`. The packaged server uses
   host-generated context on real model tool calls, not its plugin-folder cwd or tool
   arguments. If unusable, report `workspace_problem` and stop; do not invent metadata,
   change permissions, or bypass the refusal with another route.
2. **Bind deliberately.** Nothing is open until you bind. `openpaperdesign_canvases` lists the
   existing canvases (projects directly under `design/`) in this workspace, read-only. Use
   `bind_project` with one of them, or another existing project folder inside the workspace
   (a folder with `project.json`), or `create_project` for a new one under `design/<name>/`.
   If several projects could be meant, ask which. Binding again switches projects; a failed bind keeps the
   current one. Once you've called `open_editor`, switching stops the server the browser tab
   saves to, so the person's unsaved edits there would be lost. The switch is then refused
   until you call it again with `force: true`. Ask the person first (let their edits save, or
   confirm losing them), and never pass `force` on your own. The same is true when this
   session ends.
3. **Read the page, then edit.** Before your first change to a page, read that page:
   `get_tree_summary` with `nodeId` `"root"` (add `pageId` for another page), then
   `get_node_info` for what you will change. Don't rely on the summary `bind_project`
   returned. Reads count per page and only when they succeed: read every page a change
   touches, and call `get_basic_info` before renaming, adding or removing pages (it returns
   the page list). A change to a page you haven't read in this session, or that changed
   since you read it, is refused as a `conflict` and nothing is written. Then edit with the design
   tools (`write_html`, `create_shape`, `update_styles`, `move_nodes`, `delete_nodes`,
   `rename_pages`, …). Each change is saved to the project at once.
4. **Open the canvas.** Call `open_editor` and open the returned
   `http://127.0.0.1:<port>/#token=…` URL in the in-app browser. Its result also names the
   workspace, the bound project and its canvas; check they're the ones meant. Say whether
   the canvas actually loaded. The token is for this session only: don't write it into files
   or commits.
5. **Work together.** Your saved changes show up in the canvas when the person has nothing
   unsaved there. If the person has an unsaved edit while you write the same page, their save
   is refused and the page shows a conflict; nothing is overwritten either way. Let the person
   decide: their "Reload (discard my unsaved change)" shows your version. Prefer to edit
   after they've saved. If the page changed after you last read it (the person saved, or it
   was renamed), your next change to that page fails with a `conflict` error and nothing is
   written: read the page again and redo the change on what is there now.

Limits (state them when relevant):

- One bound project per session, inside this workspace only. Export, project creation
  outside `design/` and other projects are refused.
- Only the observed managed workspace-write profile (network `restricted` or `enabled`)
  and exact host `disabled` profile are supported. Read-only/unknown profiles cannot bind
  or open a writable browser.
  `disabled` never disables plugin scope checks. Missing/changed context after pinning
  retires the session and its browser server; use a fresh supported session. Checks happen
  on tool calls, so idle host permission changes are not detected instantly. This adapter
  does not provide OS sandboxing. Protected metadata paths remain off limits. A Git
  worktree's regular `.git` file is opaque; its pointer never expands project scope.
- Images must be `data:` URIs in HTML `<img>` or CSS; local file paths are refused, and
  inline `<svg>` may not contain `<image>`.
- A native paper-rs app editing the same project at the same time is not protected.
- Image fills aren't drawn in the browser canvas yet; multi-page editing in the browser is
  limited to the active page.

The OpenPaperDesign home (`openpaperdesign_home`, in the Codex sidebar and as a chat tab) is a
static view. Its buttons send you requests like the steps above as ordinary user messages;
they grant no workspace and name no authority. Follow the same steps and refusals. When a
request names a workspace or project, continue only if `openpaperdesign_status` shows
exactly that. Opening an existing project changes nothing in it: no `DESIGN.md`, token,
asset or folder is created, copied or moved. A new canvas follows the
[design](../design/SKILL.md) method.

`openpaperdesign_projects` lists the Codex projects saved on this computer, with every root
folder: discovery only. When the person picks a folder there, use it only if
`openpaperdesign_status` reports exactly that folder as this chat's workspace. Otherwise, ask
them to open that project in Codex. Never switch folders, read the listed folder, or treat a
listed root as permission.

For visual direction and review, use the [design](../design/SKILL.md) and
[review design](../review-design/SKILL.md) skills; this editor is their editable companion.
