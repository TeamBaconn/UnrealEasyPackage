//! Build **history** (M4): a lean, self-contained JSON record per build under
//! `.uep/history/<buildId>/` plus auto-derived flat tags (`docs/data-storage.md`,
//! `docs/requirement.md` R5). The runner writes a record on finish; the Build tab
//! lists them, the Dashboard aggregates them, and Build Logs replays a past one.
//!
//! The normalized **SQLite index** ([`index`]) is the derived, rebuildable cache the
//! Build tab pages/filters against; the JSON records remain the source of truth, so
//! the index can be deleted and is reconciled on a folder-count drift.
#![allow(dead_code)]

pub mod index;
pub mod schema;
pub mod store;
pub mod tags;
