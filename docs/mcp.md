# FieldCraft and AI agents (MCP)

FieldCraft speaks the [Model Context Protocol](https://modelcontextprotocol.io) over stdio
(JSON-RPC 2.0, one message per line; protocol revisions 2025-06-18, 2025-03-26 and 2024-11-05).
An agent builds and solves models the way a person does: every tool runs engine commands, the
same ones the UI, scripts, the command line and the control channel run, and never opens a
dialog or a file picker. The server is `crates/mcp` (`fieldcraft-mcp`); `fieldcraft-cli mcp`
runs it.

## Start it

```sh
# Headless (the default): an in-process session, no window; screenshots are CPU renders.
fieldcraft-cli mcp
fieldcraft-cli mcp --headless

# Bridged to the running app, so you can watch the agent work:
fieldcraft --control 7070 &
fieldcraft-cli mcp --connect 127.0.0.1:7070
```

With `--connect`, an app that cannot be reached when the server starts gives a headless session
instead (a note goes to stderr). stdout carries only protocol messages; errors go to stderr.
Bridged screenshots are written to a temporary file by the app and read back by the server, so
both must run on the same computer.

### Claude Code

```sh
claude mcp add fieldcraft -- /path/to/fieldcraft-cli mcp
# bridged to a running app:
claude mcp add fieldcraft -- /path/to/fieldcraft-cli mcp --connect 127.0.0.1:7070
# kept to one project folder:
claude mcp add fieldcraft -- /path/to/fieldcraft-cli mcp --automation-read-root /work/bracket --automation-write-root /work/bracket/out
```

Build the binary first (`cargo build --release -p fieldcraft-cli`; it is
`target/release/fieldcraft-cli`). The same setup as a project `.mcp.json`, or in another client's
configuration (Claude Desktop's `claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "fieldcraft": { "command": "/path/to/fieldcraft-cli", "args": ["mcp"] }
  }
}
```

## Confining file access

```sh
fieldcraft-cli mcp --automation-read-root /work/bracket --automation-write-root /work/bracket/out
```

The two flags keep the agent to the folders you give it:

- Every file a command reads must lie inside the **read root**: `open`, `file.open`,
  `geometry.open`, `geometry.import.mesh`, the design and meshes a `.fieldcraft` file opens
  again, `file.recovery.list` and `file.recover` with a `dir`.
- Every file a command writes must lie inside the **write root**: `save`, `file.save` and
  `file.save_as` (also a save to the model's own file), `write_deck` and `deck.write`,
  `screenshot {path}`, `file.recovery.discard` with a `dir`.
- Read and write access are separate. Give the same folder twice for both; a root left out
  grants none of its access (with only `--automation-read-root`, nothing can be written). Without
  either flag nothing is confined.
- A path is made absolute against the server's working folder, then its links are followed: a
  file (or folder) link that leads out of a root is outside it. For a file that does not exist
  yet, its nearest existing folder's links are followed and the rest must be plain names, so
  `..` below a folder that does not exist is refused, as is a link that leads nowhere (writing
  through it would create its target). The result is compared with the root by whole folder
  names: `/work2/x` is not inside `/work`. On Windows device names (`CON`, `NUL`, `COM1`…) are
  refused.
- A refused path is an ordinary tool error the agent can read and correct, for example
  `automation path rejected: outside the write root /work/bracket/out: /etc/x.inp` or
  `automation filesystem access is not granted: write authority is absent: /work/bracket/out/x.inp`.
  A design or mesh a model refers to outside the read root is not opened; the model opens
  without it and says so in `warnings`.
- The app's own recovery folder (autosaves, `file.recovery.*` without `dir`) is not confined:
  no agent names a path there.
- The roots must be existing folders; a missing one stops the server with an error. Each flag
  is given once (`--flag <dir>` or `--flag=<dir>`).

The flags confine the headless server and imply `--headless`; `--connect` with them is an error,
since the server would otherwise drive an app that is not confined.

A path is checked, then opened: another program that swaps a link in between can still win that
race, so the roots keep an agent to the files it was given rather than guarding against other
software on the computer.

## Tools

| Tool | Arguments | What it does |
|---|---|---|
| `list_commands` | `filter?` | Every command with its label and parameter documentation; `filter` matches id and label. |
| `execute` | `command`, `params?` | Run any command (`list_commands`). A failure leaves the model unchanged and names what to fix. |
| `batch` | `commands: [{command, params}]` | Run steps in order, stop at the first error; the reply has every result and the failing step (`failed.index`, `failed.command`, `failed.error`). Steps before it stay applied (`undo` takes them back). |
| `inspect_model` | — | `model.info` (counts, units, definitions), `model.activity` (the step table) and `geometry.bodies` (bodies with measures and persistent face names, which sets name as `{body, face}`). |
| `check_model` | — | `model.check`: errors and warnings, each naming its item. |
| `open` | `path` | By extension: `.fieldcraft` (`file.open`), `.solidcraft` (`geometry.open`), `.3mf` (`geometry.import.mesh`). |
| `save` | `path?` | `file.save`: to `path` or the model's own file. |
| `write_deck` | `format` (abaqus, nastran, lsdyna), `path`, `steps?` | `deck.write`. |
| `mesh` | `method`, `params?` | Runs the mesher (`tet`: `mesh.tet.generate`; `planar`: `mesh.planar`; the hex and block meshers as they land) and replies with its result, `model.info` and `mesh.quality`. |
| `solve` | `analysis` (static, frequency, explicit, fluid; heat as it lands), `step?`, `options?` | `solve.<analysis>` with the step and the solver's caps and settings. |
| `show_results` | `step?`, `field?`, `bands?`, `deformed?`, `scale?`, `mode?`, `frame?`, `time?` | The `results.*` commands its arguments ask for; the reply has the contour range (smallest and largest value and their nodes: `field: "displacement"` gives the largest displacement), modes or frames. |
| `probe` | `step?`, `nodes?`, `elements?`, `points?`, `forces?`, `mode?`, `frame?` | `solve.probe`: stored results at nodes, elements (explicit) or points (fluid). |
| `reactions` | `step?`, `sets?`, `about?` | `results.reactions`: reaction force and moment per node set and in total. |
| `screenshot` | `view?`, `camera?`, `source?` (window, model), `width?`, `height?`, `path?` | A PNG image content block and what it shows. Headless: a CPU render framing the model, the geometry and the results display (deformed shape, contour bands). Bridged: the whole app window drawn offscreen (`ui.capture`, the default) or a model render with the app's camera (`ui.render`). `view` turns the camera to a standard view first; `camera` `{yaw, pitch, distance?}` to any orientation, in degrees (yaw 0 looks along +Y, −45 is the iso view's; pitch is the elevation, clamped to ±90) and model units (`ui.camera`; headless, the picture is framed unless `distance` is given). Give one of the two; the orientation is kept for the next picture, so a sequence of `camera` angles orbits smoothly. |
| `undo`, `redo` | — | `edit.undo`, `edit.redo`. |

Resources: `fieldcraft://model` (`model.info`) and `fieldcraft://commands` (the command catalog).

Errors: an unknown tool, or arguments that do not match the tool's schema (a missing or unknown
argument, a wrong type, a value outside an enum), are JSON-RPC `-32602` errors; an engine failure
is a tool result with `isError: true` and the engine's message.

## Limits

The server treats its input as hostile. A message line may be at most 16 MiB (the control
channel's limit); a longer one is answered with `request too large` and skipped, and the server
keeps serving. A JSON-RPC batch holds at most 100 messages, a `batch` call at most 10 000
commands; JSON nested deeper than 128 levels is a parse error. Picture sides are clamped to
16–4096 pixels. Every command keeps its own caps (element counts, solver degrees of freedom and
factor size, iterations), and an engine failure or panic leaves the model as it was.

## A typical session

```text
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"me","version":"1"}}}
→ {"jsonrpc":"2.0","method":"notifications/initialized"}
→ {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"open","arguments":{"path":"/work/steps.solidcraft"}}}
→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect_model","arguments":{}}}
← … "bodies": [{"name": "Steps", "faces": [{"name": "F1:-x"}, {"name": "F1:+x"}, …]}] …
→ {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"mesh","arguments":{"method":"tet","params":{"body":"Steps","tolerance":0.5}}}}
→ {"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"batch","arguments":{"commands":[
     {"command":"model.units","params":{"system":"mm-t-s-N-MPa"}},
     {"command":"model.set.create","params":{"name":"all","geometry":[{"body":"Steps"}]}},
     {"command":"model.set.create","params":{"name":"root","geometry":[{"body":"Steps","face":"F1:-x"}]}},
     {"command":"model.set.create","params":{"name":"tip","geometry":[{"body":"Steps","face":"F1:+x"}]}},
     {"command":"model.material.create","params":{"name":"Steel","elastic":{"type":"isotropic","young":210000,"poisson":0.3},"density":7.85e-9}},
     {"command":"model.section.create","params":{"name":"solid","set":"all","material":"Steel","kind":{"type":"solid"}}},
     {"command":"model.boundary_condition.create","params":{"name":"clamp","type":"fixed","target":{"set":"root"}}},
     {"command":"model.load.create","params":{"name":"push","type":"force","target":{"set":"tip"},"vector":[0,0,-1000]}},
     {"command":"model.step.create","params":{"name":"Load","procedure":{"type":"static"},"boundary_conditions":["clamp"],"loads":["push"]}}]}}}
→ {"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"solve","arguments":{"analysis":"static","step":"Load"}}}
→ {"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"show_results","arguments":{"step":"Load","field":"displacement"}}}
← … "contour": {"field": "displacement", "range": {"max": {"node": 25, "value": 0.00485}, …}} …
→ {"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"screenshot","arguments":{"view":"iso"}}}
← {"jsonrpc":"2.0","id":8,"result":{"content":[{"type":"image","mimeType":"image/png","data":"iVBORw0…"},{"type":"text","text":"{\"source\":\"model\",…}"}]}}
```

`crates/mcp/tests/workflow.rs` runs this session on a part of the meshing corpus
(`tests/corpus/stepped-block`) and checks the results only through MCP; `crates/mcp/src/tests.rs`
sends malformed messages, huge parameters, paths outside the roots and unknown tools.
