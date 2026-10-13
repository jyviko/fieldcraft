//! The capability catalogue: everything ModelCraft does or will do, in one table.
//!
//! Each [`Capability`] is a toolbar entry: its tab and panel, its label and one-line tip, and
//! either the command of the same id it runs ([`Status::Ready`]) or the milestone that brings it
//! ([`Status::Planned`], shown greyed as "Coming soon"). Planned entries are not commands: a
//! command always runs. The icon of an entry is `assets/icons/<id>.svg`.
//!
//! Every registered command is claimed here, as an entry's command or as one of its related
//! commands ([`Capability::more`], the entry's drop-down); the tests check it. A new command
//! claims its place by turning its planned entry into a ready one (or by adding an entry with
//! an icon), see `docs/features.md`.

use std::fmt::Write;

use serde::{Deserialize, Serialize};

/// The toolbar tabs, in the order of an analyst's workflow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tab {
    #[default]
    Geometry,
    Mesh,
    Properties,
    Assembly,
    Steps,
    Loads,
    Interactions,
    Solve,
    Results,
    Tools,
}

impl Tab {
    pub const ALL: [Tab; 10] =
        [Tab::Geometry, Tab::Mesh, Tab::Properties, Tab::Assembly, Tab::Steps, Tab::Loads, Tab::Interactions, Tab::Solve, Tab::Results, Tab::Tools];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Geometry => "Geometry",
            Tab::Mesh => "Mesh",
            Tab::Properties => "Properties",
            Tab::Assembly => "Assembly",
            Tab::Steps => "Steps",
            Tab::Loads => "Loads & BCs",
            Tab::Interactions => "Interactions",
            Tab::Solve => "Solve",
            Tab::Results => "Results",
            Tab::Tools => "Tools",
        }
    }
}

/// Whether an entry runs today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Runs the command whose id is the entry's.
    Ready,
    /// Coming in this roadmap milestone (`M2`…).
    Planned { milestone: &'static str },
    /// Runs `deck.write` with this `format` once `modelcraft_decks::FORMATS` has a writer for it;
    /// until then coming in `milestone`.
    Deck { format: &'static str, milestone: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capability {
    /// Our own dotted id; also the icon's file name.
    pub id: &'static str,
    pub label: &'static str,
    pub tab: Tab,
    pub panel: &'static str,
    /// One line: what it does.
    pub tip: &'static str,
    pub status: Status,
    /// Related commands in the entry's drop-down (edit, delete…).
    pub more: &'static [&'static str],
}

impl Capability {
    /// The command it runs; `None` while it is coming soon.
    pub fn command(&self) -> Option<&'static str> {
        match self.status {
            Status::Ready => Some(self.id),
            Status::Planned { .. } => None,
            Status::Deck { format, .. } => modelcraft_decks::format(format).map(|_| crate::cmd::deck::WRITE.id),
        }
    }

    /// The parameters its command starts with (an object; empty for most entries).
    pub fn preset(&self) -> serde_json::Value {
        match self.status {
            Status::Deck { format, .. } => serde_json::json!({ "format": format }),
            Status::Ready | Status::Planned { .. } => serde_json::json!({}),
        }
    }

    /// The milestone that brings it, while it is coming soon.
    pub fn milestone(&self) -> Option<&'static str> {
        match self.status {
            Status::Planned { milestone } => Some(milestone),
            Status::Deck { milestone, .. } if self.command().is_none() => Some(milestone),
            Status::Ready | Status::Deck { .. } => None,
        }
    }

    /// The hover text: label, tip and command, or when it comes.
    pub fn tooltip(&self) -> String {
        match (self.command(), self.milestone()) {
            (Some(command), _) => format!("{}\n{}\n{command}", self.label, self.tip),
            (None, m) => format!("{} — coming soon ({})\n{}", self.label, m.unwrap_or("later"), self.tip),
        }
    }
}

const fn ready(id: &'static str, label: &'static str, tab: Tab, panel: &'static str, tip: &'static str) -> Capability {
    Capability { id, label, tab, panel, tip, status: Status::Ready, more: &[] }
}

const fn ready_more(
    id: &'static str,
    label: &'static str,
    tab: Tab,
    panel: &'static str,
    tip: &'static str,
    more: &'static [&'static str],
) -> Capability {
    Capability { more, ..ready(id, label, tab, panel, tip) }
}

const fn soon(id: &'static str, label: &'static str, tab: Tab, panel: &'static str, milestone: &'static str, tip: &'static str) -> Capability {
    Capability { id, label, tab, panel, tip, status: Status::Planned { milestone }, more: &[] }
}

const fn deck(id: &'static str, label: &'static str, format: &'static str, tip: &'static str) -> Capability {
    Capability { id, label, tab: Tab::Solve, panel: "Decks", tip, status: Status::Deck { format, milestone: "M3" }, more: &[] }
}

use Tab::*;

/// Every capability in toolbar order: grouped by tab (in [`Tab::ALL`] order), then by panel.
pub static CATALOGUE: &[Capability] = &[
    // ---- Geometry ----
    ready("geometry.open", "Open Design", Geometry, "Import", "Open a SolidCraft design: its bodies with persistent face names"),
    ready("geometry.import.mesh", "Tagged Mesh", Geometry, "Import", "Read a face-tagged surface mesh (3MF)"),
    ready("geometry.bodies", "Bodies", Geometry, "Inspect", "List the open design's bodies, volumes and named faces"),
    ready("geometry.tessellate", "Tessellate", Geometry, "Inspect", "Tessellate a body into a face-tagged boundary surface"),
    ready(
        "mesh.surface.check",
        "Check Surface",
        Geometry,
        "Inspect",
        "Weld, closed, manifold, orientation and self-intersection checks; feature edges",
    ),
    soon("geometry.partition", "Partition", Geometry, "Prepare", "M5", "Split bodies and faces as SolidCraft features"),
    soon("geometry.datum", "Datum Plane", Geometry, "Prepare", "M5", "Construction planes for partitions"),
    soon("geometry.midsurface", "Midsurface", Geometry, "Prepare", "M8", "Extract midsurfaces of thin parts for shell meshes"),
    // ---- Mesh ----
    soon("mesh.size.global", "Global Size", Mesh, "Size", "M2", "One element size for the whole part"),
    soon("mesh.size.seed", "Seed Edges", Mesh, "Size", "M2", "Element counts, sizes and bias per edge and face"),
    soon("mesh.size.adapt", "Curvature", Mesh, "Size", "M2", "Refine by curvature and proximity"),
    ready(
        "mesh.tet.generate",
        "Tet Mesh",
        Mesh,
        "Tet",
        "Constrained Delaunay tet mesh of a body: every boundary triangle kept, faces tagged (refinement comes next)",
    ),
    soon("mesh.order.quadratic", "Quadratic", Mesh, "Tet", "M2", "Second-order elements with mid-side nodes on the geometry"),
    ready("mesh.delaunay.points", "Delaunay Points", Mesh, "Tet", "Delaunay tetrahedralization of a point list (debugging aid)"),
    ready(
        "mesh.planar",
        "2D Mesh",
        Mesh,
        "2D",
        "Quad or triangle mesh of a planar face or polygon in the XY plane for plane stress, plane strain and axisymmetric models; edges tagged",
    ),
    soon("mesh.quad.faces", "Quad Faces", Mesh, "Structured Hex", "M5", "All-quad meshes of faces on shared edge nodes"),
    soon("mesh.hex.mapped", "Mapped Hex", Mesh, "Structured Hex", "M5", "Mapped hex mesh of six-sided regions"),
    soon("mesh.hex.sweep", "Sweep Hex", Mesh, "Structured Hex", "M5", "Hex mesh swept along extrusions and revolutions"),
    soon("mesh.hex.decompose", "Decompose", Mesh, "Structured Hex", "M5", "Find sweepable and mappable regions and propose the splits"),
    soon("mesh.transition", "Transition", Mesh, "Structured Hex", "M5", "Conforming tet–hex interfaces with pyramids"),
    soon("mesh.hex.generate", "All-Hex", Mesh, "Automatic Hex", "M6", "Octree dual all-hex mesh of any part"),
    soon("mesh.hex.dominant", "Hex-Dominant", Mesh, "Automatic Hex", "M6", "Hex-dominant mesh by recombining tets"),
    soon("mesh.hex.polycube", "Polycube", Mesh, "Automatic Hex", "M6", "Polycube hex mesh of blocky parts"),
    soon("mesh.smooth", "Smooth", Mesh, "Improve", "M6", "Smooth node positions, keeping every element valid"),
    soon("mesh.untangle", "Untangle", Mesh, "Improve", "M6", "Repair inverted elements"),
    soon("mesh.layers", "Layers", Mesh, "Improve", "M6", "Boundary padding and layers through thin walls"),
    ready_more("mesh.node.add", "Node", Mesh, "Edit", "Add a node", &["mesh.node.remove"]),
    ready_more(
        "mesh.element.add",
        "Element",
        Mesh,
        "Edit",
        "Add an element on existing nodes, or a mapped block of them",
        &["mesh.element.remove", "mesh.block"],
    ),
    soon("mesh.node.merge", "Merge Nodes", Mesh, "Edit", "M6", "Merge coincident nodes"),
    soon("mesh.region.remesh", "Remesh Region", Mesh, "Edit", "M6", "Remesh a region in place; split and collapse elements"),
    soon("mesh.import.orphan", "Orphan Mesh", Mesh, "Edit", "M6", "Import a mesh without geometry"),
    ready("mesh.quality", "Quality", Mesh, "Verify", "Quality metrics and the certified validity check, failing elements listed"),
    soon("mesh.verify", "Verify Set", Mesh, "Verify", "M2", "Quality and validity per set"),
    ready_more("mesh.tag", "Tag Geometry", Mesh, "Verify", "Tag elements, faces, edges and nodes with the geometry they mesh", &["mesh.tag.clear"]),
    // ---- Properties ----
    ready_more(
        "model.material.create",
        "Material",
        Properties,
        "Materials",
        "Elastic (isotropic, orthotropic) material with density",
        &["model.material.edit", "model.material.delete"],
    ),
    ready("model.material.plasticity", "Plasticity", Properties, "Materials", "Von Mises plasticity: bilinear, hardening table or Johnson–Cook"),
    soon("model.material.hyperelastic", "Hyperelastic", Properties, "Materials", "M8", "Hyperelastic material models"),
    ready("model.material.damage", "Damage", Properties, "Materials", "Johnson–Cook damage and element erosion"),
    soon("model.material.thermal", "Thermal", Properties, "Materials", "M8", "Conductivity, specific heat and expansion"),
    soon(
        "model.material.eos",
        "Equation of State",
        Properties,
        "Materials",
        "M11",
        "Polynomial, ideal gas, Mie–Grüneisen, JWL and tabulated equations of state",
    ),
    ready_more(
        "model.section.create",
        "Section",
        Properties,
        "Sections",
        "Solid, shell, beam, rod or 2D (plane stress, plane strain, axisymmetric) section assigned to a set, with its element formulation",
        &["model.section.edit", "model.section.delete"],
    ),
    soon("model.section.composite", "Composite", Properties, "Sections", "M8", "Composite layups and continuum shells"),
    soon("model.orientation", "Orientation", Properties, "Sections", "M8", "Material orientations"),
    ready_more(
        "model.set.create",
        "Set",
        Properties,
        "Sets",
        "A set of nodes, elements and faces, or of named geometry that survives remeshing",
        &["model.set.edit", "model.set.delete", "model.set.resolve"],
    ),
    ready("model.units", "Units", Properties, "Model", "The model's unit system (never converted)"),
    // ---- Assembly ----
    soon("assembly.instance", "Instances", Assembly, "Instances", "M7", "Part instances placed from SolidCraft occurrences"),
    // ---- Steps ----
    ready_more(
        "model.step.create",
        "Step",
        Steps,
        "Steps",
        "A linear static or frequency step with its output",
        &["model.step.edit", "model.step.delete"],
    ),
    ready("model.step.activate", "Activate", Steps, "Steps", "Choose which loads and boundary conditions act in a step"),
    ready("model.activity", "Step Table", Steps, "Steps", "The step by load and boundary condition table"),
    soon("model.amplitude", "Amplitude", Steps, "Procedures", "M7", "Time and frequency amplitudes"),
    soon("model.step.nonlinear", "Nonlinear", Steps, "Procedures", "M7", "Nonlinear static steps"),
    soon("model.step.explicit", "Explicit", Steps, "Procedures", "M8", "Explicit dynamic steps"),
    soon("model.step.heat", "Heat Transfer", Steps, "Procedures", "M8", "Heat transfer steps"),
    // ---- Loads & BCs ----
    ready_more(
        "model.boundary_condition.create",
        "Boundary Condition",
        Loads,
        "Boundary Conditions",
        "Fixed, displacement or symmetry condition on a set or reference point",
        &["model.boundary_condition.edit", "model.boundary_condition.delete"],
    ),
    ready_more(
        "model.load.create",
        "Load",
        Loads,
        "Loads",
        "Force, moment, pressure, traction, gravity or body force",
        &["model.load.edit", "model.load.delete"],
    ),
    ready_more(
        "model.fluid_domain.create",
        "Fluid Domain",
        Loads,
        "Flow",
        "The box of fluid cells, its fluid, and the obstacle bodies voxelised from their face-tagged surfaces",
        &["model.fluid_domain.edit", "model.fluid_domain.delete"],
    ),
    ready(
        "model.fluid_domain.boundary",
        "Inlet & Outlet",
        Loads,
        "Flow",
        "Velocity inlets, pressure outlets, walls, moving and slip walls, periodic faces",
    ),
    // ---- Interactions ----
    ready_more(
        "model.reference_point.create",
        "Reference Point",
        Interactions,
        "Constraints",
        "A reference point for couplings, loads and boundary conditions",
        &["model.reference_point.edit", "model.reference_point.delete"],
    ),
    ready_more(
        "model.coupling.create",
        "Coupling",
        Interactions,
        "Constraints",
        "Kinematic or distributing coupling of a set to a reference point",
        &["model.coupling.edit", "model.coupling.delete"],
    ),
    soon("interaction.tie", "Tie", Interactions, "Constraints", "M7", "Tie non-matching meshes together"),
    soon("interaction.contact.general", "General Contact", Interactions, "Contact", "M7", "Contact between everything that touches, with friction"),
    soon("interaction.contact.pair", "Contact Pair", Interactions, "Contact", "M7", "Contact between two surfaces"),
    soon("interaction.contact.find", "Find Contacts", Interactions, "Contact", "M7", "Detect contact pairs automatically"),
    soon("interaction.rigid_wall", "Rigid Wall", Interactions, "Contact", "M10", "Rigid walls for explicit dynamics"),
    soon("interaction.bolt", "Bolt Load", Interactions, "Connectors", "M7", "Bolt pretension"),
    ready_more(
        "model.spring.create",
        "Spring",
        Interactions,
        "Connectors",
        "Spring or dashpot to the ground or between two points, along a direction or their line",
        &["model.spring.edit", "model.spring.delete"],
    ),
    ready_more(
        "model.mass.create",
        "Point Mass",
        Interactions,
        "Connectors",
        "Point mass and rotary inertia at nodes or a reference point",
        &["model.mass.edit", "model.mass.delete"],
    ),
    soon("interaction.connector", "Connector", Interactions, "Connectors", "M7", "Connector elements between points"),
    soon("interaction.fsi", "FSI Interface", Interactions, "Coupled", "M13", "The fluid–structure interface"),
    // ---- Solve ----
    ready("solve.run", "Solve", Solve, "Run", "Solve every step in order and show the results"),
    ready("model.check", "Check Model", Solve, "Check", "Name every missing or bad definition before writing or solving"),
    deck("deck.write.abaqus", "Abaqus Input", "abaqus", "Write an Abaqus input file (.inp)"),
    deck("deck.write.nastran", "Nastran Deck", "nastran", "Write Nastran bulk data (SOL 101, 103)"),
    deck("deck.write.lsdyna", "LS-DYNA Deck", "lsdyna", "Write an LS-DYNA keyword file"),
    soon("deck.read", "Read Deck", Solve, "Decks", "M9", "Read a deck, keeping keywords ModelCraft does not know"),
    ready("solve.static", "Static", Solve, "Implicit", "Linear static solve of a step with ModelCraft's own solver"),
    ready("solve.frequency", "Frequency", Solve, "Implicit", "Natural frequencies and mode shapes of a frequency step"),
    ready(
        "solve.explicit",
        "Explicit",
        Solve,
        "Explicit",
        "Transient dynamics of an explicit step by central differences (contact and erosion coming in M10)",
    ),
    soon("solve.blast", "Blast & Impact", Solve, "Explicit", "M14", "EOS-driven fluids with fluid–structure contact (ALE or SPH)"),
    ready("solve.fluid", "Lattice Boltzmann", Solve, "Fluids", "Lattice Boltzmann flow of a fluid step on the CPU, with wall forces"),
    soon("solve.fsi", "Fluid–Structure", Solve, "Fluids", "M13", "Coupled fluid and structure solve"),
    // ---- Results ----
    ready_more("results.deformed", "Deformed", Results, "Display", "The deformed shape of a solved step, to scale", &["results.clear"]),
    ready("results.contour", "Contours", Results, "Display", "Contours of displacement and stress, with a legend"),
    ready("results.modes", "Mode Shapes", Results, "Display", "Mode shapes and frequencies of a frequency step"),
    ready_more(
        "results.animate",
        "Animate",
        Results,
        "Display",
        "Animate the deformed shape, a mode or an explicit step's frames",
        &["results.frame"],
    ),
    ready_more(
        "solve.probe",
        "Probe",
        Results,
        "Query",
        "Displacement, rotation, reaction and stress at nodes, element section forces, a mode shape, or an explicit field frame (or click the model)",
        &["solve.history"],
    ),
    ready("results.reactions", "Reactions", Results, "Query", "Reaction force and moment sums per set"),
    soon("results.free_body", "Free Body", Results, "Query", "M9", "Free-body forces across a cut"),
    soon("results.xy", "XY Plot", Results, "Plots", "M9", "Plot a quantity against time or another"),
    soon("results.path", "Path Plot", Results, "Plots", "M9", "Plot a quantity along a path"),
    soon("results.cut", "Cut", Results, "Plots", "M9", "Section cuts through the model"),
    soon("results.display_group", "Display Group", Results, "Plots", "M9", "Show and hide parts of the model"),
    soon("results.streamlines", "Streamlines", Results, "Plots", "M12", "Streamlines and flow fields"),
    soon("results.read", "Read Results", Results, "Files", "M9", "Read results in public formats"),
    soon("results.convergence", "Convergence", Results, "Files", "M9", "Mesh convergence studies"),
    // ---- Tools ----
    ready("model.info", "Model Info", Tools, "Model", "Node and element counts and the model summary"),
    ready("file.open", "Open Model", Tools, "File", "Open a .modelcraft model"),
    ready("file.sample", "Samples", Tools, "File", "Open a sample model, ready to solve"),
    ready("file.save", "Save", Tools, "File", "Save the model to its .modelcraft file"),
    ready("file.save_as", "Save As", Tools, "File", "Save the model to a new .modelcraft file"),
    ready_more(
        "file.recover",
        "Recover",
        Tools,
        "File",
        "Open a model autosaved by an app that crashed or closed with unsaved changes",
        &["file.recovery.list", "file.recovery.discard"],
    ),
    ready("edit.undo", "Undo", Tools, "History", "Undo the last command that changed the model"),
    ready("edit.redo", "Redo", Tools, "History", "Redo the command undone last"),
];

/// The catalogue entry with this id.
pub fn find(id: &str) -> Option<&'static Capability> {
    CATALOGUE.iter().find(|c| c.id == id)
}

/// The entries of a tab (one run of the catalogue).
pub fn tab_entries(tab: Tab) -> &'static [Capability] {
    let start = CATALOGUE.iter().position(|c| c.tab == tab).unwrap_or(CATALOGUE.len());
    let rest = CATALOGUE.get(start..).unwrap_or_default();
    rest.get(..rest.iter().take_while(|c| c.tab == tab).count()).unwrap_or_default()
}

/// The panels of a tab in order, each the run of its entries (never empty).
pub fn panels(tab: Tab) -> impl Iterator<Item = &'static [Capability]> {
    tab_entries(tab).chunk_by(|a, b| a.panel == b.panel)
}

/// `docs/features.md`: every capability by tab and panel, ready or coming soon.
pub fn features_page() -> String {
    let ready = CATALOGUE.iter().filter(|c| c.command().is_some()).count();
    let mut md = String::from(
        "# Features\n\n\
         Generated from the capability catalogue (`crates/engine/src/catalogue.rs`) by\n\
         `cargo run -p modelcraft-cli -- features > docs/features.md`; a test fails when it is out of date.\n\
         Every entry is a toolbar button. **Ready** entries run their command; **coming soon** entries are\n\
         shown greyed with the roadmap milestone that brings them (ROADMAP.md).\n\n",
    );
    let _ = writeln!(md, "{ready} ready, {} coming soon.", CATALOGUE.len() - ready);
    for tab in Tab::ALL {
        let _ = write!(md, "\n## {}\n\n| Panel | Capability | Status | Command |\n|---|---|---|---|\n", tab.label());
        for c in tab_entries(tab) {
            let status = c.milestone().map_or_else(|| "ready".to_string(), |m| format!("coming soon ({m})"));
            let preset = c.preset();
            let main =
                c.command().map(|id| if preset.as_object().is_some_and(|o| !o.is_empty()) { format!("`{id} {preset}`") } else { format!("`{id}`") });
            let commands: Vec<String> = main.into_iter().chain(c.more.iter().map(|id| format!("`{id}`"))).collect();
            let _ = writeln!(md, "| {} | {} — {} | {status} | {} |", c.panel, c.label, c.tip, commands.join(", "));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands, find_command};

    #[test]
    fn every_command_is_claimed_and_every_ready_entry_runs_a_command() {
        let mut claimed: Vec<&str> = Vec::new();
        for c in CATALOGUE {
            if let Some(command) = c.command() {
                assert!(find_command(command).is_some(), "{}: `{command}` is not a registered command", c.id);
                claimed.push(command);
            } else {
                assert!(c.more.is_empty(), "{}: a planned entry has no commands", c.id);
            }
            for m in c.more {
                assert!(find_command(m).is_some(), "{}: `{m}` is not a registered command", c.id);
                claimed.push(m);
            }
        }
        for spec in commands() {
            let n = claimed.iter().filter(|c| **c == spec.id).count();
            assert!(n >= 1, "command `{}` has no catalogue entry (a ready entry or an entry's `more`)", spec.id);
        }
    }

    /// Deck entries follow the writers: one entry per format, ready exactly when its writer is in
    /// `modelcraft_decks::FORMATS`, running `deck.write` with its format.
    #[test]
    fn deck_entries_follow_the_writers() {
        let decks: Vec<(&Capability, &str)> =
            CATALOGUE.iter().filter_map(|c| if let Status::Deck { format, .. } = c.status { Some((c, format)) } else { None }).collect();
        for f in modelcraft_decks::FORMATS {
            assert!(decks.iter().any(|(_, name)| *name == f.name), "deck format `{}` has no catalogue entry", f.name);
        }
        for (c, format) in decks {
            let written = modelcraft_decks::format(format).is_some();
            assert_eq!(c.command(), written.then_some("deck.write"), "{}", c.id);
            assert_eq!(c.milestone().is_none(), written, "{}", c.id);
            assert_eq!(c.preset(), serde_json::json!({"format": format}));
        }
        assert_eq!(find("deck.write.abaqus").and_then(Capability::command), Some("deck.write"));
    }

    #[test]
    fn entries_are_well_formed() {
        let mut ids: Vec<&str> = CATALOGUE.iter().map(|c| c.id).collect();
        for c in CATALOGUE {
            assert!(c.id.split('.').count() >= 2 && c.id.chars().all(|ch| ch.is_ascii_lowercase() || ch == '.' || ch == '_'), "{}", c.id);
            assert!(!c.label.is_empty() && !c.panel.is_empty() && !c.tip.is_empty(), "{}", c.id);
            if let Some(m) = c.milestone() {
                assert!(m.strip_prefix('M').is_some_and(|n| n.parse::<u32>().is_ok_and(|n| (1..=14).contains(&n))), "{}: {m}", c.id);
            }
        }
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "catalogue ids are unique");
        // Tabs are runs in Tab::ALL order, and each panel is one run within its tab.
        let total: usize = Tab::ALL.iter().map(|t| tab_entries(*t).len()).sum();
        assert_eq!(total, CATALOGUE.len(), "every tab's entries are together, in Tab::ALL order");
        let order: Vec<Tab> =
            CATALOGUE.iter().map(|c| c.tab).collect::<Vec<_>>().chunk_by(|a, b| a == b).filter_map(|r| r.first().copied()).collect();
        assert_eq!(order, Tab::ALL);
        for tab in Tab::ALL {
            let names: Vec<&str> = panels(tab).filter_map(|p| p.first().map(|c| c.panel)).collect();
            assert!(!names.is_empty(), "{tab:?} has no entries");
            assert!(names.iter().enumerate().all(|(i, n)| !names[..i].contains(n)), "{tab:?}: a panel's entries are together: {names:?}");
        }
    }

    #[test]
    fn features_page_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/features.md");
        let committed = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(committed == features_page(), "docs/features.md is out of date: run `cargo run -p modelcraft-cli -- features > docs/features.md`");
    }
}
