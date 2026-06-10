//! Tauri-agnostic Unreal project & engine detection.
//!
//! Submodules are pure logic where possible (inputs → data / errors) so they're
//! easy to test and reuse; the `#[tauri::command]` wrappers live in
//! `crate::commands`. `#![allow(dead_code)]` quiets a few forward-looking
//! helpers that only tests use today.
#![allow(dead_code)]

pub mod args;
pub mod engine;
pub mod maps;
pub mod targets;
pub mod uplugin;
pub mod uproject;

use engine::EngineInfo;
use maps::MapInventory;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use targets::TargetInfo;
use uproject::ProjectType;

/// Errors surfaced by detection. Coarse on purpose; the command layer maps these
/// to a serializable `AppError` for the UI.
#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} is not a valid .uproject (could not parse)")]
    Parse(PathBuf),
    #[error("{0}")]
    Invalid(String),
    #[error("no valid engine for association {association}")]
    EngineNotFound { association: String },
}

/// Everything detected when a project is opened (fresh, never persisted).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DetectedProject {
    pub name: String,
    pub uproject_path: String,
    pub project_root: String,
    pub engine_association: String,
    pub engine: EngineInfo,
    pub project_type: ProjectType,
    pub targets: Vec<TargetInfo>,
    pub maps: MapInventory,
    pub plugins: Vec<String>,
}

/// Full detection for an opened project. `saved_engine` is the user-confirmed
/// engine path for this association (from storage), tried before auto-resolve.
pub fn detect_project(
    uproject_path: &Path,
    saved_engine: Option<&Path>,
) -> Result<DetectedProject, DetectError> {
    let proj = uproject::parse_uproject(uproject_path)?;
    let project_root = uproject_path
        .parent()
        .ok_or_else(|| DetectError::Invalid("project has no parent directory".into()))?;
    let name = uproject_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Project")
        .to_string();
    let engine = engine::resolve_engine(&proj.engine_association, saved_engine).ok_or_else(|| {
        DetectError::EngineNotFound {
            association: proj.engine_association.clone(),
        }
    })?;

    Ok(DetectedProject {
        name,
        uproject_path: uproject_path.display().to_string(),
        project_root: project_root.display().to_string(),
        project_type: uproject::detect_project_type(project_root),
        targets: targets::scan_targets(&project_root.join("Source")),
        maps: maps::scan_maps(project_root),
        plugins: uproject::enabled_plugins(&proj),
        engine,
        engine_association: proj.engine_association,
    })
}

/// Everything detected when a **plugin** is opened (fresh, never persisted). Lean
/// by design: a plugin packages standalone (no engine association, no targets/maps),
/// so this carries only what the Actions tab needs - identity + the `.uplugin`'s
/// `EngineVersion` hint to pre-select a matching engine.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DetectedPlugin {
    /// File stem of the `.uplugin` (the canonical plugin name UAT uses).
    pub name: String,
    pub friendly_name: String,
    pub version_name: String,
    pub uplugin_path: String,
    /// Directory containing the `.uplugin` (where `-package` output and the
    /// `.uep/local.json` remembered-engines file are anchored).
    pub plugin_root: String,
    /// `.uplugin`'s `EngineVersion` (e.g. `"5.5"`), or `None` if unset.
    pub engine_version: Option<String>,
}

/// Detection for an opened plugin - descriptor parse only (no engine resolution;
/// the compile engine is chosen per-package).
pub fn detect_plugin(uplugin_path: &Path) -> Result<DetectedPlugin, DetectError> {
    let plugin = uplugin::parse_uplugin(uplugin_path)?;
    let plugin_root = uplugin_path
        .parent()
        .ok_or_else(|| DetectError::Invalid("plugin has no parent directory".into()))?;
    let name = uplugin_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Plugin")
        .to_string();
    let friendly_name = if plugin.friendly_name.trim().is_empty() {
        name.clone()
    } else {
        plugin.friendly_name.clone()
    };
    let engine_version = Some(plugin.engine_version.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    Ok(DetectedPlugin {
        name,
        friendly_name,
        version_name: plugin.version_name,
        uplugin_path: uplugin_path.display().to_string(),
        plugin_root: plugin_root.display().to_string(),
        engine_version,
    })
}
