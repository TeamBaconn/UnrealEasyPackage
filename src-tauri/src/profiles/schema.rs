//! The serde schema behind both **templates** (global, project-agnostic) and
//! **profiles** (project-local, concrete). One `BuildConfig` backs both: a
//! template leaves the project-specific fields empty; a profile fills them
//! (copy-on-create). See `docs/data-storage.md` §"Profile / template schema".
//!
//! `#[serde(rename_all = "camelCase")]` keeps the on-disk JSON keys stable;
//! `tauri-specta` emits the matching TS types for the editor (one source of
//! truth). Each per-phase cfg owns its `enabled` toggle (default on, except the two
//! opt-in app phases - Copy Extras / Clean-up). The Stage-gate for Pak/Archive
//! isn't stored; the editor derives it from the registry's `gated_by`. Tags
//! (platform/config/target/status) are **derived** by `history/`, never stored here.

use serde::{Deserialize, Serialize};

/// Current on-disk schema version (bumped only on breaking JSON changes).
pub const SCHEMA_VERSION: u32 = 1;

// ── enums ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub enum Platform {
    #[default]
    Win64,
    Linux,
    Mac,
}

impl Platform {
    /// The `-platform=` flag value.
    pub fn uat(self) -> &'static str {
        match self {
            Platform::Win64 => "Win64",
            Platform::Linux => "Linux",
            Platform::Mac => "Mac",
        }
    }
    /// The staged platform folder name (UE5 dropped the `NoEditor` suffix); also
    /// the `{platform}` token value.
    pub fn folder(self) -> &'static str {
        match self {
            Platform::Win64 => "Windows",
            Platform::Linux => "Linux",
            Platform::Mac => "Mac",
        }
    }
}

// `Ord` follows declaration order (Debug < DebugGame < Development < Test <
// Shipping) - the canonical order `staged_configs()` sorts into, so the `{config}`
// token, the `-clientconfig` list, and the history tags are deterministic regardless
// of tick order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize, specta::Type)]
pub enum Configuration {
    Debug,
    DebugGame,
    #[default]
    Development,
    Test,
    Shipping,
}

impl Configuration {
    /// The `-clientconfig=` flag value (and the `{config}` token value).
    pub fn as_str(self) -> &'static str {
        match self {
            Configuration::Debug => "Debug",
            Configuration::DebugGame => "DebugGame",
            Configuration::Development => "Development",
            Configuration::Test => "Test",
            Configuration::Shipping => "Shipping",
        }
    }
}

/// A footprint reclaim category - the stable, serializable bucket a Clean-tab leaf and a
/// Clean-up phase selection belong to (`docs/build-footprint.md`). Grouped on the Clean
/// tab as **Save** (`Staged`/`Cooked`/`Shader`), **Binaries** (`BinariesGame` +
/// `BinariesPlugin`), **Intermediate** (`IntermediateGame` = the build cache, `Intermediate
/// Other` = the editor/tool/scratch rest of the main `Intermediate/`, and `IntermediatePlugin`),
/// and a standalone **Cache** (`DerivedData`, the local `DerivedDataCache/`). `IntermediateGame`
/// is editor-safe (build only); `IntermediateOther` is what forces a full editor recompile,
/// so it stays out of the auto Clean-up phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CleanupCategory {
    Staged,
    Cooked,
    Shader,
    BinariesGame,
    BinariesPlugin,
    IntermediateGame,
    IntermediateOther,
    IntermediatePlugin,
    DerivedData,
    /// The Steam upload scratch dir (`.uep/steam-build-output/`) - steamcmd's chunk
    /// cache + logs + resolved run VDFs. Regenerable; safe to wipe.
    SteamBuildOutput,
}

impl CleanupCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            CleanupCategory::Staged => "staged",
            CleanupCategory::Cooked => "cooked",
            CleanupCategory::Shader => "shader",
            CleanupCategory::BinariesGame => "binariesGame",
            CleanupCategory::BinariesPlugin => "binariesPlugin",
            CleanupCategory::IntermediateGame => "intermediateGame",
            CleanupCategory::IntermediateOther => "intermediateOther",
            CleanupCategory::IntermediatePlugin => "intermediatePlugin",
            CleanupCategory::DerivedData => "derivedData",
            CleanupCategory::SteamBuildOutput => "steamBuildOutput",
        }
    }

    /// Parse a stored camelCase token; `None` for an unknown/legacy token. The profile
    /// loader (`store::load_all`) uses this to drop stale `cleanup.categories` entries
    /// from a profile saved against an older category set, so it still loads instead of
    /// failing to deserialize.
    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "staged" => CleanupCategory::Staged,
            "cooked" => CleanupCategory::Cooked,
            "shader" => CleanupCategory::Shader,
            "binariesGame" => CleanupCategory::BinariesGame,
            "binariesPlugin" => CleanupCategory::BinariesPlugin,
            "intermediateGame" => CleanupCategory::IntermediateGame,
            "intermediateOther" => CleanupCategory::IntermediateOther,
            "intermediatePlugin" => CleanupCategory::IntermediatePlugin,
            "derivedData" => CleanupCategory::DerivedData,
            "steamBuildOutput" => CleanupCategory::SteamBuildOutput,
            _ => return None,
        })
    }
}

/// Incremental cook strategy. `None` ⇒ full cook (default, the only release-safe
/// choice). `ModifiedOnly` ⇒ legacy `-iterativecooking` (dev-only staleness
/// footgun - `docs/build-commands.md` §7). `ModifiedAndDependencies` ⇒
/// `-cookincremental` (UE 5.6+). Mutually exclusive with a Build clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum IncrementalCookMode {
    #[default]
    None,
    ModifiedOnly,
    ModifiedAndDependencies,
}

/// `All` ⇒ `-allmaps`; `List` ⇒ `-map=A+B` (project-specific, empty in a template).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum CookMaps {
    #[default]
    All,
    List(Vec<String>),
}

// ── per-phase config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CookCfg {
    /// Whether the Cook phase runs. On by default; off ⇒ reuse an existing cook
    /// (the downstream BuildCookRun keeps `-skipcook`).
    #[serde(default = "on")]
    pub enabled: bool,
    #[serde(default)]
    pub maps: CookMaps,
    /// `-cookcultures=` (empty ⇒ project cultures).
    #[serde(default)]
    pub cultures: Vec<String>,
    #[serde(default)]
    pub incremental: IncrementalCookMode,
    /// `-SkipCookingEditorContent` (smaller cooked output).
    #[serde(default)]
    pub skip_editor_content: bool,
    /// `-AdditionalCookerOptions="…"` - verbatim cooker escape hatch.
    #[serde(default)]
    pub additional_options: String,
}

impl Default for CookCfg {
    fn default() -> Self {
        CookCfg {
            enabled: true,
            maps: CookMaps::default(),
            cultures: Vec::new(),
            incremental: IncrementalCookMode::default(),
            skip_editor_content: false,
            additional_options: String::new(),
        }
    }
}

/// Build phase. On by default; off ⇒ reuse existing binaries (downstream phases
/// keep `-skipbuild`). `clean` forces a from-scratch build; `-noP4` is a default-on
/// toggle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BuildCfg {
    /// Whether the Build (compile) phase runs.
    #[serde(default = "on")]
    pub enabled: bool,
    /// `-clean` - wipe intermediates first (mutually exclusive with incremental cook).
    #[serde(default)]
    pub clean: bool,
    /// `-noP4` - disable Perforce. Default on (the common solo-dev case).
    #[serde(default = "on")]
    pub no_p4: bool,
    /// Verbatim args appended to the UBT build command.
    #[serde(default)]
    pub additional_args: String,
}

impl Default for BuildCfg {
    fn default() -> Self {
        BuildCfg {
            enabled: true,
            clean: false,
            no_p4: true,
            additional_args: String::new(),
        }
    }
}

/// Archive phase - copies the finished build to `output`. On by default. **Requires
/// Stage** (it archives the staged tree), so it is forced off when Stage is off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCfg {
    #[serde(default = "on")]
    pub enabled: bool,
    /// Verbatim args merged into the shared Stage·Pak·Archive command.
    #[serde(default)]
    pub additional_args: String,
}

impl Default for ArchiveCfg {
    fn default() -> Self {
        ArchiveCfg {
            enabled: true,
            additional_args: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StageCfg {
    /// Whether the Stage phase runs. **Gate for Pak + Archive** - both run inside
    /// the staged tree, so turning Stage off forces both off.
    #[serde(default = "on")]
    pub enabled: bool,
    /// `-prereqs` (stage the VC++ redist installer).
    #[serde(default = "on")]
    pub prereqs: bool,
    /// `-distribution` (store-ready flag; mainly console/mobile).
    #[serde(default)]
    pub for_distribution: bool,
    /// `false` ⇒ emit `-nodebuginfo` (drop `.pdb`, a footprint lever).
    #[serde(default)]
    pub debug_symbols: bool,
    /// `-separatedebuginfo` (stage symbols into a separate dir).
    #[serde(default)]
    pub separate_debug_info: bool,
    /// Verbatim args merged into the shared Stage·Pak·Archive command.
    #[serde(default)]
    pub additional_args: String,
}

impl Default for StageCfg {
    fn default() -> Self {
        StageCfg {
            enabled: true,
            prereqs: true,
            for_distribution: false,
            debug_symbols: false,
            separate_debug_info: false,
            additional_args: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PakCfg {
    /// `-pak` (off ⇒ loose files).
    #[serde(default = "on")]
    pub enabled: bool,
    /// `-iostore` (`.utoc`/`.ucas`; pulls Pak on).
    #[serde(default = "on")]
    pub io_store: bool,
    /// `-compressed` (Oodle/Kraken payload).
    #[serde(default = "on")]
    pub compressed: bool,
    /// `-package` (platform-native distributable).
    #[serde(default)]
    pub package: bool,
    /// Verbatim args merged into the shared Stage·Pak·Archive command.
    #[serde(default)]
    pub additional_args: String,
}

impl Default for PakCfg {
    fn default() -> Self {
        PakCfg {
            enabled: true,
            io_store: true,
            compressed: true,
            package: false,
            additional_args: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CopyItem {
    /// Project-relative source (file or folder).
    pub from: String,
    /// Build-output-relative destination (default the build root).
    #[serde(default = "dot")]
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CopyExtrasCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub items: Vec<CopyItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanupCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub categories: Vec<CleanupCategory>,
    /// Keep artifacts when a build fails (the default), so a failure stays
    /// debuggable.
    #[serde(default = "on")]
    pub only_on_success: bool,
}

impl Default for CleanupCfg {
    fn default() -> Self {
        CleanupCfg {
            enabled: false,
            categories: Vec::new(),
            only_on_success: true,
        }
    }
}

/// One Steam depot to upload. Mirrors the `CopyItem` shape so the editor reuses the
/// Copy-Extras list editor. The `path` is the depot's content-root-relative source
/// root (default `"."` = the whole archive tree), rendered into the depot VDF's
/// `FileMapping` `LocalPath` when the depot template is first created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DepotItem {
    /// Steam depot id (numeric string, e.g. `"481"`).
    pub depot_id: String,
    /// Content-root-relative source root (default `"."`).
    #[serde(default = "dot")]
    pub path: String,
}

/// Upload-to-Steam phase (`docs/build-commands.md` §11). Opt-in (default off) like the
/// other post-archive app phases. These are the **managed** fields written into the
/// committed `app_build.vdf` (`AppID`/`Desc`/`Preview`/`SetLive` + the depot list);
/// unknown user-added VDF keys are preserved on regeneration. `ContentRoot`/`BuildOutput`
/// are injected at run, never stored here; steamcmd path + build account are machine-local
/// (`storage::SteamLocalSettings`), not in the committed profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SteamUploadCfg {
    #[serde(default)]
    pub enabled: bool,
    /// Steam application id (numeric string).
    #[serde(default)]
    pub app_id: String,
    /// Build description (visible only in your Steamworks builds page).
    #[serde(default)]
    pub description: String,
    /// `SetLive` beta branch, or empty (upload only). Never the public `default` branch
    /// (Steam blocks that from scripts).
    #[serde(default)]
    pub branch: String,
    /// `"Preview" "1"` dry-run: validate + build the manifest without uploading. Default
    /// **off** (a normal build uploads for real); turn it on for a dry run.
    #[serde(default)]
    pub preview: bool,
    #[serde(default)]
    pub depots: Vec<DepotItem>,
}

impl Default for SteamUploadCfg {
    fn default() -> Self {
        SteamUploadCfg {
            enabled: false,
            app_id: String::new(),
            description: String::new(),
            branch: String::new(),
            preview: false,
            depots: Vec::new(),
        }
    }
}

/// Per-phase config in pipeline order. Each phase owns its `enabled` toggle plus
/// its parameters; Build/Cook/Stage/Pak/Archive each carry a verbatim
/// `additional_args` escape hatch. Archive's output destination lives in `Output`
/// (the editor's Archive island shows both).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Phases {
    #[serde(default)]
    pub build: BuildCfg,
    #[serde(default)]
    pub cook: CookCfg,
    #[serde(default)]
    pub stage: StageCfg,
    #[serde(default)]
    pub pak: PakCfg,
    #[serde(default)]
    pub archive: ArchiveCfg,
    #[serde(default)]
    pub copy_extras: CopyExtrasCfg,
    /// Upload the archived build to Steam (runs after Copy Extras). Off by default.
    #[serde(default)]
    pub steam_upload: SteamUploadCfg,
    #[serde(default)]
    pub cleanup: CleanupCfg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    /// REQUIRED on a profile (Archive is mandatory). May be empty in a template.
    pub base_dir: String,
    #[serde(default = "default_folder")]
    pub folder_template: String,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            base_dir: String::new(),
            folder_template: default_folder(),
        }
    }
}

/// Backs **both** a template and a profile (`docs/data-storage.md`). A template
/// leaves `target` / `cook.maps` / `copy_extras.items` empty; a profile fills
/// them. Self-contained: editing a template later never mutates existing profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BuildConfig {
    pub schema_version: u32,
    /// Stable id (file stem on disk).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub platform: Platform,
    /// Client configs to build AND stage into the one package (e.g. Development +
    /// Shipping): one cook, one exe per config. Non-empty (enforced by
    /// `validate_profile`). Every config becomes its own history tag; the `{config}`
    /// token joins them with `+`; the cook uses just the first (it is config-agnostic).
    /// `staged_configs()` returns them deduped and in canonical order.
    #[serde(default = "default_configs")]
    pub configs: Vec<Configuration>,
    /// Project-specific (`None` in templates).
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub phases: Phases,
    #[serde(default)]
    pub output: Output,
    /// Provenance only (which template a profile was created from); no live link.
    #[serde(default)]
    pub based_on_template: Option<String>,
    /// `true` only for the fixed built-in templates (Development, Shipping, …) -
    /// cannot be deleted or edited; the clone bases. Never set on profiles or
    /// user templates (the copy-on-create transforms reset it).
    #[serde(default)]
    pub builtin: bool,
}

impl Default for BuildConfig {
    fn default() -> Self {
        BuildConfig {
            schema_version: SCHEMA_VERSION,
            id: String::new(),
            name: String::new(),
            platform: Platform::default(),
            configs: default_configs(),
            target: None,
            phases: Phases::default(),
            output: Output::default(),
            based_on_template: None,
            builtin: false,
        }
    }
}

impl BuildConfig {
    /// The client configs to build and stage: deduped, in canonical order, and
    /// guaranteed non-empty (falls back to Development if somehow empty). Drives the
    /// Build-phase loop (one compile each), the Stage `-clientconfig=A+B` list, the
    /// `{config}` token (joined with `+`), and the per-config history tags. The cook
    /// uses only the first entry (cook output is config-agnostic).
    pub fn staged_configs(&self) -> Vec<Configuration> {
        let mut out = self.configs.clone();
        out.sort();
        out.dedup();
        if out.is_empty() {
            out.push(Configuration::Development);
        }
        out
    }

    /// The staged configs joined with `+` (e.g. `Development+Shipping`) - the single source
    /// of order for the Stage `-clientconfig=A+B` list and the `{config}` output-folder token,
    /// so the folder name can never drift from the client-config list. Built from
    /// [`staged_configs`](Self::staged_configs) (already deduped and canonically ordered).
    pub fn staged_config_join(&self) -> String {
        self.staged_configs().iter().map(|c| c.as_str()).collect::<Vec<_>>().join("+")
    }

    /// Lightweight validation for **saving a profile** (`docs/data-storage.md` -
    /// "authoritative validation runs in `profiles/`"). Returns every problem so
    /// the editor can show them at once. Templates skip the output check (a
    /// template's base dir may be empty until cloned into a profile).
    pub fn validate_profile(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.id.trim().is_empty() {
            errs.push("profile id is empty".into());
        }
        if self.name.trim().is_empty() {
            errs.push("profile name is empty".into());
        }
        if self.configs.is_empty() {
            errs.push("at least one build configuration must be selected".into());
        }
        if self.output.base_dir.trim().is_empty() {
            errs.push("output base directory is required".into());
        }
        if self.output.folder_template.trim().is_empty() {
            errs.push("output folder template is empty".into());
        }
        if self.phases.copy_extras.enabled {
            for (i, item) in self.phases.copy_extras.items.iter().enumerate() {
                if item.from.trim().is_empty() {
                    errs.push(format!("copy-extras item {} has an empty source", i + 1));
                }
            }
        }
        if self.phases.cleanup.enabled && self.phases.cleanup.categories.is_empty() {
            errs.push("clean-up is enabled but no categories are selected".into());
        }
        if self.phases.steam_upload.enabled {
            let steam = &self.phases.steam_upload;
            // Steam upload pushes the archived build, so it requires Archive (registry
            // gated_by = [Archive]); Archive itself only runs inside the Stage·Pak·Archive unit.
            if !(self.phases.stage.enabled && self.phases.archive.enabled) {
                errs.push(
                    "Steam upload requires Archive - enable the Archive phase (it needs Stage), or turn Steam upload off".into(),
                );
            }
            if steam.app_id.trim().is_empty() {
                errs.push("Steam upload is enabled but the App ID is empty".into());
            }
            if steam.depots.is_empty() {
                errs.push("Steam upload is enabled but no depots are configured".into());
            }
            for (i, d) in steam.depots.iter().enumerate() {
                if d.depot_id.trim().is_empty() {
                    errs.push(format!("Steam depot {} has an empty depot id", i + 1));
                }
            }
            // The public `default` branch can't be pushed live from a script (Steam blocks it).
            if steam.branch.trim().eq_ignore_ascii_case("default") {
                errs.push(
                    "Steam \"set live\" branch can't be \"default\" - Steam blocks the public branch from scripts; use a beta branch or leave it empty".into(),
                );
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }
}

// ── serde default helpers ────────────────────────────────────────────────────
fn on() -> bool {
    true
}
fn dot() -> String {
    ".".into()
}
fn default_folder() -> String {
    "{project}-{platform}-{config}-{date}".into()
}
fn default_configs() -> Vec<Configuration> {
    vec![Configuration::Development]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_doc() {
        let p = BuildConfig::default();
        assert_eq!(p.schema_version, SCHEMA_VERSION);
        assert_eq!(p.platform, Platform::Win64);
        assert_eq!(p.configs, vec![Configuration::Development]);
        // every build phase enabled by default; the two app phases off by default
        assert!(p.phases.build.enabled && p.phases.cook.enabled && p.phases.stage.enabled);
        assert!(p.phases.pak.enabled && p.phases.archive.enabled);
        assert!(!p.phases.copy_extras.enabled && !p.phases.cleanup.enabled);
        // Steam upload is opt-in (off), and its dry-run Preview defaults off (real upload).
        assert!(!p.phases.steam_upload.enabled && !p.phases.steam_upload.preview);
        // free-phase / per-phase defaults
        assert!(p.phases.build.no_p4 && !p.phases.build.clean);
        assert_eq!(p.phases.cook.incremental, IncrementalCookMode::None);
        assert!(p.phases.pak.io_store && p.phases.pak.compressed && !p.phases.pak.package);
        assert!(p.phases.stage.prereqs && !p.phases.stage.debug_symbols);
        assert!(!p.phases.stage.for_distribution && !p.phases.stage.separate_debug_info);
        assert!(p.phases.cleanup.only_on_success);
        assert_eq!(p.output.folder_template, "{project}-{platform}-{config}-{date}");
    }

    #[test]
    fn platform_has_distinct_flag_and_folder_forms() {
        assert_eq!(Platform::Win64.uat(), "Win64");
        assert_eq!(Platform::Win64.folder(), "Windows"); // the {platform} token form
    }

    #[test]
    fn json_round_trips_with_camelcase_keys() {
        let mut p = BuildConfig::default();
        p.id = "dev".into();
        p.name = "Development".into();
        p.target = Some("SampleProjectSteam".into());
        p.output.base_dir = "C:/Builds".into();
        p.phases.cook.maps = CookMaps::List(vec!["Entry".into(), "Arena".into()]);
        p.configs = vec![Configuration::Development, Configuration::Shipping];

        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"schemaVersion\""), "keys must be camelCase");
        assert!(json.contains("\"copyExtras\""));
        assert!(json.contains("\"configs\""));
        let back: BuildConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn serde_defaults_fill_a_minimal_json() {
        // Only the non-defaulted fields present - everything else defaults in.
        let json = r#"{ "schemaVersion": 1, "id": "x", "name": "X", "output": { "baseDir": "C:/Out" } }"#;
        let p: BuildConfig = serde_json::from_str(json).unwrap();
        assert_eq!(p.platform, Platform::Win64);
        assert!(p.phases.pak.enabled);
        assert!(p.phases.cleanup.only_on_success);
        assert_eq!(p.output.folder_template, "{project}-{platform}-{config}-{date}");
        // A minimal JSON with no `configs` key defaults to [Development].
        assert_eq!(p.configs, vec![Configuration::Development]);
    }

    #[test]
    fn staged_configs_are_canonical_ordered_and_deduped() {
        let mut p = BuildConfig::default(); // configs = [Development]
        assert_eq!(p.staged_configs(), vec![Configuration::Development]);

        // Canonical (enum) order regardless of stored order: DebugGame < Shipping.
        p.configs = vec![Configuration::Shipping, Configuration::DebugGame];
        assert_eq!(
            p.staged_configs(),
            vec![Configuration::DebugGame, Configuration::Shipping]
        );

        // Duplicates collapse.
        p.configs = vec![
            Configuration::Development,
            Configuration::Shipping,
            Configuration::Shipping,
        ];
        assert_eq!(
            p.staged_configs(),
            vec![Configuration::Development, Configuration::Shipping]
        );

        // Empty falls back to Development (defensive; validate_profile rejects empty on save).
        p.configs = vec![];
        assert_eq!(p.staged_configs(), vec![Configuration::Development]);
    }

    #[test]
    fn validate_profile_requires_at_least_one_config() {
        let mut p = BuildConfig::default();
        p.id = "p".into();
        p.name = "P".into();
        p.output.base_dir = "C:/Out".into();
        p.configs = vec![];
        let errs = p.validate_profile().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("configuration")));
    }

    #[test]
    fn validate_profile_flags_missing_required_fields() {
        let blank = BuildConfig::default(); // empty id/name/baseDir
        let errs = blank.validate_profile().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("id")));
        assert!(errs.iter().any(|e| e.contains("name")));
        assert!(errs.iter().any(|e| e.contains("base directory")));

        let mut ok = BuildConfig::default();
        ok.id = "p".into();
        ok.name = "P".into();
        ok.output.base_dir = "C:/Out".into();
        assert!(ok.validate_profile().is_ok());
    }

    #[test]
    fn validate_profile_catches_enabled_but_empty_phases() {
        let mut p = BuildConfig::default();
        p.id = "p".into();
        p.name = "P".into();
        p.output.base_dir = "C:/Out".into();
        p.phases.cleanup.enabled = true; // no categories
        p.phases.copy_extras.enabled = true;
        p.phases.copy_extras.items = vec![CopyItem { from: "  ".into(), to: ".".into() }];
        let errs = p.validate_profile().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("categories")));
        assert!(errs.iter().any(|e| e.contains("empty source")));
    }

    #[test]
    fn validate_profile_rejects_default_steam_branch() {
        let mut p = BuildConfig::default();
        p.id = "p".into();
        p.name = "P".into();
        p.output.base_dir = "C:/Out".into();
        p.phases.steam_upload.enabled = true;
        p.phases.steam_upload.app_id = "480".into();
        p.phases.steam_upload.depots = vec![DepotItem { depot_id: "481".into(), path: ".".into() }];
        p.phases.steam_upload.branch = "Default".into(); // case-insensitive
        let errs = p.validate_profile().unwrap_err();
        assert!(errs.iter().any(|e| e.to_lowercase().contains("default")));
        // a beta branch (or empty) is fine
        p.phases.steam_upload.branch = "beta".into();
        assert!(p.validate_profile().is_ok());
    }

    #[test]
    fn validate_profile_rejects_steam_upload_without_archive() {
        let mut p = BuildConfig::default();
        p.id = "p".into();
        p.name = "P".into();
        p.output.base_dir = "C:/Out".into();
        p.phases.steam_upload.enabled = true;
        p.phases.steam_upload.app_id = "480".into();
        p.phases.steam_upload.depots = vec![DepotItem { depot_id: "481".into(), path: ".".into() }];
        p.phases.steam_upload.branch = "beta".into();
        // Archive on (default) - Steam upload is allowed.
        assert!(p.validate_profile().is_ok());
        // Turn Archive off: Steam upload now has nothing fresh to push, so it's rejected.
        p.phases.archive.enabled = false;
        let errs = p.validate_profile().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("Steam upload requires Archive")));
    }
}
