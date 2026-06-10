//! Pure execution-plan builder.
//!
//! The arg builder (`unreal::args`) emits the run's **execution units** in
//! pipeline order: `[Build (Editor)?, Build, Cook, Stage·Pak·Archive, Copy
//! Extras?, Clean-up?]`. This groups them into sequential **stages** (barriers)
//! with the one MVP overlap the engine allows - **Build (game) ∥ Cook** share a
//! stage (`docs/build-commands.md` §8). The executor runs stages in order and the
//! units within a stage concurrently; the graph renders each unit as a node and
//! `stage index` as its column (parallel siblings share a column).
//!
//! Deriving the schedule here (not inside the async executor) keeps it pure and
//! unit-testable; the executor just walks the result.

use crate::pipeline::PhaseId;
use crate::unreal::args::PhaseCommand;

/// One scheduling stage: indices into the units slice that run concurrently.
/// Stages themselves run strictly in order (a barrier between each).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    pub units: Vec<usize>,
}

fn is_editor_build(c: &PhaseCommand) -> bool {
    c.phase == PhaseId::Build && c.label.contains("Editor")
}

/// Group execution units into ordered stages. Robust to the unit set actually
/// emitted (Blueprint projects have no editor build; app phases appear only when
/// enabled): scan by role rather than assume positions.
pub fn plan(units: &[PhaseCommand]) -> Vec<Stage> {
    let mut stages: Vec<Stage> = Vec::new();

    // ① the editor build(s) first - cooking needs a built editor (C++ only).
    for (i, c) in units.iter().enumerate() {
        if is_editor_build(c) {
            stages.push(Stage { units: vec![i] });
        }
    }

    // ② the one real overlap: game build ∥ cook (both after the editor exists).
    let game_build = units.iter().position(|c| c.phase == PhaseId::Build && !is_editor_build(c));
    let cook = units.iter().position(|c| c.phase == PhaseId::Cook);
    let overlap: Vec<usize> = [game_build, cook].into_iter().flatten().collect();
    if !overlap.is_empty() {
        stages.push(Stage { units: overlap });
    }

    // ③ the strict tail - Stage·Pak·Archive, then the app phases - each its own
    //    barrier, in emitted order.
    for (i, c) in units.iter().enumerate() {
        match c.phase {
            PhaseId::Stage | PhaseId::CopyExtras | PhaseId::Cleanup => {
                stages.push(Stage { units: vec![i] })
            }
            _ => {}
        }
    }

    stages
}

/// The stage index (graph column) for each unit, indexed by unit position.
/// Units not scheduled (shouldn't happen for a well-formed plan) default to 0.
pub fn levels(units: &[PhaseCommand]) -> Vec<u32> {
    let mut out = vec![0u32; units.len()];
    for (level, stage) in plan(units).iter().enumerate() {
        for &u in &stage.units {
            out[u] = level as u32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::PhaseKind;

    fn unit(phase: PhaseId, label: &str) -> PhaseCommand {
        PhaseCommand {
            phase,
            label: label.to_string(),
            kind: if matches!(phase, PhaseId::CopyExtras | PhaseId::Cleanup) {
                PhaseKind::App
            } else {
                PhaseKind::External
            },
            program: Some("prog".into()),
            args: vec![],
            preview: String::new(),
        }
    }

    /// The C++ shape the arg builder emits for a pak'd profile with both app phases.
    fn cpp_units() -> Vec<PhaseCommand> {
        vec![
            unit(PhaseId::Build, "Build (Editor)"),
            unit(PhaseId::Build, "Build"),
            unit(PhaseId::Cook, "Cook"),
            unit(PhaseId::Stage, "Stage · Pak · Archive"),
            unit(PhaseId::CopyExtras, "Copy Extras"),
            unit(PhaseId::Cleanup, "Clean-up"),
        ]
    }

    #[test]
    fn cpp_schedule_overlaps_game_build_and_cook() {
        let u = cpp_units();
        let s = plan(&u);
        // editor(0) | game∥cook(1,2) | stage(3) | copy(4) | clean(5)
        assert_eq!(s.len(), 5);
        assert_eq!(s[0].units, vec![0]); // editor build alone
        assert_eq!(s[1].units, vec![1, 2]); // game build ∥ cook - the one overlap
        assert_eq!(s[2].units, vec![3]); // stage·pak·archive
        assert_eq!(s[3].units, vec![4]); // copy extras
        assert_eq!(s[4].units, vec![5]); // clean-up
    }

    #[test]
    fn levels_match_stage_columns() {
        let u = cpp_units();
        assert_eq!(levels(&u), vec![0, 1, 1, 2, 3, 4]);
    }

    #[test]
    fn blueprint_has_no_editor_build_but_still_overlaps() {
        let u = vec![
            unit(PhaseId::Build, "Build"),
            unit(PhaseId::Cook, "Cook"),
            unit(PhaseId::Stage, "Stage · Pak · Archive"),
        ];
        let s = plan(&u);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].units, vec![0, 1]); // build ∥ cook from the start
        assert_eq!(s[1].units, vec![2]);
    }

    #[test]
    fn app_phases_absent_when_not_enabled() {
        let u = vec![
            unit(PhaseId::Build, "Build"),
            unit(PhaseId::Cook, "Cook"),
            unit(PhaseId::Stage, "Stage · Pak · Archive"),
        ];
        let s = plan(&u);
        assert!(s.iter().all(|st| st.units.iter().all(|&i| u[i].phase != PhaseId::CopyExtras)));
    }
}
