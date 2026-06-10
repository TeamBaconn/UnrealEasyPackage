//! Build **profiles** (project-local) and **templates** (global) - the serde
//! schema, JSON persistence, and copy-on-create transforms. One `BuildConfig`
//! backs both stores; see `docs/data-storage.md`. Pure modules behind the thin
//! `#[tauri::command]` wrappers in `crate::commands`.
//!
//! `#![allow(dead_code)]` quiets helpers used only by the command layer (compiled
//! out of the `cargo test --lib` binary).
#![allow(dead_code)]

pub mod schema;
pub mod store;
pub mod templates;
