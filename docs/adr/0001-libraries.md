# ADR 0001: Libraries and licences

Status: accepted (2026-10-13)

## Context

ModelCraft is MIT OR Apache-2.0. Everything it links must allow that, and the meshing and solver
algorithms, where the obvious open-source implementations are GPL-family, must come from papers.

## Decision

**Geometry: SolidCraft's kernel**, as a git dependency pinned by rev
(`solidcraft-kernel`, `solidcraft-geom`; MIT OR Apache-2.0, built on the Apache-2.0 truck
crates). ModelCraft uses its B-rep bodies and its tessellation; it does not read CAD exchange
files itself. Only `modelcraft-geometry` may depend on it.

**Meshing: our own**, implemented from published papers (`docs/meshing/literature.md`).
Robust predicates follow Shewchuk's adaptive-precision arithmetic (the paper; his reference
`predicates.c` is public domain). A permissive crate may be adopted instead after review; record
it here. *Decided 2026-10-13:* our own implementation, `modelcraft_mesh::predicates`, written from
Shewchuk 1997, Devillers–Teillaud 2011 and Attene 2020 (no `unsafe`, deterministic, wasm-safe, a
validated coordinate domain instead of Shewchuk's overflow/underflow caveat, and indirect
predicates that `robust` lacks). `robust` (georust, MIT OR Apache-2.0) and `num-bigint` /
`num-rational` / `num-traits` (MIT OR Apache-2.0) are dev-dependencies only: differential test
oracles, never linked into ModelCraft.

**Linear algebra:** to be decided in the solver task, among permissively licensed pure-Rust
crates (for example `faer`, MIT). Record the choice here.

**Allowed:** MIT, BSD-2/3, Apache-2.0, ISC, Zlib, Unicode, MPL-2.0 (as an unmodified dependency),
CC0 and public domain.

**Rejected — never read, copy or link:**

| Project | Licence | Would have offered |
|---|---|---|
| TetGen | AGPL-3.0 | constrained Delaunay tetrahedralization |
| CGAL | GPL-3.0 / LGPL-3.0 | Delaunay, mesh generation |
| Gmsh | GPL-2.0+ | tet and hex meshing |
| Netgen | LGPL-2.1 | tet meshing |
| CalculiX | GPL-2.0 | implicit solver, Abaqus-like input |
| Code_Aster | GPL-3.0 | implicit solver |
| OpenRadioss | AGPL-3.0 | explicit solver |
| Elmer | GPL-2.0 / LGPL-2.1 | multiphysics solver |
| deal.II, FreeFEM, Salome | LGPL | FE libraries, pre-processing |
| OpenCASCADE | LGPL-2.1 | B-rep kernel |
| Attene's CDT, Indirect_Predicates, VolumeMesher | GPL / LGPL | robust CDT, indirect predicates (implement from the papers) |
| TetWild (original), Evocube, robustPolycube, ultimaille, invertible-maps | GPL-3.0 / AGPL-3.0 | tet meshing, polycube, injective maps |
| libHexEx, libHexHex, QGP3D, MC3D, CoMISo, Graphite | GPL | integer-grid-map hex extraction, quantization |
| Shewchuk's Triangle | non-commercial only | 2D meshing |
| geogram `third_party/tetgen`, `third_party/triangle` and their wrappers | AGPL / non-commercial | (the rest of geogram is BSD-3 and readable) |
| libigl `copyleft/` modules | GPL | (libigl core is MPL-2.0 and readable) |
| OpenVolumeMesh, CGoGN | believed LGPL, unverified | treat as forbidden until checked |

**Repositories without a licence are all rights reserved: never read them** (for example Gao
2019's octree hex code, LoopyCuts). Readable references for the meshing work (re-implement from
the paper, note anything adapted in ATTRIBUTION.md): Shewchuk's `predicates.c` (public domain),
`georust/robust` (Apache-2.0), geogram (BSD-3, exclusions above), fTetWild (MPL-2.0), CinoLib,
Gen-Adapt-Ref, HexaLab, robust_hex_dominant_meshing (MIT), quartet (ISC). The full table with
sources is in `docs/meshing/literature.md`.

**Test geometry** follows the same rule as test data: nothing produced with a commercial CAD or
CAE product. That excludes ABC (Onshape), MAMBO (files from commercial translators) and the HexMe
models that come from ABC, GrabCAD or Siemens NX; HexMe's Gmsh-made models are allowed (CC-BY-4.0).
Thingi10K is not used either (owner decision 2026-10-13: authoring tools unrecorded). **The test
corpus is our own:** parts built with SolidCraft (scripts or designs authored in SolidCraft,
committed as their recipes so the geometry is regenerated, not stored) and analytic shapes.

**Commercial products** (Abaqus, Nastran, LS-DYNA, LS-PrePost, HyperMesh, ANSA, Femap, Patran,
ANSYS) are never run, scripted or used as a source of data. Deck formats come from the vendors'
public reference manuals, described in our own words with our own examples.

## Consequences

- The tet and hex meshers are significant original work; the literature survey decides the
  order and the methods.
- SolidCraft and ModelCraft move together: bumping the pinned rev bumps the repeated
  `[patch.crates-io]` entries in the same commit.
