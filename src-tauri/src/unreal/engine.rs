//! Engine resolution + validity.
//!
//! Resolution order (per `docs/requirement.md` R2): a **saved** engine path for
//! the project's `EngineAssociation` (if still valid) → **auto-resolve** from
//! the association (Windows registry / Mac-Linux `Install.ini`) → otherwise the
//! caller prompts the user to **Locate** the folder. A confirmed path is saved
//! by the storage layer, keyed by association.
//!
//! Validity is decided by `<root>/Engine/Build/Build.version` (which also yields
//! the version) - the same file UE itself uses.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl EngineVersion {
    /// `5.5` - the version users recognize (patch dropped).
    pub fn short(&self) -> String {
        format!("{}.{}", self.major, self.minor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    /// Custom/source build (a GUID association, resolved via HKCU Builds / Install.ini).
    Source,
    /// Launcher / binary install (a version-string association).
    Launcher,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineInfo {
    pub root: PathBuf,
    pub version: EngineVersion,
    pub kind: EngineKind,
}

/// The relevant fields of `Engine/Build/Build.version`.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct BuildVersionFile {
    major_version: u32,
    minor_version: u32,
    #[serde(default)]
    patch_version: u32,
}

/// Parse a `Build.version` JSON body (pure).
pub fn parse_build_version(text: &str) -> Option<EngineVersion> {
    let b: BuildVersionFile = serde_json::from_str(text).ok()?;
    Some(EngineVersion {
        major: b.major_version,
        minor: b.minor_version,
        patch: b.patch_version,
    })
}

/// Read the engine version from a candidate root, or `None` if it isn't a valid
/// engine folder. `Some(_)` is the canonical "this is a real engine" signal.
pub fn engine_version(root: &Path) -> Option<EngineVersion> {
    let text = std::fs::read_to_string(root.join("Engine/Build/Build.version")).ok()?;
    parse_build_version(&text)
}

/// Is this folder a valid Unreal Engine root?
pub fn is_valid_engine_root(root: &Path) -> bool {
    engine_version(root).is_some()
}

/// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` (source build) vs a version string.
pub fn is_guid_association(assoc: &str) -> bool {
    let s = assoc.trim();
    s.len() == 38 && s.starts_with('{') && s.ends_with('}')
}

pub fn kind_for_association(assoc: &str) -> EngineKind {
    if is_guid_association(assoc) {
        EngineKind::Source
    } else {
        EngineKind::Launcher
    }
}

/// Resolve an engine for a project: prefer the `saved` path (if still valid),
/// else auto-resolve from the association. `None` ⇒ caller must prompt Locate.
pub fn resolve_engine(association: &str, saved: Option<&Path>) -> Option<EngineInfo> {
    if let Some(saved) = saved {
        if let Some(version) = engine_version(saved) {
            return Some(EngineInfo {
                root: saved.to_path_buf(),
                version,
                kind: kind_for_association(association),
            });
        }
    }
    let root = auto_resolve_engine(association)?;
    let version = engine_version(&root)?;
    Some(EngineInfo {
        root,
        version,
        kind: kind_for_association(association),
    })
}

/// Validate a manually-picked engine folder; `Some(_)` ⇒ valid (caller saves it).
pub fn validate_located_engine(dir: &Path, association: &str) -> Option<EngineInfo> {
    let version = engine_version(dir)?;
    Some(EngineInfo {
        root: dir.to_path_buf(),
        version,
        kind: kind_for_association(association),
    })
}

/// Validate a standalone engine folder (no project association) - used by the plugin
/// Actions tab's engine picker. A browsed/remembered engine has no association to
/// classify it, so it's reported as `Source` (a custom build); launcher installs are
/// surfaced separately by [`enumerate_registry_engines`] with the correct kind. The
/// `kind` here is display-only - `BuildPlugin -rocket` is emitted regardless.
pub fn engine_at(dir: &Path) -> Option<EngineInfo> {
    let version = engine_version(dir)?;
    Some(EngineInfo {
        root: dir.to_path_buf(),
        version,
        kind: EngineKind::Source,
    })
}

/// Enumerate every Unreal Engine registered on this machine, validated (only roots
/// with a readable `Engine/Build/Build.version` are returned). Source/custom builds
/// come from `HKCU\…\Epic Games\Unreal Engine\Builds` (GUID → path); launcher
/// installs from `HKLM\…\EpicGames\Unreal Engine\<ver>\InstalledDirectory`. Powers
/// the plugin packaging engine dropdown (R-plugin). Best-effort: a missing key or a
/// stale entry is simply skipped.
#[cfg(windows)]
pub fn enumerate_registry_engines() -> Vec<EngineInfo> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let mut out: Vec<EngineInfo> = Vec::new();

    // Source builds (GUID associations): value name = GUID, value data = engine root.
    if let Ok(builds) =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Epic Games\Unreal Engine\Builds")
    {
        for name in builds.enum_values().flatten().map(|(n, _)| n) {
            if let Ok(path) = builds.get_value::<String, _>(&name) {
                let root = PathBuf::from(path);
                if let Some(version) = engine_version(&root) {
                    out.push(EngineInfo { root, version, kind: EngineKind::Source });
                }
            }
        }
    }

    // Launcher installs: each version subkey carries an `InstalledDirectory`.
    if let Ok(root_key) =
        RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\EpicGames\Unreal Engine")
    {
        for ver in root_key.enum_keys().flatten() {
            if let Ok(sub) = root_key.open_subkey(&ver) {
                if let Ok(dir) = sub.get_value::<String, _>("InstalledDirectory") {
                    let root = PathBuf::from(dir);
                    if let Some(version) = engine_version(&root) {
                        out.push(EngineInfo { root, version, kind: EngineKind::Launcher });
                    }
                }
            }
        }
    }

    dedup_by_root(out)
}

/// Mac/Linux: `Install.ini` `[Installations]` lists GUID → path source builds. The
/// MVP enumerates those; launcher installs (LauncherInstalled.dat) are out of scope.
#[cfg(not(windows))]
pub fn enumerate_registry_engines() -> Vec<EngineInfo> {
    let mut out: Vec<EngineInfo> = Vec::new();
    if let Some(cfg) = dirs::config_dir() {
        if let Ok(ini) = ini::Ini::load_from_file(cfg.join("Epic/UnrealEngine/Install.ini")) {
            if let Some(section) = ini.section(Some("Installations")) {
                for (_assoc, path) in section.iter() {
                    let root = PathBuf::from(path);
                    if let Some(version) = engine_version(&root) {
                        out.push(EngineInfo { root, version, kind: EngineKind::Source });
                    }
                }
            }
        }
    }
    dedup_by_root(out)
}

/// Drop duplicate engine roots (case-insensitive path compare), keeping first-seen -
/// so a path that appears as both a launcher and a source entry isn't listed twice.
fn dedup_by_root(engines: Vec<EngineInfo>) -> Vec<EngineInfo> {
    let mut seen = std::collections::HashSet::new();
    engines
        .into_iter()
        .filter(|e| seen.insert(e.root.display().to_string().to_lowercase()))
        .collect()
}

/// Windows: source GUIDs live in `HKCU\…\Epic Games\Unreal Engine\Builds`;
/// launcher versions in `HKLM\…\EpicGames\Unreal Engine\<ver>\InstalledDirectory`.
#[cfg(windows)]
fn auto_resolve_engine(association: &str) -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    if is_guid_association(association) {
        let builds = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Epic Games\Unreal Engine\Builds")
            .ok()?;
        let path: String = builds.get_value(association).ok()?;
        Some(PathBuf::from(path))
    } else {
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(format!(r"SOFTWARE\EpicGames\Unreal Engine\{association}"))
            .ok()?;
        let path: String = key.get_value("InstalledDirectory").ok()?;
        Some(PathBuf::from(path))
    }
}

/// Mac/Linux: associations live in `Install.ini` `[Installations]` under the
/// platform application-settings dir (`~/Library/Application Support/Epic` on
/// macOS, `~/.config/Epic` on Linux - both = `dirs::config_dir()`).
#[cfg(not(windows))]
fn auto_resolve_engine(association: &str) -> Option<PathBuf> {
    let ini_path = dirs::config_dir()?.join("Epic/UnrealEngine/Install.ini");
    let ini = ini::Ini::load_from_file(ini_path).ok()?;
    let path = ini.section(Some("Installations"))?.get(association)?;
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn write_engine(root: &Path, body: &str) {
        fs::create_dir_all(root.join("Engine/Build")).unwrap();
        fs::write(root.join("Engine/Build/Build.version"), body).unwrap();
    }

    #[test]
    fn parses_build_version() {
        let v = parse_build_version(r#"{"MajorVersion":5,"MinorVersion":5,"PatchVersion":4}"#).unwrap();
        assert_eq!((v.major, v.minor, v.patch), (5, 5, 4));
        assert_eq!(v.short(), "5.5");
    }

    #[test]
    fn guid_vs_version_association() {
        assert!(is_guid_association("{11111111-2222-3333-4444-555555555555}"));
        assert!(!is_guid_association("5.5"));
        assert_eq!(
            kind_for_association("{11111111-2222-3333-4444-555555555555}"),
            EngineKind::Source
        );
        assert_eq!(kind_for_association("5.5"), EngineKind::Launcher);
    }

    #[test]
    fn engine_version_reads_build_version() {
        let dir = tempdir().unwrap();
        write_engine(dir.path(), r#"{"MajorVersion":5,"MinorVersion":3,"PatchVersion":2}"#);
        assert_eq!(engine_version(dir.path()).unwrap().short(), "5.3");
        assert!(is_valid_engine_root(dir.path()));
    }

    #[test]
    fn empty_dir_is_not_an_engine() {
        let dir = tempdir().unwrap();
        assert!(!is_valid_engine_root(dir.path()));
    }

    #[test]
    fn saved_path_preferred_when_valid() {
        let dir = tempdir().unwrap();
        write_engine(dir.path(), r#"{"MajorVersion":5,"MinorVersion":5}"#);
        let info = resolve_engine("{11111111-2222-3333-4444-555555555555}", Some(dir.path())).unwrap();
        assert_eq!(info.root, dir.path());
        assert_eq!(info.kind, EngineKind::Source);
        assert_eq!(info.version.short(), "5.5");
    }

    #[test]
    fn invalid_saved_path_falls_through() {
        // saved points at a non-engine dir, association is a GUID with no registry
        // entry on this machine → expect None (caller would prompt Locate).
        let dir = tempdir().unwrap();
        let bogus = "{00000000-0000-0000-0000-000000000000}";
        assert!(resolve_engine(bogus, Some(dir.path())).is_none());
    }

    // Machine-specific engine resolution (registry lookup + real engine path)
    // is NOT an automated test - it can't pass without this machine. Verify by
    // hand: open the reference project -> correct engine root / version / kind,
    // then report the result.
}
