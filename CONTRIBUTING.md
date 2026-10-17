# Contributing to FieldCraft

FieldCraft is one open-source modeler for structures, heat and fluids, written in pure Rust: open a
parametric SolidCraft part, mesh it (tet or all-hex), define the analysis, solve it with our own
implicit, explicit, thermal or lattice Boltzmann solvers, and look at verified results — from the
desktop app, the command line, or an AI agent over MCP.

Help is welcome, whether that's a one-line fix, a verification benchmark, a file format or owning a
whole solver. This page covers how to build it, the few rules that aren't negotiable, where help is
needed most, and how to claim work. The rules every change follows, by a person or an agent, are in
[AGENTS.md](AGENTS.md).

## Get it running (about 10 minutes on a fast machine)

You need stable Rust 1.95 or newer.

```sh
git clone https://github.com/tangency-labs/fieldcraft && cd fieldcraft
cargo run --release -p fieldcraft -- --sample cantilever   # the desktop app on a sample model
cargo run -p fieldcraft-cli -- commands                    # every command and its parameters
cargo test -p fieldcraft-solver                            # the tests of one crate
cargo xtask oracle --quick                                 # the verification cases (docs/oracle.md)
```

`develop` is the trunk and `main` holds released code. The layers and crates are in
[docs/architecture.md](docs/architecture.md); every user-visible capability is a command in
`crates/engine/src/cmd/` ([docs/features.md](docs/features.md) lists them).

## Where help is wanted most

See the [`help wanted`](https://github.com/tangency-labs/fieldcraft/labels/help%20wanted) and
[`good first issue`](https://github.com/tangency-labs/fieldcraft/labels/good%20first%20issue) labels, and
the `area:` labels for your field. In order of how much they would help users:

1. **Reproducible meshes** ([#115](https://github.com/tangency-labs/fieldcraft/issues/115)). The same part
   must give the same tet mesh on every platform and every run, as the solvers already do.
2. **All-hex meshing of thin and curved parts** ([#85](https://github.com/tangency-labs/fieldcraft/issues/85)).
   The octree mesher over-refines thin walls; concave transition templates and an up-front depth
   are next. Hex meshing is the centre of gravity of the project.
3. **Getting results in and out**: ParaView export ([#90](https://github.com/tangency-labs/fieldcraft/issues/90)),
   Gmsh import ([#91](https://github.com/tangency-labs/fieldcraft/issues/91)), Abaqus deck reading
   ([#98](https://github.com/tangency-labs/fieldcraft/issues/98)), CSV and STL
   ([#92](https://github.com/tangency-labs/fieldcraft/issues/92), [#93](https://github.com/tangency-labs/fieldcraft/issues/93)).
4. **Verification benchmarks** ([#97](https://github.com/tangency-labs/fieldcraft/issues/97),
   [#106](https://github.com/tangency-labs/fieldcraft/issues/106)). An engineer with a published benchmark
   can add it as an oracle case without touching solver code.
5. **Nonlinear elements and materials**: incompatible modes at finite strain
   ([#82](https://github.com/tangency-labs/fieldcraft/issues/82)), kinematic hardening
   ([#81](https://github.com/tangency-labs/fieldcraft/issues/81)), and the material library
   ([#99](https://github.com/tangency-labs/fieldcraft/issues/99)).
6. **The app**: typed command forms ([#112](https://github.com/tangency-labs/fieldcraft/issues/112)), XY and
   path plots ([#94](https://github.com/tangency-labs/fieldcraft/issues/94), [#95](https://github.com/tangency-labs/fieldcraft/issues/95)).

Bigger areas on the [roadmap](ROADMAP.md) — contact, dynamics, composites, coupled fields, wave
propagation, turbulence — are open to anyone who wants to own one: start a thread in
[Discussions](https://github.com/tangency-labs/fieldcraft/discussions).

## Before you start: claim an issue

Every change starts from an issue, and only the maintainer merges.

1. Pick an open issue labelled **help wanted** or **good first issue** that is not labelled
   **claimed**. If what you want to do has no issue, open one first and wait for a reply.
2. Comment that you would like to take it. The maintainer assigns it to you and labels it
   **claimed**; then it is yours. Work on unassigned issues may not be accepted.
3. Open a draft pull request early, linked with `Closes #N`. An issue with no visible progress
   for 14 days goes back to the pool.

Issues labelled **in progress** are being worked on by the core team; please don't start on them.

## Reporting a problem or an idea

Use the templates under **New issue**. Show it: for anything you can see — the view, a mesh, a
panel, a plot — add a screenshot or a short screen recording (drag it into the issue) and mark
the problem on it. For a mesh, a section (`fieldcraft-cli snapshot … --section`) shows what the
outside hides. A picture is often the fastest way for someone else to see what you see.

## Making the change

1. Branch from `develop` (the trunk; `main` holds released code only) and open the pull request
   against `develop`. Keep it to one topic.
2. Before pushing: `cargo fmt --all`, and `cargo clippy -p <crate> --all-targets -- -D warnings`
   and `cargo test -p <crate>` for the crates you touched. CI runs the full gate on your pull
   request once the maintainer approves the run.
3. User-visible behaviour is a command (`crates/engine/src/cmd/`) with tests, including hostile
   parameters.
4. Meshing and solver algorithms cite the paper they come from in the module docs.
5. A change you can see (the view, meshes, panels, results, icons) shows a picture before and after
   in the pull request.
6. Never contribute code, text or data from GPL-family projects or commercial CAE products
   (see [docs/adr/0001-libraries.md](docs/adr/0001-libraries.md)).

By contributing you agree your work is licensed under MIT OR Apache-2.0.

`develop` and `main` are protected: changes land on `develop` only through pull requests that the
maintainer has reviewed,
as rebase merges (keep each commit building and passing its tests). Be kind: see the
[code of conduct](CODE_OF_CONDUCT.md).
