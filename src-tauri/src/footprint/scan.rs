//! Footprint **scanner** (M5, R3). Resolves the reclaimable artifacts an Unreal project
//! scatters into the Clean-tab node tree - **Save** / **Binaries** / **Intermediate** /
//! **Cache** - sizing each leaf, and exposes the same resolution to the cleaner so scan
//! and delete can never disagree about what a node maps to. Pure fs + data; the IPC
//! command runs it on a blocking thread (`docs/requirement.md` R3).
//!
//! The tree is built once by [`collect`]; [`scan`] turns it into the serializable
//! [`FootprintReport`], and [`Collected::into_leaves`] hands the cleaner the flat leaves.
//! Intermediate splits into **Game** (one node per build target - the editor-safe build
//! cache), **Other** (the editor/tool targets + scratch, i.e. the rest of the main
//! `Intermediate/`; wiping it forces a full editor recompile), and **Plugin** (one node for
//! all plugins' `Intermediate`). Binaries' **Plugin** is likewise one node for all plugins.
//! Game + Other partition the whole `Intermediate/` - tick Game for a dev-safe build clean,
//! Game + Other to wipe it entirely.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;

use super::rules::{self, Role, TargetScope};
use crate::profiles::schema::CleanupCategory;

/// One concrete removal unit: a whole directory, or a set of files sharing a target
/// prefix in one `Binaries/<plat>` dir. Shared by scan (size it) and clean (remove it).
pub struct Target {
    /// Display form (Files: `Binaries/Win64/SampleProjectSteam.* (5 files)`).
    pub rel: String,
    /// Project-relative path the cleaner runs the guardrail on (Files: the containing dir).
    pub guard_rel: String,
    /// The dir itself (Dir) or the files' containing dir (Files) - for "open in explorer".
    pub abs: PathBuf,
    pub kind: TargetKind,
    pub size_bytes: u64,
}

pub enum TargetKind {
    Dir,
    /// Exactly these files are removed (never the whole dir - it holds third-party deps).
    Files(Vec<PathBuf>),
}

// ── serializable report (the Clean-tab tree) ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FootprintLocation {
    pub path: String,
    pub rel: String,
    pub size_bytes: f64,
}

/// A node in the Clean-tab tree. A **leaf** is selectable and carries a `category` +
/// `locations`; a **grouping** node (`Save`, `Binaries`, `Intermediate` → `Game`/`Plugin`)
/// has an empty `id`, `selectable = false`, and aggregates its `children`.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FootprintNode {
    /// Stable id the cleaner resolves back to targets (empty for grouping nodes).
    pub id: String,
    pub label: String,
    pub category: Option<CleanupCategory>,
    pub selectable: bool,
    pub size_bytes: f64,
    pub locations: Vec<FootprintLocation>,
    pub children: Vec<FootprintNode>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FootprintReport {
    /// Top-level groups: Save, Binaries, Intermediate, Cache.
    pub groups: Vec<FootprintNode>,
    pub total_bytes: f64,
}

// ── internal resolved leaf ────────────────────────────────────────────────────────────

/// A resolved selectable leaf: its stable id, category, and concrete removal targets.
pub struct Leaf {
    pub id: String,
    pub label: String,
    pub category: CleanupCategory,
    pub targets: Vec<Target>,
    /// Overrides the displayed size (otherwise the sum of `targets`). Used by **Other**,
    /// whose delete target is the *whole* `Intermediate/` but whose row shows only the
    /// remainder (whole - the game targets listed above it), so Game's children stay
    /// additive and the totals don't double-count.
    pub display_bytes: Option<u64>,
}

impl Leaf {
    fn size(&self) -> u64 {
        self.display_bytes.unwrap_or_else(|| self.targets.iter().map(|t| t.size_bytes).sum())
    }
    fn locations(&self) -> Vec<FootprintLocation> {
        let mut v: Vec<FootprintLocation> = self
            .targets
            .iter()
            .map(|t| FootprintLocation { path: t.abs.to_string_lossy().to_string(), rel: t.rel.clone(), size_bytes: t.size_bytes as f64 })
            .collect();
        v.sort_by(|a, b| b.size_bytes.partial_cmp(&a.size_bytes).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
    fn node(&self) -> FootprintNode {
        FootprintNode {
            id: self.id.clone(),
            label: self.label.clone(),
            category: Some(self.category),
            selectable: true,
            size_bytes: self.size() as f64,
            locations: self.locations(),
            children: Vec::new(),
        }
    }
}

/// A grouping node (`Save`, `Binaries`, an Intermediate `Game`/`Plugin` section). Not
/// selectable; its size is the sum of its children.
fn group(label: &str, children: Vec<FootprintNode>) -> FootprintNode {
    let size_bytes = children.iter().map(|c| c.size_bytes).sum();
    FootprintNode { id: String::new(), label: label.to_string(), category: None, selectable: false, size_bytes, locations: Vec::new(), children }
}

// ── the resolved tree ─────────────────────────────────────────────────────────────────

/// The fully-resolved footprint, built once and shared by scan (→ report) and clean (→
/// flat leaves).
pub struct Collected {
    save: Vec<Leaf>,
    binaries_game: Leaf,
    /// One leaf covering **all** plugins' `Binaries` (selecting Plugin cleans them all).
    binaries_plugin: Option<Leaf>,
    /// One leaf per game **target** - the editor-safe build cache. Rendered as children of
    /// the Intermediate **Game** group, alongside `intermediate_other`.
    intermediate_game: Vec<Leaf>,
    /// **Other**: deletes the *whole* main `Intermediate/` (no per-folder scan). Its row
    /// shows the remainder (whole - game) and sits as the last child under Game; ticking it
    /// implies every game target is wiped too.
    intermediate_other: Option<Leaf>,
    /// One leaf covering **all** plugins' `Intermediate`.
    intermediate_plugin: Option<Leaf>,
    derived: Leaf,
    /// The Steam upload scratch dir (`.uep/steam-build-output`) - steamcmd's chunk cache +
    /// logs + resolved run VDFs. Rendered under Cache alongside the DDC.
    steam_build_output: Leaf,
}

impl Collected {
    /// Flat leaves (consumes self). The cleaner's view: it matches leaf ids / categories
    /// and removes their targets.
    pub fn into_leaves(self) -> Vec<Leaf> {
        let mut leaves = self.save;
        leaves.push(self.binaries_game);
        leaves.extend(self.binaries_plugin);
        leaves.extend(self.intermediate_game);
        leaves.extend(self.intermediate_other);
        leaves.extend(self.intermediate_plugin);
        leaves.push(self.derived);
        leaves.push(self.steam_build_output);
        leaves
    }
}

/// Resolve every category to its concrete targets - the single source of truth scan and
/// clean both derive from.
pub fn collect(root: &Path, scope: &TargetScope) -> Collected {
    let intermediate_game = intermediate_game_leaves(root, scope);
    let game_total: u64 = intermediate_game.iter().map(Leaf::size).sum();
    Collected {
        save: vec![
            simple_leaf(root, CleanupCategory::Staged, "Staged build"),
            simple_leaf(root, CleanupCategory::Cooked, "Cooked content"),
            simple_leaf(root, CleanupCategory::Shader, "Shader"),
        ],
        binaries_game: binaries_game_leaf(root, scope),
        binaries_plugin: plugin_agg_leaf(root, "Binaries", CleanupCategory::BinariesPlugin, "binariesPlugin"),
        intermediate_game,
        intermediate_other: intermediate_other_leaf(root, game_total),
        intermediate_plugin: plugin_agg_leaf(root, "Intermediate", CleanupCategory::IntermediatePlugin, "intermediatePlugin"),
        derived: simple_leaf(root, CleanupCategory::DerivedData, "Derived data cache"),
        steam_build_output: simple_leaf(root, CleanupCategory::SteamBuildOutput, "Steam build output"),
    }
}

/// Scan the project into the Clean-tab tree. Walks only resolvable category targets -
/// never the whole project - so it can never size (or offer) source/recovery dirs.
pub fn scan(root: &Path, scope: &TargetScope) -> FootprintReport {
    let c = collect(root, scope);

    // Save - fixed leaves, always shown.
    let save = group("Save", c.save.iter().map(Leaf::node).collect());

    // Binaries - Game (one leaf) + Plugin (one leaf for all plugins), the latter only if present.
    let mut bin_children = vec![c.binaries_game.node()];
    if let Some(p) = &c.binaries_plugin {
        bin_children.push(p.node());
    }
    let binaries = group("Binaries", bin_children);

    // Intermediate - Game group (per-target build caches + an `Other` child that wipes the
    // whole main Intermediate) + Plugin (all plugins). Game's children partition the folder.
    let mut game_children: Vec<FootprintNode> = c.intermediate_game.iter().map(Leaf::node).collect();
    if let Some(o) = &c.intermediate_other {
        game_children.push(o.node());
    }
    let mut int_children = Vec::new();
    if !game_children.is_empty() {
        int_children.push(group("Game", game_children));
    }
    if let Some(p) = &c.intermediate_plugin {
        int_children.push(p.node());
    }
    let intermediate = group("Intermediate", int_children);

    let cache = group("Cache", vec![c.derived.node(), c.steam_build_output.node()]);

    // Whole-tree total = sum of the four group totals (each already sums its leaves),
    // so there's no second pass over every leaf.
    let total_bytes = save.size_bytes + binaries.size_bytes + intermediate.size_bytes + cache.size_bytes;

    FootprintReport { groups: vec![save, binaries, intermediate, cache], total_bytes }
}

// ── resolvers ─────────────────────────────────────────────────────────────────────────

/// A fixed (non-compile) category → its dirs that exist on disk. Always returns a leaf
/// (size 0 if none present) so the Save/Cache structure is stable.
fn simple_leaf(root: &Path, cat: CleanupCategory, label: &str) -> Leaf {
    let targets = rules::simple_paths(cat).iter().filter_map(|p| dir_target(root, p)).collect();
    Leaf { id: cat.as_str().to_string(), label: label.to_string(), category: cat, targets, display_bytes: None }
}

/// The project's game-target `Binaries/<plat>/<target>*` files (third-party + editor
/// files left). One leaf, with one `Files` target per `(plat, target)` group.
fn binaries_game_leaf(root: &Path, scope: &TargetScope) -> Leaf {
    let bin = root.join("Binaries");
    let mut targets = Vec::new();
    for plat in subdirs(&bin) {
        let plat_rel = format!("Binaries/{plat}");
        let mut groups: BTreeMap<String, (Vec<PathBuf>, u64)> = BTreeMap::new();
        for (name, path, size) in files(&root.join(&plat_rel)) {
            if rules::classify(&name, scope) == Some(Role::Build) {
                let e = groups.entry(rules::first_token(&name).to_string()).or_default();
                e.0.push(path);
                e.1 += size;
            }
        }
        for (token, (paths, size)) in groups {
            let n = paths.len();
            let rel = format!("{plat_rel}/{token}.* ({n} file{})", if n == 1 { "" } else { "s" });
            targets.push(Target { rel, guard_rel: plat_rel.clone(), abs: root.join(&plat_rel), kind: TargetKind::Files(paths), size_bytes: size });
        }
    }
    Leaf { id: "binariesGame".to_string(), label: "Game".to_string(), category: CleanupCategory::BinariesGame, targets, display_bytes: None }
}

/// One leaf covering **every** plugin's `<sub>` dir (`Binaries` or `Intermediate`) - the
/// whole dirs are the removal units. Selecting "Plugin" cleans them all. `None` if no
/// plugin has that dir.
fn plugin_agg_leaf(root: &Path, sub: &str, cat: CleanupCategory, id: &str) -> Option<Leaf> {
    let targets: Vec<Target> = plugin_bases(root).iter().filter_map(|base| dir_target(root, &format!("{base}/{sub}"))).collect();
    (!targets.is_empty()).then(|| Leaf { id: id.to_string(), label: "Plugin".to_string(), category: cat, targets, display_bytes: None })
}

/// The project's game-target intermediate dirs, **one leaf per build target** (editor
/// targets + scratch excluded - those go to `Other`). A target's leaf removes its whole
/// target dir across every platform / build mode. `id` = `intermediateGame:<target>`.
///
/// Handles both UBT layouts: targets directly under `Intermediate/Build/<plat>/<target>`
/// (older) and nested under an **architecture** folder `Intermediate/Build/<plat>/<arch>/<target>`
/// (newer UE - `<arch>` = `x64`/`arm64`, which itself classifies to nothing). Both fold into
/// one per-target leaf by name.
fn intermediate_game_leaves(root: &Path, scope: &TargetScope) -> Vec<Leaf> {
    let mut by_target: BTreeMap<String, Vec<Target>> = BTreeMap::new();
    let build_root = root.join("Intermediate/Build");
    for plat in subdirs(&build_root) {
        let plat_rel = format!("Intermediate/Build/{plat}");
        for entry in subdirs(&root.join(&plat_rel)) {
            let entry_rel = format!("{plat_rel}/{entry}");
            match rules::classify(&entry, scope) {
                Some(Role::Build) => push_target(root, &mut by_target, entry, &entry_rel),
                Some(Role::Editor) => {} // editor target → Other
                None => {
                    // `entry` may be an architecture folder - descend one level for targets.
                    for tgt in subdirs(&root.join(&entry_rel)) {
                        if rules::classify(&tgt, scope) == Some(Role::Build) {
                            let rel = format!("{entry_rel}/{tgt}");
                            push_target(root, &mut by_target, tgt, &rel);
                        }
                    }
                }
            }
        }
    }
    by_target
        .into_iter()
        .map(|(tgt, targets)| Leaf { id: format!("intermediateGame:{tgt}"), label: tgt, category: CleanupCategory::IntermediateGame, targets, display_bytes: None })
        .collect()
}

fn push_target(root: &Path, by_target: &mut BTreeMap<String, Vec<Target>>, name: String, rel: &str) {
    if let Some(t) = dir_target(root, rel) {
        by_target.entry(name).or_default().push(t);
    }
}

/// **Other** - the catch-all under Game: its delete target is the *whole* main `Intermediate/`
/// (no per-folder scan), but its row shows only the **remainder** (whole - the game targets
/// listed above), so Game's children stay additive. `None` when there's nothing beyond the
/// game targets (remainder 0) or no `Intermediate/`. `game_total` is the summed size of the
/// per-target leaves.
fn intermediate_other_leaf(root: &Path, game_total: u64) -> Option<Leaf> {
    let whole = dir_target(root, "Intermediate")?;
    let remainder = whole.size_bytes.saturating_sub(game_total);
    (remainder > 0).then(|| Leaf {
        id: "intermediateOther".to_string(),
        label: "Other".to_string(),
        category: CleanupCategory::IntermediateOther,
        targets: vec![whole],
        display_bytes: Some(remainder),
    })
}

// ── fs helpers ──────────────────────────────────────────────────────────────────────

fn dir_target(root: &Path, rel: &str) -> Option<Target> {
    let abs = root.join(rel);
    abs.is_dir().then(|| {
        let size = dir_size(&abs);
        Target { rel: rel.to_string(), guard_rel: rel.to_string(), abs, kind: TargetKind::Dir, size_bytes: size }
    })
}

/// Each plugin root (`Plugins/<…>`), found by its `.uplugin`.
fn plugin_bases(root: &Path) -> Vec<String> {
    let mut bases = Vec::new();
    let plugins = root.join("Plugins");
    if plugins.is_dir() {
        for e in WalkDir::new(&plugins).into_iter().flatten() {
            if e.file_type().is_file() && e.path().extension().map_or(false, |x| x.eq_ignore_ascii_case("uplugin")) {
                if let Some(dir) = e.path().parent() {
                    if let Ok(rel) = dir.strip_prefix(root) {
                        bases.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
    }
    bases.sort();
    bases
}

fn subdirs(dir: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                v.push(e.file_name().to_string_lossy().to_string());
            }
        }
    }
    v.sort();
    v
}

fn files(dir: &Path) -> Vec<(String, PathBuf, u64)> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() {
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                v.push((e.file_name().to_string_lossy().to_string(), p, size));
            }
        }
    }
    v
}

fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn put(root: &Path, rel: &str, bytes: usize) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; bytes]).unwrap();
    }
    fn scope() -> TargetScope {
        TargetScope::new(vec!["SampleProjectSteam".into()], Some("SampleProjectEditor".into()))
    }
    fn seed(root: &Path) {
        // Intermediate - game (2 configs) + editor + UBT scratch + plugin.
        put(root, "Intermediate/Build/Win64/SampleProjectSteam/Development/a.obj", 100);
        put(root, "Intermediate/Build/Win64/SampleProjectSteam/Shipping/b.obj", 60);
        put(root, "Intermediate/Build/Win64/SampleProjectEditor/Development/e.obj", 200);
        put(root, "Intermediate/Build/Win64/UnrealEditor/u.obj", 50);
        put(root, "Intermediate/Build/Win64/x64/junk.bin", 999); // unclassified
        // Binaries - game files, editor file, third-party.
        put(root, "Binaries/Win64/SampleProjectSteam.exe", 300);
        put(root, "Binaries/Win64/SampleProjectSteam.pdb", 30);
        put(root, "Binaries/Win64/UnrealEditor-SampleProject.dll", 400); // editor → left
        put(root, "Binaries/Win64/boost_thread-mt-x64.dll", 999); // third-party → left
        // Plugin - whole Binaries + Intermediate dirs.
        put(root, "Plugins/Cool/Cool.uplugin", 1);
        put(root, "Plugins/Cool/Binaries/Win64/UnrealEditor-Mod.dll", 10);
        put(root, "Plugins/Cool/Intermediate/Build/Win64/SampleProjectEditor/p.obj", 20);
        // Save.
        put(root, "Saved/Cooked/Windows/a.uasset", 300);
        put(root, "Saved/Shaders/PCD3D_SM5/s.bin", 25);
        put(root, "Saved/ShaderDebugInfo/d.bin", 5);
        put(root, "Saved/StagedBuilds/Windows/g.exe", 400);
        // Cache.
        put(root, "DerivedDataCache/x.ddc", 70);
        // Protected - must never be sized.
        put(root, "Content/Map.umap", 9999);
        put(root, "Saved/Autosaves/r.umap", 9999);
    }

    fn leaf<'a>(c: &'a Collected, id: &str) -> &'a Leaf {
        c.save
            .iter()
            .chain(std::iter::once(&c.binaries_game))
            .chain(&c.binaries_plugin)
            .chain(&c.intermediate_game)
            .chain(&c.intermediate_other)
            .chain(&c.intermediate_plugin)
            .chain(std::iter::once(&c.derived))
            .chain(std::iter::once(&c.steam_build_output))
            .find(|l| l.id == id)
            .unwrap_or_else(|| panic!("no leaf {id}"))
    }

    #[test]
    fn collect_resolves_each_bucket() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        let c = collect(root, &scope());

        assert_eq!(leaf(&c, "staged").size(), 400);
        assert_eq!(leaf(&c, "cooked").size(), 300);
        assert_eq!(leaf(&c, "shader").size(), 25 + 5); // Shaders + ShaderDebugInfo
        assert_eq!(leaf(&c, "derivedData").size(), 70);
        assert_eq!(leaf(&c, "binariesGame").size(), 300 + 30); // game files only

        // Intermediate Game - one leaf per target (sums its build modes), editor + x64 excluded.
        assert_eq!(c.intermediate_game.len(), 1);
        assert_eq!(leaf(&c, "intermediateGame:SampleProjectSteam").size(), 100 + 60); // Development + Shipping
        assert_eq!(c.intermediate_game[0].label, "SampleProjectSteam");

        // Plugin - one aggregated leaf each (all plugins), not per-plugin.
        assert_eq!(leaf(&c, "binariesPlugin").size(), 10);
        assert_eq!(leaf(&c, "intermediatePlugin").size(), 20);
        assert_eq!(c.binaries_plugin.as_ref().unwrap().label, "Plugin");

        // Other - the rest of the main Intermediate: editor target + tool + x64 scratch
        // (NOT the game target). Game + Other = the whole main Intermediate.
        assert_eq!(leaf(&c, "intermediateOther").size(), 200 + 50 + 999);
        let other = c.intermediate_other.as_ref().unwrap();
        assert_eq!(other.label, "Other");
        assert!(!other.locations().iter().any(|l| l.rel.contains("SampleProjectSteam"))); // game stays
    }

    #[test]
    fn scan_totals_and_omits_protected() {
        let d = tempdir().unwrap();
        let root = d.path();
        seed(root);
        let r = scan(root, &scope());

        // total = every leaf; Game + Other now cover the whole Intermediate (incl. editor/x64).
        let expected = 400 + 300 + 30 /*save: staged,cooked,shader*/
            + 330 /*binariesGame*/ + 10 /*plugin bin*/
            + 100 + 60 /*intermediate game*/ + (200 + 50 + 999) /*intermediate other*/ + 20 /*plugin intermediate*/
            + 70 /*ddc*/;
        assert_eq!(r.total_bytes, expected as f64);
        assert_eq!(r.groups.len(), 4);

        // No protected/unclassified path leaks into any location.
        fn walk<'a>(n: &'a FootprintNode, out: &mut Vec<&'a str>) {
            out.extend(n.locations.iter().map(|l| l.rel.as_str()));
            for c in &n.children {
                walk(c, out);
            }
        }
        let mut rels = Vec::new();
        for g in &r.groups {
            walk(g, &mut rels);
        }
        assert!(!rels.iter().any(|r| r.contains("Content") || r.contains("Autosaves") || r.contains("boost")));
    }

    #[test]
    fn intermediate_game_folds_arch_nested_and_legacy_layouts() {
        let d = tempdir().unwrap();
        let root = d.path();
        // newer UBT layout: Build/<plat>/<arch>/<target>/<config> - the bulk lives here
        put(root, "Intermediate/Build/Win64/x64/SampleProjectSteam/Development/a.obj", 5000);
        put(root, "Intermediate/Build/Win64/x64/SampleProjectEditor/Development/e.obj", 400); // editor → excluded
        put(root, "Intermediate/Build/Win64/x64/UnrealEditor/Development/u.obj", 99); // tool → excluded
        // legacy flat layout for the same target also present
        put(root, "Intermediate/Build/Win64/SampleProjectSteam/Development/b.obj", 70);
        let c = collect(root, &scope());

        assert_eq!(c.intermediate_game.len(), 1);
        let l = leaf(&c, "intermediateGame:SampleProjectSteam");
        assert_eq!(l.size(), 5000 + 70); // arch-nested + legacy, editor/tool excluded
        assert_eq!(l.targets.len(), 2);
    }

    #[test]
    fn steam_build_output_is_scanned_under_cache() {
        let d = tempdir().unwrap();
        let root = d.path();
        put(root, ".uep/steam-build-output/dev/output/chunk.csm", 500);
        put(root, "DerivedDataCache/x.ddc", 70);
        // Other .uep/ data must never be sized by the scanner.
        put(root, ".uep/history/rec/build.json", 9999);
        put(root, ".uep/steam-config/dev/app_build.vdf", 9999);

        let c = collect(root, &scope());
        assert_eq!(leaf(&c, "steamBuildOutput").size(), 500);

        let r = scan(root, &scope());
        let cache = r.groups.iter().find(|g| g.label == "Cache").unwrap();
        assert!(cache.children.iter().any(|n| n.id == "steamBuildOutput"));
        // No committed/derived .uep/ data leaked into any location.
        fn walk<'a>(n: &'a FootprintNode, out: &mut Vec<&'a str>) {
            out.extend(n.locations.iter().map(|l| l.rel.as_str()));
            for c in &n.children {
                walk(c, out);
            }
        }
        let mut rels = Vec::new();
        for g in &r.groups {
            walk(g, &mut rels);
        }
        assert!(!rels.iter().any(|r| r.contains("history") || r.contains("steam-config")));
    }

    #[test]
    fn empty_project_is_stable() {
        let d = tempdir().unwrap();
        let r = scan(d.path(), &scope());
        assert_eq!(r.total_bytes, 0.0);
        assert_eq!(r.groups.len(), 4);
    }
}
