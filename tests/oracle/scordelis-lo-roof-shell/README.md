# Scordelis–Lo roof with shell elements

Our model of the published problem: a cylindrical roof of radius 25, length 50 and thickness 0.25,
spanning ±40°, E = 4.32 × 10⁸, ν = 0, carrying its own weight (90 per area, a body force 90/t = 360
per volume), supported by rigid diaphragms at its curved ends and free along its straight edges.
This is `scordelis-lo-roof-solid` with flat quad4 (MITC4) shells in place of the hex20 layer, so the
same reference is checked by two element routes; the reference value, 0.3024 for the vertical
deflection at the middle of a free edge, is the one the case already cited (MacNeal and Harder,
*A proposed standard set of problems to test finite element accuracy*, FEAD 1 (1985) 3–20, Fig. 8,
Table 5(a); Belytschko et al., CMAME 51 (1985) 221–258).

The quarter model: x from the mid-span (x = 0, symmetry in x) to the diaphragm (x = 25,
u_y = u_z = rot_x = 0), φ from the crown (φ = 0, symmetry in y) to the free edge at 40°, with
R sin φ = y and R cos φ = z on the mid-surface R = 25. The measured node is the free-edge mid-span
node (0, 16.0697, 19.1511), the node the solid case measures too.

## The mesh

`mesh.block`'s 2D form takes its four corners at z = 0, so it cannot mesh a patch on a cylindrical
surface (it can be curved in its own plane, but it is planar): the recipe writes the 289 nodes and
256 quad4 elements one by one with `mesh.node.add` and `mesh.element.add`, the route
[docs/verification.md](../../../docs/verification.md) allows ("nodes and elements written one
by one"). Node 1 + i + 17 j lies at
(25 i/16, 25 sin(40° j/16), 25 cos(40° j/16)); element 1 + i + 16 j is
[node(i, j), node(i + 1, j), node(i + 1, j + 1), node(i, j + 1)], counter-clockwise seen from
outside, so its normal points away from the axis — the same mesh the solver's own Scordelis–Lo
benchmark builds in code
([crates/solver/tests/benchmarks.rs](../../../crates/solver/tests/benchmarks.rs)), so the command
route and the in-code route are comparable.

The recipe was generated with this script (`python3 cylinder_recipe.py scordelis-lo 16`):

```python
import json, math

def scordelis_lo(n):
    label = lambda i, j: 1 + i + (n + 1) * j
    commands = []
    for j in range(n + 1):
        for i in range(n + 1):
            x, phi = 25.0 * i / n, math.radians(40.0) * j / n
            commands.append({"command": "mesh.node.add",
                             "params": {"label": label(i, j),
                                        "xyz": [x, 25.0 * math.sin(phi), 25.0 * math.cos(phi)]}})
    for j in range(n):
        for i in range(n):
            nodes = [label(i, j), label(i + 1, j), label(i + 1, j + 1), label(i, j + 1)]
            commands.append({"command": "mesh.element.add",
                             "params": {"label": 1 + i + n * j, "kind": "quad4", "nodes": nodes}})
    return commands + [
        {"command": "model.set.create", "params": {"name": "shell", "elements": list(range(1, n * n + 1))}},
        {"command": "model.set.create", "params": {"name": "diaphragm", "nodes": [label(n, j) for j in range(n + 1)]}},
        {"command": "model.set.create", "params": {"name": "midspan", "nodes": [label(0, j) for j in range(n + 1)]}},
        {"command": "model.set.create", "params": {"name": "crown", "nodes": [label(i, 0) for i in range(n + 1)]}},
        {"command": "model.material.create", "params": {"name": "Mat", "elastic": {"type": "isotropic", "young": 4.32e8, "poisson": 0.0}}},
        {"command": "model.section.create", "params": {"name": "shell", "set": "shell", "material": "Mat", "kind": {"type": "shell", "thickness": 0.25}}},
        {"command": "model.boundary_condition.create", "params": {"name": "diaphragm", "type": "displacement", "target": {"set": "diaphragm"}, "values": [None, 0, 0, 0, None, None]}},
        {"command": "model.boundary_condition.create", "params": {"name": "midspan", "type": "symmetry", "target": {"set": "midspan"}, "plane": "x"}},
        {"command": "model.boundary_condition.create", "params": {"name": "crown", "type": "symmetry", "target": {"set": "crown"}, "plane": "y"}},
        {"command": "model.load.create", "params": {"name": "weight", "type": "body_force", "set": "shell", "vector": [0, 0, -360.0]}},
        {"command": "model.step.create", "params": {"name": "Load", "procedure": {"type": "static"}, "boundary_conditions": ["diaphragm", "midspan", "crown"], "loads": ["weight"]}},
        {"command": "solve.static", "params": {}, "id": "solve"},
    ]

print("[\n " + ",\n ".join(json.dumps(c) for c in scordelis_lo(16)) + "\n]")
```

## Refinement

The flat MITC4 shell approaches the reference from below and keeps closing the gap, so the mesh
error is inside the tolerance and shrinking. Every row is the listing above with the `16` in its
last line replaced by the row's divisions and run through `cargo xtask oracle` as a scratch case;
the case mesh is 16 × 16:

| cells | u_z | error |
|---|---|---|
| 4 × 4 | −0.28459 | −5.89% |
| 8 × 8 | −0.29401 | −2.77% |
| 16 × 16 (this case) | −0.29901009815961804 | −1.12% |
| 32 × 32 | −0.30060 | −0.60% |
| 64 × 64 | −0.30118 | −0.40% |
| 128 × 128 | −0.30143 | −0.32% |

The solid case of the same roof reports −0.3014 on its 16 × 16 × 1 hex20 mesh, i.e. 0.33% low: two
element formulations, two meshes, one reference.

## Tolerance

2%, the grading the sibling solid case already records for this problem from MacNeal and Harder (an
element is graded within 2% of the theoretical value on a prescribed coarse mesh, and their
normalised target is 0.3024). 16 × 16 is our choice of mesh — the one the solver's in-code gate test
uses — and it is 1.12% low; its own next refinement (32 × 32) moves the value by 0.53% of the
reference, a quarter of the tolerance, on a mesh error that falls monotonically. The tolerance is
the source's grading, not a cover for the discretisation error.

`fixture.json` is the accepted value at 16 × 16 (−0.29901009815961804) with the runner's default
band: the mesh is written by the recipe, not generated from geometry, so it is bit-identical on
every platform and the result is frozen.

## Running it

```sh
cargo xtask oracle --case scordelis-lo-roof-shell
cargo xtask oracle --bless scordelis-lo-roof-shell   # only after the case passes
```
