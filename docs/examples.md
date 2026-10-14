# Examples

Every example is a built-in sample: open it from **File ▸ Samples** (or the start screen), with
`fieldcraft --sample NAME`, or with the `file.sample` command (`{"name": "drop-test"}`), then
press **Solve** (`solve.run`). Each opens fully defined — geometry, mesh, materials, boundary
conditions, loads and steps — in mm, t, s, N and MPa, and opens and solves within half a minute. The geometry is our
own: SolidCraft scripts run when the sample opens (a recipe from [`tests/corpus`](../tests/corpus)
or one written in `crates/engine/src/sample.rs`). Each sample has a test in
`crates/engine/src/sample.rs` that opens it, checks it clean, solves it and holds its result to
the check given below.

## Drop test (`drop-test`)

![The bracket at 0.1 ms after striking the tilted floor, von Mises stress up to 123 MPa in the bend](media/examples/drop-test.png)

An aluminium L-bracket (60 × 40 mm foot, 50 mm upright, 8 mm thick, R6 fillet) falls 2 m onto a
rigid floor tilted 15°, so the end of its foot strikes first. Explicit dynamics for 1 ms with
gravity: 1 911 swept hex8 elements (one-point, stiffness hourglass control), bilinear plasticity
(yield 240 MPa), the floor a rigid wall. About 8 400 increments, a few seconds.

**Look at:** the frames from the strike (Results ▸ Frame): the compression wave leaves the
struck corner and the upright's inertia bends the fillet, where von Mises peaks near 120 MPa —
below yield, so the drop leaves no permanent set. The energies plot shows the kinetic energy
turning into vibration with a balance under 0.01 %.

**Check:** the mass equals ρ V of the bracket (0.5 %); it meets the floor at √(2 g h) =
6.26 m/s, with ½ m v² = 1 748 mJ of kinetic energy (1 %); energy is conserved (balance < 1 %);
by 1 ms the floor has taken about half its downward momentum.

## Rubber seal (`rubber-seal`)

![The gasket ring squeezed 25 %, bulging outward, von Mises stress peaking at the bonded edges](media/examples/rubber-seal.png)

A rubber gasket ring (20 mm inside radius, 30 mm outside, 10 mm high) squeezed 2.5 mm (25 %)
between two plates bonded to it. Generic rubber, E = 6 MPa, ν = 0.49, Mooney–Rivlin with
C01 = 0.2 MPa; 1 000 hybrid (u–p) hex8 elements in four mapped quarter blocks. Nonlinear static
with large deformation: Newton–Raphson in nine increments, under a second.

**Look at:** the deformed shape at scale 1 — the free inner and outer walls bulge out, and
von Mises peaks (about 7 MPa) where the bulge meets the bonded plates. Results ▸ Increment steps
through the converged increments.

**Check:** the same ring between frictionless plates compresses evenly, and its force matches
the closed form for an incompressible Mooney–Rivlin solid in uniaxial compression,
P = 2 (λ − λ⁻²)(C10 + C01/λ) per original area: 3 430 N against 3 466 N (within 3 %). Bonded,
the bulge makes it stiffer (shape factor 0.5): 6 518 N, held between 1.05 and 2.5 times the
frictionless force. The groove and the plates are prescribed displacements today; with implicit
contact (#161) the ring will be pressed by contact against a groove.

## Flow past a bracket (`flow-past`)

Air (ρ = 1.2 kg/m³, μ = 1.8 × 10⁻⁵ Pa s) at 25 mm/s past the same L-bracket standing on the
floor, the upright's back to the wind. A 240 × 120 × 100 mm box of 2.5 mm cells (184 320),
uniform inlet, outlet at zero pressure, a no-slip floor and slip walls at the sides and top;
D3Q19 lattice Boltzmann (TRT) for 16 s, about 6.7 flow-throughs, ten frames. Reynolds number
83 on the upright's 50 mm height: laminar, a steady wake. About 10 s.

**Look at:** the step's report (`solve.fluid`, or Results after Solve): the force on every
named face of the bracket and on each side of the box, final and time-averaged, and the mass
history; `solve.probe` with `points` gives the velocity and pressure in any cell, and with
`forces` each face's force history.

**Check:** the drag on the bracket, as a coefficient on the upright's frontal area
(40 × 50 mm), is 2.3 — a bluff plate standing on a wall at low Reynolds number in a box it
blocks by 17 %, held between 1 and 3.5 (a flat plate broadside to the flow is about 1.2 at high
Reynolds number and rises as it falls); the lattice's mass changes by less than 1 %.

There is no picture yet: the viewport does not draw flow results (velocity slices and
streamlines are planned in M12), so this example is numbers only for now.
