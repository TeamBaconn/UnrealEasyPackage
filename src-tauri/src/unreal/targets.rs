//! Build targets from `Source/*.Target.cs`.
//!
//! The target name is the file stem (`SampleProjectSteam.Target.cs` →
//! `SampleProjectSteam`). The `TargetType` is read from `Type = TargetType.X` -
//! but a target may set no `Type` and instead **inherit** it from a base class
//! (e.g. `SampleProjectSteamTarget : SampleProjectTarget`), so resolution follows
//! the C# base class through the other local targets before falling back.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub enum TargetType {
    Game,
    Editor,
    Client,
    Server,
    Program,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    pub name: String,
    pub target_type: TargetType,
}

impl TargetInfo {
    /// Targets that produce a packageable build (Editor/Program are excluded).
    pub fn packageable(&self) -> bool {
        matches!(
            self.target_type,
            TargetType::Game | TargetType::Client | TargetType::Server
        )
    }
}

struct ParsedTarget {
    name: String,
    class_name: Option<String>,
    base_class: Option<String>,
    explicit_type: Option<TargetType>,
}

fn target_type_from_str(s: &str) -> TargetType {
    match s {
        "Game" => TargetType::Game,
        "Editor" => TargetType::Editor,
        "Client" => TargetType::Client,
        "Server" => TargetType::Server,
        "Program" => TargetType::Program,
        _ => TargetType::Unknown,
    }
}

fn parse_target_cs(name: &str, content: &str) -> ParsedTarget {
    // Patterns are static - compile once, reuse across every target file scanned.
    static CLASS_RE: OnceLock<Regex> = OnceLock::new();
    static TYPE_RE: OnceLock<Regex> = OnceLock::new();
    // `public class XTarget : YTarget` - capture class + its base.
    let (class_name, base_class) = CLASS_RE
        .get_or_init(|| Regex::new(r"class\s+(\w+)\s*:\s*(\w+)").unwrap())
        .captures(content)
        .map(|c| (Some(c[1].to_string()), Some(c[2].to_string())))
        .unwrap_or((None, None));
    // `Type = TargetType.Game;`
    let explicit_type = TYPE_RE
        .get_or_init(|| Regex::new(r"Type\s*=\s*TargetType\.(\w+)").unwrap())
        .captures(content)
        .map(|c| target_type_from_str(&c[1]));
    ParsedTarget {
        name: name.to_string(),
        class_name,
        base_class,
        explicit_type,
    }
}

/// Resolve target types across a set of `(name, source)` pairs, following base
/// classes for targets that don't set `Type` directly. Pure - unit-testable.
pub fn resolve_targets(files: &[(String, String)]) -> Vec<TargetInfo> {
    let parsed: Vec<ParsedTarget> = files
        .iter()
        .map(|(n, c)| parse_target_cs(n, c))
        .collect();
    let by_class: HashMap<&str, usize> = parsed
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.class_name.as_deref().map(|c| (c, i)))
        .collect();

    let mut out: Vec<TargetInfo> = parsed
        .iter()
        .map(|p| TargetInfo {
            name: p.name.clone(),
            target_type: resolve_type(p, &parsed, &by_class, 0),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn resolve_type(
    p: &ParsedTarget,
    all: &[ParsedTarget],
    by_class: &HashMap<&str, usize>,
    depth: usize,
) -> TargetType {
    if let Some(t) = p.explicit_type {
        return t;
    }
    if depth <= 8 {
        if let Some(base) = p.base_class.as_deref() {
            if let Some(&idx) = by_class.get(base) {
                return resolve_type(&all[idx], all, by_class, depth + 1);
            }
        }
    }
    // No explicit type and base isn't another local target (e.g. `TargetRules`):
    // a project target that only inherits is, in practice, a Game target.
    TargetType::Game
}

/// Read `Source/*.Target.cs` (targets live at the `Source/` root) and resolve.
pub fn scan_targets(source_dir: &Path) -> Vec<TargetInfo> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(source_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(stem) = file_name.strip_suffix(".Target.cs") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    files.push((stem.to_string(), content));
                }
            }
        }
    }
    resolve_targets(&files)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T_BASE: &str = r#"
public class SampleProjectTarget : TargetRules
{
    public SampleProjectTarget(TargetInfo Target) : base(Target)
    {
        Type = TargetType.Game;
        ExtraModuleNames.Add("SampleProject");
    }
}"#;

    const T_EDITOR: &str = r#"
public class SampleProjectEditorTarget : TargetRules
{
    public SampleProjectEditorTarget(TargetInfo Target) : base(Target)
    {
        Type = TargetType.Editor;
    }
}"#;

    // The wrinkle: sets no Type - inherits Game from SampleProjectTarget.
    const T_STEAM: &str = r#"
public class SampleProjectSteamTarget : SampleProjectTarget
{
    public SampleProjectSteamTarget(TargetInfo Target) : base(Target)
    {
        CustomConfig = "Steam";
        ApplySteamTarget(this);
    }
}"#;

    fn sampleproject_targets() -> Vec<TargetInfo> {
        resolve_targets(&[
            ("SampleProject".into(), T_BASE.into()),
            ("SampleProjectEditor".into(), T_EDITOR.into()),
            ("SampleProjectSteam".into(), T_STEAM.into()),
        ])
    }

    #[test]
    fn resolves_explicit_types() {
        let t = sampleproject_targets();
        let by = |n: &str| t.iter().find(|x| x.name == n).unwrap().target_type;
        assert_eq!(by("SampleProject"), TargetType::Game);
        assert_eq!(by("SampleProjectEditor"), TargetType::Editor);
    }

    #[test]
    fn steam_inherits_game_from_base_class() {
        let t = sampleproject_targets();
        let steam = t.iter().find(|x| x.name == "SampleProjectSteam").unwrap();
        assert_eq!(steam.target_type, TargetType::Game);
    }

    #[test]
    fn packageable_excludes_editor() {
        let targets = sampleproject_targets();
        let pkg: Vec<&str> = targets
            .iter()
            .filter(|t| t.packageable())
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(pkg, vec!["SampleProject", "SampleProjectSteam"]);
    }

    #[test]
    fn unknown_base_with_no_type_defaults_game() {
        let t = resolve_targets(&[("Weird".into(), "public class WeirdTarget : SomeMystery {}".into())]);
        assert_eq!(t[0].target_type, TargetType::Game);
    }
}
