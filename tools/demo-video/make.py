#!/usr/bin/env python3
"""Regenerate the README demo video: an agent analyses an I-beam cantilever in FieldCraft over MCP.

Every analysis step goes through the MCP server (`fieldcraft-cli mcp --connect`), bridged to
the FieldCraft desktop app, and every frame is its window captured offscreen through the MCP
`screenshot` tool (`source: window`). A caption bar names the tool behind each stage
(caption.py draws it; ffmpeg's `overlay` filter composites it). See README.md beside this file.

Python standard library only; ffmpeg encodes the frames.
"""

import argparse
import base64
import json
import math
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import caption

ROOT = Path(__file__).resolve().parents[2]

# The analysis: a cantilever I-beam lying along x (tests/corpus/cantilever-i-beam: 40 x 40 mm,
# 3 mm flanges, 2 mm web upright, 200 mm long), clamped at x = 0 with 1000 N down (-z) on its free
# end, so it bends downward on screen (z is up).
PART = "cantilever-i-beam"
BODY = "Beam"
ROOT_FACE = "F2:start"
TIP_FACE = "F2:top"
LOAD_N = 1000.0
LENGTH_MM = 200.0
DEPTH_MM = 40.0  # overall; the flanges are 40 wide and 3 thick, the web 2 thick
FLANGE_MM = 3.0
WEB_MM = 2.0
YOUNG_MPA = 210000.0
POISSON = 0.3
SIZE_MM = 4.0
STEP = "Load"

# Frames per second of the video; each stage holds its picture for a number of frames.
FPS = 10
HOLD = 25  # a workflow stage (2.5 s, long enough to read its caption and the browser)
HOLD_SHORT = 20  # one of the model definition steps
HOLD_LONG = 30  # the stress result
SWING = 15  # pictures from undeformed to the largest exaggeration, eased
MAX_SCALE = 150.0  # the largest displacement (0.18 mm) drawn about 26 mm on the 200 mm beam
# The results camera: the iso orientation (yaw -45, pitch 35.26 degrees), drawn back from the
# fitted distance (about 300 mm for this beam) so the contour legend over the viewport's corner
# leaves the beam clear. The closing orbit turns it round the beam, one picture per frame.
ISO = (-45.0, 35.26)
RESULTS_DISTANCE_MM = 420.0
ORBIT = 60
ORBIT_YAW = (ISO[0], -165.0)
ORBIT_PITCH = (ISO[1], 20.0)

# The captured window, in points (the app's own pixels-per-point scale on top); the video's size.
WINDOW = (1280, 800)

# The caption bar, a band under the window (so it never hides the model, the legend or a panel).
CAPTION_FONT = ROOT / "crates" / "ui-egui" / "fonts" / "Inter-SemiBold.ttf"
CAPTION_HEIGHT = 48


def log(msg):
    print(f"demo-video: {msg}", file=sys.stderr, flush=True)


def target_dir():
    return Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))


def build():
    cargo = os.environ.get("CARGO", "cargo")
    for args in (["-p", "fieldcraft", "-p", "fieldcraft-cli"], ["-p", "fieldcraft-testkit", "--bin", "corpus"]):
        subprocess.run([cargo, "build", "--release", *args], cwd=ROOT, check=True)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_port(port, app, timeout=180.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if app.poll() is not None:
            raise SystemExit(f"FieldCraft exited with {app.returncode} before its control channel opened")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1.0):
                return
        except OSError:
            time.sleep(0.5)
    raise SystemExit(f"FieldCraft's control channel on port {port} did not open within {timeout:.0f} s")


class Mcp:
    """A JSON-RPC client of `fieldcraft-cli mcp` over stdio."""

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
        self.font = caption.Font.load(CAPTION_FONT)
        self.captions = {}  # (tool, detail) -> its picture
        self.caption = None
        self.shots = []  # (file, frames, caption picture)

    def say(self, tool, detail):
        """Caption the next pictures with the MCP tool being called and what it does."""
        key = (tool, detail)
        if key not in self.captions:
            path = self.dir / f"caption{len(self.captions):03d}.png"
            path.write_bytes(caption.caption_bar(self.font, (WINDOW[0], CAPTION_HEIGHT), tool, detail))
            self.captions[key] = path
        self.caption = self.captions[key]

    def shoot(self, frames, view=None, camera=None):
        if self.caption is None:
            raise SystemExit("film: a picture without a caption")
        args = {"source": "window", "width": WINDOW[0], "height": WINDOW[1]}
        if view:
            args["view"] = view
        if camera:
            args["camera"] = camera
        content = self.mcp.tool("screenshot", args)
        png = next((c["data"] for c in content if c.get("type") == "image"), None)
        if png is None:
            raise SystemExit("screenshot: no image in the reply")
        path = self.dir / f"shot{len(self.shots):04d}.png"
        path.write_bytes(base64.b64decode(png))
        self.shots.append((path, frames, self.caption))

    def hold(self, frames):
        """Hold the last picture, under the current caption, when the window has not changed."""
        if not self.shots:
            raise SystemExit("film: nothing to hold")
        self.shots.append((self.shots[-1][0], frames, self.caption))

    def sequence(self, folder):
        """Lay the video out as numbered frames and their captions (links to the pictures);
        return the two patterns and the frame count."""
        shutil.rmtree(folder, ignore_errors=True)
        (folder / "captions").mkdir(parents=True)
        n = 0
        for shot, frames, bar in self.shots:
            for _ in range(frames):
                os.link(shot, folder / f"{n:05d}.png")
                os.link(bar, folder / "captions" / f"{n:05d}.png")
                n += 1
        return str(folder / "%05d.png"), str(folder / "captions" / "%05d.png"), n


def hand_calculation():
    """Beam theory for the cantilever: tip deflection in bending (P L^3 / 3 E I) plus shear
    (P L / G A_web, the web carrying the shear) and the bending stress at the root (P L c / I)."""
    b, h, tf, tw = 40.0, DEPTH_MM, FLANGE_MM, WEB_MM
    inertia = (b * h**3 - (b - tw) * (h - 2 * tf) ** 3) / 12.0
    shear_modulus = YOUNG_MPA / (2.0 * (1.0 + POISSON))
    bending = LOAD_N * LENGTH_MM**3 / (3.0 * YOUNG_MPA * inertia)
    shear = LOAD_N * LENGTH_MM / (shear_modulus * (h - tf) * tw)
    return {"bending_mm": bending, "shear_mm": shear, "tip_mm": bending + shear, "root_stress_mpa": LOAD_N * LENGTH_MM * (h / 2) / inertia}


def root_bending_stress(mcp, work):
    """The bending stress at the root from the solution: stress_xx along the top flange's upper
    face, averaged across it at each station, fitted by a line between one beam depth from either
    end (where beam theory holds) and extrapolated to x = 0. At the root itself the clamp, which
    stops the section's Poisson contraction, adds a local peak beam theory does not have.

    The node coordinates come from the Abaqus deck (`write_deck`); the stresses from `probe`."""
    deck = work / "ibeam.inp"
    mcp.call("write_deck", {"format": "abaqus", "path": str(deck)})
    stations = {}
    in_nodes = False
    for line in deck.read_text().splitlines():
        if line.startswith("*"):
            in_nodes = line.upper().split(",")[0].strip() == "*NODE"
            continue
        fields = [f.strip() for f in line.split(",")]
        if in_nodes and len(fields) >= 4:
            label, x, z = int(fields[0]), float(fields[1]), float(fields[3])
            if abs(z - DEPTH_MM) < 1e-6 and DEPTH_MM <= x <= LENGTH_MM - DEPTH_MM:
                stations.setdefault(x, []).append(label)
    if len(stations) < 2:
        raise SystemExit(f"root stress: {len(stations)} stations on the top flange in {deck}")
    probed = mcp.call("probe", {"step": STEP, "nodes": [n for nodes in stations.values() for n in nodes]})
    sxx = {r["node"]: r["stress"][0] for r in probed["nodes"]}
    xs = sorted(stations)
    ys = [sum(sxx[n] for n in stations[x]) / len(stations[x]) for x in xs]
    mx, my = sum(xs) / len(xs), sum(ys) / len(ys)
    slope = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)
    return my - slope * mx


def against_beam_theory(result):
    """The solution against beam theory: tip deflection within 3 %, root stress within 2 %."""
    hand = hand_calculation()
    result["hand_calculation"] = hand
    for got, want, tol, what in (
        (result["tip_deflection_mm"], hand["tip_mm"], 0.03, "tip deflection (mm)"),
        (result["root_stress_mpa"], hand["root_stress_mpa"], 0.02, "root bending stress (MPa)"),
    ):
        log(f"{what}: {got:.4f}, beam theory {want:.4f} ({(got - want) / want:+.1%})")
        if abs(got - want) > tol * want:
            raise SystemExit(f"{what} {got:.4f} is more than {tol:.0%} from beam theory's {want:.4f}")
    return result


def ease(i, n):
    """Step i of n of a cosine ease, 0 to 1: slow out, slow in."""
    return (1 - math.cos(math.pi * i / n)) / 2


def batch(mcp, commands):
    reply = mcp.call("batch", {"commands": commands})
    if not reply.get("ok"):
        raise SystemExit(f"batch failed: {reply.get('failed')}")


def analyse(mcp, film, design, work):
    """The analysis, every step an MCP tool call, with the pictures a viewer follows."""
    # 1. Open the part.
    film.say("open", f"tests/corpus/{PART}")
    mcp.call("open", {"path": str(design)})
    film.shoot(HOLD, view="iso")

    # 2. Hex mesh: the extruded section sweeps into structured hexahedra.
    film.say("mesh", f"hex sweep {SIZE_MM:g} mm")
    mesh = mcp.call("mesh", {"method": "hex_sweep", "params": {"body": BODY, "size": SIZE_MM}})
    log(f"mesh: {mesh['mesh']['elements']} elements, {mesh['mesh']['nodes']} nodes")
    film.shoot(HOLD)

    # 3. Material and section, then supports, then the load and the step: one picture each, so
    # the browser's counts grow as the model is defined.
    film.say("batch", "steel, solid section")
    batch(mcp, [
        {"command": "model.units", "params": {"system": "mm-t-s-N-MPa"}},
        {"command": "model.set.create", "params": {"name": "beam", "geometry": [{"body": BODY}]}},
        {"command": "model.material.create", "params": {"name": "Steel", "elastic": {"type": "isotropic", "young": YOUNG_MPA, "poisson": POISSON}, "density": 7.85e-9}},
        {"command": "model.section.create", "params": {"name": "solid", "set": "beam", "material": "Steel", "kind": {"type": "solid"}, "formulation": {"incompatible_modes": True}}},
    ])
    film.shoot(HOLD_SHORT)
    film.say("batch", "clamp at x = 0")
    batch(mcp, [
        {"command": "model.set.create", "params": {"name": "root", "geometry": [{"body": BODY, "face": ROOT_FACE}]}},
        {"command": "model.boundary_condition.create", "params": {"name": "clamp", "type": "fixed", "target": {"set": "root"}}},
    ])
    film.shoot(HOLD_SHORT)
    film.say("batch", f"{LOAD_N:g} N tip load, static step")
    batch(mcp, [
        # The end section stays plane: a kinematic coupling to a point on the beam's axis carries
        # the load, so no load-introduction peak hides the root stress.
        {"command": "model.set.create", "params": {"name": "tip", "geometry": [{"body": BODY, "face": TIP_FACE}]}},
        {"command": "model.reference_point.create", "params": {"name": "end", "xyz": [LENGTH_MM, 0, DEPTH_MM / 2]}},
        {"command": "model.coupling.create", "params": {"name": "end_plate", "reference_point": "end", "set": "tip", "kind": "kinematic"}},
        {"command": "model.load.create", "params": {"name": "tip_load", "type": "force", "target": {"reference_point": "end"}, "vector": [0, 0, -LOAD_N]}},
        {"command": "model.step.create", "params": {"name": STEP, "procedure": {"type": "static"}, "boundary_conditions": ["clamp"], "loads": ["tip_load"]}},
    ])
    check = mcp.call("check_model")
    if not check.get("ready"):
        raise SystemExit(f"the model is not ready: {check}")
    film.shoot(HOLD_SHORT)
    film.say("check_model", "ready")
    film.hold(HOLD_SHORT)

    # 4. Solve.
    film.say("solve", "static")
    solve = mcp.call("solve", {"analysis": "static", "step": STEP})
    log(f"solve: {solve.get('equations')} equations")
    film.shoot(HOLD_SHORT)

    # 5. Results: the tip deflection, then von Mises contours on the undeformed shape.
    disp = mcp.call("show_results", {"step": STEP, "field": "displacement"})
    tip = disp["contour"]["range"]["max"]["value"]
    root_stress = root_bending_stress(mcp, work)
    film.say("show_results", "von Mises")
    vm = mcp.call("show_results", {"step": STEP, "field": "von_mises", "bands": 12, "deformed": True, "scale": 0})
    peak = vm["contour"]["range"]["max"]["value"]
    log(f"tip deflection {tip:.4f} mm, peak von Mises {peak:.1f} MPa")
    film.shoot(HOLD_LONG, camera={"yaw": ISO[0], "pitch": ISO[1], "distance": RESULTS_DISTANCE_MM})

    # 6. The deformed shape, swung from undeformed to the exaggeration and back, then out again.
    eased = [MAX_SCALE * ease(i, SWING) for i in range(1, SWING + 1)]
    for scale in eased + eased[-2::-1] + [0.0] + eased:
        film.say("show_results", f"deformed \u00d7{scale:.0f}")
        mcp.call("show_results", {"step": STEP, "deformed": True, "scale": scale})
        film.shoot(1)
    film.hold(HOLD_SHORT)

    # 7. A slow orbit round the deformed beam, one `screenshot` camera angle per frame.
    film.say("screenshot", "camera orbit")
    for i in range(1, ORBIT + 1):
        t = ease(i, ORBIT)
        yaw = ORBIT_YAW[0] + t * (ORBIT_YAW[1] - ORBIT_YAW[0])
        pitch = ORBIT_PITCH[0] + t * (ORBIT_PITCH[1] - ORBIT_PITCH[0])
        film.shoot(1, camera={"yaw": yaw, "pitch": pitch})
    film.hold(HOLD)
    return against_beam_theory({"tip_deflection_mm": tip, "root_stress_mpa": root_stress, "peak_von_mises_mpa": peak, "elements": mesh["mesh"]["elements"], "nodes": mesh["mesh"]["nodes"]})


def encode(film, frames_dir, mp4, gif):
    ffmpeg = os.environ.get("FFMPEG", shutil.which("ffmpeg") or "ffmpeg")
    pattern, captions, count = film.sequence(frames_dir / "sequence")
    log(f"encoding {len(film.shots)} pictures as {count} frames, {count / FPS:.1f} s")
    # The window pictures scaled to the window's size in points, with their caption bars laid
    # over a band added below.
    src = ["-framerate", str(FPS), "-i", pattern, "-framerate", str(FPS), "-i", captions]
    width, height = WINDOW
    captioned = (f"[0:v]scale={width}:{height}:flags=lanczos,pad={width}:{height + CAPTION_HEIGHT}:0:0[w];"
                 f"[w][1:v]overlay=0:{height}")
    common = [ffmpeg, "-hide_banner", "-loglevel", "error", "-y"]
    mp4.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [*common, *src, "-filter_complex", captioned, "-c:v", "libx264", "-preset", "veryslow", "-crf", "26",
         "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(mp4)],
        check=True,
    )
    subprocess.run(
        [*common, *src, "-filter_complex",
         f"{captioned},scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];"
         "[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle", str(gif)],
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
    work = Path(tempfile.mkdtemp(prefix="fieldcraft-demo-"))
    frames_dir = args.frames.resolve() if args.frames else work / "frames"
    frames_dir.mkdir(parents=True, exist_ok=True)
    design = work / "ibeam.solidcraft"
    subprocess.run([str(bin_dir / "corpus"), "--part", PART, "--design", str(design)], check=True)

    port = free_port()
    app_log = open(work / "app.log", "w")
    app = subprocess.Popen([str(bin_dir / "fieldcraft"), "--control", str(port)], stdout=app_log, stderr=subprocess.STDOUT)
    mcp = None
    try:
        wait_for_port(port, app)
        mcp = Mcp(bin_dir / "fieldcraft-cli", port)
        film = Film(mcp, frames_dir)
        result = analyse(mcp, film, design, work)
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
