//! Where the model is in the analysis workflow: the setup checklist ([`Session::progress`]).
//!
//! The stages run in the usual order — geometry, preparation, mesh, material, section, sets,
//! boundary conditions, loads, step, solve, results — each with whether it is done, the next one
//! to do, still to do, blocked (its command cannot run yet) or optional, and the command that
//! provides it. It is a guide, never a gate: a script can define things in any order, and
//! [`crate::requirement`] still decides what each command needs.
//!
//! Each stage is a [`Requirement`] check (a count), and Solve also reads the model check's first
//! error, so the whole list costs one [`fieldcraft_model::Model::check`]: the desktop reads it
//! once per model change, and `model.check` returns it as `progress`.

use fieldcraft_model::{Procedure, Severity};
use serde::Serialize;
use serde_json::{Value, json};

use crate::Session;
use crate::requirement::Requirement;

/// A step of the analysis workflow, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Geometry,
    /// Seeds and partitions that steer the mesh (optional).
    Prepare,
    Mesh,
    Material,
    Section,
    Sets,
    BoundaryConditions,
    Loads,
    Step,
    Solve,
    Results,
}

impl Stage {
    pub const ALL: [Stage; 11] = [
        Stage::Geometry,
        Stage::Prepare,
        Stage::Mesh,
        Stage::Material,
        Stage::Section,
        Stage::Sets,
        Stage::BoundaryConditions,
        Stage::Loads,
        Stage::Step,
        Stage::Solve,
        Stage::Results,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Stage::Geometry => "Geometry",
            Stage::Prepare => "Seeds & partitions",
            Stage::Mesh => "Mesh",
            Stage::Material => "Material",
            Stage::Section => "Section",
            Stage::Sets => "Sets",
            Stage::BoundaryConditions => "Boundary conditions",
            Stage::Loads => "Loads",
            Stage::Step => "Step",
            Stage::Solve => "Solve",
            Stage::Results => "Results",
        }
    }

    /// What it checks; its provider is the command the stage starts.
    fn requirement(self) -> Requirement {
        match self {
            Stage::Geometry => Requirement::Geometry,
            Stage::Prepare => Requirement::Seed,
            Stage::Mesh => Requirement::Analysable,
            Stage::Material => Requirement::Material,
            Stage::Section => Requirement::Section,
            Stage::Sets => Requirement::Set,
            Stage::BoundaryConditions => Requirement::BoundaryCondition,
            Stage::Loads => Requirement::Load,
            Stage::Step => Requirement::Step,
            Stage::Solve => Requirement::Result,
            Stage::Results => Requirement::ResultsShown,
        }
    }
}

/// Where a stage is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Done,
    /// The first stage still to do.
    Next,
    /// Still to do after the next one; its command can already run.
    Todo,
    /// Its command cannot run yet (`detail` says why).
    Blocked,
    /// Not done, and not needed for this model.
    Optional,
}

/// One row of the checklist.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StageState {
    pub stage: Stage,
    pub label: &'static str,
    pub status: Status,
    /// The command that does it.
    pub command: &'static str,
    /// Where that command is on the toolbar ("Properties › Material").
    pub place: String,
    /// Why it is blocked: what its command misses, or the model check's first error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Session {
    /// The setup checklist for the current model (one model check).
    pub fn progress(&self) -> Vec<StageState> {
        let first_error = self.model.check().into_iter().find(|d| d.severity == Severity::Error);
        self.progress_given(first_error.map(|d| d.to_string()))
    }

    /// The checklist, given the model check's first error (`model.check` has run it already).
    pub(crate) fn progress_given(&self, first_error: Option<String>) -> Vec<StageState> {
        let m = &self.model;
        let analysable = Requirement::Analysable.met(self);
        // Models without elements (point masses, fluid domains) need no material or section.
        let elementless = analysable && m.elements().is_empty();
        let modal_only = !m.steps().is_empty() && m.steps().iter().all(|st| matches!(st.procedure, Procedure::Frequency { .. }));
        let mut next_given = false;
        Stage::ALL
            .into_iter()
            .map(|stage| {
                let r = stage.requirement();
                let done = r.met(self) || (stage == Stage::Geometry && analysable);
                let optional = match stage {
                    Stage::Prepare => true,
                    Stage::Material | Stage::Section => elementless,
                    Stage::Loads => modal_only,
                    _ => false,
                };
                let command = r.provider();
                let blocked = match crate::find_command(command).map(|spec| self.availability(spec)) {
                    Some(Err(u)) => Some(u.lines()),
                    _ if stage == Stage::Solve => first_error.clone(),
                    _ => None,
                };
                let (status, detail) = if done {
                    (Status::Done, None)
                } else if optional {
                    (Status::Optional, None)
                } else if blocked.is_some() {
                    next_given = true;
                    (Status::Blocked, blocked)
                } else if next_given {
                    (Status::Todo, None)
                } else {
                    next_given = true;
                    (Status::Next, None)
                };
                StageState { stage, label: stage.label(), status, command, place: r.place(), detail }
            })
            .collect()
    }
}

/// The checklist as JSON, with the next stage's command first for agents:
/// `{"next": {"stage", "command"} | null, "stages": [...]}`.
pub fn to_json(stages: &[StageState]) -> Value {
    let next = stages.iter().find(|s| matches!(s.status, Status::Next | Status::Blocked));
    json!({
        "next": next.map(|s| json!({"stage": s.stage, "command": s.command, "status": s.status, "detail": s.detail})),
        "stages": stages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Status::*;

    fn statuses(s: &Session) -> Vec<(Stage, Status)> {
        s.progress().into_iter().map(|st| (st.stage, st.status)).collect()
    }

    fn status(s: &Session, stage: Stage) -> Status {
        statuses(s).into_iter().find(|(st, _)| *st == stage).map(|(_, st)| st).unwrap()
    }

    #[test]
    fn empty_model_starts_with_geometry() {
        let s = Session::default();
        let p = s.progress();
        assert_eq!(p.len(), Stage::ALL.len());
        assert_eq!((p[0].stage, p[0].status, p[0].command), (Stage::Geometry, Next, "geometry.open"));
        assert_eq!(status(&s, Stage::Prepare), Optional);
        // Meshing needs geometry; solving needs a mesh.
        assert_eq!(status(&s, Stage::Mesh), Blocked);
        assert!(p[2].detail.as_deref().is_some_and(|d| d.contains("geometry")));
        assert_eq!(status(&s, Stage::Material), Todo);
        assert_eq!(status(&s, Stage::Solve), Blocked);
        assert_eq!(statuses(&s).iter().filter(|(_, st)| *st == Next).count(), 1);
    }

    #[test]
    fn a_meshed_model_asks_for_its_material_next() {
        let mut s = Session::default();
        let cube = [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]];
        s.execute("mesh.block", &json!({"body": "B", "kind": "hex8", "corners": cube, "divisions": [1, 1, 1]})).unwrap();
        assert_eq!(status(&s, Stage::Geometry), Done);
        assert_eq!(status(&s, Stage::Mesh), Done);
        assert_eq!(status(&s, Stage::Material), Next);
        assert_eq!(status(&s, Stage::Section), Blocked);
        assert_eq!(status(&s, Stage::Step), Todo);
    }

    #[test]
    fn a_defined_model_is_ready_to_solve_then_to_show_results() {
        let mut s = Session::default();
        crate::cmd::solve::tests::cantilever(&mut s);
        let before = statuses(&s);
        for (stage, st) in &before[..9] {
            assert!(matches!(st, Done | Optional), "{stage:?} {st:?}");
        }
        assert_eq!(status(&s, Stage::Solve), Next);
        assert_eq!(to_json(&s.progress())["next"]["command"], "solve.run");
        s.execute("solve.run", &json!({})).unwrap();
        // A solve shows its results.
        assert!(statuses(&s).iter().all(|(_, st)| matches!(st, Done | Optional)));
        assert!(to_json(&s.progress())["next"].is_null());
        s.execute("results.clear", &json!({})).unwrap();
        assert_eq!(status(&s, Stage::Solve), Done);
        assert_eq!((status(&s, Stage::Results), to_json(&s.progress())["next"]["command"].clone()), (Next, json!("results.contour")));
    }

    #[test]
    fn model_check_returns_the_checklist() {
        let mut s = Session::default();
        let v = s.execute("model.check", &json!({})).unwrap();
        assert_eq!(v["progress"]["next"]["command"], "geometry.open");
        assert_eq!(v["progress"]["stages"].as_array().map(Vec::len), Some(Stage::ALL.len()));
    }
}
