# Control protocol

`fieldcraft --control PORT` (or `FIELDCRAFT_CONTROL_PORT=PORT`) listens on `127.0.0.1:PORT` for
JSON lines. Each request is one line `{"id": …, "method": "…", "params": {…}}`; each reply is one
line `{"id": …, "ok": true, "result": …}` or `{"id": …, "ok": false, "error": "…"}`. Requests are
handled on the UI thread between frames; pointer and keyboard input is injected as real egui
events, one pointer event per frame. The protocol has the same shape as SolidCraft's.

```text
fieldcraft --sample --control 7070          # or --sample cantilever | impact
{"id":1,"method":"engine.execute","params":{"command":"model.info","params":{}}}
→ {"id":1,"ok":true,"result":{"elements":{"hex8":64,"tet4":162},"nodes":199}}
{"id":2,"method":"ui.render","params":{"path":"/tmp/model.png","width":900,"height":600}}
```

The channel is untrusted input. A request line may be at most 16 MiB; a longer one is answered
with `{"ok": false, "error": "request too large"}` and the connection is closed, as are
connections that send eight lines that are not JSON. At most 16 connections are served at once,
at most 64 queued requests run per frame so a flood cannot starve drawing, and a request the UI
thread has not answered within 60 seconds gets `{"ok": false, "error": "timeout"}`. Numbers are
checked and clamped (drag steps, wheel lines, window size); `ui.render` refuses an image size out of range.

## Engine

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `command`: command id, `params`: JSON object | the command's result |
| `engine.script` | `commands`: `[{command, params}, …]` | stops at the first error; `journal`: commands that changed the model so far |
| `engine.commands` | — | every command: `id`, `label`, `params` (the parameter documentation), `schema` (its parameters as JSON Schema) and `requires` (what the session must hold before it runs: `design`, `geometry`, `nodes`, `mesh`, `analysable`, `material`, `step`, `result`, …; `engine.execute` of a command missing one fails without doing anything, naming each missing piece and its toolbar entry) |

Commands never open forms or file pickers when run this way. `fieldcraft-cli commands` prints the
same registry. `file.sample {name}` opens a sample (`parts`, `cantilever`, `impact`) and
`solve.run {}` solves every step; the Results workspace comes up when a result appears (a static step with its contours on the undeformed shape: `results.deformed {}` shows the deformed shape). The
ribbon follows the commands run here, as it does for scripts, MCP, the command search and the
browser: a command that changes the model or the results display brings the tab where its
toolbar entry lives (Mesh for `mesh.*`, Properties, Steps, Loads & BCs… for `model.*`, Solve,
Results), as a click there would. Queries (`model.check`, `mesh.quality`…) and the Tools tab's
commands (files, undo, `model.info`) leave the ribbon where it is.

## UI

| Method | Params | Notes |
|---|---|---|
| `ui.inspect` | — | view settings (`ui`), camera, viewport rect (points), renderer (`gpu`/`cpu`) and adapter, open command form, status bar, node and element counts, the results display (`results`: what the `results.*` commands reply: view, `stale`, contour range, modes), the probed node (`probe`), `solve` (the title bar's Solve: `enabled`, and `unavailable` as `ui.toolbar`'s entries give it, listing everything `solve.run` misses), `busy` (the background job: a file opening, a solve, a mesher run from its form by a click on OK), `prompt` (`{"unsaved": path}`, `{"design": path}` (a design of another part opened over a meshed or defined model), `"recover"`, `"samples"`), `recent` (recent files), `selected` (the browser's selected definition), `browser` (each top-level browser node's setup state: `[{node, state, tip, add}]`, `state` `done`, `next`, `blocked` (⚠: its command cannot run yet; `tip` says what it needs and where), `todo`, `optional` or null for nodes that are no setup stage, `add` its ghost rows `{stage, label, command, status}`; a ghost row's region is `layout`'s `browser/add/<stage>`), `nav` (the left-drag mode), `layout` (each region of the window drawn last frame as `[x, y, w, h]` in points: `app_bar`, `ribbon`, `workspace_switcher`, `document_tab`, `app_bar:<action>`, `browser`, `viewport`, `nav_bar`, `axis_view`, `selection_bar`, `legend` (while results are shown), `context`, `command_line`, `timeline`, `toolbar_dropdown`, `palette`), version |
| `ui.set` | any view setting: `style` (`shaded_edges`, `shaded`, `wireframe`), `show_nodes`, `show_geometry`, `show_mesh`, `show_loads` (boundary condition and load symbols), `show_interactions` (reference points, couplings, rigid walls), `perspective`, `dark`, `show_tree` (the browser), `show_timeline`, `workspace` (`prepare`, `model`, `analysis`, `results`), `tab` (the toolbar tab: `geometry`, `mesh`, `properties`, `assembly`, `steps`, `loads`, `interactions`, `solve`, `results`, `tools`; a tab of another workspace brings that workspace), `palette` (the command search open), `menu` (the open menu: `file`, `help`, `workspace`, `view`, `more`, or a panel's drop-down as `<tab>/<panel>` such as `mesh/Tet`; `null` closes it) | an unknown setting or value is an error; returns the settings |
| `ui.view` | `view`: `front`, `back`, `left`, `right`, `top`, `bottom`, `iso`, `home` (iso and fit), `fit` | every view frames the model as drawn (the deformed shape and the setup's symbols included) in the part of the viewport the overlays (`layout`'s `legend`, `selection_bar`, `axis_view`, `nav_bar`) leave free; `fit` keeps the direction of view. Opening another model, sample or design frames it in the home view (the same design opened again after an edit keeps the camera), and results that come up with part of the model out of sight are framed. Returns the camera |
| `ui.camera` | `yaw`, `pitch` (degrees), `distance?` (model units) | turns the camera to any orientation about its target: `yaw` about Z (0 looks along +Y, the front view; −45 is the iso view's, −90 the right view's), `pitch` the elevation above the XY plane; `distance` moves the eye (the zoom of an orthographic view). Angles must be finite and the distance above 0: the yaw is wrapped, the pitch clamped to ±90 and the distance to 0.001–10⁹; anything else is an error. A sequence of calls orbits smoothly. Returns the camera (`ui.inspect`'s, angles in radians) |
| `ui.toolbar` | — | the ribbon: `workspace`, `workspaces`, `tab`, `tabs` (the workspace's), `menu` (the open menu), `panels` (each panel label's `menu` key and `rect`), and every catalogue entry (docs/features.md) with `id`, `label`, `tab`, `panel`, `command` (null while coming soon), `more` (its related commands), `coming_soon` (the milestone, or null), `enabled` (false while coming soon or while its command misses what it needs), `unavailable` (null, or what its command misses: `reason`, the tooltip's lines such as "Needs a material — Properties › Material", and `missing`, each with `requirement`, `what`, `provider` (the command that provides it) and `place` (its toolbar entry)), `fit` — `full` (a button), `collapsed` (its panel is one button), `more` (its panel is in the » menu) or `menu` (in its panel's drop-down) — and `rect` (screen points, last frame, on the shown tab; for `menu` entries the row while the drop-down is open) |
| `ui.start` | `command`: a command or toolbar entry id | like a click on the toolbar, a drop-down row or a search result: a command without parameters runs, one with parameters opens its form in the context panel; an entry that is coming soon is an error naming its milestone. Never opens a native file picker: file entries (open, save as, write deck) open their command's form |
| `ui.select` | `kind` (`set`, `material`, `section`, `step`, `boundary_condition`, `load`, `reference_point`, `coupling`, `fluid_domain`), `name`; nothing to deselect | selects a definition as a browser click does; its properties (with Edit and Delete) show in the context panel |
| `ui.select` (viewport) | any of `filter`, `box_mode`, `mode`, `clear`, `at`, `box`, `geometry`, `nodes`, `elements`, `faces` (and no `kind`) | changes the viewport's selection (see [Selection](#selection)); returns `ui.selection`'s reply |
| `ui.selection` | — | the viewport's selection: `filter`, `box_mode` (box selection on), `count`, `capped`, `summary`, `items`, `params` (the selection as a set's members), `hover` (what a click would pick under the pointer) |
| `ui.prompt` | `answer`: for `unsaved` `save`, `discard` or `cancel`; for `design` `new` (a new model with the design), `keep` (the design over the current model) or `cancel`; for `replace_mesh` `replace` (runs the form, replacing the mesh) or `cancel` (the form stays open); for `recover` `recover` (with `id` from `file.recovery.list`) or `close`; for `samples` `open` (with `name`) or `close` | answers the window's question (`ui.inspect`'s `prompt`) |
| `ui.dialog` | `params?` (JSON object, fills the form), `use_selection?`, `run?`, `submit?`, `cancel?` | drives the open command form, in that order; `use_selection` is the form's Target row (see [Selection](#selection)); `run` returns the command's result and closes the form on success; `submit` is the form's OK button: a mesher that would replace a body's mesh first asks (`prompt` `replace_mesh`, with the question and what else goes: sets listing the old labels, solved results), and meshers run in the background (`busy`); a form opened by another form's **Create …** button (a reference field with nothing to pick) goes back to that form on `run` (with the new item picked) or `cancel` |
| `ui.click` | `x`, `y` (screen points), `button?`: `left`/`right`/`middle`, `shift?`, `ctrl?`, `alt?`, `double?` | real pointer input; in the viewport a click selects what is under it (Shift adds, Ctrl/Cmd adds or removes, a click on nothing clears the selection), and a click on a results plot also probes the nearest node of the face under it (`solve.probe`; `ui.inspect` shows the values) |
| `ui.move` | `x`, `y` | |
| `ui.drag` | `x0`, `y0`, `x1`, `y1`, `button?`, `shift?`, `ctrl?`, `steps?` (1–200) | press, move, release: in the viewport a left drag orbits (with box selection on, it selects the visible nodes, element faces or elements in the box), a right or middle drag (or Shift + left) pans |
| `ui.scroll` | `x`, `y`, `delta?` (wheel lines, −100–100; positive zooms in) | zooms at the point |
| `ui.key` | `key` (egui key name, `Enter`, `Escape`…), `cmd?`, `shift?` | `F` over the viewport frames the model; `Escape` clears the selection (and then cancels an open form) |
| `ui.text` | `text` | typed text (into the focused field) |
| `ui.drop` | `paths`: 1–16 file paths | drops the files on the window: the next frame opens them as a drop from the file manager does, and as a file picked in the Open dialog does (a `.fieldcraft` model, a `.solidcraft` design, a `.3mf` tagged mesh; `ui.inspect`'s `busy` while they open). The native picker cannot be driven over the channel; this is the same route |
| `ui.worldToScreen` | `points`: `[[x, y, z]…]` (at most 1000) | screen points in the viewport drawn last frame (null when not on screen) |
| `ui.resize` | `width`, `height` | window inner size in points |
| `ui.fullscreen` | `on?` (default true) | into or out of full screen; on macOS the title bar's left group moves into the traffic lights' room while there are none |
| `ui.screenshot` | `path?` | PNG of the window, taken a few frames later; waits for a presented frame; when none comes within 8 seconds (a hidden window, a locked screen) it answers with `ui.capture`'s offscreen picture (`"offscreen": true`). Without `path`: `fieldcraft-shot.png` in the temporary folder |
| `ui.capture` | `path?`, `width?`, `height?` (points; default the window's), `scale?` (pixels per point, 0.5–4; default the window's) | PNG of the whole window (title bar, ribbon, open menus and search, panels, forms and the viewport) drawn offscreen, without fade-in animations: the UI pass runs in a separate egui context and is rasterised on the CPU, so it works with the window hidden, minimised or the screen locked. At most 8192 pixels a side and 16 megapixels. Without `path`: `fieldcraft-shot.png` in the temporary folder |
| `ui.render` | `path`, `width?` (default 1280), `height?` (default 800): whole pixels, each 1–8192 and at most 16 megapixels in all | CPU render of the model with the current camera, view settings and selection highlight (no window or GPU needed). A size out of range, or a scene over 20 million primitives, is an error |
| `app.quit` | — | closes the window |

## Selection

The viewport's selection belongs to the window, not the model: there is no engine command for
it, and nothing about it is saved or journalled. What is selected is kept in the model's
persistent terms, so it reads as a set definition: geometry by name (`{"body", "face"}`, or
`edge`, `vertex`, or the body alone; a face picked on the mesh is named through the mesh's
geometry tags, so it stays selected through a remesh) and the mesh by label (`nodes`,
`elements`, element `faces` as `{"element", "face"}`). Labels the model no longer has drop out
after an edit.

`ui.select` on the viewport takes, in this order:

- `filter`: what a pick selects: `faces`, `edges`, `vertices`, `bodies` (geometry), `nodes`,
  `element_faces`, `elements` (the mesh). The selection bar at the top of the viewport shows it.
- `box_mode`: `true` or `false` turns box selection on or off (a left drag draws the box).
- `mode`: how the items change the selection: `replace` (default), `add`, `remove`, `toggle`.
- `clear: true`: empty the selection first.
- `geometry`, `nodes`, `elements`, `faces`: items by name and label, as `model.set.create`
  takes them (at most 100 000 in each list; labels from 1).
- `at`: `[x, y]` in window points: pick there as a click does, with the filter (nothing under
  the point selects nothing; with `replace` it clears the selection).
- `box`: `[x0, y0, x1, y1]` in window points: the visible nodes, element faces or elements
  inside the box (other filters refuse a box). Faces in front hide what is behind them.

A selection holds at most 100 000 items; past that it is cut and `capped` is true. Picking
runs on the CPU against the triangles the viewport draws (on the GPU too), with a watertight
ray–triangle test: the nearest face wins, nodes, vertices and edges are the nearest on screen
within a few pixels that are not hidden, and ties go to the lower index, so the same pick
gives the same answer every time.

Command forms that apply to part of the model have a Target row with Use selection
(`ui.dialog {"use_selection": true}`): `model.set.create` and `model.set.edit` take the selection
as members; `model.boundary_condition.*`, `model.load.*`, `model.section.*` and
`model.coupling.*` get a set made from it (`model.set.create`, named after the definition:
`Clamp-set`, then `Clamp-set-2`…) in `target: {"set"}` or `set`. A right click in the viewport
offers Create set from selection, which opens `model.set.create`'s form filled in.

```text
{"id":1,"method":"ui.view","params":{"view":"right"}}
{"id":2,"method":"ui.select","params":{"filter":"element_faces","box":[300,120,1100,700]}}
→ {"id":2,"ok":true,"result":{"count":8,"params":{"faces":[{"element":20,"face":4},…]},…}}
{"id":3,"method":"ui.start","params":{"command":"model.set.create"}}
{"id":4,"method":"ui.dialog","params":{"params":{"name":"end"},"use_selection":true,"run":true}}
```

## Files

User clicks (title bar File menu, toolbar entries, Cmd/Ctrl+O, S, Shift+S), drops on the window and
files named on the command line go through native file pickers and open in the background
(`ui.inspect`'s `busy`); commands sent here take their paths as parameters and run at once
(`file.open`, `file.save_as`, `geometry.open`, `geometry.import.mesh`, `deck.write`).
