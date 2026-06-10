# Build Footprint & Cleanup

> Main doc: [../CLAUDE.md](../CLAUDE.md) · Requirements: [requirement.md](requirement.md)

This explains **how an Unreal build scatters data across your project**, **what's safe vs unsafe to delete**, and the **internet findings** that back it up. It's the reference behind the tool's footprint-management feature.

## TL;DR
A single *Development* package of `SampleProject` (UE 5.5) left **~38 GB of regenerable build artifacts** to produce a **~4.8 GB** shippable package. Almost all of that 38 GB is safe to delete and is **regenerated automatically** on the next build/cook - the problem is it's spread across many folders (and even outside the project), so it's hard to find and clean by hand.

## How a build scatters your data
`RunUAT BuildCookRun` is a pipeline. Each step writes its output to a **different folder**, so one build leaves artifacts in many places:

```
            COMPILE                 COOK                STAGE               ARCHIVE
 Source ──► Binaries/      ──► Saved/Cooked/   ──► Saved/StagedBuilds/ ──► <archive dir>
 (.cpp,     Intermediate/       <Platform>/         <Platform>/             (your output)
  .uasset)  + the same per
            plugin
```

Two things blow up size:
1. **Compile artifacts are mirrored per module.** The project gets `Binaries/` + `Intermediate/`, and **every enabled plugin** gets its *own* `Binaries/` + `Intermediate/`. `Intermediate/` (object files, generated headers, shader-compile-worker output) is usually the single biggest folder.
2. **The "4 copies" problem.** Your game data is duplicated as it moves down the pipeline - original `Content/` → cooked copy in `Saved/Cooked/` → staged copy in `Saved/StagedBuilds/` → archived copy in your output dir. That's up to **4 copies** of essentially the same assets, in 4 locations.

In `SampleProject` the example script archives *into* `Build/`, so the final copy lands there - a 5th place to remember.

## What it looked like in SampleProject
Measured after one Development build (sizes illustrative):

| Folder | Size | Step that created it | Safe to delete? |
|---|---|---|---|
| `Intermediate/` | **21 GB** | compile (`-build`) | ✅ yes |
| `Plugins/*/{Binaries,Intermediate}` (29 dirs) | 4.2 GB | compile (`-build`) | ✅ yes |
| `Build/` (archive output here) | 4.8 GB | archive (`-archive`) | ⚠️ output only |
| `Saved/` total | 4.8 GB | cook/stage/runtime | ✅ mostly |
| ↳ `Saved/StagedBuilds/` | 3.1 GB | stage (`-stage`) | ✅ yes |
| ↳ `Saved/Cooked/` | 1.6 GB | cook (`-cook`) | ✅ yes |
| `Binaries/Win64/` | 3.2 GB | compile (`-build`) | ✅ yes |
| `.vs/` | 0.4 GB | IDE | ✅ yes |
| `Content/` | 2.1 GB | **you (source)** | ❌ never |

**~38 GB regenerable vs ~4.8 GB you actually ship.** (`.git`, ~5 GB, is version control - not a build artifact.)

## The full map
### In-project (relative to project root)
| Path | Category | Created / grown by | Delete? | Notes |
|---|---|---|---|---|
| `Content/` | Source asset | you | ❌ never | Your assets |
| `Config/` | Source config | you | ❌ never | `.ini` settings |
| `Source/` | Source code | you | ❌ never | C++ |
| `<Project>.uproject` | Source | you | ❌ never | Project descriptor |
| `Plugins/<N>/{Source,Content,Config,*.uplugin}` | Source | you | ❌ never | Plugin sources |
| `Binaries/` | Compile output | `-build` | ✅ | DLL/exe/pdb; regenerated on build or editor open |
| `Plugins/*/Binaries/` | Compile output | `-build` | ✅ | Mirrored per plugin |
| `Intermediate/` | Compile intermediate | `-build` | ✅ | obj, generated headers, ShaderCompileWorker - usually #1 offender |
| `Plugins/*/Intermediate/` | Compile intermediate | `-build` | ✅ | Mirrored per plugin |
| `DerivedDataCache/` (local) | Cache | cook / editor | ✅ (slow regen) | Local DDC, if not pointed at shared |
| `Saved/Cooked/<Platform>/` | Cook output | `-cook` | ✅ | Cooked assets |
| `Saved/StagedBuilds/<Platform>/` | Stage output | `-stage` | ✅ | Staged loose/pak build |
| `Saved/Shaders/`, `Saved/ShaderDebugInfo/` | Shader cache | cook | ✅ | |
| `Saved/Logs/` | Logs | editor / build / run | ✅ | Keep latest if debugging |
| `Saved/{Crashes,Temp,Diff,SourceControl}` | Editor/runtime cache | editor / run | ✅ | Regenerable scratch/diagnostics |
| `Saved/{Autosaves,Backup,Screenshots,Collections,Config}` | **Recovery / user data** | editor | ❌ never | Recovery copies, screenshots, asset collections, local editor prefs - *not* safe to blanket-delete |
| `Build/` | **Mixed** | packaging resources + `-archivedirectory` output | ⚠️ partial | **Keep** icons, `PakBlacklist*.txt`, `steam_appid.txt`; **delete** archived/staged output |
| `.vs/`, `.idea/` | IDE cache | IDE | ✅ | |
| `<Project>.sln`, `*.DotSettings.user` | Generated project files | GenerateProjectFiles | ✅ | |

### External (outside the project root) - easy to forget
| Path | Category | Delete? | Notes |
|---|---|---|---|
| `%LOCALAPPDATA%\UnrealEngine\Common\DerivedDataCache` | Shared DDC | ✅ (slow regen) | Grows to many GB; shared across all projects; the main out-of-project footprint |
| `<Engine>/Engine/Programs/AutomationTool/Saved/Logs` | UAT logs | ✅ | Build-tool logs |
| `<Engine>/Engine/Saved/` | Engine cache/logs | ✅ | Engine-side caches |

## Safe vs not safe - the short version
- **✅ Safe to delete (auto-regenerated):** `Binaries`, `Intermediate`, all `Plugins/*/{Binaries,Intermediate}`, the pipeline outputs in `Saved` (`Cooked`/`StagedBuilds`/`ShaderDebugInfo`), the shader/derived caches (`DerivedDataCache` local + shared, `Saved/Shaders`), `.vs`, `.idea`, generated `<Project>.sln`. Deleting these forces a fresh recompile/recook (slower next build) but never touches your work.
- **⚠️ Mixed - be selective:** `Build/` holds both packaging *resources* you author (icons, `PakBlacklist*.txt`, `steam_appid.txt`) and build *output* (when used as the archive dir). Delete the output, keep the resources.
- **❌ Never delete (your work):** `Content`, `Config`, `Source`, `<Project>.uproject`, `*.uplugin`, plugin `Source`/`Content`/`Config`, `.git`, **and the recovery/user-data subfolders of `Saved/`** - `Autosaves`, `Backup`, `Screenshots`, `Collections`, `Config`. ⚠️ `Saved/` is **not** uniformly safe: it interleaves regenerable pipeline output with crash-recovery copies, screenshots, and local prefs, so never blanket-delete the whole tree.

## What the tool deletes - cleanup policy (the implemented scope)
The categorization above is what Unreal *scatters*; the tool's cleanup is deliberately **narrower and allow-list-based** - it deletes only explicitly-named category roots, never a recursive sweep of a folder that could hold your work. Encoded in `src-tauri/src/footprint/rules.rs` (the single source of truth; the **Clean** tab, the per-node delete, and the Clean-up phase all derive from it).

**The reclaim surface - four buckets.** The Clean tab groups everything regenerable into Save / Binaries / Intermediate / Cache:

| Group | Leaf | What it removes | Editor impact |
|---|---|---|---|
| **Save** | Staged build | `Saved/StagedBuilds` | none |
| | Cooked content | `Saved/Cooked` | none - editor reads uncooked `Content/` |
| | Shader cache | `Saved/Shaders` + `Saved/ShaderDebugInfo` | shader rebuild |
| **Binaries** | Game | `Binaries/<plat>/<gameTarget>*` files | none |
| | Plugin | **all** plugins' `Plugins/*/Binaries` (one option - cleans every plugin) | plugin modules relink next open |
| **Intermediate → Game** | per build target | `Intermediate/Build/<plat>/[<arch>/]<gameTarget>` - **one option per build target** (e.g. `SampleProjectSteam`), removing all of its build-mode folders | none |
| | Other | wipes the **whole** main `Intermediate/` (no per-folder scan); its row shows the **remainder** (editor target + engine tools + scratch like `ShaderAutogen`/`BuildRules`); ticking it auto-ticks every game target | **full recompile** next open |
| **Intermediate → Plugin** | - | **all** plugins' `Plugins/*/Intermediate` (one option - cleans every plugin) | plugin modules rebuild next open |
| **Cache** | Derived data cache | local `DerivedDataCache/` | slow regen |

**Intermediate → Game is the build cache; its `Other` child is the catch-all wipe.** Tick a build target for a dev-safe clean (the editor keeps its compile cache). Tick **Other** (or the **Game** group checkbox) to wipe the whole main `Intermediate/` - Other deletes the directory wholesale (it doesn't enumerate folders), so ticking it shows every game target ticked too. Game's children (per-target + Other) are additive: each target shows its size, Other shows the remainder, and together they sum to the whole `Intermediate/`.

Game-vs-editor classification is by the **first token** of a dir/file name (split on `-`/`.`) against the detected target names: only **game** (packageable) targets get a per-target Game option (and Binaries → Game); the **editor** target, engine tools (`UnrealEditor` / `ShaderCompileWorker` / `UnrealPak`), scratch, and third-party files are never offered *individually* - the editor cache only goes when you tick **Other** (Intermediate), and editor/third-party Binaries are simply left. Newer UBT nests the target under an **architecture** folder (`Intermediate/Build/<plat>/<arch>/<target>`, `<arch>` = `x64`/`arm64`); the scanner descends through `<arch>` to find the per-target dirs (the bulk of `Intermediate` lives there), so each target sizes correctly instead of leaving 20 GB unattributed.

**Deliberately out of scope** (verified decisions, not oversights):
- **Editor compile artifacts in the auto Clean-up phase** - `Intermediate → Other` (the editor cache) is offered on the interactive tab but never by the on-success Clean-up phase, which must not force an editor recompile.
- **Shared DDC** (`%LOCALAPPDATA%\…\Common\DerivedDataCache`) and **engine-external** paths (`<Engine>/…/Saved`) - cleanup stays strictly project-scoped; it never writes outside the open project.
- **`Build/`** and **archive outputs** - outputs are managed in **build History** (delete + open-location), not here; `Build/` mixes authored resources with output and can't be safely auto-classified.
- **IDE/project files** (`.vs`, `.idea`, `.sln`, `*.DotSettings.user`) and **misc `Saved/` caches** (`Logs`/`Crashes`/`Temp`/…) - trivial in size, and the misc caches sit next to recovery/user data, so they're left alone.

**Clean tab vs. the Clean-up build phase.** The interactive **Clean** tab auto-rescans whenever you open it and lets you tick individual nodes (one build target's intermediate; "Other" to wipe the whole main Intermediate; "Plugin" for all plugins); it deletes **by node id**. A segmented **bar** of the reclaimable composition (selected categories full color, unselected a lighter tint) sits in a summary panel **on top** of the category list, with a clickable legend; there's no per-row "safe" column. The **Clean-up phase** (a profile's optional terminal step) deletes **by category** after a successful build - pick from the dev-safe buckets (Save / Binaries Game·Plugin / Intermediate Game·Plugin / Derived data cache); Intermediate → Other is tab-only, so the phase never touches the editor cache.

## Build-step → footprint mapping
What each `BuildCookRun` step generates - i.e. which Clean-up categories a build of that profile produces:
- `-build` → `Binaries/`, `Intermediate/`, `Plugins/*/{Binaries,Intermediate}`, `.vs/`, `<Project>.sln`
- `-cook` → `Saved/Cooked/`, `Saved/Shaders/` (+`ShaderDebugInfo`), DDC growth (local **and** shared)
- `-stage` → `Saved/StagedBuilds/`
- `-pak` → `.pak`/`.ucas`/`.utoc` inside staged + archive output
- `-archive -archivedirectory=X` → the archived copy at `X`

So a profile that only builds (no cook/stage) generates compile artifacts only - its `Saved/Cooked` and `Saved/StagedBuilds` are stale and safe to purge. You pick the matching categories in that profile's Clean-up phase.

## Internet findings (verified)
- **Generated folders are safe to delete and auto-regenerate.** Epic's community guidance lists `.vs`, `Binaries`, `DerivedDataCache`, `Intermediate`, `Plugins/*/Intermediate`, `Saved`, and the generated `.sln` as deletable at any time - they're recreated when you next open/build the project, and deleting them does **not** affect source assets or code. Common reasons: shrink the project, fix packaging errors, force a fresh shader/class recompile. - [UE Community Wiki - Cleaning Your Project](https://unrealcommunity.wiki/6100e8169c9d1a89e0c344bf), [Epic forums - folders safe to delete](https://forums.unrealengine.com/t/folders-safe-to-delete-to-recompile-everything/669919)
- **Same set is safe to strip before archiving/sharing a project.** - [Epic forums - which folders to delete when archiving](https://forums.unrealengine.com/t/which-project-folders-can-safely-be-deleted-when-archiving/473224)
- **The staging/archive pipeline creates ~4 copies.** For a Windows build the staged folder defaults to `Saved/StagedBuilds/<Platform>`; `-archive`/`-archivedirectory=` copies that staged folder elsewhere, so the archive contains the same files as `Saved/StagedBuilds`. Net result: original assets + cooked-in-`Saved` + staged + archived = four copies. - [botman99 - Unreal Automation Tool / BuildCookRun reference](https://github.com/botman99/ue4-unreal-automation-tool)
- **The shared DDC can quietly become the biggest offender.** `%LOCALAPPDATA%\UnrealEngine\Common\DerivedDataCache` is shared across projects and grows to many GB; it's safe to delete (regenerates, slowly). - [Epic forums - shared DDC getting huge, safe to delete?](https://forums.unrealengine.com/t/c-users-appdata-local-unrealengine-common-deriveddatacache-is-getting-so-big-safe-to-delete/1488007)