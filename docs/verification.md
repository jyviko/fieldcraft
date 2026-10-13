# Verification and validation

Every solver capability is checked against three kinds of reference, and every accepted result is
frozen so it cannot drift.

| Kind | What it checks | Sources |
|---|---|---|
| Theory | The code solves the equations right | Closed-form solutions (beams, plates, Lamé, Kirsch, waves, shocks, Poiseuille…), patch tests, convergence rates, mesh invariants |
| Experiment | The model predicts reality | Measurements published in papers and reports (Taylor bar, cavity and cylinder flows, FSI experiments…) |
| Published benchmarks | Agreement with established codes on standard problems | NAFEMS benchmarks, vendor benchmark and verification manuals (e.g. Abaqus Benchmarks and Verification manuals), other published reference solutions |

Rules:
- Each case is **our own model** of the problem, built from the published description (geometry,
  material, loads, boundary conditions). We never run a commercial product and never copy its
  input files, manual text or figures; only the published numbers are used, with a citation.
- Each case lives in `tests/oracle/<case>/`:
  - `recipe.json` — the model as ModelCraft commands (or a SolidCraft recipe for geometry);
  - `expected.json` — the reference values with `kind` (theory | experiment | benchmark),
    `source` (citation: document, table or figure, page), `quantity`, `value`, `tolerance` and
    the reason for that tolerance (mesh, measurement scatter, model differences);
  - `fixture.json` — ModelCraft's accepted result, written by `cargo xtask oracle --bless <case>`
    only after the case passes against `expected.json`.
- `cargo xtask oracle` runs every case, compares with `expected.json` (pass/fail with error) and
  with `fixture.json` (any change beyond the fixture tolerance fails CI, even if the reference
  still passes), and writes `docs/oracle.md` with the table of cases, sources, errors and status.
- A fixture is updated only deliberately (`--bless`), in a PR that says why the result moved
  (a better element, a finer default mesh), never to hide a regression.
- The theory suite is the gate for every PR; experiment and benchmark cases run in CI too, with
  their cited tolerances.
