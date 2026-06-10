//! App settings (M6): theme + build-finish notification preferences, persisted as
//! one app-folder JSON (`settings.json`). Plain `std::fs`, dir passed in so the
//! load/save stays pure-ish + unit-testable - same shape as `storage.rs`.
//!
//! Missing/corrupt file ⇒ `AppSettings::default()` (self-healed, never an error):
//! settings are non-critical preferences, so a bad file should degrade to defaults
//! rather than block the app. Read by `commands` (Settings window) and by the
//! `runner` (to decide whether to toast on finish).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Two explicit color schemes (no system/auto for MVP). Lowercased over the IPC
/// boundary so it maps straight onto Mantine's `'light' | 'dark'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Light
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: Theme,
    /// Show an OS toast when a build finishes (success/failed/cancelled).
    pub notify_on_finish: bool,
    /// Attach a sound to that toast (only meaningful when `notify_on_finish`).
    pub notify_sound: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        // Notify by default - builds are long and the user tabs away; both are one
        // toggle-flip to silence in the Settings window.
        AppSettings {
            theme: Theme::default(),
            notify_on_finish: true,
            notify_sound: true,
        }
    }
}

fn settings_file(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

/// Load settings, falling back to defaults if the file is missing or unreadable.
pub fn load(dir: &Path) -> AppSettings {
    std::fs::read_to_string(settings_file(dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, settings: &AppSettings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(settings_file(dir), json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let dir = std::env::temp_dir().join("uep-settings-missing-test");
        let _ = std::fs::remove_dir_all(&dir);
        let s = load(&dir);
        assert_eq!(s, AppSettings::default());
        assert_eq!(s.theme, Theme::Light);
        assert!(s.notify_on_finish && s.notify_sound);
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join("uep-settings-roundtrip-test");
        let _ = std::fs::remove_dir_all(&dir);
        let want = AppSettings {
            theme: Theme::Dark,
            notify_on_finish: false,
            notify_sound: false,
        };
        save(&dir, &want).unwrap();
        assert_eq!(load(&dir), want);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join("uep-settings-corrupt-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(settings_file(&dir), "{ not json").unwrap();
        assert_eq!(load(&dir), AppSettings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
