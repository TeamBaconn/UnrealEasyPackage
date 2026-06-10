# User Experience

> Main doc: [../CLAUDE.md](../CLAUDE.md) · Requirements: [requirement.md](requirement.md) · Data: [data-storage.md](data-storage.md)

High-level walkthrough of the app's screens, navigation, and user flow. Requirement tags (R1-R6) refer to [requirement.md](requirement.md).

## Mental model
The app has **one gate, one main window with 3 side-nav tabs, two auxiliary windows, and a Settings modal** opened on demand:

```
Project Selection (gate)
        │ valid project
        ▼
Main Window ── side nav ──┬─ Dashboard      (graphs / comparisons)
                          ├─ Build          (run + build history)
                          └─ Clean          (scan + reclaim space)

Windows (opened on demand):
   Build Settings   - manage profiles & templates     (from Build)
   Build Logs       - live stream or past-build log    (from Build)

Modal (opened over the current surface):
   Settings         - theme, notifications, version    (from gate or main)
```

So there are **7 surfaces total**: 1 gate + 3 nav screens + 2 windows + 1 modal.

## High-level flow
```
First run ─► Project Selection ─►(validate .uproject + engine)─► Main: Dashboard
Returning ─► Project Selection (recents) ─► open ─────────────► Main

Build a game
  Build tab ─► pick profile (dropdown) ─┐
  or open Build Settings ─► New (from template) ─► set target/maps ─┐
                                                                    │
                              ─► ▶ Run ─►(folder exists? confirm replace)─► Build Logs
                                            (live: stream • filter • cancel)
                                            │ on finish → OS notification + history record
                                            ▼
                              Build history row (time • size • duration • status • tags)
                                 ├─ Log ─────────► Build Logs (past: read • filter)
                                 └─ Open location ─►(modified-date check)─► build folder

Reclaim space
  Clean tab ─► auto-scan on open (size + location, grouped Save / Binaries / Intermediate / Cache)
                 ├─ tick any nodes (one per build target; Plugin = all plugins)
                 └─ Intermediate: Game target = build only · Game→Other = wipe whole Intermediate

Compare
  Dashboard ─► graphs across history (size / duration / status, by tag: platform / config / target)
```

---

## 1. Project Selection *(gate)*
**Purpose:** choose what to work on - a project **or** a plugin - and validate it before entering.

- **Open** - a two-item menu: **Project** (pick a `.uproject`) or **Plugin** (pick a `.uplugin`).
  - *Project:* **two checks must pass before open is allowed:** **(1)** the **project** is valid (the `.uproject` exists and parses) and **(2)** a **valid engine folder** is resolved. Engine resolution prefers a **saved path** (from a prior Locate), then auto-resolves the descriptor's engine association, then prompts the user to **Locate…** the engine folder - the chosen path is **saved** for next time. **If either the project or the engine is stale, the user must replace it (re-locate) before Open enables.**
  - *Plugin:* opens immediately on a valid `.uplugin` - **no engine step** (a plugin packages standalone; the compile engine is chosen later, in the Tools tab).
- **Recents** - quick-access list of **both** projects and plugins. Each row shows the name; a **plugin** entry is marked with a small neutral **Plugin** tag next to its name (projects are untagged), in place of the old project-only Ready/Invalid status column. Projects also show their engine (version + path); plugins show "-". Validity is checked on display; an invalid descriptor shows a **fix** affordance (re-pick the file), and a per-row **⋯ menu** offers *open folder / change path / (projects: open engine folder, change engine path) / remove from recents*. Entries are removed only on explicit Remove.
- The **Settings** modal (§8) can be opened from here too.
- On open of a **project** → auto-detect target(s), maps, and enabled plugins (always fresh - re-run on every open, never cached), then land on the Dashboard. On open of a **plugin** → land on the plugin shell's Tools tab (§9).

*Serves: R2, R7.*

## 2. Main Window *(shell)*
Persistent **left side-nav** plus, in the side-nav itself, the open item's identity (at the top) and **Change Project** (back to the gate) + **Settings** (§8) at the foot. There is no separate header bar; auxiliary windows (Build Settings, Build Logs) are **separate windows**, not nav tabs. The nav tabs depend on what's open:
- **Project:** three tabs - **Dashboard · Build · Clean** (§§3-5).
- **Plugin:** a single **Tools** tab (§9) - Dashboard/Build/Clean are project concerns and don't apply.

## 3. Dashboard
**Purpose:** visualize and compare builds over time.

- Graphs built from build history: **build size over time**, **warnings & errors over time**, and **build status** (cumulative pass/fail) - broken down by tag dimensions (platform / config / target). Build **duration** is shown as a KPI tile (latest + vs-previous delta), not a trend chart.
- **Per-phase timing** is surfaced in the **Build Logs** pipeline graph (per-node elapsed time), not on the Dashboard - see [build-commands.md](build-commands.md#8-phase-decomposition-separate-processes-parallelism-and-timing).
- At-a-glance trends so the user can see, e.g., a profile's output size creeping up across builds.

*Serves: R5.*

## 4. Build
**Purpose:** launch builds and browse build history. This is the operational hub.

- **Profile selector** - a dropdown to pick the build profile to run, plus a button that opens the **Build Settings** window for full configuration.
- **Run** - starts the selected profile and opens **Build Logs** (live), where the run renders as a **dynamic pipeline graph** + streaming log. If the profile's resolved output folder already exists, **prompt to confirm replace** before overwriting. On completion, writes a build-history record.
- **Build history list** - recent + historical builds with columns: build time (timestamp), build size, duration, status.
  - **Filter** by tags: platform, config, target, status (success / failed / …).
  - **Delete** a single entry, selected entries, or all.
  - **Row actions:** **Log** → opens Build Logs for that build; **Open location** → opens that build's output folder, after a **modified-date check** confirming the folder still holds *that* build and warning if it's changed or missing.

*Serves: R1 (select & run), R4 (logs), R5 (history).*

## 5. Clean
**Purpose:** see what builds leave on disk and reclaim space.

- **Auto-scans on open** (and a manual **Re-scan**); results group into **Save** (Staged / Cooked / Shader), **Binaries** (Game + Plugin), **Intermediate** (Game - one per build target + Plugin), and **Cache** (local DerivedDataCache) - each node with **size (GB)** and **location(s)**. "Plugin" is one option that cleans every plugin's dirs.
- **Reclaimable bar** - a segmented bar of the composition by category, each segment a distinct **color** (lighter tint when unselected), with a clickable **legend** and the total + "Selected to remove" beside it. Sits in a summary panel **on top** of the directory list. No per-row "safe" column.
- Intermediate → **Game** lists one option per build target (editor-safe build clean) plus an **Other** child that wipes the **whole** main `Intermediate/` (⚠ full editor recompile; ticking it auto-ticks every game target). **Plugin** (all plugins) is a separate sibling. Tick a build target to clean and keep developing; tick **Other** (or the Game group) to wipe everything.
- Before deleting, a confirm lists every folder to be removed and the space reclaimed; source/recovery dirs and the editor cache are never offered. (See [build-footprint.md](build-footprint.md) for the categorization rules.)

*Serves: R3.*

## 6. Build Settings *(window)*
**Purpose:** manage build **profiles** (per-project) and **templates** (global, reusable bases).

- **Left panel** - list of the project's profiles; **Add (+)** clones a **template** (a fixed built-in like *Development*/*Shipping*, or a saved one) or **an existing profile** - there is **no blank/empty** option. Each profile row has a **⋯ menu**: *Make this a template* (save it as a reusable global base), *Clone* (self-contained copy, `<name> (copy)`), and *Delete*.
- **Right panel** - the settings form for the selected profile:
  - *Profile-level:* **name**, platform, **target** (from detection). The build **configuration** is set in the **Build** phase, and the **output destination** in the **Archive** phase (base dir + folder name, tokens in an ⓘ) - the latter is what the Run-time duplicate-folder check (§4) resolves.
  - *Per-phase sections* - **Build · Cook · Stage · Pak · Archive · Copy Extras · Clean-up** (generated from the phase registry, so new phases appear automatically), each a collapsible **island** with an **enable toggle** and that phase's config (**no command is shown**). Configs: **Build** → configuration + clean build + No Perforce (default on); **Cook** → cook mode (by the book) + **maps & cultures multi-select boxes** (Select All/None) + incremental cook (full / iterative / incremental) + skip editor content + additional cooker options; **Stage** → prerequisites + for distribution + debug symbols + separate debug info; **Pak** → I/O Store + compress + native package; **Archive** → the output destination (base dir + folder name); **Copy Extras** → `{ from, to }` rows where **from** is project-relative with a file/folder **picker** and **to** is typed by hand; **Clean-up** → footprint categories (multi-select) + run-on-success (no sizes - unknowable before a build). **Every phase is toggleable (enabled by default)** - turn any off to skip it. The one dependency: **Pak and Archive require Stage** (both run inside the staged tree), so they're greyed off with a "needs Stage" hint when Stage is off. Build/Cook/Stage/Pak/Archive each have an **Additional args** escape hatch; since Stage/Pak/Archive run as one process, their additional args **merge into the single command**.
- **Templates** - global reusable bases: the fixed built-ins (*Development*, *Shipping*, …) are undeletable + read-only seed bases. Create your own by **saving a profile as a template** (a row's **⋯ → Make this a template**) - there is no separate Templates manager, and no blank template.
- **Tags** are auto-derived (platform / config / target / status) - not manually editable.
- **Unsaved changes** - a bottom action bar with **Save** / **Discard** and a guard against losing changes.

*Serves: R1 (consumes R2 detected data).*

## 7. Build Logs *(window)*
**Purpose:** the live pipeline graph + live/historical build logs.

- **Pipeline graph** - a **dynamic Jenkins-style stage view** at the top: one node per phase (Build · Cook · Stage · Pak · Archive · Copy Extras · Clean-up) with status (pending / running / success / failed / skipped) and elapsed time, and **parallel branches** for concurrent phases (e.g. Build ∥ Cook). Generated from the phase registry - not hardcoded - so added phases appear automatically; click a node to jump to that phase's log section. **Replayable for past builds** from the stored per-phase timing.
- **Live build** - stream the running process's console output in real time, with progress/status and a **Cancel** action.
- **Past build** - load and read a completed build's saved log (`build.log`, co-located with its record).
- **Filter** - toggle to show only **errors**, only **warnings**, or all (this is the "separate warnings/errors view" expressed as filters).

*Serves: R4.*

## 8. Settings *(modal)*
**Purpose:** lightweight app preferences. Openable from either the Project Selection gate or the Main window.

- **Theme** (e.g. light / dark).
- **Notification preference** (build-finish toast/sound on/off).
- **Version number** (about).

*Serves: R6.*

## 9. Plugin · Tools *(plugin shell)*
**Purpose:** package a `.uplugin` for distribution (e.g. FAB). Shown only when a **plugin** was opened (§1); reuses the shell layout with a single **Tools** tab holding a **collapsible Package Plugin island** (the streaming log opens in its own window, §10).

- **Package Plugin** island - runs `RunUAT BuildPlugin … -rocket` (standalone; no host project). A collapse/expand header (chevron) hides the form; the **Package Plugin** button is pinned at the **bottom** of the island. Fields:
  - **Unreal Engine** - a dropdown of engines to compile with (the **Rescan** icon sits by the label; **Browse…** sits in the same row as the dropdown). Each entry shows the **version in bold** and its **path** in dim text, the paths column-aligned. Engines come from auto-detected machine installs (source builds + launcher installs, each validated) plus any folder added via **Browse…** (validated + **remembered for this plugin**; stale ones drop off). The `.uplugin`'s `EngineVersion` pre-selects a match.
  - **Output folder** - the base directory to write into (Browse… or type), remembered per plugin.
  - **Folder name** - a token template (`{plugin} {version} {engine} {date} {time}`, case-preserving, default `{plugin}-{version}`), remembered per plugin.
  - **Delete Binaries and Intermediate after packaging** - checkbox, default on, labeled as the FAB-submission requirement.

*Serves: R7.*

## 10. Plugin Log *(window)*
**Purpose:** show the live `BuildPlugin` output. Opens automatically when packaging starts (§9). Same design as **Build Logs** (§7) minus the pipeline graph (a plugin package is a single command, not a DAG):

- A top bar with a status badge (live/done/failed/cancelled) + elapsed, **Cancel** while running, and **Open output folder** on success.
- The severity-tinted streaming **console** with the **All / Warnings / Errors** filter (same as Build Logs).
- A **Command** island showing the resolved `RunUAT BuildPlugin …` command (copyable).
- **No history:** closing the window **while the process is running stops it** - a confirm ("Discard this process?") explains that discarding stops the package and to keep it running keep the window open. Once the process has finished/failed, the window closes freely (there's nothing to keep).

*Serves: R7.*

---

## Cross-cutting behaviors
- **Notifications:** OS toast on build success/failure (respects the Settings preference). *(R4, R6)*
- **One logs window, two modes:** opened live from a running build, or historical from a Build-history row's **Log** action. *(R4)*
- **Build-record integrity:** "Open location" uses a **modified-date check** to confirm a build folder still matches its record. *(R5)*
- **History index sync:** on launch a *light* check - build-folder **count** vs index row count - detects drift; if found, a non-blocking **prompt** offers to update (the *heavy* reconcile). *(R5)*

## Conventions & decisions
- **Build settings = global templates + local profiles.** Templates hold project-agnostic bases; profiles are created from a template (copy-on-create) and add the project-specific target & maps.
- **Tags are auto-derived flat values** (platform, config, target, status) - no key/value, no custom tags; they double as the reverse-mappable record of a build's config.
- **Build integrity** uses a **modified-date check** (not a checksum) to confirm a build folder still matches its record.
- **Output folder naming** uses placeholder templates (`{project}`, `{platform}`, `{config}`, `{profile}`, `{target}`, `{date}`, `{time}`) so each build lands in its own predictable folder; a duplicate folder triggers a confirm-replace prompt at Run.
