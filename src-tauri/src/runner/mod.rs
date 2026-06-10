//! The **runner** (M3): executes a profile's resolved pipeline as a Jenkins-style
//! DAG of separately-timed phases, streams classified logs, and supports cancel
//! (`docs/requirement.md` R4, `docs/build-commands.md` §8).
//!
//! Split so the tricky logic stays pure + unit-tested while the process/IPC layer
//! is compiled out of the test binary (it needs Tauri's `AppHandle`/event runtime
//! - the same reason `commands`/`state` are `cfg(not(test))`):
//!
//! - [`classify`] - pure per-line severity classifier (tested vs R4 cases).
//! - [`plan`] - pure execution-plan builder (the Build ∥ Cook overlap; tested).
//! - `exec` - the async executor + event/IPC types (`spawn_run`, `RunSnapshot`, …).

pub mod classify;
pub mod plan;

#[cfg(not(test))]
mod exec;
#[cfg(not(test))]
pub use exec::{
    spawn_commandlet, spawn_plugin_package, spawn_run, ActiveRun, CommandletInputs, LogLine,
    PluginPackageInputs, RunInputs, RunSnapshot,
};
