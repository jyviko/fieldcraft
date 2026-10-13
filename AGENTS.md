# ModelCraft — instructions for agents

ModelCraft is a clean-room, open-source, pure-Rust CAE modeling application: it turns geometry
into analysis-ready finite element models (volume meshes, materials, sections, boundary
conditions, loads and steps), writes them as Abaqus input, Nastran bulk data and LS-DYNA keyword
decks, and solves linear static and modal problems with its own solver. It targets the
pre-processing done today in Abaqus/CAE, Nastran pre-processors and LS-PrePost, and aims to beat
them on meshing robustness, speed, openness and agent control.

Siblings with the same conventions: `../solidcraft` (parametric CAD, Fusion-class), and the
storytold Craft apps (`storytold/cadcraft`, `vectorcraft`, `photocraft`, …). Code adapted from
them (MIT OR Apache-2.0) is noted in ATTRIBUTION.md.

## Product name
ModelCraft everywhere: crates `modelcraft-*`, binaries `modelcraft` and `modelcraft-cli`,
`.modelcraft` models, `MODELCRAFT_*` environment variables, docs, CI and packaging.

## Start every session here
1. Read `ROADMAP.md` (milestones, what is next) and your task.
2. `docs/architecture.md`: crates, layers, the meshing pipeline.
3. `docs/adr/`: library decisions (what we may depend on, what is rejected for licence reasons).
4. Meshing work: `docs/meshing/literature.md` (the papers each algorithm comes from).

## Never crash
People trust ModelCraft with their models; a crash loses their work. **This outranks feature work.**
- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`,
  `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`).
- **Errors are `Result<T, E>`** through the crate's error type and `?`. An unfinished feature
  returns an error ("not supported yet: …"); it never panics.
- **Input-derived numbers are hostile** (command parameters, model files, solver decks,
  control-channel input): `get()` not `[i]`, finite checks, caps on sizes and loop counts.
- **Bound every iterative algorithm.** Mesh refinement, smoothing, untangling and solver
  iterations have explicit iteration and element-count caps and report when they stop early.
- **SolidCraft calls are guarded:** everything that calls `solidcraft-kernel` goes through
  `solidcraft_kernel::guard` inside `modelcraft-geometry`, which turns a panic into an error.
- **Last-resort guard:** `Session::execute` runs every command under `catch_unwind`; an error or
  an escaped panic restores the model and reports `EngineError`.
- **Prove it:** every crash fix lands with a regression test (see `hostile_params_never_panic`).
- Every production crate root carries
  `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`.

## Non-negotiables
- **Independent.** Behaviour is designed from public documentation, published papers, general
  CAE conventions and user feedback. Never copy another product's code, icons, artwork, help
  text, element or material libraries, templates or examples.
- **No commercial CAE products.** Agents must not drive, script, automate or extract data from
  commercial CAE or meshing products (Abaqus, MSC/NX/Simcenter Nastran, LS-DYNA, LS-PrePost,
  HyperMesh, ANSA, Femap, Patran, ANSYS, …): no test data, measurements, decks, results or ids
  produced with them. Deck formats come from the vendors' publicly available reference manuals;
  we write our own descriptions and our own examples, never copy manual text or example decks.
- **Expected values are analytic.** Reference tests compare with closed-form solutions (patch
  tests, beams, plates, Lamé cylinders, Kirsch plates, natural frequencies) and with mesh
  invariants (positive Jacobians, conserved volume, watertight boundaries).
- **Algorithms come from papers.** Meshing and solver methods are implemented from published
  papers and theses, cited in the module docs and in `docs/meshing/literature.md`.
- **Never read, copy or link GPL/LGPL/AGPL code**: TetGen, CGAL, Gmsh, Netgen, CalculiX,
  Code_Aster, OpenRadioss, Elmer, deal.II, FreeFEM, Salome, OpenFOAM — don't even read their
  sources. Permissively licensed code (MIT, BSD, Apache-2.0, MPL-2.0) may be read for reference;
  anything adapted is noted in ATTRIBUTION.md with its licence.
- **Assets — absolutely essential.** ModelCraft contains **no iconography, images, fonts or
  artwork from commercial CAE products — ever.** Icons are original artwork made with VectorCraft:
  commit the editable `.vectorcraft` source beside the exported SVG/PNG. Any file asset must be
  original or openly licensed and **must have a row in ATTRIBUTION.md** (`cargo xtask assets`
  enforces it). Screenshots of commercial software are never committed.
- **Test geometry is our own.** Meshing and solver test parts are built with SolidCraft and
  committed as recipes (scripts or designs) that regenerate them; analytic shapes are generated in
  code. No public CAD datasets (ABC, MAMBO, Thingi10K, GrabCAD) and nothing made in a commercial
  CAD or CAE product.
- **Everything is a command.** User-visible behaviour = a command in `crates/engine/src/cmd/*`
  (`CommandSpec`: our own dotted, lower-case id `<area>.<thing>[.<variant>]` such as
  `mesh.node.add` or `mesh.tet.generate`, never another product's keyword; label; params doc;
  JSON `run`) + tests. The UI, scripts, CLI and control channel all reach the same commands.
- **Programmatic calls never open dialogs.**
- **Layering** is enforced by `cargo xtask layers` (`docs/architecture.md`): L0 `model` → L1
  `geometry`, `mesh`, `solver`, `decks`, `render` → L2 `doc` → L3 `io` → L4 `engine` → L5
  `ui-egui`, `mcp` → apps. Nothing below L5 depends on egui/eframe/winit/wgpu/rfd. **Only
  `crates/geometry` may use `solidcraft-*`.**
- **Determinism.** The same input gives the same mesh and the same deck, bit for bit, on every
  platform: no hash-order iteration into output, no thread-count-dependent results.
- **Robust geometry.** Meshing decisions (orientation, in-sphere, intersection) use exact or
  adaptive-precision predicates, never a bare floating-point sign.
- **Units.** The model is unit-consistent and never converts (the user picks e.g. mm, t, s, N,
  MPa); geometry from SolidCraft is in millimetres. Z is up.
- **Rust only.** **Never break wasm**: everything below the apps builds for
  `wasm32-unknown-unknown` (`cargo xtask wasm`, part of `cargo xtask ci`).
- **Quality gates:** GitHub CI runs `cargo xtask ci` (fmt, assets, licences, layers, clippy -D
  warnings, tests, wasm) on every PR; don't run the full gate locally. Before pushing: `cargo fmt
  --all`, `cargo check` the touched crates, and run the tests you added or changed. Commit after
  each working step (a layer that compiles with its tests passing), not one big commit per PR.
- **Publishing**: `origin` is github.com/jyviko/modelcraft. Changes land through pull requests:
  branch, push, open a PR, let CI run, then `gh pr merge --squash` when green. Rebase onto `main`
  (never merge `main` into a branch); never force-push `main`. Never commit `plan/`,
  `.agent-files/`, `worktrees/` or `target*/`; check `git ls-files` before pushing.

## Running
- `cargo run -p modelcraft-cli -- commands` lists every command with its parameters.
- `cargo run -p modelcraft-cli -- run script.json` runs a script (a JSON array of
  `{"command", "params"}`) and prints the model summary.
- Build with `CARGO_TARGET_DIR=<repo>/target`. Parallel agents: separate target dirs, one
  worktree each.

## Roadmap
`ROADMAP.md` (committed) tracks milestones. Update it whenever a milestone task lands.
