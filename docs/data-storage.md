# Data Storage

> Main doc: [../CLAUDE.md](../CLAUDE.md) · Requirements: [requirement.md](requirement.md)

How the app stores everything: where it lives, the format, and why.

## Principles
- **Two stores:** *global* (the **OS per-user app-data dir**, resolved by Tauri's `path` API) and *per-project* (in the project's `.uep/` folder).
- **Build settings split in two:** global **templates** (project-agnostic bases, reusable across projects) and project-local **profiles** (the concrete per-project config). Both are **copy-on-create only - never blank**: a profile clones a template (or another profile); a user template clones an existing template (a fixed built-in, or another). No live inheritance, so editing a template or the source never mutates existing copies.
- **JSON = source of truth + flexible/low-volume config.** **SQLite = a derived, queryable index** (normalized) for things the Dashboard/history browse at scale.
- **Build records are lean & self-contained:** timing (total + per-phase), size, output location, and flat tags - never reading back from a profile. Stored *per-project*, so there's no global stale data, they survive `Saved/` cleaning, and they travel with the project.
- **Logic is Rust, storage is plain:** `std::fs` for JSON + `rusqlite` for the index (or `tauri-plugin-sql`); Tauri resolves the app-data dir (`path` API → `app_data_dir()`) and the modules take it as input.
- **Project contents aren't cached; the engine path is:** targets / project type / maps / plugins are re-detected **fresh on every project open** (cheap, avoids stale results) - kept in memory, never written to disk. The **engine path** is the deliberate exception - a user-locatable, slow-to-rediscover setting **persisted per project** (in that project's `.uep/local.json`, machine-local & git-ignored) and **re-validated on every use** (descriptor auto-resolution is only the fallback when no valid saved path exists). It's per-project, **not** keyed by `EngineAssociation`, so two projects that share an association can point at different engine builds.

## Locations

### Global - OS app-data dir (`app_data_dir()`)
Stored in the per-user app-data dir Tauri resolves from the bundle `identifier` (`com.unrealeasypackage.app`): on Windows `%APPDATA%\com.unrealeasypackage.app\` (Roaming), with the platform-equivalent elsewhere. Always writable per-user, and in `tauri dev` there's no bundled `.exe` to sit next to - so we use this rather than a portable folder beside the executable.

| File | Contents | Req |
|---|---|---|
| `settings.json` | theme, notification preference (app version is read from the package, not stored) | R6 |
| `recent-projects.json` | recents (projects **and** plugins): `path` (a `.uproject` or `.uplugin`), `name`, `kind` (`project`/`plugin`), `lastOpened`; validity re-checked on display (not stored); invalid entries are kept + flagged (fix / open-folder / remove via the row menu), removed only on explicit Remove. Reads the legacy `uprojectPath` key as an alias + defaults a missing `kind` to `project`, so pre-plugin recents still load (R2, R7) | R2 |
| `templates/<id>.json` | project-agnostic build **templates** (platform, config, per-phase config, output template); a set of fixed built-ins (e.g. *Development*, *Shipping*; `builtin: true`, undeletable + read-only) are **self-healed** on launch; user templates are saved from a profile (*Make this a template*) | R1 |

*(Concrete profiles are project-local; only the reusable templates live globally.)*

### Per-project - `<Project>\.uep\`
```
.uep/
  profiles/
    <profileId>.json          # build profiles - COMMITTED (shared with team)
  history/
    <buildId>/                # one self-contained folder per build
      metadata.json           #   the build record (source of truth)
      build.log               #   our own copy of the log (separate from Saved/Logs)
  cache/
    history.db                # SQLite index over all metadata.json (derived, disposable)
  local.json                  # machine-local overrides (engine-path) - git-IGNORED
  .gitignore                  # nested ignore (below) - git-IGNORED (ignores itself)
```

**Nested `.uep/.gitignore`** (NOT committed - it ignores itself, so only `profiles/`
ever lands in git). The app regenerates it locally on open and self-heals an existing
one - appending any required rule it's missing (e.g. `.gitignore`, `local.json`) while
leaving user edits intact. Git still honors an ignored `.gitignore`, so the rules apply
even though the file is never tracked:
```
.gitignore
history/
cache/
local.json
```

### Per-plugin - `<PluginRoot>\.uap\settings.json` (R7)
When a **plugin** is opened (a `.uplugin`, not a `.uproject`), everything the Actions tab remembers lives in **one plain JSON beside the `.uplugin`**, in its own `.uap/` folder:
```
.uap/
  settings.json   # { engines: [paths], outputDir: "…", folderName: "…" }
  .gitignore      # contents: `*`  → the whole .uap/ folder is invisible to git
```
- `engines` - engine roots the user browsed for; a stale one (no longer a valid engine) is pruned on the next list.
- `outputDir` / `folderName` - the last-used package output base dir + folder-name template, recalled on re-open (saved on change and on package).

Deliberately its **own** `.uap/` (separate from the engine-side `.uep/` convention) and under the **plugin** folder, **not** the host project root. The scaffolded `.uap/.gitignore` is just `*`, which ignores every file in the folder - including itself - so the whole thing is **machine-local and never committed** with a distributed plugin.

## JSON vs SQLite - who owns what
| Data | Store | Format | Role |
|---|---|---|---|
| App settings, recent projects | app folder | JSON | small, flexible |
| Build **templates** | app folder `templates/` | JSON (1 file/template) | reusable project-agnostic bases |
| Build **profiles** | `.uep/profiles/` | JSON (1 file/profile) | flexible config, git-friendly diffs |
| Build record + log | `.uep/history/<buildId>/` | JSON + text | **source of truth**, self-contained |
| Build history index | `.uep/cache/history.db` | SQLite | **derived** - fast filter/sort/aggregate |

## Build settings: templates (global) vs profiles (local)
- **Template** *(global, `templates/<id>.json`)* - only project-agnostic fields: platform, config, **per-phase toggles + per-phase config** (cook/stage/pak options, Clean-up categories), the output **folder-name template**, and the output base dir **only when project-relative** (an absolute base dir is machine-specific, so it's stripped). A set of fixed built-ins (e.g. *Development*, *Shipping*; `builtin: true`, undeletable + read-only, self-healed on launch) are the seed clone bases; users add templates by **saving a profile as a template** (*Make this a template*), then rename/edit/delete those copies. No blank template.
- **Profile** *(local, `.uep/profiles/<id>.json`, committed)* - a concrete build config for *this* project: the base fields (copied from the chosen template or **another profile (clone)** - never blank) **plus** the project-specific bits - `target`, `maps`, and **Copy Extras mappings** (project-relative paths). Stable `id`, `schemaVersion`, `name`, optional `basedOnTemplate` (provenance only). Self-contained: changing the template later does not alter existing profiles.

## Profile / template schema (serde)
One `BuildConfig` struct backs **both** - a *template* leaves the project-specific fields empty; a *profile* fills them (copy-on-create). `#[serde(rename_all = "camelCase")]` keeps the on-disk JSON keys unchanged; `tauri-specta` emits the matching TS types for the editor (one source of truth). **Every phase carries an `enabled` flag - all phases are toggleable, on by default.** The one dependency: **Pak and Archive run inside the staged tree, so they require Stage** and are forced off when Stage is off (declared as registry data - `pipeline`'s `gated_by` - from which the editor derives Pak/Archive's locked-off state; not trusted from the JSON). The [phase registry](build-commands.md#8-phase-decomposition-separate-processes-parallelism-and-timing) pairs each phase with its command/action builder; authoritative validation runs in `profiles/` (serde + a hand-rolled `BuildConfig::validate_profile`). Tags (platform/config/target/status) are **derived**, never stored here.

```rust
// src-tauri/src/profiles/schema.rs
// #[serde(rename_all = "camelCase")] on each struct → on-disk JSON keys are unchanged.
// tauri-specta emits the matching TS types for the editor (one source of truth).
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub enum Platform { #[default] Win64, Linux, Mac }            // MVP: Win64; others reserved

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub enum Configuration { Debug, DebugGame, #[default] Development, Test, Shipping }

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]   // Save: staged/cooked/shader · Binaries split game/plugin · Intermediate split game/other/plugin · derivedData
pub enum CleanupCategory { Staged, Cooked, Shader, BinariesGame, BinariesPlugin, IntermediateGame, IntermediateOther, IntermediatePlugin, DerivedData }
// IntermediateOther = the whole-Intermediate wipe (tab-only; never offered by the auto Clean-up phase).
// A profile's stored cleanup.categories deserialize tolerantly - unknown/legacy tokens are dropped, not errored.

#[derive(Serialize, Deserialize, Default)]
pub enum CookMaps { #[default] All, List(Vec<String>) }      // All ⇒ -allmaps; List ⇒ -map=A+B (project-specific)

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]                          // wire: none | modifiedOnly | modifiedAndDependencies
pub enum IncrementalCookMode { #[default] None, ModifiedOnly, ModifiedAndDependencies }
// None ⇒ full cook · ModifiedOnly ⇒ -iterativecooking (UE 5.5+) · ModifiedAndDependencies ⇒ -cookincremental (UE 5.6+)

// EVERY phase carries `enabled` (toggleable, default on). The only dependency:
// Pak + Archive run inside the staged tree, so they require Stage (declared via the
// registry's `gated_by`; the editor derives the locked-off state, not the JSON).
// Build/Cook/Stage/Pak/Archive each
// also carry a verbatim `additionalArgs` escape hatch (Archive writes to `output`).
// (`on` default needs a manual `Default` impl in Rust; shown here illustratively.)

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildCfg {
    #[serde(default = "on")] pub enabled:         bool,      // off ⇒ no Build unit (downstream keeps -skipbuild)
    #[serde(default)]        pub clean:           bool,      // true ⇒ -clean (excl. with cook.incremental)
    #[serde(default = "on")] pub no_p4:           bool,      // -noP4 (default on; on the BuildCookRun calls)
    #[serde(default)]        pub additional_args: String,    // appended to the UBT command
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCfg {                                      // requires Stage (writes the build to `output`)
    #[serde(default = "on")] pub enabled:         bool,      // off ⇒ stage unit omits -archive
    #[serde(default)]        pub additional_args: String,    // merged into the shared BuildCookRun line
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookCfg {                                         // cook mode is always By-the-book in MVP - not stored
    #[serde(default = "on")] pub enabled:              bool,            // off ⇒ no Cook unit (downstream keeps -skipcook)
    #[serde(default)]        pub maps:                 CookMaps,
    #[serde(default)]        pub cultures:             Vec<String>,        // -cookcultures=
    #[serde(default)]        pub incremental:          IncrementalCookMode, // None / iterative / incremental (excl. with build.clean)
    #[serde(default)]        pub skip_editor_content:  bool,               // true ⇒ -SkipCookingEditorContent
    #[serde(default)]        pub additional_options:   String,             // -AdditionalCookerOptions (escape hatch)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageCfg {                                        // gate for Pak + Archive (both run inside the staged tree)
    #[serde(default = "on")] pub enabled:             bool,  // off ⇒ no Stage·Pak·Archive unit at all
    #[serde(default = "on")] pub prereqs:             bool,  // -prereqs
    #[serde(default)]        pub for_distribution:    bool,  // -distribution
    #[serde(default)]        pub debug_symbols:       bool,  // false ⇒ emit -nodebuginfo
    #[serde(default)]        pub separate_debug_info: bool,  // -separatedebuginfo
    #[serde(default)]        pub additional_args:     String, // merged into the shared BuildCookRun line
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PakCfg {                                          // free toggle
    #[serde(default = "on")] pub enabled:         bool,      // → -pak
    #[serde(default = "on")] pub io_store:        bool,      // -iostore
    #[serde(default = "on")] pub compressed:      bool,      // -compressed
    #[serde(default)]        pub package:         bool,      // -package (native package)
    #[serde(default)]        pub additional_args: String,    // merged into the shared BuildCookRun line
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyItem { pub from: String, #[serde(default = "dot")] pub to: String }

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CopyExtrasCfg {                                   // free; project-specific
    #[serde(default)] pub enabled: bool,
    #[serde(default)] pub items:   Vec<CopyItem>,            // from (proj-rel) → to (build-rel)
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CleanupCfg {                                      // free
    #[serde(default)]        pub enabled:         bool,
    #[serde(default)]        pub categories:      Vec<CleanupCategory>,
    #[serde(default = "on")] pub only_on_success: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Phases {                                          // pipeline order; Archive writes to `output`
    pub build:       BuildCfg,
    pub cook:        CookCfg,
    pub stage:       StageCfg,
    pub pak:         PakCfg,
    pub archive:     ArchiveCfg,
    pub copy_extras: CopyExtrasCfg,
    pub cleanup:     CleanupCfg,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub base_dir: String,                                   // REQUIRED (Archive is mandatory)
    #[serde(default = "default_folder")] pub folder_template: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildConfig {                                    // backs BOTH template & profile
    pub schema_version: u32,                                // = 1
    pub id:             String,                             // stable
    pub name:           String,
    #[serde(default)] pub platform: Platform,
    #[serde(default)] pub config:   Configuration,
    pub target:         Option<String>,                    // project-specific (None in templates)
    pub phases:         Phases,
    pub output:         Output,
    #[serde(default)] pub based_on_template: Option<String>, // provenance only (profiles)
    #[serde(default)] pub builtin:           bool,         // fixed built-in template (undeletable/read-only); never set on profiles or user templates
}
// Template = BuildConfig with project-specific fields empty; Profile = same struct, filled.
// The built-ins (Development, Shipping, …) have builtin: true; copy-on-create always resets it to false.
// helpers: fn on() -> bool { true }   fn dot() -> String { ".".into() }
//          fn default_folder() -> String { "{project}-{platform}-{config}-{date}".into() }
```

The only template↔profile difference is the **project-specific fields** (`target`, `phases.cook.maps`, `phases.copyExtras.items`) - empty in a template, filled in a profile - so cloning a template ⇒ profile is a structural copy. The reverse - *Make this a template* - **genericizes**: a template must hold **no project- or machine-specific data**, so `target` and `phases.cook.maps` are cleared, while `phases.copyExtras.items` and `output.baseDir` are kept **only when project-relative** - absolute paths are stripped (base dir emptied, absolute Copy Extras sources dropped). Rule of thumb: **relative is portable (kept), absolute is machine-specific (stripped)**; any new template-eligible field follows it. The arg builder ([build-commands.md §6](build-commands.md#6-arg-builder-rules-for-the-unreal-module)) turns this into the per-phase commands/actions and enforces the dependency/lock rules in code, not from the JSON. Each external phase's `additionalArgs` is appended to *its* command; because **Stage / Pak / Archive run as one `BuildCookRun`**, their three strings are concatenated into that single line (Stage→Pak→Archive order).

## Build records (JSON - source of truth)
`.uep/history/<buildId>/metadata.json` is deliberately lean. No profile snapshot, no resolved command, no per-build footprint size - its categorical identity is just a flat tag list:

```jsonc
{
  "schemaVersion": 1,
  "buildId": "20260603-143501-a1b2",          // sortable: YYYYMMDD-HHMMSS-<rand>
  "startedAtMs": 1748960101000,                // build start, epoch milliseconds (f64; specta forbids u64 over IPC)
  "duration": 1042,                            // wall-clock total seconds (may be < Σ phases when parallel)
  "buildSize": 4831838208,                     // bytes
  "warningCount": 12,                          // warn/error lines tallied by the classifier (real child output only)
  "errorCount": 0,
  "outputPath": "C:\\Builds\\sampleproject-windows-development-20260603",
  "outputMtimeMs": 1748961080000,              // epoch ms - basis for the modified-date integrity check
  "phases": [                                  // one entry per executed unit (offset+duration → trends & overlap; no end stamp)
    { "phase": "Build",                 "startOffset": 0,   "duration": 420, "status": "Success" },
    { "phase": "Cook",                  "startOffset": 75,  "duration": 540, "status": "Success" }, // overlaps Build (ran in parallel)
    { "phase": "Stage · Pak · Archive", "startOffset": 615, "duration": 364, "status": "Success" }, // Stage+Pak+Archive run as ONE BuildCookRun unit → one record
    { "phase": "CopyExtras",            "startOffset": 979, "duration": 3,   "status": "Success" },
    { "phase": "Cleanup",               "startOffset": 982, "duration": 60,  "status": "Success" }
  ],
  "tags": ["Win64", "Development", "SampleProjectSteam", "Success"]
}
```
The log lives next to this file as `build.log` (path derived from `buildId` - not stored), a copy independent of Unreal's `Saved/Logs` so footprint cleanup can't lose it.

### When data is written (build lifecycle)
- **During the build:** *nothing* is written to `history/` and no DB row exists yet - the record (duration, final size, status) and the log lines are accumulated **in the in-memory run snapshot** (`runner::exec`), tailed live by the UI via `uep://run-log` events.
- **On finish / fail / cancel:** create `.uep/history/<buildId>/`, write `metadata.json`, write the accumulated log as `build.log`, and upsert the SQLite row - one finalize step, so `history/` only ever holds *completed* builds (keeping folder-count == row-count valid).
- **On crash/kill mid-build:** nothing is persisted to `history/` or `cache/` (the in-memory snapshot is simply lost), so a crashed run leaves no partial record to clean up.

### Tags are the canonical descriptor
A build keeps no profile snapshot - tags carry its categorical identity, and generation/reversal share **one vocabulary** so they round-trip losslessly:
- **Generate** (build → tags): platform (`Win64`/`Linux`/…), client config (`Development`/`Shipping`/…), target (`SampleProjectSteam`), status (`Success`/`Failed`/`Cancelled`).
- **Reverse** (tags → partial profile): match each tag against the known platform/config/status vocabularies; the leftover is the target. "What platform was this build?" = the tag that's a known platform.

Both directions live in **one Rust module** (`history/`) so they never drift. (Tradeoff: two profiles with identical tags are indistinguishable in history - acceptable; add a tag if finer distinction is ever needed.)

## History index (SQLite - derived, normalized)
A small normalized schema so tag filtering and Dashboard aggregation are clean joins:

```sql
-- scalar facts, one row per build
CREATE TABLE builds (
  build_id     TEXT PRIMARY KEY,
  started_at   TEXT NOT NULL,        -- ISO start time (sortable)
  duration     INTEGER,              -- seconds
  build_size   INTEGER,              -- bytes
  output_path  TEXT,                 -- for "Open location"
  output_mtime TEXT                  -- modified-date integrity basis
);

-- distinct flat tag values
CREATE TABLE tags (
  tag_id INTEGER PRIMARY KEY,
  value  TEXT NOT NULL UNIQUE        -- e.g. 'Win64', 'Development', 'Success'
);

-- many-to-many
CREATE TABLE build_tags (
  build_id TEXT    NOT NULL REFERENCES builds(build_id) ON DELETE CASCADE,
  tag_id   INTEGER NOT NULL REFERENCES tags(tag_id),
  PRIMARY KEY (build_id, tag_id)
);
CREATE INDEX ix_build_tags_tag ON build_tags(tag_id);
CREATE INDEX ix_builds_started ON builds(started_at);
```

(Log path and profile id/name aren't columns - the log is derived from `build_id`, and profile identity is recovered from tags.) Examples:

```sql
-- Build list: Win64 + Success, newest first
SELECT b.* FROM builds b
JOIN build_tags bt ON bt.build_id = b.build_id
JOIN tags t        ON t.tag_id    = bt.tag_id
WHERE t.value IN ('Win64','Success')
GROUP BY b.build_id
HAVING COUNT(*) = 2            -- has both tags
ORDER BY b.started_at DESC;

-- Dashboard: size & duration trend for one tag dimension
SELECT b.started_at, b.build_size, b.duration
FROM builds b
JOIN build_tags bt ON bt.build_id = b.build_id
JOIN tags t        ON t.tag_id    = bt.tag_id
WHERE t.value = 'Development'
ORDER BY b.started_at;
```

### Per-phase timing (`build_phases`)
Per-phase durations (written by the [R4 runner](requirement.md#r4--build-process--logs), graphed by [R5](requirement.md#r5--build-history--metrics)) live in a small normalized child table so the Dashboard can trend a single phase or stack a build's phases - derived from `metadata.json.phases`, rebuilt with the rest of the index. Phases are the same seven the [profile verbs](build-commands.md#5-what-to-expose-vs-auto-manage-vs-hide) expose; the design rationale (what's parallelizable, why offset+duration) is in [build-commands.md](build-commands.md#8-phase-decomposition-separate-processes-parallelism-and-timing). No stored `ordinal`: the canonical pipeline order is the `pipeline` module's **phase registry** `order` (Build→Cook→Stage→Pak→Archive→CopyExtras→Cleanup, extensible), so order is derivable from `phase` and the timeline sorts on `start_offset` - a stored column would be redundant.

```sql
CREATE TABLE build_phases (
  build_id     TEXT    NOT NULL REFERENCES builds(build_id) ON DELETE CASCADE,
  phase        TEXT    NOT NULL,      -- registry phase id: Build|Cook|Stage|Pak|Archive|CopyExtras|Cleanup (extensible)
  start_offset INTEGER NOT NULL,      -- seconds from build start (overlap / Gantt; also the timeline sort)
  duration     INTEGER NOT NULL,      -- seconds (Σ phases ≠ builds.duration when parallel)
  status       TEXT    NOT NULL,      -- 'Success' | 'Failed' | 'Skipped' | 'Cancelled'
  PRIMARY KEY (build_id, phase)
);
CREATE INDEX ix_build_phases_phase ON build_phases(phase);
```

```sql
-- Dashboard: one build's phase breakdown (stacked bar / Gantt)
SELECT phase, start_offset, duration, status
FROM build_phases WHERE build_id = ? ORDER BY start_offset;

-- Trend: Cook time across builds
SELECT b.started_at, p.duration
FROM build_phases p JOIN builds b ON b.build_id = p.build_id
WHERE p.phase = 'Cook' ORDER BY b.started_at;
```

The index is **fully rebuildable from the `metadata.json` files**, so it can be deleted/regenerated at any time.

**As built** (`src-tauri/src/history/index.rs`): the columns mirror the record's `f64`/`u32` (`REAL`/`INTEGER`, not ISO-text); the `builds` table also carries `schema_version`, `warning_count`, and `error_count` (the latter two added at `PRAGMA user_version = 2`), and `build_phases` keeps `kind`/`command` + an `ord` so a row round-trips a `BuildRecord` losslessly. The Build tab queries pages via the `list_history_page(offset, limit, filter)` command (SQL `LIMIT`/`OFFSET` + a tag-membership filter); the Dashboard still aggregates client-side over the file-based `list_history`.

## Keeping the index in sync
- **Normal operation:** when a build finishes, the app writes the build folder **and** upserts its row (`runner::exec::write_history`); a `delete_history` removes the row - so counts stay in sync.
- **Light check on each history read (covers boot)** *(cheap)*: `open_synced` compares the **count of `history/` subfolders** with `SELECT COUNT(*) FROM builds`. One `readdir` length + one count query - no per-file reads, no integrity checks.
- **Missing or stale-schema DB** (deleted, first run, or `PRAGMA user_version` mismatch): nothing to compare against, so the app **rebuilds it automatically** from the `metadata.json` files. Deleting `history.db` is safe and is effectively the manual *Rebuild index*.
- **On drift** (DB present but folder count ≠ row count): `open_synced` **reconciles automatically** - reindex from the `metadata.json` files, **no prompt**. *(As built this is automatic, not the prompt-on-confirm flow originally sketched here, and there's no separate "Rebuild index" button - deleting `history.db` forces a full rebuild. Count alone can miss a simultaneous add+remove that nets to zero; deleting the `.db` covers that rare case.)*

## Schema & migration
- JSON carries `schemaVersion`; SQLite uses `PRAGMA user_version`.
- Because the index is derived, migrations are low-risk: on a version bump, rebuild `history.db` from the JSON source of truth rather than migrating in place.
