# Filleted L-bracket: peak stress on the fillet (fine mesh)

The corpus part `filleted-bracket` (an 8 mm L-bracket with an R6 inside fillet and a hole through
each leg), built with SolidCraft from `design.json`. The foot face `F2:side:l1` is clamped and a
2 kN load in +x acts on the top face `F2:side:l5` (steel, E = 210 GPa, ν = 0.3). The leg bends
about its root, where the fillet concentrates the stress; beam theory gives about 197 MPa nominal
at the root (6FL/(bt²) with L = 42 mm, b = 40 mm, t = 8 mm).

`mesh.tet.generate` makes a quality tet10 mesh (size 5.66 mm, 31.8° per element along curved
faces), with the boundary and mid-edge nodes on the exact fillet cylinder. The peak nodal von
Mises stress on the fillet over a four-level study (sizes 16, 11.3, 8, 5.66 mm with 90°, 64°,
45°, 32°): 213.3, 210.6, 214.3, 213.3 MPa. The coarse and the fine level are the two cases.
