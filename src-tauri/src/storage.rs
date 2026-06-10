//! Persistence: app-folder recents + per-project machine-local settings.
//!
//! Plain JSON via `std::fs`. Functions take their target directory as input so
//! they stay pure-ish and unit-testable. Per `docs/data-storage.md`: recents hold
//! only project identity (engine is re-validated on display, never stored here).
//! The engine-path override is **per-project** (`<project>/.uep/local.json`, git-
//! ignored) - so two projects sharing an `EngineAssociation` can point at
//! different engine builds, and nothing machine-specific is committed.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What a recents entry points at - a project `.uproject` or a plugin `.uplugin`.
/// The gate shows this as a PROJECT / PLUGIN tag and routes the open accordingly.
/// `#[serde(default)]` ⇒ pre-feature recents (which had no `kind`) load as projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum RecentKind {
    #[default]
    Project,
    Plugin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentRecord {
    pub name: String,
    /// The descriptor path - a `.uproject` or `.uplugin`. The stored key is `path`;
    /// `uprojectPath` is read as an alias so recents written before plugin support
    /// (which only held projects) still load.
    #[serde(alias = "uprojectPath")]
    pub path: String,
    /// Project vs plugin (defaults to project for pre-feature records).
    #[serde(default)]
    pub kind: RecentKind,
    /// Epoch milliseconds (f64 - specta forbids u64 across the IPC boundary; ms
    /// is exact in f64 for ~285k years).
    pub last_opened_ms: f64,
    /// User pinned this entry to the top of the list. `#[serde(default)]` so
    /// recents written before this field load as unpinned.
    #[serde(default)]
    pub starred: bool,
}

fn recents_file(dir: &Path) -> PathBuf {
    dir.join("recent-projects.json")
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}

// ── recents ──────────────────────────────────────────────────────────────────
pub fn load_recents(dir: &Path) -> Vec<RecentRecord> {
    read_json(&recents_file(dir))
}

pub fn save_recents(dir: &Path, recents: &[RecentRecord]) -> std::io::Result<()> {
    write_json(&recents_file(dir), &recents)
}

/// Insert/refresh a recent at the front (dedup by descriptor path). Preserves an
/// existing pin - `open_project`/`open_plugin` rebuild the record with `starred`
/// defaulted off, so re-opening a pinned entry must not silently unpin it.
pub fn upsert_recent(dir: &Path, mut record: RecentRecord) -> std::io::Result<()> {
    let mut recents = load_recents(dir);
    record.starred |= recents.iter().any(|r| r.path == record.path && r.starred);
    recents.retain(|r| r.path != record.path);
    recents.insert(0, record);
    save_recents(dir, &recents)
}

pub fn remove_recent(dir: &Path, path: &str) -> std::io::Result<()> {
    let mut recents = load_recents(dir);
    recents.retain(|r| r.path != path);
    save_recents(dir, &recents)
}

/// Set (or clear) a recent's pin flag. No-op if the path isn't in the list.
pub fn set_recent_starred(dir: &Path, path: &str, starred: bool) -> std::io::Result<()> {
    let mut recents = load_recents(dir);
    for r in recents.iter_mut().filter(|r| r.path == path) {
        r.starred = starred;
    }
    save_recents(dir, &recents)
}

// ── per-project machine-local settings (`.uep/local.json`, git-ignored) ──────
// Overrides that are specific to THIS project on THIS machine and must never be
// committed - currently just the engine-path override. Keeping it per-project
// (not an app-folder map keyed by association) is what lets two projects with the
// same `EngineAssociation` resolve to different engine builds.

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalSettings {
    /// User-confirmed engine root for this project; overrides association auto-resolve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    engine_path: Option<String>,
}

fn local_settings_file(project_root: &Path) -> PathBuf {
    project_root.join(".uep").join("local.json")
}

/// This project's engine-path override, if the user set one (empty ⇒ `None`).
pub fn get_project_engine_path(project_root: &Path) -> Option<PathBuf> {
    let s: LocalSettings = read_json(&local_settings_file(project_root));
    s.engine_path
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
}

/// Save this project's engine-path override into `.uep/local.json`. Ensures the
/// `.uep/.gitignore` covers `local.json` first, so the override never gets committed.
pub fn set_project_engine_path(project_root: &Path, engine_root: &str) -> std::io::Result<()> {
    let _ = ensure_uep_dir(project_root);
    let path = local_settings_file(project_root);
    let mut s: LocalSettings = read_json(&path);
    s.engine_path = Some(engine_root.to_string());
    write_json(&path, &s)
}

// ── per-plugin machine-local settings (`<plugin_root>/.uap/settings.json`) ────────
// Everything the Actions tab remembers for a plugin lives in ONE plain JSON beside
// the `.uplugin`, in its own `.uap/` folder (kept separate from the engine's `.uep/`
// convention, and never in the host project root). Git-ignored so a distributed
// plugin never carries it.

/// The plugin's machine-local Actions settings (one JSON file).
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettings {
    /// Engine roots the user browsed for (remembered; stale ones pruned when listed).
    #[serde(default)]
    pub engines: Vec<String>,
    /// Last-used package output base dir.
    #[serde(default)]
    pub output_dir: String,
    /// Last-used package folder-name template.
    #[serde(default)]
    pub folder_name: String,
}

fn plugin_settings_file(plugin_root: &Path) -> PathBuf {
    plugin_root.join(".uap").join("settings.json")
}

// `*` ignores everything in `.uap/` - including this `.gitignore` itself - so the whole
// folder is invisible to the plugin's git repo (git still honors an ignored `.gitignore`).
const UAP_GITIGNORE: &str = "# UnrealEasyPackage machine-local plugin settings - not committed.\n*\n";

/// Load the plugin's `.uap/settings.json` (defaults if absent).
pub fn load_plugin_settings(plugin_root: &Path) -> PluginSettings {
    read_json(&plugin_settings_file(plugin_root))
}

/// Write the plugin's `.uap/settings.json`, scaffolding `.uap/.gitignore` so the file
/// is never committed with the plugin.
pub fn save_plugin_settings(plugin_root: &Path, settings: &PluginSettings) -> std::io::Result<()> {
    let uap = plugin_root.join(".uap");
    std::fs::create_dir_all(&uap)?;
    let gitignore = uap.join(".gitignore");
    if std::fs::metadata(&gitignore).is_err() {
        let _ = std::fs::write(&gitignore, UAP_GITIGNORE);
    }
    write_json(&plugin_settings_file(plugin_root), settings)
}

// ── per-project `.uep/` scaffolding ──────────────────────────────────────────

/// Contents of the managed `.uep/.gitignore` (`docs/data-storage.md` §"Per-project").
/// Only `profiles/` is committed and shared with the team; everything else here is
/// either derived/regenerable (`history/`, `cache/`) or machine-local (`local.json`).
/// The `.gitignore` itself ignores itself, so it's never committed either - the app
/// regenerates it locally on open (git still honors an ignored `.gitignore`).
const UEP_GITIGNORE: &str = "\
# Managed by UnrealEasyPackage - regenerated locally on open, don't commit it.
# Only profiles/ is shared; everything else here is derived or machine-local.
.gitignore
history/
cache/
local.json
";

/// Lines `ensure_uep_dir` guarantees exist even in a pre-existing `.gitignore`
/// (rules added after the original scaffold). Keeps user edits intact.
const UEP_GITIGNORE_REQUIRED: &[&str] = &[".gitignore", "history/", "cache/", "local.json"];

/// Ensure `<project>/.uep/` exists with the managed `.gitignore`. Idempotent and
/// non-destructive: a fresh dir gets the full file; an existing `.gitignore` keeps
/// all user edits, and we only *append* any required rule it's missing (so an old
/// scaffold still ends up ignoring `local.json`). Best-effort - callers ignore the result.
pub fn ensure_uep_dir(project_root: &Path) -> std::io::Result<()> {
    let uep = project_root.join(".uep");
    std::fs::create_dir_all(&uep)?;
    let gitignore = uep.join(".gitignore");
    match std::fs::read_to_string(&gitignore) {
        Err(_) => std::fs::write(&gitignore, UEP_GITIGNORE)?,
        Ok(existing) => {
            let missing: Vec<&str> = UEP_GITIGNORE_REQUIRED
                .iter()
                .copied()
                .filter(|rule| !existing.lines().any(|l| l.trim() == *rule))
                .collect();
            if !missing.is_empty() {
                let sep = if existing.ends_with('\n') { "" } else { "\n" };
                std::fs::write(&gitignore, format!("{existing}{sep}{}\n", missing.join("\n")))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn rec(name: &str, path: &str, ms: f64) -> RecentRecord {
        RecentRecord {
            name: name.into(),
            path: path.into(),
            kind: RecentKind::Project,
            last_opened_ms: ms,
            starred: false,
        }
    }

    #[test]
    fn recents_upsert_dedup_and_order() {
        let d = tempdir().unwrap();
        let p = d.path();
        upsert_recent(p, rec("A", "X", 1.0)).unwrap();
        upsert_recent(p, rec("B", "Y", 2.0)).unwrap();
        upsert_recent(p, rec("A2", "X", 3.0)).unwrap(); // dedup X, move to front

        let r = load_recents(p);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].path, "X");
        assert_eq!(r[0].name, "A2");
        assert_eq!(r[1].path, "Y");

        remove_recent(p, "X").unwrap();
        let r = load_recents(p);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "Y");
    }

    #[test]
    fn legacy_recent_with_uproject_path_alias_still_loads() {
        let d = tempdir().unwrap();
        let p = d.path();
        // A pre-plugin record: `uprojectPath` key, no `kind`.
        std::fs::write(
            recents_file(p),
            r#"[{"name":"Old","uprojectPath":"C:/Old.uproject","lastOpenedMs":1.0}]"#,
        )
        .unwrap();
        let r = load_recents(p);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "C:/Old.uproject");
        assert_eq!(r[0].kind, RecentKind::Project);
    }

    #[test]
    fn plugin_settings_roundtrip_in_uap() {
        let d = tempdir().unwrap();
        let plugin = d.path().join("PluginA");
        std::fs::create_dir_all(&plugin).unwrap();
        assert!(load_plugin_settings(&plugin).engines.is_empty());

        let s = PluginSettings {
            engines: vec!["C:/Engine/CustomEngine".to_string()],
            output_dir: "C:/FAB".to_string(),
            folder_name: "{plugin}-{version}".to_string(),
        };
        save_plugin_settings(&plugin, &s).unwrap();

        let back = load_plugin_settings(&plugin);
        assert_eq!(back.engines, vec!["C:/Engine/CustomEngine".to_string()]);
        assert_eq!(back.output_dir, "C:/FAB");
        assert_eq!(back.folder_name, "{plugin}-{version}");

        // It lives in its own `.uap/` beside the .uplugin and is git-ignored.
        assert!(plugin.join(".uap").join("settings.json").exists());
        let gi = std::fs::read_to_string(plugin.join(".uap").join(".gitignore")).unwrap();
        assert!(gi.lines().any(|l| l.trim() == "*"));
    }

    #[test]
    fn star_persists_and_survives_reopen() {
        let d = tempdir().unwrap();
        let p = d.path();
        upsert_recent(p, rec("A", "X", 1.0)).unwrap();
        set_recent_starred(p, "X", true).unwrap();
        assert!(load_recents(p)[0].starred);

        // Re-opening the project (upsert with a defaulted record) keeps the pin.
        upsert_recent(p, rec("A2", "X", 9.0)).unwrap();
        assert!(load_recents(p)[0].starred);

        set_recent_starred(p, "X", false).unwrap();
        assert!(!load_recents(p)[0].starred);
    }

    #[test]
    fn project_engine_path_roundtrip_and_is_per_project() {
        let d = tempdir().unwrap();
        let a = d.path().join("ProjA");
        let b = d.path().join("ProjB");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();

        assert!(get_project_engine_path(&a).is_none());
        set_project_engine_path(&a, "C:/Engine/CustomEngine").unwrap();
        assert_eq!(
            get_project_engine_path(&a).unwrap(),
            PathBuf::from("C:/Engine/CustomEngine")
        );
        // Overriding ProjA must NOT bleed into ProjB (the whole point - they may
        // share an EngineAssociation but resolve to different engine builds).
        assert!(get_project_engine_path(&b).is_none());

        // The override file is git-ignored by the scaffolded `.uep/.gitignore`.
        let gi = std::fs::read_to_string(a.join(".uep").join(".gitignore")).unwrap();
        assert!(gi.lines().any(|l| l.trim() == "local.json"));
    }

    #[test]
    fn missing_files_default_empty() {
        let d = tempdir().unwrap();
        assert!(load_recents(d.path()).is_empty());
        assert!(get_project_engine_path(d.path()).is_none());
    }

    #[test]
    fn ensure_uep_dir_writes_gitignore_then_is_non_destructive() {
        let d = tempdir().unwrap();
        let root = d.path();

        ensure_uep_dir(root).unwrap();
        let gi = root.join(".uep").join(".gitignore");
        let body = std::fs::read_to_string(&gi).unwrap();
        assert!(body.contains("history/") && body.contains("cache/"));
        assert!(body.lines().any(|l| l.trim() == "local.json"));
        // The .gitignore ignores itself, so it's never committed (only profiles/ is).
        assert!(body.lines().any(|l| l.trim() == ".gitignore"));

        // A user-edited .gitignore keeps its edits; only missing required rules are
        // appended (here `local.json`, which post-dates the original scaffold).
        std::fs::write(&gi, "history/\ncache/\nextra/\n").unwrap();
        ensure_uep_dir(root).unwrap();
        let body = std::fs::read_to_string(&gi).unwrap();
        assert!(body.contains("extra/"));
        assert!(body.lines().any(|l| l.trim() == "local.json"));
    }
}
