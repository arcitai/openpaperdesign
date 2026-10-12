---
name: openpaperdesign
description: Bind a design project in this workspace, edit it over MCP, and open its editable canvas in the in-app browser so the person can edit it too.
metadata:
  version: "0.1.0"
---

# openpaperdesign editor

An editable design canvas (paper-rs) for one project in this workspace. You edit through the
`openpaperdesign` MCP tools; the person edits the same project in the in-app browser. Both
see each other's changes live.

1. **Check setup.** Call `openpaperdesign_status`. If the tools are missing or the browser
   editor is "not built", use the [setup skill](../setup/SKILL.md) and say what is missing.
2. **Bind deliberately.** Nothing is open until you bind. Use `bind_project` with an existing
   project folder inside the workspace (a folder with `project.json`, usually under
   `design/`), or `create_project` for a new one under `design/<name>/`. If several projects
   could be meant, ask which. Binding again switches; changes are already saved.
3. **Edit** with the design tools (`get_basic_info`, `write_html`, `create_shape`,
   `update_styles`, `move_nodes`, `delete_nodes`, `get_screenshot`, …). Each change is saved
   to the project at once.
4. **Open the canvas.** Call `open_editor` and open the returned
   `http://127.0.0.1:<port>/#token=…` URL in the in-app browser. The token is for this
   session only: don't write it into files or commits.
5. **Work together.** The person's edits and yours merge per layer. If both change the same
   layer, the page shows a conflict instead of overwriting; let the person decide.

Limits (state them when relevant):

- One bound project per session, inside this workspace only. Export, project creation
  outside `design/` and other projects are refused.
- Images must be `data:` URIs; local file paths are refused.
- A native paper-rs app editing the same project at the same time is not protected.
- Image fills aren't drawn in the browser canvas yet; multi-page editing in the browser is
  limited to the active page.

For visual direction and review, use the [design](../design/SKILL.md) and
[review design](../review-design/SKILL.md) skills; this editor is their editable companion.
