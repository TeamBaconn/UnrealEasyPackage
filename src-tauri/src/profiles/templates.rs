//! The fixed built-in **templates** seeded into the app folder
//! (`templates/<id>.json`). These are project-agnostic, **undeletable +
//! read-only** clone bases (e.g. *Development*, *Shipping*), **self-healed** -
//! restored on every launch so they can never go missing (`docs/data-storage.md`).
//!
//! Users never create a template (or profile) from blank - only by **cloning**
//! an existing one. Project-specific fields (`target`, `cook.maps`,
//! `copyExtras.items`) and `base_dir` stay empty in a template; the user fills
//! them when cloning a template into a profile.

use std::path::Path;

use super::schema::{BuildConfig, Configuration, Phases, StageCfg};
use super::store;

/// The fixed built-in templates. *Development* keeps debug symbols (you debug dev
/// builds); *Shipping* drops them (`-nodebuginfo`, a footprint win). All are
/// project-agnostic (empty target + output base dir) and `builtin: true`.
pub fn builtins() -> Vec<BuildConfig> {
    // Development is the schema default (configs defaults to [Development]) plus the
    // one real deviation: keep debug symbols, since you debug dev builds.
    let development = BuildConfig {
        id: "builtin-development".into(),
        name: "Development".into(),
        builtin: true,
        phases: Phases {
            stage: StageCfg { debug_symbols: true, ..Default::default() },
            ..Default::default()
        },
        ..Default::default()
    };

    // Shipping differs from the default only in `configs`; dropping debug symbols
    // (`-nodebuginfo`) is already the default, so nothing else to set.
    let shipping = BuildConfig {
        id: "builtin-shipping".into(),
        name: "Shipping".into(),
        builtin: true,
        configs: vec![Configuration::Shipping],
        ..Default::default()
    };

    vec![development, shipping]
}

/// Whether `id` is a fixed built-in (cannot be deleted or overwritten). Matches the
/// ids minted in `builtins()` without constructing the configs.
pub fn is_builtin(id: &str) -> bool {
    matches!(id, "builtin-development" | "builtin-shipping")
}

/// Ensure the built-ins exist and are canonical - write each one (overwriting).
/// They are immutable and undeletable, so always restoring them keeps the clone
/// bases trustworthy even if a file was removed out-of-band. Call before listing
/// templates or cloning from one.
pub fn ensure_builtins(templates_dir: &Path) -> std::io::Result<()> {
    for tmpl in builtins() {
        store::save(templates_dir, &tmpl)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn builtins_are_a_fixed_set() {
        let b = builtins();
        let names: Vec<&str> = b.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Development", "Shipping"]);
        assert!(b.iter().all(|t| t.builtin), "all are fixed built-ins");
        // project-agnostic
        assert!(b.iter().all(|t| t.target.is_none() && t.output.base_dir.is_empty()));

        let ship = b.iter().find(|t| t.name == "Shipping").unwrap();
        assert_eq!(ship.configs, vec![Configuration::Shipping]);
        assert!(!ship.phases.stage.debug_symbols, "shipping drops symbols");

        assert!(is_builtin("builtin-development"));
        assert!(is_builtin("builtin-shipping"));
        assert!(!is_builtin("my-template"));
    }

    #[test]
    fn builtins_are_self_healing_and_never_duplicated() {
        let d = tempdir().unwrap();
        ensure_builtins(d.path()).unwrap();
        assert_eq!(store::load_all(d.path()).len(), 2);

        // Removed out-of-band ⇒ restored on the next ensure (it cannot be deleted
        // via the app, but the file must self-heal regardless).
        store::delete(d.path(), "builtin-shipping").unwrap();
        assert_eq!(store::load_all(d.path()).len(), 1);
        ensure_builtins(d.path()).unwrap();
        assert_eq!(store::load_all(d.path()).len(), 2);
    }
}
