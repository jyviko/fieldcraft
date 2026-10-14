#!/usr/bin/env python3
"""Regenerate the README demo video: an I-beam cantilever analysed by an agent over MCP.

Every analysis step goes through the MCP server (`modelcraft-cli mcp --connect`), bridged to
the desktop app, and every frame is the app window captured offscreen through the MCP
`screenshot` tool (`source: window`). See README.md beside this file.

Python standard library only; ffmpeg encodes the frames.
"""

import argparse
import base64
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# The analysis: the corpus I-beam (tests/corpus/i-beam, 40 x 40 mm, 3 mm flanges, 2 mm web,
# 200 mm long along z) clamped at z = 0 with 1000 N down (-y) on its free end.
PART = "i-beam"
BODY = "Beam"
ROOT_FACE = "F2:start"
TIP_FACE = "F2:top"
LOAD_N = 1000.0
SIZE_MM = 4.0
STEP = "Load"

# Frames per second of the video; each stage holds its picture for a number of frames.
FPS = 10
HOLD = 14  # a workflow stage (1.4 s)
HOLD_SHORT = 10  # one of the model definition steps
HOLD_LONG = 20  # the stress result
SWING = 12  # pictures from undeformed to the largest exaggeration
MAX_SCALE = 150.0  # the largest displacement (0.18 mm) drawn about 26 mm on the 200 mm beam

# The captured window, in points (the app's own pixels-per-point scale on top).
WINDOW = (1280, 800)


def log(msg):
    print(f"demo-video: {msg}", file=sys.stderr, flush=True)


def target_dir():
    return Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))


def build():
    cargo = os.environ.get("CARGO", "cargo")
    for args in (["-p", "modelcraft", "-p", "modelcraft-cli"], ["-p", "modelcraft-testkit", "--bin", "corpus"]):
        subprocess.run([cargo, "build", "--release", *args], cwd=ROOT, check=True)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_port(port, app, timeout=180.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if app.poll() is not None:
            raise SystemExit(f"the app exited with {app.returncode} before its control channel opened")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1.0):
                return
        except OSError:
            time.sleep(0.5)
    raise SystemExit(f"the app's control channel on port {port} did not open within {timeout:.0f} s")


class Mcp:
    """A JSON-RPC client of `modelcraft-cli mcp` over stdio."""

    def __init__(self, cli, port):
        self.proc = subprocess.Popen(
            [str(cli), "mcp", "--connect", f"127.0.0.1:{port}"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
        )
        self.next_id = 0
        info = self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "demo-video", "version": "1"}})
        self.notify("notifications/initialized")
        log(f"MCP server: {info.get('serverInfo', {})}")

    def notify(self, method):
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n")
        self.proc.stdin.flush()

    def request(self, method, params):
        self.next_id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            raise SystemExit(f"the MCP server closed its output ({method})")
        reply = json.loads(line)
        if "error" in reply:
            raise SystemExit(f"{method}: {reply['error']}")
        return reply["result"]

    def tool(self, name, arguments=None):
        """Call a tool; return its content blocks (an error result stops the run)."""
        result = self.request("tools/call", {"name": name, "arguments": arguments or {}})
        content = result.get("content", [])
        if result.get("isError"):
            text = " ".join(c.get("text", "") for c in content)
            raise SystemExit(f"{name} failed: {text}")
        return content

    def call(self, name, arguments=None):
        """Call a tool whose reply is JSON text."""
        content = self.tool(name, arguments)
        return json.loads(content[0]["text"]) if content else None

    def close(self):
        self.proc.stdin.close()
        self.proc.wait(timeout=30)


class Film:
    """The frames of the video: each screenshot is written once and held for a number of frames."""

    def __init__(self, mcp, frames_dir):
        self.mcp = mcp
        self.dir = frames_dir
        self.shots = []  # (file, frames)

    def shoot(self, frames, view=None):
        args = {"source": "window", "width": WINDOW[0], "height": WINDOW[1]}
        if view:
            args["view"] = view
        content = self.mcp.tool("screenshot", args)
        png = next((c["data"] for c in content if c.get("type") == "image"), None)
        if png is None:
            raise SystemExit("screenshot: no image in the reply")
        path = self.dir / f"shot{len(self.shots):04d}.png"
        path.write_bytes(base64.b64decode(png))
        self.shots.append((path, frames))

    def sequence(self, folder):
        """Lay the video out as numbered frames (links to the shots); return their pattern and count."""
        shutil.rmtree(folder, ignore_errors=True)
        folder.mkdir(parents=True)
        n = 0
        for shot, frames in self.shots:
            for _ in range(frames):
                os.link(shot, folder / f"{n:05d}.png")
                n += 1
        return str(folder / "%05d.png"), n


def batch(mcp, commands):
    reply = mcp.call("batch", {"commands": commands})
    if not reply.get("ok"):
        raise SystemExit(f"batch failed: {reply.get('failed')}")


def analyse(mcp, film, design):
    """The analysis, every step an MCP tool call, with the pictures a viewer follows."""
    # 1. Open the part.
    mcp.call("open", {"path": str(design)})
    film.shoot(HOLD, view="iso")

    # 2. Hex mesh: the extruded section sweeps into structured hexahedra.
    mesh = mcp.call("mesh", {"method": "hex_sweep", "params": {"body": BODY, "size": SIZE_MM}})
    log(f"mesh: {mesh['mesh']['elements']} elements, {mesh['mesh']['nodes']} nodes")
    film.shoot(HOLD)

    # 3. Material and section, then supports, then the load and the step: one picture each, so
    # the browser's counts grow as the model is defined.
    batch(mcp, [
        {"command": "model.units", "params": {"system": "mm-t-s-N-MPa"}},
        {"command": "model.set.create", "params": {"name": "beam", "geometry": [{"body": BODY}]}},
        {"command": "model.material.create", "params": {"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}, "density": 7.85e-9}},
        {"command": "model.section.create", "params": {"name": "solid", "set": "beam", "material": "Steel", "kind": {"type": "solid"}, "formulation": {"incompatible_modes": True}}},
    ])
    film.shoot(HOLD_SHORT)
    batch(mcp, [
        {"command": "model.set.create", "params": {"name": "root", "geometry": [{"body": BODY, "face": ROOT_FACE}]}},
        {"command": "model.boundary_condition.create", "params": {"name": "clamp", "type": "fixed", "target": {"set": "root"}}},
    ])
    film.shoot(HOLD_SHORT)
    batch(mcp, [
        # The end section stays plane: a kinematic coupling to a point on the beam's axis carries
        # the load, so no load-introduction peak hides the root stress.
        {"command": "model.set.create", "params": {"name": "tip", "geometry": [{"body": BODY, "face": TIP_FACE}]}},
        {"command": "model.reference_point.create", "params": {"name": "end", "xyz": [0, 20, 200]}},
        {"command": "model.coupling.create", "params": {"name": "end_plate", "reference_point": "end", "set": "tip", "kind": "kinematic"}},
        {"command": "model.load.create", "params": {"name": "tip_load", "type": "force", "target": {"reference_point": "end"}, "vector": [0, -LOAD_N, 0]}},
        {"command": "model.step.create", "params": {"name": STEP, "procedure": {"type": "static"}, "boundary_conditions": ["clamp"], "loads": ["tip_load"]}},
    ])
    check = mcp.call("check_model")
    if not check.get("ready"):
        raise SystemExit(f"the model is not ready: {check}")
    film.shoot(HOLD_SHORT)

    # 4. Solve.
    solve = mcp.call("solve", {"analysis": "static", "step": STEP})
    log(f"solve: {solve.get('equations')} equations")

    # 5. Results: the tip deflection, then von Mises contours on the true (scale 1) shape.
    disp = mcp.call("show_results", {"step": STEP, "field": "displacement"})
    tip = disp["contour"]["range"]["max"]["value"]
    vm = mcp.call("show_results", {"step": STEP, "field": "von_mises", "bands": 12, "deformed": True, "scale": 1})
    peak = vm["contour"]["range"]["max"]["value"]
    log(f"tip deflection {tip:.4f} mm, peak von Mises {peak:.1f} MPa")
    film.shoot(HOLD_LONG, view="iso")

    # 6. The deformed shape, swung from the true shape to an exaggeration and back, in the iso
    # view and then from the side, ending held at the exaggeration.
    ramp = [max(1.0, MAX_SCALE * i / SWING) for i in range(SWING + 1)]
    for view, scales in (("iso", ramp[1:] + ramp[-2::-1]), ("right", ramp[1:])):
        for i, scale in enumerate(scales):
            mcp.call("show_results", {"step": STEP, "deformed": True, "scale": scale})
            film.shoot(1, view=view if i == 0 else None)
    film.shoot(HOLD_LONG)
    return {"tip_deflection_mm": tip, "peak_von_mises_mpa": peak, "elements": mesh["mesh"]["elements"], "nodes": mesh["mesh"]["nodes"]}


def encode(film, frames_dir, mp4, gif):
    ffmpeg = os.environ.get("FFMPEG", shutil.which("ffmpeg") or "ffmpeg")
    pattern, count = film.sequence(frames_dir / "sequence")
    log(f"encoding {len(film.shots)} pictures as {count} frames, {count / FPS:.1f} s")
    src = ["-framerate", str(FPS), "-i", pattern]
    common = [ffmpeg, "-hide_banner", "-loglevel", "error", "-y"]
    mp4.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [*common, *src, "-vf", "scale=1280:-2:flags=lanczos", "-c:v", "libx264", "-preset", "veryslow", "-crf", "26",
         "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(mp4)],
        check=True,
    )
    palette = frames_dir / "palette.png"
    subprocess.run([*common, *src, "-vf", "scale=960:-1:flags=lanczos,palettegen=stats_mode=diff", str(palette)], check=True)
    subprocess.run(
        [*common, *src, "-i", str(palette), "-lavfi",
         "scale=960:-1:flags=lanczos[v];[v][1:v]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle", str(gif)],
        check=True,
    )


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, default=ROOT / "docs" / "media", help="folder for ibeam-analysis.mp4 and .gif")
    ap.add_argument("--frames", type=Path, help="keep the captured pictures in this folder")
    ap.add_argument("--no-build", action="store_true", help="use the binaries already built")
    args = ap.parse_args()

    if not args.no_build:
        build()
    bin_dir = target_dir() / "release"
    work = Path(tempfile.mkdtemp(prefix="modelcraft-demo-"))
    frames_dir = args.frames.resolve() if args.frames else work / "frames"
    frames_dir.mkdir(parents=True, exist_ok=True)
    design = work / "ibeam.solidcraft"
    subprocess.run([str(bin_dir / "corpus"), "--part", PART, "--design", str(design)], check=True)

    port = free_port()
    app_log = open(work / "app.log", "w")
    app = subprocess.Popen([str(bin_dir / "modelcraft"), "--control", str(port)], stdout=app_log, stderr=subprocess.STDOUT)
    mcp = None
    try:
        wait_for_port(port, app)
        mcp = Mcp(bin_dir / "modelcraft-cli", port)
        film = Film(mcp, frames_dir)
        result = analyse(mcp, film, design)
        encode(film, frames_dir, args.out / "ibeam-analysis.mp4", args.out / "ibeam-analysis.gif")
        print(json.dumps(result, indent=2))
    finally:
        if mcp:
            mcp.close()
        app.terminate()
        try:
            app.wait(timeout=10)
        except subprocess.TimeoutExpired:
            app.kill()
        app_log.close()
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
