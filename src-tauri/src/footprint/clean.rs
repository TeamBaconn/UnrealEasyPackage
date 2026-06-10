//! Footprint **cleanup** (M5, R3): delete resolved targets and report bytes reclaimed.
//! Targets come from the **same** [`scan::collect`] the scanner and confirm-prompt use, so
//! what's deleted is exactly what was shown. Two entry points share the resolver and the
//! guardrail:
//! - [`clean_by_ids`] - the Clean tab's node selection (leaf ids).
//! - [`clean_categories`] - the Clean-up build phase's category selection.
//!
//! Defense-in-depth: every target is re-checked against [`rules::is_cleanup_path`] and
//! canonicalized inside the project root before removal.

use std::path::Path;

use serde::Serialize;

use super::rules::{self, TargetScope};
use super::scan::{self, Target, TargetKind};
use crate::profiles::schema::CleanupCategory;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub rel: String,
    pub path: String,
    pub size_bytes: f64,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanOutcome {
    pub removed: Vec<Removed>,
    pub reclaimed_bytes: f64,
}

/// Delete the Clean tab's selected nodes. `node_ids` are leaf ids from the scan (e.g.
/// `intermediateGame:<target>`, `intermediateOther`, `binariesPlugin`). `intermediateOther`
/// wipes the whole main `Intermediate/`, so it subsumes the per-target game leaves - those
/// are skipped to avoid double-removing/double-counting.
pub fn clean_by_ids(project_root: &Path, node_ids: &[String], scope: &TargetScope) -> CleanOutcome {
    let leaves = scan::collect(project_root, scope).into_leaves();
    let wipe_intermediate = node_ids.iter().any(|id| id == "intermediateOther");
    let mut acc = Acc::default();
    for leaf in leaves {
        if wipe_intermediate && leaf.category == CleanupCategory::IntermediateGame {
            continue;
        }
        if node_ids.iter().any(|id| *id == leaf.id) {
            acc.remove_all_of(project_root, leaf.targets);
        }
    }
    acc.into_outcome()
}

/// Delete every target whose category is in `categories` - the Clean-up build phase's
/// view. `IntermediateOther` (the editor cache) is simply never offered by the phase.
pub fn clean_categories(project_root: &Path, categories: &[CleanupCategory], scope: &TargetScope) -> CleanOutcome {
    let leaves = scan::collect(project_root, scope).into_leaves();
    let mut acc = Acc::default();
    for leaf in leaves {
        if categories.contains(&leaf.category) {
            acc.remove_all_of(project_root, leaf.targets);
        }
    }
    acc.into_outcome()
}

#[derive(Default)]
struct Acc {
    removed: Vec<Removed>,
    reclaimed: f64,
}

impl Acc {
    fn remove_all_of(&mut self, project_root: &Path, targets: Vec<Target>) {
        for t in targets {
            let deleted = guarded_remove(project_root, &t);
            if deleted {
                self.reclaimed += t.size_bytes as f64;
            }
            self.removed.push(Removed { rel: t.rel, path: t.abs.to_string_lossy().to_string(), size_bytes: t.size_bytes as f64, deleted });
        }
    }
    fn into_outcome(self) -> CleanOutcome {
        CleanOutcome { removed: self.removed, reclaimed_bytes: self.reclaimed }
    }
}

/// Remove one resolved target under the guardrail: its path must be a cleanup path (not
/// protected) and canonicalize inside the project root (no `..`/junction escape). A `Dir`
/// is removed whole; a `Files` target removes only its enumerated files (never the dir -
/// it also holds third-party deps).
fn guarded_remove(project_root: &Path, t: &Target) -> bool {
    if !rules::is_cleanup_path(&t.guard_rel) {
        return false;
    }
    let Ok(root_c) = project_root.canonicalize() else {
        return false;
    };
    match &t.kind {
        TargetKind::Dir => {
            let Ok(abs_c) = t.abs.canonicalize() else {
                return false;
            };
            if abs_c == root_c || !abs_c.starts_with(&root_c) {
                return false;
            }
            std::fs::remove_dir_all(&abs_c).is_ok()
        }
        TargetKind::Files(list) => {
            if list.is_empty() {
                return false;
            }
            let mut all_ok = true;
            for f in list {
                match f.canonicalize() {
                    Ok(fc) if fc.starts_with(&root_c) => {
                        if std::fs::remove_file(&fc).is_err() {
                            all_ok = false;
                        }
                    }
                    _ => all_ok = false,
                }
            }
            all_ok
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use CleanupCategory::*;

    fn put(root: &Path, rel: &str, bytes: usize) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; bytes]).unwrap();
    }
    fn exists(root: &Path, rel: &str) -> bool {
        root.join(rel).exists()
    }
    fn scope() -> TargetScope {
        TargetScope::new(vec!["SampleProjectSteam".into()], Some("SampleProjectEditor".into()))
    }
    fn seed(root: &Path) {
        put(root, "Intermediate/Build/Win64/SampleProjectSteam/Development/a.obj", 100);
        put(root, "Intermediate/Build/Win64/SampleProjectSteam/Shipping/b.obj", 60);
        put(root, "Intermediate/Build/Win64/SampleProjectEditor/Development/e.obj", 200);
        put(root, "Intermediate/Build/Win64/x64/junk.bin", 999);
        put(root, "Binaries/Win64/SampleProjectSteam.exe", 300);
        put(root, "Binaries/Win64/SampleProjectSteam.pdb", 30);
        put(root, "Binaries/Win64/UnrealEditor-SampleProject.dll", 400);
        put(root, "Binaries/Win64/boost_thread-mt-x64.dll", 999);
        put(root, "Plugins/Cool/Cool.uplugin", 1);
        put(root, "Plugins/Cool/Binaries/Win64/UnrealEditor-Mod.dll", 10);
        put(root, "Plugins/Cool/Intermediate/Build/Win64/SampleProjectEditor/p.obj", 20);
        put(root, "Saved/StagedBuilds/Windows/g.exe", 400);
        put(root, "Content/Map.umap", 9999);
        put(root, "Saved/Autosaves/r.umap", 9999);
    }

    #[test]
    fn clean_categories_intermediate_game_removes_whole_target_spares_editor_and_scratch() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        let out = clean_categories(root, &[IntermediateGame], &scope());

        // The whole game-target dir (both build modes) goes.
        assert!(!exists(root, "Intermediate/Build/Win64/SampleProjectSteam"));
        assert!(exists(root, "Intermediate/Build/Win64/SampleProjectEditor")); // editor preserved
        assert!(exists(root, "Intermediate/Build/Win64/x64")); // unclassified preserved
        assert!(exists(root, "Plugins/Cool/Intermediate")); // plugin is a separate category
        assert_eq!(out.reclaimed_bytes, (100 + 60) as f64);
    }

    #[test]
    fn clean_categories_game_binaries_removes_only_game_files() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        let out = clean_categories(root, &[BinariesGame], &scope());

        assert!(!exists(root, "Binaries/Win64/SampleProjectSteam.exe"));
        assert!(!exists(root, "Binaries/Win64/SampleProjectSteam.pdb"));
        assert!(exists(root, "Binaries/Win64/UnrealEditor-SampleProject.dll")); // editor file kept
        assert!(exists(root, "Binaries/Win64/boost_thread-mt-x64.dll")); // third-party kept
        assert!(exists(root, "Binaries/Win64")); // dir stays
        assert_eq!(out.reclaimed_bytes, (300 + 30) as f64);
    }

    #[test]
    fn clean_by_ids_other_wipes_whole_main_intermediate_not_plugins() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        let out = clean_by_ids(root, &["intermediateOther".to_string()], &scope());

        // Other = wipe the whole main Intermediate (game + editor + scratch).
        assert!(!exists(root, "Intermediate"));
        assert!(exists(root, "Plugins/Cool/Intermediate")); // plugins are a separate sibling
        assert!(exists(root, "Binaries/Win64/SampleProjectSteam.exe")); // binaries untouched
        assert_eq!(out.reclaimed_bytes, (100 + 60 + 200 + 999) as f64); // whole main Intermediate
    }

    #[test]
    fn clean_by_ids_other_plus_game_does_not_double_count() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        // The UI auto-ticks game targets when Other is on; the game leaf must be skipped so
        // the whole-dir wipe isn't counted twice.
        let out = clean_by_ids(root, &["intermediateGame:SampleProjectSteam".to_string(), "intermediateOther".to_string()], &scope());

        assert!(!exists(root, "Intermediate"));
        assert_eq!(out.reclaimed_bytes, (100 + 60 + 200 + 999) as f64); // once, not + the 160 game again
    }

    #[test]
    fn clean_by_ids_one_target_removes_all_its_build_modes() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        let out = clean_by_ids(root, &["intermediateGame:SampleProjectSteam".to_string()], &scope());

        assert!(!exists(root, "Intermediate/Build/Win64/SampleProjectSteam")); // both modes gone
        assert!(exists(root, "Intermediate/Build/Win64/SampleProjectEditor")); // editor untouched
        assert_eq!(out.reclaimed_bytes, (100 + 60) as f64);
    }

    #[test]
    fn clean_by_ids_plugin_leaf_removes_all_plugin_dirs() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        let out = clean_by_ids(root, &["intermediatePlugin".to_string()], &scope());

        assert!(!exists(root, "Plugins/Cool/Intermediate"));
        assert!(exists(root, "Intermediate/Build/Win64/SampleProjectSteam")); // project intermediate untouched
        assert_eq!(out.reclaimed_bytes, 20.0);
    }

    #[test]
    fn clean_removes_nested_plugins() {
        // Plugins can nest under a group dir (e.g. `Plugins/COG/Cog/...`); the guardrail
        // must allow those compile dirs, not just `Plugins/<name>/...`.
        let d = tempdir().unwrap();
        let root = d.path();
        put(root, "Plugins/Group/Cog/Cog.uplugin", 1);
        put(root, "Plugins/Group/Cog/Binaries/Win64/UnrealEditor-Cog.dll", 500);
        put(root, "Plugins/Group/Cog/Intermediate/Build/x.obj", 300);
        put(root, "Plugins/Group/Cog/Source/Cog.cpp", 9999); // source must survive

        let bins = clean_by_ids(root, &["binariesPlugin".to_string()], &scope());
        let ints = clean_by_ids(root, &["intermediatePlugin".to_string()], &scope());

        assert!(!exists(root, "Plugins/Group/Cog/Binaries"));
        assert!(!exists(root, "Plugins/Group/Cog/Intermediate"));
        assert!(exists(root, "Plugins/Group/Cog/Source")); // protected
        assert_eq!(bins.reclaimed_bytes, 500.0);
        assert_eq!(ints.reclaimed_bytes, 300.0);
    }

    #[test]
    fn full_clean_never_touches_source_or_recovery() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        clean_categories(root, &rules::ALL_CATEGORIES, &scope());
        assert!(exists(root, "Content"));
        assert!(exists(root, "Saved/Autosaves"));
    }

    #[test]
    fn empty_selection_is_a_noop() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        assert_eq!(clean_categories(root, &[], &scope()).reclaimed_bytes, 0.0);
        assert!(clean_by_ids(root, &[], &scope()).removed.is_empty());
        assert!(exists(root, "Intermediate/Build/Win64/SampleProjectSteam/Development"));
    }
}
