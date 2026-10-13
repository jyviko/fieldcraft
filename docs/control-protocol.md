# Control protocol

`modelcraft --control PORT` (or `MODELCRAFT_CONTROL_PORT=PORT`) listens on `127.0.0.1:PORT` for
JSON lines. Each request is one line `{"id": …, "method": "…", "params": {…}}`; each reply is one
line `{"id": …, "ok": true, "result": …}` or `{"id": …, "ok": false, "error": "…"}`. Requests are
handled on the UI thread between frames; pointer and keyboard input is injected as real egui
events, one pointer event per frame. The protocol has the same shape as SolidCraft's.

```text
modelcraft --sample --control 7070
{"id":1,"method":"engine.execute","params":{"command":"model.info","params":{}}}
→ {"id":1,"ok":true,"result":{"elements":{"hex8":64,"tet4":162},"nodes":199}}
{"id":2,"method":"ui.render","params":{"path":"/tmp/model.png","width":900,"height":600}}
```

The channel is untrusted input. A request line may be at most 16 MiB; a longer one is answered
with `{"ok": false, "error": "request too large"}` and the connection is closed, as are
connections that send eight lines that are not JSON. At most 16 connections are served at once,
at most 64 queued requests run per frame so a flood cannot starve drawing, and a request the UI
thread has not answered within 60 seconds gets `{"ok": false, "error": "timeout"}`. Numbers are
checked and clamped (image sizes, drag steps, wheel lines, window size).

## Engine

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `command`: command id, `params`: JSON object | the command's result |
| `engine.script` | `commands`: `[{command, params}, …]` | stops at the first error; `journal`: commands that changed the model so far |
| `engine.commands` | — | every command: `id`, `label`, `params` (the parameter documentation) |

Commands never open forms when run this way. `modelcraft-cli commands` prints the same registry.

## UI

| Method | Params | Notes |
|---|---|---|
| `ui.inspect` | — | view settings (`ui`), camera, viewport rect (points), renderer (`gpu`/`cpu`) and adapter, open command form, status bar, node and element counts, version |
| `ui.set` | any view setting: `style` (`shaded_edges`, `shaded`, `wireframe`), `show_nodes`, `perspective`, `dark`, `show_tree`, `show_commands` | an unknown setting or value is an error; returns the settings |
| `ui.view` | `view`: `front`, `back`, `left`, `right`, `top`, `bottom`, `iso`, `home` (iso and fit), `fit` | returns the camera |
| `ui.start` | `command` | like clicking the command in the Commands panel: a command without parameters runs, one with parameters opens its form |
| `ui.dialog` | `params?` (JSON object, fills the form), `run?`, `cancel?` | drives the open command form; `run` returns the command's result and closes the form on success |
| `ui.click` | `x`, `y` (screen points), `button?`: `left`/`right`/`middle`, `shift?`, `ctrl?`, `alt?`, `double?` | real pointer input |
| `ui.move` | `x`, `y` | |
| `ui.drag` | `x0`, `y0`, `x1`, `y1`, `button?`, `shift?`, `ctrl?`, `steps?` (1–200) | press, move, release: in the viewport a left drag orbits, a right or middle drag (or Shift + left) pans |
| `ui.scroll` | `x`, `y`, `delta?` (wheel lines, −100–100; positive zooms in) | zooms at the point |
| `ui.key` | `key` (egui key name, `Enter`, `Escape`…), `cmd?`, `shift?` | `F` over the viewport frames the model |
| `ui.text` | `text` | typed text (into the focused field) |
| `ui.worldToScreen` | `points`: `[[x, y, z]…]` (at most 1000) | screen points in the viewport drawn last frame (null when not on screen) |
| `ui.resize` | `width`, `height` | window inner size in points |
| `ui.screenshot` | `path?` | PNG of the window, taken a few frames later; needs a presented frame (a hidden window or a locked screen answers with an error: use `ui.render`). Without `path`: `modelcraft-shot.png` in the temporary folder |
| `ui.render` | `path`, `width?` (default 1280), `height?` (default 800), each 16–8192 | CPU render of the model with the current camera and view settings (no window or GPU needed) |
| `app.quit` | — | closes the window |
