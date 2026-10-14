# ModelCraft

An open-source, clean-room CAE modeling application in pure Rust: geometry to analysis-ready
finite element models, with robust tetrahedral and hexahedral meshing, solver decks for Abaqus,
Nastran and LS-DYNA, and its own linear static and modal solver. Desktop, browser and AI agents
drive the same commands.

**Status:** early. The workspace, quality gates, the FE model, the command engine and the
geometry boundary (SolidCraft's kernel and tessellator) are in place; meshing is next. See
[ROADMAP.md](ROADMAP.md).

## Build

```sh
cargo xtask ci                                  # fmt, assets, licences, layers, clippy, tests, wasm
cargo xtask test --heavy                        # the heavy (ignored) tests and the doc tests
cargo run -p modelcraft-cli -- commands         # every command and its parameters
cargo run -p modelcraft-cli -- run script.json  # run a command script
cargo run -p modelcraft-cli -- mcp              # MCP server on stdio for AI agents (docs/mcp.md)
```

Rust 1.95 or later, with the `wasm32-unknown-unknown` target for `cargo xtask ci`.

## Documentation

- [Architecture](docs/architecture.md): crates, layers and the meshing pipeline
- [Libraries and licences](docs/adr/0001-libraries.md)
- [AI agents (MCP)](docs/mcp.md): `claude mcp add modelcraft -- /path/to/modelcraft-cli mcp`
- [Control protocol](docs/control-protocol.md): driving the desktop app
- [Contributing](CONTRIBUTING.md) and [agent instructions](AGENTS.md)

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Notices
are in [NOTICE](NOTICE); third-party licences in [THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt).

ModelCraft is an independent project, not affiliated with, sponsored or endorsed by Dassault
Systèmes, MSC Software, Siemens or Ansys. Abaqus, Nastran, MSC Nastran, Simcenter Nastran and
LS-DYNA are trademarks of their owners and are named only to describe file compatibility.
