//! Footprint **categorization rules** (R3, `docs/build-footprint.md`) - the single
//! source of truth for *what cleanup may delete* and *what it must never touch*. The
//! scanner ([`super::scan`]) and cleaner ([`super::clean`]) derive from the maps here.
//!
//! **The Clean-tab model.** Reclaimable artifacts group into four buckets:
//! - **Save** - `Saved/StagedBuilds` (Staged), `Saved/Cooked` (Cooked), and
//!   `Saved/Shaders` + `Saved/ShaderDebugInfo` (Shader).
//! - **Binaries** - the **game** target's `Binaries/<plat>/<target>*` files (third-party
//!   `boost_*`/`tbb*` and the editor's `UnrealEditor-*` are left), and each **plugin**'s
//!   whole `Plugins/<name>/Binaries`.
//! - **Intermediate** - the **game** target's `Intermediate/Build/<plat>/<target>/<config>`
//!   dirs (one per build target × build mode, editor excluded) and each **plugin**'s whole
//!   `Plugins/<name>/Intermediate`; plus a wholesale "remove all" that wipes the entire
//!   `Intermediate/` tree (editor + UBT scratch included) - the only path that touches the
//!   editor compile cache.
//! - **Cache** - the project-local `DerivedDataCache/`.
//!
//! Game-vs-editor classification is by the **first token** of a dir/file name (split on
//! `-`/`.`) against the detected target names. Locked scope decisions: DDC is
//! project-local only; inside `Saved/` only the named pipeline outputs are deletable;
//! recovery/user-data subfolders are guardrailed; `Build/`, archive outputs, IDE files and
//! engine-external paths are out of scope (outputs → History).

use crate::profiles::schema::CleanupCategory;

/// Compile-artifact role: the packaged game vs the editor (incl. engine tools).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Build,
    Editor,
}

/// Engine/tool targets that are always editor-side (built by the editor build and
/// needed to open/cook). Their `Intermediate/Build` subdirs + `Binaries` files group
/// with the editor target.
pub const TOOL_TARGETS: &[&str] = &["UnrealEditor", "ShaderCompileWorker", "UnrealPak"];

/// Which target names count as "build" (game/client/server) vs "editor" when
/// classifying compile artifacts. Built backend-side from detected targets - the
/// Footprint tab uses all non-editor targets; the Clean-up phase uses the profile's one
/// target. Never crosses IPC (no `specta`).
#[derive(Debug, Clone, Default)]
pub struct TargetScope {
    pub build: Vec<String>,
    /// Editor target name(s) - `TOOL_TARGETS` are appended by [`TargetScope::new`].
    pub editor: Vec<String>,
}

impl TargetScope {
    pub fn new(build: Vec<String>, editor_target: Option<String>) -> Self {
        let mut editor: Vec<String> = TOOL_TARGETS.iter().map(|s| s.to_string()).collect();
        if let Some(e) = editor_target {
            if !e.trim().is_empty() {
                editor.push(e);
            }
        }
        TargetScope { build, editor }
    }

    fn role_of(&self, token: &str) -> Option<Role> {
        if self.build.iter().any(|t| t.eq_ignore_ascii_case(token)) {
            Some(Role::Build)
        } else if self.editor.iter().any(|t| t.eq_ignore_ascii_case(token)) {
            Some(Role::Editor)
        } else {
            None
        }
    }
}

/// The leading token of a target dir/file name, used for classification:
/// `UnrealEditor-SampleProject.dll` → `UnrealEditor`; `SampleProjectSteam.exe` →
/// `SampleProjectSteam`; `boost_thread-mt-x64.dll` → `boost_thread` (matches nothing →
/// left alone).
pub fn first_token(name: &str) -> &str {
    name.split(['-', '.']).next().unwrap_or(name)
}

/// Classify a compile-artifact dir/file name into a role, or `None` (third-party /
/// unknown → never touched).
pub fn classify(name: &str, scope: &TargetScope) -> Option<Role> {
    scope.role_of(first_token(name))
}

/// Project-relative dirs for a fixed (non-compile) category. The compile categories
/// (`*Game`/`*Plugin`) resolve per-target / per-plugin in [`super::scan`], so they map to
/// no fixed paths here.
pub fn simple_paths(cat: CleanupCategory) -> &'static [&'static str] {
    use CleanupCategory::*;
    match cat {
        Staged => &["Saved/StagedBuilds"],
        Cooked => &["Saved/Cooked"],
        Shader => &["Saved/Shaders", "Saved/ShaderDebugInfo"],
        DerivedData => &["DerivedDataCache"],
        SteamBuildOutput => &[".uep/steam-build-output"],
        BinariesGame | BinariesPlugin | IntermediateGame | IntermediateOther | IntermediatePlugin => &[],
    }
}

/// All categories, in Clean-tab display order - the complete reclaim surface (used by
/// full-clean and tests).
#[allow(dead_code)]
pub const ALL_CATEGORIES: [CleanupCategory; 10] = [
    CleanupCategory::Staged,
    CleanupCategory::Cooked,
    CleanupCategory::Shader,
    CleanupCategory::BinariesGame,
    CleanupCategory::BinariesPlugin,
    CleanupCategory::IntermediateGame,
    CleanupCategory::IntermediateOther,
    CleanupCategory::IntermediatePlugin,
    CleanupCategory::DerivedData,
    CleanupCategory::SteamBuildOutput,
];

// ── guardrail (defense-in-depth; the resolver already only yields valid targets) ──────

/// Protected project-relative paths cleanup must **never** delete - source, VCS,
/// packaging resources, and the recovery/user-data subfolders of `Saved/`.
pub const PROTECTED: &[&str] = &[
    "Content",
    "Config",
    "Source",
    "Build", // mixed packaging resources + archive output → out of scope entirely
    ".git",
    "Saved/Autosaves",
    "Saved/Backup",
    "Saved/Screenshots",
    "Saved/Collections",
    "Saved/Config",
];

/// The roots cleanup is allowed to remove within (project + per-plugin). Combined with
/// [`is_protected`] and the cleaner's canonicalized-containment check.
fn is_under_cleanup_root(rel: &str) -> bool {
    const ROOTS: &[&str] = &[
        "Intermediate",
        "Binaries",
        "Saved/Cooked",
        "Saved/StagedBuilds",
        "Saved/ShaderDebugInfo",
        "Saved/Shaders",
        "DerivedDataCache",
        // Only this exact `.uep/` subpath is cleanable - history/, cache/, profiles/,
        // steam-config/ (the rest of `.uep/`) stay off-limits.
        ".uep/steam-build-output",
    ];
    ROOTS.iter().any(|r| is_within(r, rel)) || is_plugin_compile(rel)
}

/// `Plugins/<…>/{Intermediate,Binaries}/...` - the per-plugin compile mirror, at **any
/// nesting depth** (plugins live in arbitrary subfolders, e.g. a plugin group
/// `Plugins/COG/Cog/Binaries`, or `Plugins/GameFeatures/Foo/Intermediate`). True iff the
/// path is under `Plugins/` and has a `Binaries`/`Intermediate` segment past the plugin
/// name (index ≥ 2).
fn is_plugin_compile(rel: &str) -> bool {
    let xs: Vec<&str> = rel.split('/').collect();
    xs.first().is_some_and(|f| f.eq_ignore_ascii_case("Plugins"))
        && xs.iter().enumerate().any(|(i, s)| i >= 2 && (s.eq_ignore_ascii_case("Binaries") || s.eq_ignore_ascii_case("Intermediate")))
}

fn normalize(rel: &str) -> String {
    rel.replace('\\', "/").trim_matches('/').trim_start_matches("./").trim_matches('/').to_string()
}

fn is_within(prefix: &str, path: &str) -> bool {
    let ps: Vec<&str> = prefix.split('/').collect();
    let xs: Vec<&str> = path.split('/').collect();
    xs.len() >= ps.len() && ps.iter().zip(&xs).all(|(p, x)| p.eq_ignore_ascii_case(x))
}

/// Plugin **source** dirs - protected even though their sibling `Binaries`/`Intermediate`
/// are deletable. Nesting-aware (matches the plugin's `Source`/`Content`/`Config` or its
/// `.uplugin` at any depth under `Plugins/`).
fn is_plugin_source(path: &str) -> bool {
    let xs: Vec<&str> = path.split('/').collect();
    if !xs.first().is_some_and(|f| f.eq_ignore_ascii_case("Plugins")) {
        return false;
    }
    xs.iter().enumerate().any(|(i, s)| {
        i >= 2 && {
            let l = s.to_ascii_lowercase();
            matches!(l.as_str(), "source" | "content" | "config") || l.ends_with(".uplugin")
        }
    })
}

/// True iff a project-relative path is in the never-delete guardrail set.
pub fn is_protected(rel: &str) -> bool {
    let n = normalize(rel);
    n.is_empty()
        || PROTECTED.iter().any(|p| is_within(p, &n))
        || n.to_ascii_lowercase().ends_with(".uproject")
        || is_plugin_source(&n)
}

/// The authoritative gate the cleaner calls on every target's path: inside a cleanup
/// root and not protected. (Containment within the project root is enforced separately
/// by the caller via canonicalization.)
pub fn is_cleanup_path(rel: &str) -> bool {
    let n = normalize(rel);
    !n.is_empty() && !is_protected(&n) && is_under_cleanup_root(&n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use CleanupCategory::*;

    fn scope() -> TargetScope {
        TargetScope::new(vec!["SampleProjectSteam".into()], Some("SampleProjectEditor".into()))
    }

    #[test]
    fn classify_splits_build_editor_tools_and_skips_thirdparty() {
        let s = scope();
        assert_eq!(classify("SampleProjectSteam", &s), Some(Role::Build));
        assert_eq!(classify("SampleProjectSteam.exe", &s), Some(Role::Build));
        assert_eq!(classify("SampleProjectSteam-Win64-Shipping.exe", &s), Some(Role::Build));
        assert_eq!(classify("SampleProjectEditor", &s), Some(Role::Editor));
        assert_eq!(classify("SampleProjectEditor.target", &s), Some(Role::Editor));
        assert_eq!(classify("UnrealEditor", &s), Some(Role::Editor)); // engine editor
        assert_eq!(classify("UnrealEditor-SampleProject.dll", &s), Some(Role::Editor));
        assert_eq!(classify("ShaderCompileWorker", &s), Some(Role::Editor));
        assert_eq!(classify("UnrealPak", &s), Some(Role::Editor));
        // third-party + UBT scratch dirs match nothing → never touched
        assert_eq!(classify("boost_thread-mt-x64.dll", &s), None);
        assert_eq!(classify("tbb.dll", &s), None);
        assert_eq!(classify("x64", &s), None);
    }

    #[test]
    fn simple_paths_map_the_fixed_categories() {
        assert_eq!(simple_paths(Staged), ["Saved/StagedBuilds"]);
        assert_eq!(simple_paths(Cooked), ["Saved/Cooked"]);
        // Shader = the editor's PC shader cache + shader debug info.
        assert_eq!(simple_paths(Shader), ["Saved/Shaders", "Saved/ShaderDebugInfo"]);
        assert_eq!(simple_paths(DerivedData), ["DerivedDataCache"]);
        assert_eq!(simple_paths(SteamBuildOutput), [".uep/steam-build-output"]);
        // Compile categories resolve per-target/plugin in `scan`, not via fixed paths.
        for c in [BinariesGame, BinariesPlugin, IntermediateGame, IntermediatePlugin] {
            assert!(simple_paths(c).is_empty());
        }
    }

    #[test]
    fn guardrail_allows_compile_roots_and_shaders_but_not_source_recovery() {
        for ok in [
            "Intermediate", // the wholesale "remove all" target
            "Intermediate/Build/Win64/SampleProjectSteam/Development",
            "Binaries/Win64",
            "Plugins/Cool/Intermediate",
            "Plugins/Cool/Binaries",
            "Plugins/Group/Nested/Binaries",     // nested plugin (e.g. a plugin group)
            "Plugins/Group/Nested/Intermediate", // nested plugin
            "Saved/Cooked",
            "Saved/Shaders",
            "Saved/ShaderDebugInfo",
            "DerivedDataCache",
            ".uep/steam-build-output",
            ".uep/steam-build-output/dev/output",
        ] {
            assert!(is_cleanup_path(ok), "{ok} should be a cleanup path");
        }
        for no in [
            "",
            "Content",
            "Source",
            "Build",
            "SampleProject.uproject",
            "Saved/Autosaves",
            "Saved/Config",
            "Plugins/Cool/Source",
            "Plugins/Cool/Cool.uplugin",
            "Plugins/Group/Nested/Source",  // nested plugin source still protected
            "Plugins/Group/Nested/Content", // nested plugin content still protected
            "Saved/Logs", // misc cache, not a cleanup root
            ".uep/history",              // build records - never touched
            ".uep/cache",                // derived index - never touched
            ".uep/profiles",             // committed profiles - never touched
            ".uep/steam-config/dev/app_build.vdf", // committed Steam config - never touched
        ] {
            assert!(!is_cleanup_path(no), "{no} must NOT be a cleanup path");
        }
    }
}
