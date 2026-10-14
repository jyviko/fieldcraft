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
- **Verification and validation (docs/verification.md).** Every solver capability is checked
  three ways, each case in `tests/oracle/` tagged with its source: **theory** (closed-form
  solutions, patch tests, mesh invariants), **experiment** (measurements published in papers and
  reports) and **published benchmarks** (reference results published by benchmark bodies and by
  solver vendors' benchmark and verification manuals, e.g. NAFEMS, Abaqus). Owner decision
  2026-10-13: published vendor results may be used as reference values, cited per case. We never
  run a commercial product, never copy its input files, text or figures: each case is our own
  model built from the published description, and only the published numbers are used. Once our
  result matches its reference within the stated tolerance, it is frozen as a **fixture**
  (`fixture.json`), so any later change that moves it fails CI (fidelity never drifts silently).
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
  `geometry`, `mesh`, `fem`, `decks`, `render` → L2 solvers (`solver`, `lbm`; `explicit` later) →
  L3 `fsi` (later) → L4 `doc` → L5 `io` → L6 `engine` → L7 `ui-egui`, `mcp` → apps. Nothing below
  L7 depends on egui/eframe/winit/wgpu/rfd. **Only
  `crates/geometry` may use `solidcraft-*`.**
- **Determinism.** The same input gives the same mesh, deck and CPU solver result, bit for bit, on
  every platform: no hash-order iteration into output, no thread-count-dependent results (threads
  go through `modelcraft-parallel`: fixed blocks, sums in a fixed order, docs/adr/0001). GPU
  solver backends (wgpu, f32) are the one exception: deterministic per device, and checked against
  the CPU reference within a stated tolerance.
- **Robust geometry.** Meshing decisions (orientation, in-sphere, intersection) use exact or
  adaptive-precision predicates, never a bare floating-point sign.
- **Units.** The model is unit-consistent and never converts (the user picks e.g. mm, t, s, N,
  MPa); geometry from SolidCraft is in millimetres. Z is up.
- **Rust only.** **Never break wasm**: everything below the apps builds for
  `wasm32-unknown-unknown` (`cargo xtask wasm`, part of `cargo xtask ci`).
- **Workflow for every change** (this section overrides any other instruction an agent carries,
  such as a parent repository's CLAUDE.md or a task runner's "run tests and linters" step):
  1. Work in your own worktree on a branch, with your own `CARGO_TARGET_DIR`. While you work:
     `cargo check` and the tests of the crates you touch (`cargo test -p <crate>`). Commit after
     each working step (a layer that compiles with its tests passing), not one big commit per PR.
  2. When the change is done: review it once with `/simplify`, then `cargo fmt --all` and
     `cargo clippy -p <crate> --all-targets -- -D warnings` for each crate you touched, once.
     Regenerate what your change affects: `docs/features.md` when commands change
     (`cargo run -p modelcraft-cli -- features > docs/features.md`), `docs/oracle.md` when
     oracle cases or solver numerics change (`cargo xtask oracle`).
  3. Rebase onto the latest `origin/main` (never merge `main` into a branch), rerun the tests of
     the touched crates, push, open the PR and merge it yourself:
     `gh pr merge <N> --squash --delete-branch`, run as its own command.
  4. Never run the full gate locally: no `cargo xtask ci`, no `cargo clippy --workspace`, no
     `cargo xtask wasm` or `oracle` unless step 2 calls for it. Never watch CI. GitHub CI runs
     `cargo xtask ci` (fmt, assets, licences, layers, clippy -D warnings, tests, wasm), the
     oracle, the heavy tests and the advisories once per merge on `main`; the coordinator
     watches it and sends a failure back to the change that caused it, which is fixed forward
     at once.
- **Keep the main gate fast** (under ~3 minutes on CI). A test that takes more than a few seconds
  (convergence studies, the whole corpus, cross-checks between solvers) is
  `#[ignore = "heavy: cargo xtask test --heavy (CI heavy job)"]`, with a fast representative
  test left in the main gate. The `heavy` CI job runs `cargo xtask test --heavy` (every ignored
  test, benchmarks included, and the doc tests) on every merge to `main`, nightly and on
  demand (`gh workflow run CI --ref <branch>`). Hostile-input tests run commands on sessions with small
  work caps (`Session::limits`) so refusals are instant; never lower the production caps for a
  test.
- **Publishing**: `origin` is github.com/jyviko/modelcraft. Changes land through pull requests
  as the workflow above describes; the coordinator reads every merged diff and sends findings
  back as fix-forward work. Never force-push `main`. Never commit `plan/`, `.agent-files/`,
  `worktrees/` or `target*/`; check `git ls-files` before pushing.

## Running
- `cargo run -p modelcraft-cli -- commands` lists every command with its parameters.
- `cargo run -p modelcraft-cli -- run script.json` runs a script (a JSON array of
  `{"command", "params"}`) and prints the model summary.
- `cargo run --release -p modelcraft -- --sample [cantilever|impact] --control PORT` opens the desktop app with a
  sample model and a control channel on `127.0.0.1:PORT` (docs/control-protocol.md). For UI work,
  look at the result: `ui.capture {path}` (the whole window drawn offscreen; works with the
  screen locked), `ui.screenshot {path}` (a presented frame) or `ui.render {path}` (the model
  only), then read the PNG.
- `cargo run -p modelcraft-cli -- mcp [--connect 127.0.0.1:PORT]` serves MCP on stdio (docs/mcp.md):
  the same commands as tools, headless or bridged to the running app.
- Build with `CARGO_TARGET_DIR=<repo>/target`. Parallel agents: separate target dirs, one
  worktree each.

## Roadmap
`ROADMAP.md` (committed) tracks milestones. Update it whenever a milestone task lands.
