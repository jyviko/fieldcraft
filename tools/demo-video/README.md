# Demo video

`make.py` regenerates the README's demo, `docs/media/ibeam-analysis.mp4` and
`docs/media/ibeam-analysis.gif`: an agent analyses an I-beam cantilever over MCP while the
desktop app shows each step.

```sh
python3 tools/demo-video/make.py                  # build, run, encode into docs/media
python3 tools/demo-video/make.py --no-build       # with the release binaries already built
python3 tools/demo-video/make.py --frames /tmp/f  # also keep the captured pictures
```

Needs Python 3 (standard library only), a display for the app window (the pictures are drawn
offscreen, so a locked screen is fine) and `ffmpeg` with libx264 on the `PATH` (or `FFMPEG`).
It honours `CARGO_TARGET_DIR`.

What it does:

1. Builds `fieldcraft`, `fieldcraft-cli` and the testkit's `corpus` tool (release).
2. Writes the corpus I-beam (`tests/corpus/i-beam`) as a `.solidcraft` design to a temporary
   folder with `corpus --part i-beam --design`.
3. Starts the app with `--control PORT` and `fieldcraft-cli mcp --connect 127.0.0.1:PORT`, and
   speaks MCP (JSON-RPC over stdio) to it. Every step is an MCP tool call: `open` the design,
   `mesh` (`hex_sweep`, 4 mm), `batch` (steel, a solid section, the root clamp, a kinematic
   coupling of the free end to a reference point loaded with 1000 N), `check_model`, `solve`
   (`static`), and `show_results` for the von Mises contours and the deformed shape, its `scale`
   stepped from 1 to 150 and back. Every picture is the MCP `screenshot` tool with
   `source: window` (the whole window drawn offscreen, `ui.capture`), turned to the `iso` or
   `right` view by its `view` argument.
4. Encodes the pictures at 10 frames per second, each held for its stage, into the MP4 (H.264,
   1280 px) and the GIF (960 px, its own palette), and prints the tip deflection and the peak
   von Mises stress.

The theme is the app's default (dark). Beam theory for the same beam gives a tip deflection of
0.143 mm in bending plus about 0.034 mm in shear (0.177 mm) and a root bending stress of 45 MPa;
the clamped root adds a local peak above that.
