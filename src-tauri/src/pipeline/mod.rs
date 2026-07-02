//! The pipeline **phase registry** - the extensibility seam.
//!
//! The pipeline is *data, not hardcode* (see `docs/build-commands.md` §8): the
//! executor, the Jenkins-style graph, the profile editor, the per-phase timing
//! record, and the footprint map all **derive** from this registry. Adding a
//! phase is one entry here, not a rewrite.
//!
//! M2 uses the registry for its static metadata (order, deps, requiredness, kind,
//! and the `gated_by` edges the editor greys toggles by); the per-phase enabled
//! state lives on each phase's cfg (`profiles::schema`), and the editor derives
//! locked/enabled from `gated_by` itself. The arg builder (`unreal::args`) tags
//! each emitted command with its `PhaseId`, and the runner (M3) walks this graph.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Stable phase keys - the editor's per-phase keys. **camelCase on the wire**, so
/// the serialized id (`"copyExtras"`) matches the `Phases` cfg field names and the
/// TS phase keys (one source of truth; the frontend indexes phase config by this id
/// with no mapping). The human-facing name lives in `PhaseInfo::label`, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PhaseId {
    Build,
    Cook,
    Stage,
    Pak,
    Archive,
    CopyExtras,
    SteamUpload,
    Cleanup,
    /// Implicit Steam sign-in **preflight** - emitted before Build (only when the Steam upload
    /// phase is enabled) so an interactive login happens up front, not after a finished build.
    /// Not a registry phase / editor toggle (like the implicit editor build); it exists only
    /// as an emitted execution unit + graph node.
    SteamLogin,
}

/// Editor metadata. Every MVP phase is now toggleable, so the registry marks them
/// all `Optional`; the Stage-gate for Pak/Archive lives in `gated_by`, not here. The
/// other variants are kept for forthcoming phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Requiredness {
    /// Always runs, toggle locked on. (Unused in the MVP - all phases toggle.)
    Always,
    /// Required only for C++ projects.
    ForCpp,
    /// Pulled on by another enabled phase.
    IfDependedOn,
    /// Free - the user chooses (every MVP phase).
    Optional,
}

/// `External` spawns a child process (UBT / RunUAT); `App` is an in-process task
/// (fs copy, footprint cleanup) with no external command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PhaseKind {
    External,
    App,
}

/// A registry entry. Static metadata only - the command/action is produced by the
/// arg builder, keyed off `id`.
#[derive(Debug, Clone, Copy)]
pub struct PhaseDef {
    pub id: PhaseId,
    pub label: &'static str,
    /// Canonical pipeline position (the graph + record sort by this).
    pub order: u32,
    pub depends_on: &'static [PhaseId],
    /// Phases that must be enabled for this one to run at all - the editor greys
    /// (locks) this phase when any of them is off. Distinct from `depends_on`
    /// (DAG order, reusable across runs via `-skip*`): a gate means this phase runs
    /// *inside* the gating phase's output, so it can't run without it. Data-driven
    /// locking - the editor derives a phase's greyed state from this; nothing is
    /// hardcoded per phase.
    pub gated_by: &'static [PhaseId],
    pub requiredness: Requiredness,
    pub kind: PhaseKind,
}

/// Serializable view of a registry entry (for the editor over IPC).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PhaseInfo {
    pub id: PhaseId,
    pub label: String,
    pub order: u32,
    pub depends_on: Vec<PhaseId>,
    /// Gating phases (see `PhaseDef::gated_by`) - the editor derives a phase's
    /// locked state from this, so adding a gated phase needs no frontend change.
    pub gated_by: Vec<PhaseId>,
    pub requiredness: Requiredness,
    pub kind: PhaseKind,
}

/// The MVP registry, in pipeline order. Build ∥ Cook (no edge between them - both
/// sit behind the implicit editor build), then the strict Stage→Pak→Archive
/// chain, then the two app-owned phases. Deferred entries (Turnkey, server, DLC,
/// extra platforms) slot in here later with no executor/UI change.
const REGISTRY: &[PhaseDef] = &[
    PhaseDef {
        id: PhaseId::Build,
        label: "Build",
        order: 10,
        depends_on: &[],
        gated_by: &[],
        requiredness: Requiredness::Optional,
        kind: PhaseKind::External,
    },
    PhaseDef {
        id: PhaseId::Cook,
        label: "Cook",
        order: 20,
        depends_on: &[],
        gated_by: &[],
        requiredness: Requiredness::Optional,
        kind: PhaseKind::External,
    },
    PhaseDef {
        id: PhaseId::Stage,
        label: "Stage",
        order: 30,
        depends_on: &[PhaseId::Build, PhaseId::Cook],
        gated_by: &[],
        requiredness: Requiredness::Optional,
        kind: PhaseKind::External,
    },
    PhaseDef {
        id: PhaseId::Pak,
        label: "Pak",
        order: 40,
        depends_on: &[PhaseId::Stage],
        gated_by: &[PhaseId::Stage],
        requiredness: Requiredness::Optional,
        kind: PhaseKind::External,
    },
    PhaseDef {
        id: PhaseId::Archive,
        label: "Archive",
        order: 50,
        depends_on: &[PhaseId::Pak],
        gated_by: &[PhaseId::Stage],
        requiredness: Requiredness::Optional,
        kind: PhaseKind::External,
    },
    PhaseDef {
        id: PhaseId::CopyExtras,
        label: "Copy Extras",
        order: 60,
        depends_on: &[PhaseId::Archive],
        gated_by: &[],
        requiredness: Requiredness::Optional,
        kind: PhaseKind::App,
    },
    PhaseDef {
        id: PhaseId::SteamUpload,
        label: "Upload to Steam",
        order: 65,
        // Uploads the archived tree, so it depends on and is gated by Archive (the
        // editor greys it when Archive is off, like Pak/Archive gate on Stage).
        depends_on: &[PhaseId::Archive],
        gated_by: &[PhaseId::Archive],
        requiredness: Requiredness::Optional,
        kind: PhaseKind::External,
    },
    PhaseDef {
        id: PhaseId::Cleanup,
        label: "Clean-up",
        order: 70,
        depends_on: &[PhaseId::CopyExtras],
        gated_by: &[],
        requiredness: Requiredness::Optional,
        kind: PhaseKind::App,
    },
];

/// The phase definitions, in pipeline order.
pub fn registry() -> &'static [PhaseDef] {
    REGISTRY
}

/// Serializable registry for the editor.
pub fn registry_info() -> Vec<PhaseInfo> {
    REGISTRY
        .iter()
        .map(|d| PhaseInfo {
            id: d.id,
            label: d.label.to_string(),
            order: d.order,
            depends_on: d.depends_on.to_vec(),
            gated_by: d.gated_by.to_vec(),
            requiredness: d.requiredness,
            kind: d.kind,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_ordered_and_complete() {
        let r = registry();
        assert_eq!(r.len(), 8);
        let orders: Vec<u32> = r.iter().map(|d| d.order).collect();
        let mut sorted = orders.clone();
        sorted.sort();
        assert_eq!(orders, sorted, "registry must be declared in pipeline order");
        // The two app-owned terminal phases.
        assert_eq!(
            r.iter().filter(|d| d.kind == PhaseKind::App).map(|d| d.id).collect::<Vec<_>>(),
            vec![PhaseId::CopyExtras, PhaseId::Cleanup]
        );
    }

    #[test]
    fn build_and_cook_have_no_edge_between_them() {
        // The one parallelizable pair: neither depends on the other.
        let by = |id| registry().iter().find(|d| d.id == id).unwrap();
        assert!(!by(PhaseId::Build).depends_on.contains(&PhaseId::Cook));
        assert!(!by(PhaseId::Cook).depends_on.contains(&PhaseId::Build));
        // Stage joins them.
        assert!(by(PhaseId::Stage).depends_on.contains(&PhaseId::Build));
        assert!(by(PhaseId::Stage).depends_on.contains(&PhaseId::Cook));
    }

    #[test]
    fn only_pak_and_archive_are_stage_gated() {
        // Gating is data: Pak/Archive run inside the staged tree ⇒ gated by Stage;
        // everything else is freely toggleable. The editor derives its greyed
        // (locked) state from this `gated_by` - there is no per-phase code.
        let by = |id| registry().iter().find(|d| d.id == id).unwrap();
        assert_eq!(by(PhaseId::Pak).gated_by, &[PhaseId::Stage]);
        assert_eq!(by(PhaseId::Archive).gated_by, &[PhaseId::Stage]);
        // Steam upload is gated too, but by Archive (it uploads the archived tree), not Stage.
        assert_eq!(by(PhaseId::SteamUpload).gated_by, &[PhaseId::Archive]);
        for id in [PhaseId::Build, PhaseId::Cook, PhaseId::Stage, PhaseId::CopyExtras, PhaseId::Cleanup] {
            assert!(by(id).gated_by.is_empty(), "{id:?} is not gated");
        }
    }
}
