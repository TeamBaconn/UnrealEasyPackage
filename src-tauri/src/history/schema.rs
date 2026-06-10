//! The lean, self-contained **build record** (`docs/data-storage.md` §"Build
//! records") persisted per build at `.uep/history/<buildId>/metadata.json`. No
//! profile snapshot - its categorical identity is the flat
//! `tags` list (generated + reversed by [`super::tags`]). Numbers are `f64` for
//! the IPC boundary (specta forbids `u64`).

use serde::{Deserialize, Serialize};

/// Current on-disk record schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Per-phase timing - start offset + duration reconstruct overlap (a Gantt) and
/// trends without a redundant end stamp. One entry per executed pipeline phase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PhaseTiming {
    /// The execution unit's label (e.g. "Build", "Cook", "Stage · Pak · Archive").
    pub phase: String,
    /// Seconds from build start.
    pub start_offset: f64,
    /// Seconds the phase ran.
    pub duration: f64,
    /// `Success` | `Failed` | `Skipped` | `Cancelled`.
    pub status: String,
    /// `external` (child process) | `app` (in-process task). `#[serde(default)]` ⇒
    /// older records deserialize with `""`, and the UI infers it from the label.
    #[serde(default)]
    pub kind: String,
    /// Resolved command line for an external phase (empty for app phases) - lets the
    /// Build Logs "Command" island show it on replay.
    #[serde(default)]
    pub command: String,
}

/// One build's record. Self-contained and lean; the SQLite index ([`super::index`])
/// is derived from these files, so they remain the source of truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BuildRecord {
    pub schema_version: u32,
    /// Stable id (the run id); also the `<buildId>/` folder name.
    pub build_id: String,
    /// Build start, epoch milliseconds.
    pub started_at_ms: f64,
    /// Wall-clock total seconds (may be < Σ phase durations when parallel).
    pub duration: f64,
    /// Final archived build size in bytes (0 if the build never archived).
    pub build_size: f64,
    /// Warning / error lines tallied from the streamed build output by the
    /// [`super::super::runner::classify`] classifier. Counts **real child-process
    /// output only** - executor-synthesized lines (command echo, "exited with code
    /// N", cancel notices) are excluded. `#[serde(default)]` ⇒ pre-feature records
    /// read 0 and still deserialize (so they never vanish from history).
    #[serde(default)]
    pub warning_count: u32,
    #[serde(default)]
    pub error_count: u32,
    pub output_path: String,
    /// Output dir's modified time (epoch ms) - the "Open location" integrity basis.
    pub output_mtime_ms: f64,
    pub phases: Vec<PhaseTiming>,
    /// Flat, reverse-mappable tags: platform, config, target, status.
    pub tags: Vec<String>,
}
