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
| `engine.commands` | — | every command: `id`, `label`, `params` (the parameter documentation) |

Commands never open forms or file pickers when run this way. `fieldcraft-cli commands` prints the
same registry. `file.sample {name}` opens a sample (`parts`, `cantilever`, `impact`) and
`solve.run {}` solves every step; the Results workspace comes up when a result appears.

## UI

| Method | Params | Notes |
|---|---|---|
| `ui.inspect` | — | view settings (`ui`), camera, viewport rect (points), renderer (`gpu`/`cpu`) and adapter, open command form, status bar, node and element counts, the results display (`results`: what the `results.*` commands reply: view, `stale`, contour range, modes), the probed node (`probe`), `busy` (the background job: a file opening, a solve), `prompt` (`{"unsaved": path}`, `"recover"`, `"samples"`), `recent` (recent files), `selected` (the browser's selected definition), `nav` (the left-drag mode), `layout` (each region of the window drawn last frame as `[x, y, w, h]` in points: `app_bar`, `ribbon`, `workspace_switcher`, `document_tab`, `app_bar:<action>`, `browser`, `viewport`, `nav_bar`, `axis_view`, `context`, `command_line`, `timeline`, `toolbar_dropdown`, `palette`), version |
| `ui.set` | any view setting: `style` (`shaded_edges`, `shaded`, `wireframe`), `show_nodes`, `show_geometry`, `show_mesh`, `perspective`, `dark`, `show_tree` (the browser), `show_timeline`, `workspace` (`prepare`, `model`, `analysis`, `results`), `tab` (the toolbar tab: `geometry`, `mesh`, `properties`, `assembly`, `steps`, `loads`, `interactions`, `solve`, `results`, `tools`; a tab of another workspace brings that workspace), `palette` (the command search open), `menu` (the open menu: `file`, `help`, `workspace`, `view`, `more`, or a panel's drop-down as `<tab>/<panel>` such as `mesh/Tet`; `null` closes it) | an unknown setting or value is an error; returns the settings |
| `ui.view` | `view`: `front`, `back`, `left`, `right`, `top`, `bottom`, `iso`, `home` (iso and fit), `fit` | returns the camera |
| `ui.camera` | `yaw`, `pitch` (degrees), `distance?` (model units) | turns the camera to any orientation about its target: `yaw` about Z (0 looks along +Y, the front view; −45 is the iso view's, −90 the right view's), `pitch` the elevation above the XY plane; `distance` moves the eye (the zoom of an orthographic view). Angles must be finite and the distance above 0: the yaw is wrapped, the pitch clamped to ±90 and the distance to 0.001–10⁹; anything else is an error. A sequence of calls orbits smoothly. Returns the camera (`ui.inspect`'s, angles in radians) |
| `ui.toolbar` | — | the ribbon: `workspace`, `workspaces`, `tab`, `tabs` (the workspace's), `menu` (the open menu), `panels` (each panel label's `menu` key and `rect`), and every catalogue entry (docs/features.md) with `id`, `label`, `tab`, `panel`, `command` (null while coming soon), `more` (its related commands), `coming_soon` (the milestone, or null), `fit` — `full` (a button), `collapsed` (its panel is one button), `more` (its panel is in the » menu) or `menu` (in its panel's drop-down) — and `rect` (screen points, last frame, on the shown tab; for `menu` entries the row while the drop-down is open) |
| `ui.start` | `command`: a command or toolbar entry id | like a click on the toolbar, a drop-down row or a search result: a command without parameters runs, one with parameters opens its form in the context panel; an entry that is coming soon is an error naming its milestone. Never opens a native file picker: file entries (open, save as, write deck) open their command's form |
| `ui.select` | `kind` (`set`, `material`, `section`, `step`, `boundary_condition`, `load`, `reference_point`, `coupling`, `fluid_domain`), `name`; nothing to deselect | selects a definition as a browser click does; its properties (with Edit and Delete) show in the context panel |
| `ui.prompt` | `answer`: for `unsaved` `save`, `discard` or `cancel`; for `recover` `recover` (with `id` from `file.recovery.list`) or `close`; for `samples` `open` (with `name`) or `close` | answers the window's question (`ui.inspect`'s `prompt`) |
| `ui.dialog` | `params?` (JSON object, fills the form), `run?`, `cancel?` | drives the open command form; `run` returns the command's result and closes the form on success |
| `ui.click` | `x`, `y` (screen points), `button?`: `left`/`right`/`middle`, `shift?`, `ctrl?`, `alt?`, `double?` | real pointer input; a click on a results plot probes the nearest node of the face under it (`solve.probe`; `ui.inspect` shows the values) |
| `ui.move` | `x`, `y` | |
| `ui.drag` | `x0`, `y0`, `x1`, `y1`, `button?`, `shift?`, `ctrl?`, `steps?` (1–200) | press, move, release: in the viewport a left drag orbits, a right or middle drag (or Shift + left) pans |
| `ui.scroll` | `x`, `y`, `delta?` (wheel lines, −100–100; positive zooms in) | zooms at the point |
| `ui.key` | `key` (egui key name, `Enter`, `Escape`…), `cmd?`, `shift?` | `F` over the viewport frames the model |
| `ui.text` | `text` | typed text (into the focused field) |
| `ui.worldToScreen` | `points`: `[[x, y, z]…]` (at most 1000) | screen points in the viewport drawn last frame (null when not on screen) |
| `ui.resize` | `width`, `height` | window inner size in points |
| `ui.fullscreen` | `on?` (default true) | into or out of full screen; on macOS the title bar's left group moves into the traffic lights' room while there are none |
| `ui.screenshot` | `path?` | PNG of the window, taken a few frames later; waits for a presented frame; when none comes within 8 seconds (a hidden window, a locked screen) it answers with `ui.capture`'s offscreen picture (`"offscreen": true`). Without `path`: `fieldcraft-shot.png` in the temporary folder |
| `ui.capture` | `path?`, `width?`, `height?` (points; default the window's), `scale?` (pixels per point, 0.5–4; default the window's) | PNG of the whole window (title bar, ribbon, open menus and search, panels, forms and the viewport) drawn offscreen, without fade-in animations: the UI pass runs in a separate egui context and is rasterised on the CPU, so it works with the window hidden, minimised or the screen locked. At most 8192 pixels a side and 16 megapixels. Without `path`: `fieldcraft-shot.png` in the temporary folder |
| `ui.render` | `path`, `width?` (default 1280), `height?` (default 800): whole pixels, each 1–8192 and at most 16 megapixels in all | CPU render of the model with the current camera and view settings (no window or GPU needed). A size out of range, or a scene over 20 million primitives, is an error |
| `app.quit` | — | closes the window |

## Files

User clicks (title bar File menu, toolbar entries, Cmd/Ctrl+O, S, Shift+S), drops on the window and
files named on the command line go through native file pickers and open in the background
(`ui.inspect`'s `busy`); commands sent here take their paths as parameters and run at once
(`file.open`, `file.save_as`, `geometry.open`, `geometry.import.mesh`, `deck.write`).
