# Attribution

Every non-code asset in this repository (images, icons, fonts, example geometry) is listed here
with its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an
asset file is missing from this table.

**Policy (mandatory):** ModelCraft contains **no iconography, images, artwork, fonts, templates,
example decks or help text from commercial CAE products.** Every asset is original work by
ModelCraft contributors or third-party material under an open licence. Screenshots of commercial
software are never committed. Icons are made with VectorCraft; each icon's `.vectorcraft` source
is committed and listed here with its exports.

## Files

| File | Author | Source | Licence |
|---|---|---|---|

## Adapted code

| File | Adapted from | Licence |
|---|---|---|
| xtask/src/layers.rs | SolidCraft `xtask/src/layers.rs` (itself adapted from CADCraft's xtask) | MIT OR Apache-2.0 |
| xtask/src/assets.rs, xtask/src/licences.rs, xtask/src/test.rs, xtask/src/main.rs | SolidCraft `xtask/src/` | MIT OR Apache-2.0 |
| clippy.toml, rustfmt.toml | SolidCraft | MIT OR Apache-2.0 |
| crates/geometry/src/tagged_mesh.rs (package and part reading, transforms, units, caps) | SolidCraft `crates/io/src/threemf.rs` | MIT OR Apache-2.0 |
| crates/render/src/camera.rs, crates/render/src/raster.rs | SolidCraft `crates/render/src/camera.rs`, `raster.rs` | MIT OR Apache-2.0 |
| crates/ui-egui/src/gpu.rs, crates/ui-egui/src/theme.rs, crates/ui-egui/src/control.rs, crates/ui-egui/src/lib.rs (input injection, screenshots) | SolidCraft `crates/ui-egui/src/` | MIT OR Apache-2.0 |
| apps/modelcraft/src/control_server.rs | SolidCraft `apps/solidcraft/src/control_server.rs` | MIT OR Apache-2.0 |

## Test oracles

Used only by tests (dev-dependencies, not shipped); no code from them is copied or adapted.

| Used in | Oracle | Licence |
|---|---|---|
| crates/mesh/src/predicates/tests.rs, crates/mesh/tests/predicates_bench.rs | `robust` (georust), a port of Shewchuk's public-domain `predicates.c` | MIT OR Apache-2.0 |
| crates/mesh/src/predicates/tests.rs, crates/mesh/src/predicates/dyadic.rs (tests) | `num-bigint`, `num-rational`, `num-traits` (exact rationals) | MIT OR Apache-2.0 |
