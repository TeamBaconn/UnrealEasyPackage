//! `.uproject` descriptor parsing, enabled-plugin extraction, and C++ vs
//! Blueprint project-type detection. Pure functions - no Tauri, no globals.

use super::DetectError;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UProjectPlugin {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UProjectModule {
    pub name: String,
    #[serde(default, rename = "Type")]
    pub kind: String,
}

/// The subset of a `.uproject` we care about. Unknown keys are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UProject {
    #[serde(default)]
    pub file_version: u32,
    #[serde(default)]
    pub engine_association: String,
    #[serde(default)]
    pub modules: Vec<UProjectModule>,
    #[serde(default)]
    pub plugins: Vec<UProjectPlugin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
    /// Has a `Source/` tree with `*.Target.cs` / `*.Build.cs`.
    Cpp,
    Blueprint,
}

fn default_true() -> bool {
    true
}

/// Read + parse a `.uproject`, rejecting anything that isn't a plausible
/// descriptor (a real descriptor always carries a non-zero `FileVersion`).
pub fn parse_uproject(path: &Path) -> Result<UProject, DetectError> {
    let text = std::fs::read_to_string(path).map_err(|source| DetectError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let proj = parse_uproject_str(&text).map_err(|_| DetectError::Parse(path.to_path_buf()))?;
    if proj.file_version == 0 {
        return Err(DetectError::Invalid(format!(
            "{} has no FileVersion (not a project descriptor)",
            path.display()
        )));
    }
    Ok(proj)
}

/// Pure parse (no IO, no plausibility gate) - the unit-testable core.
pub fn parse_uproject_str(text: &str) -> Result<UProject, serde_json::Error> {
    serde_json::from_str(text)
}

/// Names of enabled plugins, in descriptor order.
pub fn enabled_plugins(proj: &UProject) -> Vec<String> {
    proj.plugins
        .iter()
        .filter(|p| p.enabled)
        .map(|p| p.name.clone())
        .collect()
}

/// A project is C++ if it has a `Source/` dir containing a `*.Target.cs` or
/// `*.Build.cs`; otherwise Blueprint-only. Informational - never gates builds.
pub fn detect_project_type(project_root: &Path) -> ProjectType {
    let source = project_root.join("Source");
    if source.is_dir()
        && (dir_has_suffix(&source, ".Target.cs", 3) || dir_has_suffix(&source, ".Build.cs", 3))
    {
        ProjectType::Cpp
    } else {
        ProjectType::Blueprint
    }
}

/// Whether any file under `dir` (up to `max_depth` levels) ends with `suffix`.
fn dir_has_suffix(dir: &Path, suffix: &str, max_depth: usize) -> bool {
    if max_depth == 0 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if dir_has_suffix(&path, suffix, max_depth - 1) {
                return true;
            }
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(suffix))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // The real SampleProject descriptor (16 plugins, 2 disabled).
    const SAMPLE_PROJECT: &str = r#"{
        "FileVersion": 3,
        "EngineAssociation": "{11111111-2222-3333-4444-555555555555}",
        "Category": "",
        "Description": "",
        "Modules": [
            { "Name": "SampleProject", "Type": "Runtime", "LoadingPhase": "Default" }
        ],
        "Plugins": [
            { "Name": "SamplePlugin", "Enabled": true },
            { "Name": "StateTree", "Enabled": true },
            { "Name": "CommonUI", "Enabled": true },
            { "Name": "GameplayAbilities", "Enabled": true },
            { "Name": "Chooser", "Enabled": true },
            { "Name": "SampleCamera", "Enabled": true },
            { "Name": "MotionWarping", "Enabled": true },
            { "Name": "ContextualAnimation", "Enabled": true },
            { "Name": "ModelingToolsEditorMode", "Enabled": true },
            { "Name": "ActorPalette", "Enabled": true },
            { "Name": "CommonUser", "Enabled": true },
            { "Name": "SamplePrompt", "Enabled": true },
            { "Name": "SampleSaveSystem", "Enabled": true },
            { "Name": "Paper2D", "Enabled": false },
            { "Name": "ArchVisCharacter", "Enabled": false },
            { "Name": "GameplayStateTree", "Enabled": true }
        ]
    }"#;

    #[test]
    fn parses_sampleproject_descriptor() {
        let p = parse_uproject_str(SAMPLE_PROJECT).unwrap();
        assert_eq!(p.file_version, 3);
        assert_eq!(p.engine_association, "{11111111-2222-3333-4444-555555555555}");
        assert_eq!(p.modules.len(), 1);
        assert_eq!(p.modules[0].name, "SampleProject");
        assert_eq!(p.modules[0].kind, "Runtime");
        assert_eq!(p.plugins.len(), 16);
    }

    #[test]
    fn enabled_plugins_excludes_disabled() {
        let p = parse_uproject_str(SAMPLE_PROJECT).unwrap();
        let enabled = enabled_plugins(&p);
        assert_eq!(enabled.len(), 14);
        assert!(enabled.contains(&"GameplayAbilities".to_string()));
        assert!(!enabled.contains(&"Paper2D".to_string()));
        assert!(!enabled.contains(&"ArchVisCharacter".to_string()));
    }

    #[test]
    fn plugin_enabled_defaults_true_when_absent() {
        let p = parse_uproject_str(r#"{"FileVersion":3,"Plugins":[{"Name":"Foo"}]}"#).unwrap();
        assert_eq!(enabled_plugins(&p), vec!["Foo".to_string()]);
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_uproject_str("{ not json").is_err());
    }

    #[test]
    fn project_type_cpp_when_source_has_target_cs() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("Source")).unwrap();
        fs::write(dir.path().join("Source/SampleProject.Target.cs"), "x").unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Cpp);
    }

    #[test]
    fn project_type_cpp_when_build_cs_nested() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("Source/SampleProject")).unwrap();
        fs::write(dir.path().join("Source/SampleProject/SampleProject.Build.cs"), "x").unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Cpp);
    }

    #[test]
    fn project_type_blueprint_without_source() {
        let dir = tempdir().unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Blueprint);
    }
}
