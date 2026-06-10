//! `.uplugin` descriptor parsing + lean plugin detection. Pure functions - no
//! Tauri, no globals (mirrors `uproject.rs`).
//!
//! A plugin is packaged **standalone** via `RunUAT BuildPlugin` (it does not need a
//! host `.uproject`; UAT compiles it against a chosen engine - see
//! `docs/build-commands.md` §9), so detection here only reads the descriptor: no
//! engine association, no targets, no maps. The engine to compile with is picked
//! per-package in the Actions tab.

use super::DetectError;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UPluginModule {
    pub name: String,
    #[serde(default, rename = "Type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UPluginDependency {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// The subset of a `.uplugin` we care about. Unknown keys are ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UPlugin {
    #[serde(default)]
    pub file_version: u32,
    #[serde(default)]
    pub friendly_name: String,
    #[serde(default)]
    pub version_name: String,
    /// `EngineVersion` hint (e.g. `"5.5"`) - used only to pre-select a matching
    /// engine in the package action; never a hard gate.
    #[serde(default)]
    pub engine_version: String,
    #[serde(default)]
    pub modules: Vec<UPluginModule>,
    #[serde(default)]
    pub plugins: Vec<UPluginDependency>,
}

fn default_true() -> bool {
    true
}

/// Read + parse a `.uplugin`, rejecting anything that isn't a plausible descriptor
/// (a real descriptor always carries a non-zero `FileVersion`).
pub fn parse_uplugin(path: &Path) -> Result<UPlugin, DetectError> {
    let text = std::fs::read_to_string(path).map_err(|source| DetectError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let plugin = parse_uplugin_str(&text).map_err(|_| DetectError::Parse(path.to_path_buf()))?;
    if plugin.file_version == 0 {
        return Err(DetectError::Invalid(format!(
            "{} has no FileVersion (not a plugin descriptor)",
            path.display()
        )));
    }
    Ok(plugin)
}

/// Pure parse (no IO, no plausibility gate) - the unit-testable core.
pub fn parse_uplugin_str(text: &str) -> Result<UPlugin, serde_json::Error> {
    serde_json::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A generic plugin descriptor (genericized - no real plugin name; see the
    // public-repo rule in CLAUDE.md).
    const SAMPLE_PLUGIN: &str = r#"{
        "FileVersion": 3,
        "Version": 6,
        "VersionName": "1.2.0",
        "FriendlyName": "Sample Plugin",
        "Description": "A sample plugin",
        "Category": "Gameplay",
        "CanContainContent": true,
        "Installed": true,
        "EngineVersion": "5.5",
        "Modules": [
            { "Name": "SamplePluginRuntime", "Type": "Runtime", "LoadingPhase": "Default" },
            { "Name": "SamplePluginEditor", "Type": "Editor", "LoadingPhase": "Default" }
        ],
        "Plugins": [
            { "Name": "GameplayAbilities", "Enabled": true }
        ]
    }"#;

    #[test]
    fn parses_sample_plugin_descriptor() {
        let p = parse_uplugin_str(SAMPLE_PLUGIN).unwrap();
        assert_eq!(p.file_version, 3);
        assert_eq!(p.friendly_name, "Sample Plugin");
        assert_eq!(p.version_name, "1.2.0");
        assert_eq!(p.engine_version, "5.5");
        assert_eq!(p.modules.len(), 2);
        assert_eq!(p.modules[0].name, "SamplePluginRuntime");
        assert_eq!(p.modules[1].kind, "Editor");
        assert_eq!(p.plugins.len(), 1);
        assert_eq!(p.plugins[0].name, "GameplayAbilities");
    }

    #[test]
    fn missing_optional_fields_default() {
        let p = parse_uplugin_str(r#"{"FileVersion":3}"#).unwrap();
        assert_eq!(p.file_version, 3);
        assert!(p.friendly_name.is_empty());
        assert!(p.version_name.is_empty());
        assert!(p.modules.is_empty());
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_uplugin_str("{ not json").is_err());
    }
}
