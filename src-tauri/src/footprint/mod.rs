//! **Footprint** (M5): scan the build artifacts an Unreal project scatters across
//! disk, categorize them by the step that created them, and reclaim space safely
//! (R3, `docs/build-footprint.md`). Kept Tauri-agnostic (pure fs + data), so every
//! piece is unit-testable over a `tempdir` - unlike `runner::exec`, nothing here
//! needs an `AppHandle`.
//!
//! - [`rules`] - pure categorization rules: each category → its paths/target role and the
//!   never-delete guardrail.
//! - `scan` - resolves the Save/Binaries/Intermediate/Cache node tree and sizes it via
//!   `walkdir`, off the UI thread (M5.2).
//! - `clean` - guarded deletion + reclaim accounting, by node id (tab) or category
//!   (Clean-up phase) (M5.3).

pub mod clean;
pub mod rules;
pub mod scan;
