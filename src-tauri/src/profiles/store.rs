//! Load/save of `BuildConfig` files - one JSON file per profile
//! (`.uep/profiles/<id>.json`, committed) or template (`templates/<id>.json`,
//! app folder). Both stores share this code; callers pass the directory, so the
//! functions stay pure and unit-testable (mirrors `crate::storage`).
//!
//! Also the **copy-on-create** transforms (`docs/data-storage.md` §"templates vs
//! profiles"): new-from-template, clone-from-profile, blank, and duplicate. Each
//! takes the new `id` as input - id generation needs the clock, so it lives in
//! the command layer; the transforms stay deterministic.

use std::path::{Path, PathBuf};

use super::schema::{BuildConfig, CleanupCategory, SCHEMA_VERSION};

fn file_for(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Parse a `BuildConfig`, tolerating a stale `phases.cleanup.categories` from an older
/// category set: unknown tokens are dropped (rather than failing the whole profile) so a
/// pre-rework profile still loads. Kept here (not in serde) so the IPC/specta type for
/// `BuildConfig` stays a single clean type.
fn parse(text: &str) -> Option<BuildConfig> {
    let mut v: serde_json::Value = serde_json::from_str(text).ok()?;
    if let Some(cats) = v.pointer_mut("/phases/cleanup/categories").and_then(|c| c.as_array_mut()) {
        cats.retain(|t| t.as_str().is_some_and(|s| CleanupCategory::from_token(s).is_some()));
    }
    serde_json::from_value(v).ok()
}

/// Every `BuildConfig` in `dir`, sorted by name. Unparseable/foreign files are
/// skipped (a stray non-config `.json` shouldn't break the list).
pub fn load_all(dir: &Path) -> Vec<BuildConfig> {
    let mut out: Vec<BuildConfig> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out; // missing dir ⇒ empty
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(cfg) = parse(&text) {
                out.push(cfg);
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

pub fn load_one(dir: &Path, id: &str) -> Option<BuildConfig> {
    let text = std::fs::read_to_string(file_for(dir, id)).ok()?;
    parse(&text)
}

pub fn save(dir: &Path, cfg: &BuildConfig) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(file_for(dir, &cfg.id), json)
}

pub fn delete(dir: &Path, id: &str) -> std::io::Result<()> {
    let path = file_for(dir, id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        // Already gone ⇒ deletion is idempotent.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

// ── copy-on-create transforms ────────────────────────────────────────────────

/// New profile from a template: copy every field, stamp a fresh `id`/`name`, and
/// record provenance. Templates already leave the project-specific fields empty.
/// `builtin` is reset - a copy is never itself a fixed built-in.
pub fn from_template(id: String, name: String, template: &BuildConfig) -> BuildConfig {
    BuildConfig {
        id,
        name,
        schema_version: SCHEMA_VERSION,
        based_on_template: Some(template.id.clone()),
        builtin: false,
        ..template.clone()
    }
}

/// Clone an existing config into a new, self-contained one - no live link to the
/// source (provenance dropped, `builtin` reset). Used to clone a profile→profile
/// and a template→template (the only way to create a user template).
pub fn from_clone(id: String, name: String, source: &BuildConfig) -> BuildConfig {
    BuildConfig {
        id,
        name,
        schema_version: SCHEMA_VERSION,
        based_on_template: None,
        builtin: false,
        ..source.clone()
    }
}

/// Duplicate any config into a new `<name> (copy)` with a fresh id - a
/// self-contained copy (`docs/requirement.md` R1).
pub fn duplicate(id: String, source: &BuildConfig) -> BuildConfig {
    BuildConfig {
        id,
        name: format!("{} (copy)", source.name),
        based_on_template: None,
        builtin: false,
        ..source.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::schema::{Configuration, CookMaps};
    use tempfile::tempdir;

    fn sample(id: &str, name: &str) -> BuildConfig {
        let mut c = BuildConfig::default();
        c.id = id.into();
        c.name = name.into();
        c.config = Configuration::Shipping;
        c.target = Some("SampleProjectSteam".into());
        c.output.base_dir = "C:/Builds".into();
        c
    }

    #[test]
    fn save_load_delete_round_trip() {
        let d = tempdir().unwrap();
        let c = sample("dev", "Development");
        save(d.path(), &c).unwrap();

        assert_eq!(load_one(d.path(), "dev").unwrap(), c);
        assert_eq!(load_all(d.path()).len(), 1);

        delete(d.path(), "dev").unwrap();
        assert!(load_one(d.path(), "dev").is_none());
        assert!(load_all(d.path()).is_empty());
        delete(d.path(), "dev").unwrap(); // idempotent
    }

    #[test]
    fn load_all_sorts_by_name_and_skips_junk() {
        let d = tempdir().unwrap();
        save(d.path(), &sample("b", "Zeta")).unwrap();
        save(d.path(), &sample("a", "alpha")).unwrap();
        std::fs::write(d.path().join("notes.json"), "{ not a config").unwrap();
        std::fs::write(d.path().join("readme.txt"), "ignored").unwrap();

        let names: Vec<String> = load_all(d.path()).into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["alpha".to_string(), "Zeta".to_string()]);
    }

    #[test]
    fn from_template_copies_fields_records_provenance_and_resets_builtin() {
        let mut tmpl = BuildConfig::default();
        tmpl.id = "builtin-development".into();
        tmpl.name = "Default".into();
        tmpl.config = Configuration::Shipping;
        tmpl.builtin = true; // cloning a fixed built-in...

        let p = from_template("p1".into(), "My Ship".into(), &tmpl);
        assert_eq!(p.id, "p1");
        assert_eq!(p.name, "My Ship");
        assert_eq!(p.config, Configuration::Shipping); // copied
        assert_eq!(p.based_on_template.as_deref(), Some("builtin-development"));
        assert!(!p.builtin, "...must yield an editable, non-built-in copy");
    }

    #[test]
    fn clone_is_self_contained_no_provenance_or_builtin() {
        let mut src = sample("src", "Source");
        src.based_on_template = Some("builtin-development".into());
        src.builtin = true;
        src.phases.cook.maps = CookMaps::List(vec!["Entry".into()]);

        let c = from_clone("c1".into(), "Copy".into(), &src);
        assert_eq!(c.id, "c1");
        assert_eq!(c.phases.cook.maps, CookMaps::List(vec!["Entry".into()])); // deep copy
        assert_eq!(c.based_on_template, None, "clone drops the live link");
        assert!(!c.builtin, "clone is never a built-in");
    }

    #[test]
    fn duplicate_appends_copy_suffix() {
        let src = sample("src", "Nightly");
        let dup = duplicate("dup".into(), &src);
        assert_eq!(dup.id, "dup");
        assert_eq!(dup.name, "Nightly (copy)");
        assert_eq!(dup.config, src.config);
        assert_eq!(dup.based_on_template, None);
    }
}
