# Pinched cylinder with rigid diaphragms

Our model of the published problem: a cylinder of radius 300, length 600 and thickness 3,
E = 3 × 10⁶, ν = 0.3, capped by rigid diaphragms (u_y = u_z = rot_x = 0 at the ends) and pinched by
two opposite unit forces at mid-length, one on each of two diametrically opposite generators.
Reference: 1.8248 × 10⁻⁵ radial displacement under the load (Belytschko, Stolarski, Liu, Carpenter
and Ong, *Stress projection for membrane and shear locking in shell finite elements*, CMAME 51
(1985) 221–258, the shell obstacle course) — the value the case already cited.

The eighth model: x from the loaded section (x = 0, symmetry in x) to the diaphragm (x = 300), φ
from the loaded generator (φ = 0, symmetry in y) to the side (φ = 90°, symmetry in z), with
R sin φ = y and R cos φ = z on the mid-surface R = 300. A quarter of the unit force, −0.25 in z,
acts on the single loaded node (0, 0, 300) — the model the solver's own pinched-cylinder benchmark
builds in code.

## The mesh

`mesh.block`'s 2D form takes its four corners at z = 0, so it cannot mesh a patch on a cylindrical
surface (it can be curved in its own plane, but it is planar): the recipe writes the 1089 nodes and
1024 quad4 elements one by one with `mesh.node.add` and `mesh.element.add`, the route
[docs/verification.md](../../../docs/verification.md) allows ("nodes and elements written one
by one"). Node 1 + i + 33 j lies at
(300 i/32, 300 sin(90° j/32), 300 cos(90° j/32)); element 1 + i + 32 j is
[node(i, j), node(i + 1, j), node(i + 1, j + 1), node(i, j + 1)], counter-clockwise seen from
outside, so its normal points away from the axis — the same mesh the solver's own pinched-cylinder
benchmark builds in code
([crates/solver/tests/benchmarks.rs](../../../crates/solver/tests/benchmarks.rs)).

The recipe was generated with the same script as `scordelis-lo-roof-shell`
(`python3 cylinder_recipe.py pinched-cylinder 32`):

```python
import json, math

def pinched_cylinder(n):
    label = lambda i, j: 1 + i + (n + 1) * j
    commands = []
    for j in range(n + 1):
        for i in range(n + 1):
            x, phi = 300.0 * i / n, (math.pi / 2) * j / n
            commands.append({"command": "mesh.node.add",
                             "params": {"label": label(i, j),
                                        "xyz": [x, 300.0 * math.sin(phi), 300.0 * math.cos(phi)]}})
    for j in range(n):
        for i in range(n):
            nodes = [label(i, j), label(i + 1, j), label(i + 1, j + 1), label(i, j + 1)]
            commands.append({"command": "mesh.element.add",
                             "params": {"label": 1 + i + n * j, "kind": "quad4", "nodes": nodes}})
    return commands + [
        {"command": "model.set.create", "params": {"name": "shell", "elements": list(range(1, n * n + 1))}},
        {"command": "model.set.create", "params": {"name": "diaphragm", "nodes": [label(n, j) for j in range(n + 1)]}},
        {"command": "model.set.create", "params": {"name": "middle", "nodes": [label(0, j) for j in range(n + 1)]}},
        {"command": "model.set.create", "params": {"name": "top", "nodes": [label(i, 0) for i in range(n + 1)]}},
        {"command": "model.set.create", "params": {"name": "side", "nodes": [label(i, n) for i in range(n + 1)]}},
        {"command": "model.set.create", "params": {"name": "pinch", "nodes": [label(0, 0)]}},
        {"command": "model.material.create", "params": {"name": "Mat", "elastic": {"type": "isotropic", "young": 3e6, "poisson": 0.3}}},
        {"command": "model.section.create", "params": {"name": "shell", "set": "shell", "material": "Mat", "kind": {"type": "shell", "thickness": 3.0}}},
        {"command": "model.boundary_condition.create", "params": {"name": "diaphragm", "type": "displacement", "target": {"set": "diaphragm"}, "values": [None, 0, 0, 0, None, None]}},
        {"command": "model.boundary_condition.create", "params": {"name": "middle", "type": "symmetry", "target": {"set": "middle"}, "plane": "x"}},
        {"command": "model.boundary_condition.create", "params": {"name": "top", "type": "symmetry", "target": {"set": "top"}, "plane": "y"}},
        {"command": "model.boundary_condition.create", "params": {"name": "side", "type": "symmetry", "target": {"set": "side"}, "plane": "z"}},
        {"command": "model.load.create", "params": {"name": "pinch", "type": "force", "target": {"set": "pinch"}, "vector": [0, 0, -0.25]}},
        {"command": "model.step.create", "params": {"name": "Load", "procedure": {"type": "static"}, "boundary_conditions": ["diaphragm", "middle", "top", "side"], "loads": ["pinch"]}},
        {"command": "solve.static", "params": {}, "id": "solve"},
    ]

print("[\n " + ",\n ".join(json.dumps(c) for c in pinched_cylinder(32)) + "\n]")
```

## A fixed-mesh benchmark, not a convergence study

The quantity is the deflection *under a point load*, and there a Reissner–Mindlin (MITC4) shell
carries an unbounded transverse shear force: the deflection under the load keeps growing with
refinement instead of converging. The published 1.8248 × 10⁻⁵ is a thin-shell value, and this case
compares against it on the conventional 32 × 32 eighth mesh — the mesh the solver's own
pinched-cylinder benchmark uses — not as a limit. The table shows what refinement actually does
(each row is the listing above with the `32` in its last line replaced by the row's divisions and
run through `cargo xtask oracle` as a scratch case):

| cells | u | vs reference |
|---|---|---|
| 8 × 8 | −1.3559 × 10⁻⁵ | −25.7% |
| 16 × 16 | −1.6942 × 10⁻⁵ | −7.2% |
| 24 × 24 | −1.7735 × 10⁻⁵ | −2.8% |
| 32 × 32 (this case) | −1.803845342860719 × 10⁻⁵ | −1.15% |
| 48 × 48 | −1.8275 × 10⁻⁵ | +0.15% |
| 64 × 64 | −1.8369 × 10⁻⁵ | +0.66% |

The sequence rises through the reference between 32 × 32 and 48 × 48 and keeps rising at the two
finer meshes, which is what a shear-deformable shell under a point load must do.

The solver's in-code benchmark of the same problem shows the same ascent on its 8 × 8, 16 × 16 and
32 × 32 eighth meshes (it gates 32 × 32 within 2%), so the command route and the in-code route
agree on the fixed-mesh comparison.

## Tolerance

2% covers the agreement with the thin-shell reference at the 32 × 32 eighth mesh (1.15% low), the
conventional mesh of the shell obstacle course and the one the solver's own pinched-cylinder
benchmark gates at 2%, so the command route and the in-code route are compared on equal terms. The
reference itself is quoted by Belytschko et al. to four figures (1.8248 × 10⁻⁵). Because the point
load makes the deflection mesh-dependent rather than convergent (above), this is a fixed-mesh
tolerance and not a convergence statement: the two finer meshes sit 0.15% and 0.66% above the
reference and stay inside it. The mesh is not marked `slow`: the case solves in about 0.15 s.

`fixture.json` is the accepted value at 32 × 32 (−1.803845342860719 × 10⁻⁵) with the runner's
default band: the mesh is written by the recipe, not generated from geometry, so it is bit-identical
on every platform and the result is frozen.

## Running it

```sh
cargo xtask oracle --case pinched-cylinder
cargo xtask oracle --bless pinched-cylinder   # only after the case passes
```
