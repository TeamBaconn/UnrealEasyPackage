//! Map discovery: scan `Content/**/*.umap` into `/Game/...` package paths, plus
//! the default maps declared in `Config/DefaultEngine.ini` (informational).

use serde::{Deserialize, Serialize};
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MapInventory {
    /// `/Game/...` package paths for every `.umap` under `Content/`, sorted.
    pub maps: Vec<String>,
    pub game_default: Option<String>,
    pub editor_startup: Option<String>,
}

pub fn scan_maps(project_root: &Path) -> MapInventory {
    let content = project_root.join("Content");
    let mut maps: Vec<String> = WalkDir::new(&content)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("umap"))
        .filter_map(|e| to_game_path(&content, e.path()))
        .collect();
    maps.sort();
    maps.dedup();

    let (game_default, editor_startup) = read_default_maps(project_root);
    MapInventory {
        maps,
        game_default,
        editor_startup,
    }
}

/// `Content/CartoonStreet/Levels/Level_Day.umap` → `/Game/CartoonStreet/Levels/Level_Day`.
fn to_game_path(content_root: &Path, umap: &Path) -> Option<String> {
    let rel = umap.strip_prefix(content_root).ok()?.with_extension("");
    let mut out = String::from("/Game");
    for comp in rel.components() {
        out.push('/');
        out.push_str(&comp.as_os_str().to_string_lossy());
    }
    Some(out)
}

fn read_default_maps(project_root: &Path) -> (Option<String>, Option<String>) {
    let Ok(ini) = ini::Ini::load_from_file(project_root.join("Config/DefaultEngine.ini")) else {
        return (None, None);
    };
    let section = ini.section(Some("/Script/EngineSettings.GameMapsSettings"));
    let get = |key: &str| {
        section
            .and_then(|s| s.get(key))
            .map(clean_map_ref)
            .filter(|s| !s.is_empty())
    };
    (get("GameDefaultMap"), get("EditorStartupMap"))
}

/// `/Game/SampleProject/Level/L_Home.L_Home` → `/Game/SampleProject/Level/L_Home`
/// (drops the `.ObjectName` suffix from a package reference).
fn clean_map_ref(raw: &str) -> String {
    let raw = raw.trim();
    match raw.split_once('.') {
        Some((path, _obj)) => path.to_string(),
        None => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn converts_umap_to_game_path() {
        let content = Path::new("C:/proj/Content");
        let umap = Path::new("C:/proj/Content/CartoonStreet/Levels/Level_Day.umap");
        assert_eq!(
            to_game_path(content, umap).unwrap(),
            "/Game/CartoonStreet/Levels/Level_Day"
        );
    }

    #[test]
    fn cleans_package_object_ref() {
        assert_eq!(
            clean_map_ref("/Game/SampleProject/Level/L_Home.L_Home "),
            "/Game/SampleProject/Level/L_Home"
        );
        assert_eq!(clean_map_ref("/Game/X/L_Gym"), "/Game/X/L_Gym");
    }

    #[test]
    fn scans_umaps_under_content() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("Content/A")).unwrap();
        fs::write(dir.path().join("Content/A/B.umap"), "x").unwrap();
        fs::write(dir.path().join("Content/C.umap"), "x").unwrap();
        fs::write(dir.path().join("Content/notes.txt"), "x").unwrap();
        let inv = scan_maps(dir.path());
        assert_eq!(inv.maps, vec!["/Game/A/B".to_string(), "/Game/C".to_string()]);
    }

    // Scanning a real project's Content/ for maps is NOT an automated test -
    // it can't pass without this machine. Verify by hand: open the reference
    // project -> expected maps + game default, then report the result.
}
