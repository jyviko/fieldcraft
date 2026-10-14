# Contributing

Thanks for helping. Read [AGENTS.md](AGENTS.md) first: it holds the rules every change follows,
whether a person or an agent writes it.

## Before you start: claim an issue

Every change starts from an issue, and only the maintainer merges.

1. Pick an open issue labelled **help wanted** or **good first issue** that is not labelled
   **claimed**. If what you want to do has no issue, open one first and wait for a reply.
2. Comment that you would like to take it. The maintainer assigns it to you and labels it
   **claimed**; then it is yours. Work on unassigned issues may not be accepted.
3. Open a draft pull request early, linked with `Closes #N`. An issue with no visible progress
   for 14 days goes back to the pool.

Issues labelled **in progress** are being worked on by the core team; please don't start on them.

## Making the change

1. Branch from `main`, keep a pull request to one topic.
2. Before pushing: `cargo fmt --all`, and `cargo clippy -p <crate> --all-targets -- -D warnings`
   and `cargo test -p <crate>` for the crates you touched. CI runs the full gate on your pull
   request once the maintainer approves the run.
3. User-visible behaviour is a command (`crates/engine/src/cmd/`) with tests, including hostile
   parameters.
4. Meshing and solver algorithms cite the paper they come from in the module docs.
5. Never contribute code, text or data from GPL-family projects or commercial CAE products
   (see [docs/adr/0001-libraries.md](docs/adr/0001-libraries.md)).

By contributing you agree your work is licensed under MIT OR Apache-2.0.

`main` is protected: changes land only through pull requests that the maintainer has reviewed,
as rebase merges (keep each commit building and passing its tests). Be kind: see the
[code of conduct](CODE_OF_CONDUCT.md).
