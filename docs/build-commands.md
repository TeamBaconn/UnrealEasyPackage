# Build Commands - `RunUAT BuildCookRun`

> Main doc: [../CLAUDE.md](../CLAUDE.md) · Requirements: [requirement.md](requirement.md) · Footprint: [build-footprint.md](build-footprint.md) · Data: [data-storage.md](data-storage.md)

This is the reference behind the tool's whole reason to exist: **how Unreal packages a game from the command line, what every relevant parameter does, and - the part that drives the product - which parameters a build profile should let a user edit vs. which the tool should set automatically or hide.** It's written for the [profile/template model](requirement.md#r1--build-settings-templates--profiles) and the `unreal` module's **BuildCookRun arg builder**.

> **One rule above all:** for any *specific* engine, `RunUAT BuildCookRun -help` is the authoritative flag list. Epic publishes **no complete official reference** - the community consensus is "read `ProjectParams.cs` or run `-help`." This doc is the curated, cross-checked synthesis (UE 5.3-5.5); treat the obscure/`⚠️`-marked flags as "verify against `-help`," especially for a custom **source** engine.

---

## TL;DR

- Packaging is **one command** - `RunUAT BuildCookRun …` - that runs a fixed **pipeline**: **Build → Cook → Stage → Pak/Package → Archive → (Deploy/Run)**. Each phase is a **verb flag** you opt into (`-build -cook -stage -pak -archive`).
- A real packaging line is ~20-25 tokens, but **only ~6 of them are real decisions**. The rest is boilerplate (`-noP4 -utf8output -unattended -nosplash`), values computed from the project/engine (`-project -target -build`), or out-of-scope features (DLC, chunking, encryption, mobile).
- **UnrealEasyPackage runs this as a separate-process pipeline** (Jenkins-style), not one opaque command - plus two **app-owned phases** after Archive: **Copy Extras** (copy chosen project files into the final build) and **Clean-up** (reclaim footprint / wipe staging when the build finishes). Phases come from an **extensible registry** ([§8](#8-phase-decomposition-separate-processes-parallelism-and-timing)) so contributors add more without touching the executor or UI.
- **Expose in profiles:** platform · config · target · the per-phase toggles · output (archive) dir · maps · a few packaging toggles (`-pak -iostore -compressed -prereqs`) · clean-vs-iterate · per-phase config (Copy Extras mappings, Clean-up categories) · **Cook additional-options** as the escape hatch.
- **Always set, never show:** `-noP4 -utf8output -unattended -nosplash` and `-project`.
- **Compute, don't ask:** `-target` (auto-detect, ask only if >1), `-nocompile`/`-compile` (installed vs **source** engine). **`-build` defaults on but is toggleable** - when on and binaries are current UBT just does a cheap up-to-date check (no recompile); it only builds what changed, or the engine game target the first time per config (toggle off ⇒ `-skipbuild`). *(Turnkey SDK verify and other advanced flags are **deferred past MVP**.)*
- The pipeline's copy-on-each-step design (`Content/ → Saved/Cooked/ → Saved/StagedBuilds/ → archive`) is exactly the **~4-copy footprint** problem in [build-footprint.md](build-footprint.md) - the phases a profile runs determine what's safe to clean, and the **Clean-up** phase automates it.

---

## 1. The toolchain: what actually runs

```
RunUAT.bat                         Engine/Build/BatchFiles/RunUAT.bat
   │  finds .NET, ensures AutomationTool is compiled, forwards args
   ▼
AutomationTool.exe  BuildCookRun   Engine/Binaries/DotNET/AutomationTool/
   │  BuildCookRun is ONE automation command among many (BuildGraph, RunAutomationTest, …)
   │  every flag is parsed/owned by ProjectParams.cs
   ├──► UnrealBuildTool (UBT)       compiles C++  (the Build phase delegates here)
   └──► UnrealEditor-Cmd.exe        cooks content (-run=Cook commandlet, the Cook phase)
        + UnrealPak.exe / IoStore   bundles .pak / .utoc+.ucas (the Pak phase)
```

- **RunUAT** (`Engine/Build/BatchFiles/RunUAT.bat` on Windows) is a thin bootstrapper: it locates a .NET runtime, **ensures AutomationTool is built**, then launches `AutomationTool.exe` with your command + args.
  - On an **Installed/Launcher engine** it skips compiling UAT ("Dependencies are up to date… Skipping compile").
  - On a **source/from-GitHub engine** it proactively **recompiles UAT** every run (it assumes you may have edited UAT's C# source). *This is directly relevant here - the reference engine is a custom source build, so RunUAT will tend to add `-compile` on its own; don't fight it (see [§7](#7-gotchas--footguns)).*
- **AutomationTool (UAT)** is Unreal's C#/.NET automation host. `BuildCookRun` is just one command it can run. **UAT does not compile C++ itself** - during the Build phase it spawns **UnrealBuildTool (UBT)** and checks its exit code.
- **UBT** is the actual C++ build system (parses `*.Target.cs`/`*.Build.cs`, drives compiler/linker).
- The **cooker is a commandlet**, not part of UAT: the Cook phase launches `UnrealEditor-Cmd.exe <project> -run=Cook -targetplatform=…` headless. So a working build needs a built **editor** target to cook with (hence `-nocompileeditor` only when the editor already exists).
- **The Editor's "Package Project" button** ultimately runs this same `RunUAT BuildCookRun`. In **UE5 it first runs Turnkey** to verify/update the platform SDK, so the emitted line looks like `… RunUAT.bat BuildCookRun … Turnkey -command=VerifySdk -platform=Win64 -UpdateIfNeeded BuildCookRun -nop4 -utf8output -nocompileeditor …`. The reliable way to learn the exact command for a project is: **Package once in-editor, then copy the full line from the Output Log.**

---

## 2. The packaging pipeline (the process)

A single `BuildCookRun` invocation runs these phases **strictly in order**, each gated by its verb flag:

| # | Phase | Verb | What it does | Output on disk |
|---|---|---|---|---|
| 1 | **Build / Compile** | `-build` | UBT compiles the game/server/editor target(s) for the platform/config. | `Binaries/<Platform>/`, `Intermediate/` (+ per-plugin, + Engine) |
| 2 | **Cook** | `-cook` | Editor commandlet transforms editor assets → platform-native runtime data: texture compression, shader compile, Blueprint → bytecode, **strips editor-only data**, splits into `.uasset`/`.uexp`/`.ubulk`. | `Saved/Cooked/<Platform>/` *(UE5: `…/Windows/`)* - **copy #1** |
| 3 | **Stage** | `-stage` | Copies binaries + cooked content + prereqs into a **clean, ship-shaped tree**; writes a staging **manifest** (`Manifest_*.txt`). | `Saved/StagedBuilds/<Platform>/` - **copy #2** |
| 4 | **Pak / Package** | `-pak` (`-iostore`, `-package`) | Bundles staged loose files into `.pak` and/or IoStore `.utoc`+`.ucas` containers; `-package` produces a platform-native distributable where applicable. | paks/containers written **into** the staged tree |
| 5 | **Archive** | `-archive` | Copies the finished build **out of the project** to `-archivedirectory`. | `<archivedirectory>/<Platform>/` - **copy #3** |
| 6 | **Deploy / Run** | `-deploy` / `-run` | Push to a device / launch the build (smoke test). Usually a no-op for a plain Win64 archive. | - |

**Why this matters for footprint (the duplication theme):** each phase **copies** rather than moves, so after a full `-cook -stage -pak -archive` the same game data exists in `Saved/Cooked/`, `Saved/StagedBuilds/`, **and** the archive dir - on top of the original `Content/`. That's the **~4 copies** that turn a ~4.8 GB shippable into ~38 GB of artifacts (full breakdown → [build-footprint.md](build-footprint.md)). The set of verbs a profile runs is exactly what tells the cleanup feature what's safe to purge.

**Phase composition cheat-sheet:**

| Goal | Verbs |
|---|---|
| Full clean package + archive | `-build -cook -allmaps -stage -pak -archive -archivedirectory=<out>` |
| Re-package only (binaries + cook already done) | `-skipbuild -skipcook -stage -pak -archive …` |
| Iterate content only | `-skipbuild -cook -iterate -stage -pak …` |

**Cook By The Book vs Cook On The Fly:** the default (`-cook`) cooks everything up front - this is what a packaging tool emits. `-cookonthefly` ships a thin client that requests assets at runtime from a cook server; it's a dev-iteration mode, **not** for distribution. The tool should always use Cook By The Book.

---

## 3. Anatomy of a real command

The reference project's `build_worker.bat` (UE 5.5, Steam target) - a representative solo-dev packaging line:

```bat
RunUAT BuildCookRun ^
  -project=<Project>.uproject ^   :: WHAT     - auto-filled from the open project
  -target=SampleProjectSteam ^     :: WHAT     - auto-detected; ask only if >1 target
  -platform=Win64 ^               :: EXPOSE   - platform picker (Win64 default)
  -clientconfig=Development ^     :: EXPOSE   - the #1 decision (Development vs Shipping)
  -build -cook -allmaps ^         :: EXPOSE   - verbs + cook-all-maps
  -stage -pak ^                   :: EXPOSE   - verbs / packaging toggles
  -archive -archivedirectory=<Build/> ^  :: EXPOSE - output location
  -noP4                           :: HIDE     - always set (no Perforce)
```

What an in-editor Package emits adds the **boilerplate the user never chooses**: `-utf8output -unattended -nosplash -nocompileeditor -nocompile -nocompileuat -unrealexe=…\UnrealEditor-Cmd.exe`, plus the `Turnkey -command=VerifySdk -platform=Win64 -UpdateIfNeeded` preface. Strip those out and **only ~6 tokens are genuine choices** - which is the entire premise of this tool.

> **Multi-value flags join with `+` and no spaces:** `-platform=Win64+Linux`, `-clientconfig=Development+Shipping`, `-map=Entry+Arena`, `-cookcultures=en+fr`. Flag names are case-insensitive. The arg builder must use `+`, never commas or spaces.

---

## 4. Parameter reference

Grouped by function. **Default** is best-effort for UE 5.3-5.5; `⚠️` marks flags whose existence/default/exact name should be confirmed against `-help` for the target engine. This is curated, not exhaustive (the Epic-internal long tail is omitted).

### 4.1 Project & target
| Flag | Does | Default | Notes |
|---|---|---|---|
| `-project=<path>` | Path to the `.uproject`. **Required.** | - | Quote if it has spaces. Tool auto-fills. |
| `-target=<Name>` | Which `*.Target.cs` to build (e.g. `SampleProjectSteam`). | auto | Needed when a project has multiple targets. |
| `-ScriptsForProject=<path>` | UAT-global; which project's automation scripts to load. | - | Editor always adds it; = the `.uproject`. |

### 4.2 Platform & configuration
| Flag | Does | Default | Notes |
|---|---|---|---|
| `-platform=` / `-targetplatform=` | Client platform(s). | host | `Win64`, `Linux`, `Mac`, `Android`, `IOS`, `TVOS`, `LinuxArm64`, `VisionOS`⚠️. Consoles need NDA extensions. `Win32` removed in UE5. |
| `-clientconfig=<C>` | **Client** build configuration. | `Development` | The canonical config flag for client packages. |
| `-serverconfig=<C>` | **Server** build configuration. | `Development` | Only with server builds. |
| `-serverplatform=` / `-servertargetplatform=` | Server platform(s). | = client | E.g. Linux server + Win64 client. |
| `-configuration=<C>` ⚠️ | Wrapper-level config alias. | - | **Prefer `-clientconfig`/`-serverconfig`** - `-configuration` is a UBT/`BuildGame` concept and a common newbie mistake here. |

**Valid configurations:** `Debug` · `DebugGame` (your code unoptimized, engine optimized) · `Development` (editor's own config; full logging/stats) · `Test` (Shipping speed + select diagnostics) · `Shipping` (release; strips console/stats/profiling, `check()` compiled out).

### 4.3 Build-step verbs & compile control
| Flag | Does | Notes |
|---|---|---|
| `-build` / `-skipbuild` | Run / skip the compile. | C++ projects need `-build`; Blueprint-only don't (stock engine binaries run cooked content). |
| `-cook` / `-skipcook` | Cook content / assume cooked data is current. | Core verb. `-skipcook` ⇒ a prior cook exists. |
| `-cookonthefly` | Serve cooked data at runtime from a cook server. | Dev iteration only; never for packaging. |
| `-stage` / `-skipstage` / `-nocleanstage` | Stage / reuse stage / don't wipe stage first. | `-pak`/`-archive` are meaningless without a stage. |
| `-pak` / `-skippak` | Bundle into `.pak` / reuse paks. | Off ⇒ loose files (easier to debug, thousands of files). |
| `-package` / `-skippackage` | Produce platform-native distributable. | Required for some mobile/console; mostly a finalize on Win64. |
| `-archive` + `-archivedirectory=<path>` | Copy final build out to `<path>`. | The output location. Default it **outside** the project (see [§7](#7-gotchas--footguns)). |
| `-run` / `-deploy` | Launch / push to device after packaging. | Usually omitted. |
| `-clean` | Wipe intermediates before building. | The honest cure for stale cooks. |
| `-iterate` / `-iterativecooking` | Recook only changed assets. | Fast, but **staleness footgun** - dev only, not release. |
| `-nocompileeditor` / `-skipbuildeditor` | Don't (re)build the editor used to cook. | Safe when the editor target already exists (normal case). |
| `-compile` / `-nocompile` / `-nocompileuat` | Force / skip compiling **UAT itself**. | **Engine-type dependent** - installed ⇒ `-nocompile`; source build ⇒ allow `-compile`. |
| `-ubtargs="<args>"` | Pass raw args through to UBT. | Escape hatch. |

> The verbs are **opt-in**: pass each phase you want. Don't rely on implicit defaults (sources disagree and they've shifted across versions) - the tool should always emit the verbs the profile enables, explicitly.

### 4.4 Cook options
| Flag | Does | Notes |
|---|---|---|
| `-allmaps` | Cook all project maps. | Common default. |
| `-map=A+B` | Cook/run specific map(s). | Use to restrict cook scope for speed. |
| `-cookall` / `-CookAll` | Cook all assets, not just referenced. | Bigger, slower. |
| `-mapsonly` | Cook only maps + references. | |
| `-cookcultures=en+fr` | Restrict cooked localization cultures. | Defaults to project cultures. |
| `-cookflavor=<F>` ⚠️ | Texture flavor for platforms with several (Android `ASTC`/`ETC2`/`DXT`). | **Irrelevant to Win64** - never show for Windows. |
| `-unversionedcookedcontent` | Omit version headers from cooked packages. | Normal for shipping (game = exact cook engine). No version safety net if they drift. |
| `-iterate` / `-iterativecooking` | Recook only modified assets (no deps). | Legacy iterative cook; dev-only (staleness footgun). |
| `-cookincremental` ⚠️ | Recook modified assets **and dependencies** (Zen-snapshot). | **UE 5.6+** (experimental 5.6 / beta 5.7) - newer replacement for `-iterativecooking`; not in 5.5. |
| `-AdditionalCookerOptions="<args>"` | Pass raw args to the cook commandlet. | Escape hatch (known quote-trimming bug in some versions). |
| `-SkipCookingEditorContent` / `-sks` | Skip cooking editor-only content. | Smaller output. |
| `-CookPartialGC` | GC packages during cook (lower peak memory). | Big projects. |
| `-nativizeAssets` | **Removed in UE5** (Blueprint nativization). | Do not use; will fail/no-op. |

### 4.5 Stage / pak / packaging
| Flag | Does | Notes |
|---|---|---|
| `-iostore` | Emit IoStore `.utoc`+`.ucas` containers. | UE5 high-perf format; faster loads. Driven by project `bUseIoStore` (often on for UE5 shipping). |
| `-compressed` / `-compresspak` | Compress pak/container payload. | Default compressor **Oodle/Kraken**; recommended on for distribution. |
| `-prereqs` | Stage the prerequisites installer (VC++ redist, etc.). | Needed when shipping to other machines. |
| `-applocaldirectory=<path>` | Copy app-local dependency DLLs next to the exe. | Alternative to `-prereqs` (portable, no installer). |
| `-nodebuginfo` | Don't stage `.pdb` debug symbols. | Big footprint lever; on for lean Shipping. |
| `-separatedebuginfo` | Stage symbols into a separate dir. | Keep symbols without bloating the build. |
| `-manifests` / `-createchunkinstall` | Streaming-install chunk data. | Out of MVP scope. |
| `-stagingdirectory=<path>` | Override the stage location. | Default `Saved/StagedBuilds`. |
| `-cmdline=` / `-addcmdline=` | Write/append staged `UECommandLine.txt`. | Runtime args for the packaged build. |
| `-nullrhi` | Add `-nullrhi` to client cmdline (headless). | Server/CI smoke tests. |
| `-zenstore` / `-nozenstore` | Use/disable Zen as cooked-output store. | **In UE 5.5, Zen cooked-output is opt-in, not default.** Keep cook/stage consistent. |

### 4.6 Environment / source control (UAT-global boilerplate)
| Flag | Does | Notes |
|---|---|---|
| `-noP4` / `-nop4` | Disable Perforce integration. | **Almost always wanted** - UAT assumes P4 on "build machines" and misbehaves without it. |
| `-utf8output` | Force UTF-8 console output. | Log hygiene (matters for the custom log console's stream). |
| `-unattended` | No operator present: never prompt, fail fast. | Mandatory for a spawned, streamed process. |
| `-nosplash` | Suppress the editor splash during cook. | Headless cosmetic. |
| `-buildmachine` | Mark as build-machine run (extra logging, skips some validation). | Usually leave off for a desktop tool. |
| `-NoCodeSign` / `-nocodesign` | Skip code signing. | |
| `-verbose` / `-veryverbose` | UAT log verbosity. | |
| `-help` / `-list` | Dump the authoritative per-engine flag list. | **Run this to ground-truth everything here.** |

### 4.7 Server / DLC / patching / IoStore-Zen (out of MVP scope, listed for completeness)
| Group | Flags | Notes |
|---|---|---|
| Server topology | `-server` / `-dedicatedserver`, `-noclient`, `-client`, `-serverconfig=`, `-serverplatform=`, `-servercmdline=` | Only for dedicated/listen-server profiles. |
| DLC / patch / release | `-dlcname=`, `-basedonreleaseversion=`, `-createreleaseversion=`, `-generatepatch`, `-StageBaseReleasePaks` ⚠️ | Patching/DLC pipelines; not MVP. |
| Encryption / signing | `-signpak=`, `-encryptinifiles`, `-skipencryption` | Driven by the **Crypto Keys** asset / `DefaultCrypto.ini`. Not MVP. |
| IoStore/Zen internals | `-skipiostore`, `-cook4iostore`, `-makebinaryconfig`, `-ReferenceContainerGlobalFileName=` ⚠️ | Advanced container/patch builds. |

---

## 5. What to expose vs. auto-manage vs. hide

**This is the deliverable.** The product thesis is *"show ~6 decisions, infer the rest, hard-code the boilerplate."* Every flag falls into one of four tiers.

### ✅ Tier 1 - Expose (the profile fields the user edits)
These are genuine per-build decisions. They map directly onto the [R1 profile/template fields](requirement.md#r1--build-settings-templates--profiles).

| UI field | Emits | Default | Why exposed |
|---|---|---|---|
| **Configuration** | `-clientconfig=Development\|Shipping` (+ DebugGame/Test advanced) | Development | The single most important choice. |
| **Platform** | `-platform=Win64` | Win64 | Windows-first; future Linux/Mac. |
| **Target** | `-target=<Name>` | auto | Show only when >1 target exists. |
| **Per-phase toggles** | `-build -cook -stage -pak -archive` | **all on by default, all toggleable** (Pak/Archive need Stage) | The pipeline checklist; also drives footprint cleanup & the live graph. |
| **Output (archive) dir** *(required)* | `-archive -archivedirectory=<path>` | base dir + [folder-name template](requirement.md#r1--build-settings-templates--profiles) | **Mandatory** - every build archives to an output path; default **outside** the project. |
| **Maps** | `-allmaps` *or* `-map=A+B` | all maps | From map detection. |
| **Additional args** *(per external phase)* | *(verbatim → that phase's process)* | empty | Escape hatch on **Build · Cook · Stage · Pak · Archive**. Cook → `-AdditionalCookerOptions`; Build → the UBT call; **Stage/Pak/Archive share one `BuildCookRun`, so their three strings are concatenated** into that command (Stage→Pak→Archive order). (The old global extra-args field was dropped.) |
| **No Perforce** | `-noP4` | on | Disable Perforce; **default on**, a visible toggle (off only if you run P4). |

> **Two app-owned phases** (no UAT flag) round out the pipeline, configured the same way - see [§8](#8-phase-decomposition-separate-processes-parallelism-and-timing): **Copy Extras** (a list of `{ from (project-relative, picked via a file/folder browser), to (build-relative, typed by hand) }` copies into the final build, e.g. `steam_appid.txt → .`) and **Clean-up** (post-build footprint reclaim by category - staging, cooked, intermediates, … - run on success by default).

### 🔶 Tier 2 - Secondary packaging toggles (now first-class per-phase)
Legitimate but secondary. In the MVP these are exposed as **first-class per-phase toggles** - Pak (I/O Store · compress · native package), Stage (prerequisites · debug symbols · separate debug info · for distribution), Build (clean build), Cook (**incremental cook mode**). Zen store and binary-config remain **unexposed** (advanced, deferred).

| Toggle | Emits | Suggested default |
|---|---|---|
| Use I/O Store | `-iostore` | read project `bUseIoStore`; on for Shipping |
| Compress content | `-compressed` | on for Shipping |
| Include prerequisites | `-prereqs` | on when "for distribution" |
| Create native package | `-package` | on for distribution-grade builds |
| Include debug symbols | omit `-nodebuginfo` (else set it) | off for Shipping (footprint win) |
| Clean build | `-clean` | off (mutually exclusive with incremental cook) |
| Incremental cook mode | None · `-iterativecooking` · `-cookincremental` | None (full cook); `-cookincremental` needs **UE ≥ 5.6** |
| Mark for distribution ⚠️ | `-distribution` | off (mainly console/mobile stores) |

### ⚙️ Tier 3 - Compute, don't ask (set automatically from detection; not user knobs)
| Flag | Decided by |
|---|---|
| `-project=<.uproject>` | the open project |
| `-target=<Name>` | target detection (ask only if >1) |
| `-build` | **on by default (toggleable)** - when on, a cheap up-to-date check if binaries are current; off ⇒ `-skipbuild` (reuse existing binaries) |
| `-nocompile`/`-nocompileuat` vs `-compile` | **engine type** - installed ⇒ `-nocompile`; **source build ⇒ allow `-compile`** (the reference engine is source) |
| `-nocompileeditor` / `-skipbuildeditor` | set when the editor target already exists |
| `Turnkey -command=VerifySdk …` ⏳ | **deferred past MVP** - not emitted yet; a future registry phase (assumes the Win64 SDK is already configured) |
| `-unrealexe=…\UnrealEditor-Cmd.exe` | resolved engine path (via `EngineAssociation` GUID, **not** stale `.bat` paths) |

### ❌ Tier 4 - Redundant / not needed (always set the same, or never relevant to a Win64 solo-dev MVP)
**Always-set boilerplate (hide entirely):** `-utf8output` · `-unattended` · `-nosplash`. These never change; surfacing them is pure noise. (`-noP4` used to live here but is now a **visible default-on toggle** - see Tier 1.)

**Not needed for the MVP (omit):**
| Flag(s) | Why not needed |
|---|---|
| `-cookflavor=` | Mobile texture formats - irrelevant to Win64. |
| `-server`, `-dedicatedserver`, `-noclient`, `-serverconfig=` … | No dedicated-server profile in MVP. |
| `-dlcname=`, `-basedonreleaseversion=`, `-generatepatch`, `-createreleaseversion=` | DLC/patching - out of scope. |
| `-manifests`, `-createchunkinstall` | Streaming/chunked install - out of scope. |
| `-signpak=`, `-encryptinifiles` | Pak signing/encryption - needs Crypto Keys setup; out of scope. |
| `-cookonthefly`, `-fileserver`, `-filehostip=` | Dev iteration modes, not packaging. |
| `-nativizeAssets` | Removed in UE5 - never emit. |
| `-configuration=` | Wrong flag for client packages - only ever emit `-clientconfig`. |

> **Net profile shape:** a saved profile is essentially `{ name, platform, config, target?, perPhaseConfig (cook/stage/pak/copyExtras/cleanup), outputDir, maps }`. the `unreal` module composes each phase's command/action = `boilerplate + computed flags + per-phase choices` for the **separate-process executor** and the **Build Logs** view - the profile editor shows **no command preview**.

---

## 6. Arg-builder rules for the `unreal` module

The builder must encode dependencies in **code** (the `unreal` module), not trust UI toggles:

1. **Phase dependencies:** `-pak` ⇒ `-stage`; `-archive` ⇒ `-stage`; `-iostore` ⇒ `-pak`. Quietly add the prerequisite or refuse to build the line.
2. **Clean vs incremental are mutually exclusive** - `-clean` can't combine with `-iterativecooking`/`-cookincremental`.
3. **`-build` is on by default but toggleable** - when on, BP-only ⇒ UBT does a cheap up-to-date check (compiles nothing); C++/source ⇒ it compiles only what changed (and the engine game target the first time per config). Toggle Build **off** ⇒ the Build unit is skipped and the downstream BuildCookRun calls keep `-skipbuild` (reuse existing binaries). `-nocompile`/`-compile` is a function of **engine type** (installed vs source) - cover both branches in a test where it pays.
4. **Multi-values use `+`** (`Win64+Linux`), never commas/spaces.
5. **`-clientconfig`, never `-configuration`** for client packages.
6. **Default the archive dir outside the project tree** (avoid the SampleProject `-archivedirectory=Build/` mixing).
7. **Resolve the engine exe from the `EngineAssociation` GUID** (`HKCU\…\Builds`), never from hardcoded `.bat` paths.
8. **Always emit** `-utf8output -unattended -nosplash` and the verbs the profile enables - explicitly. **`-noP4` is emitted by default but is a user-visible toggle** (off when the user runs Perforce).
9. **Generate a resolved command/action per phase** for the [separate-process executor](#8-phase-decomposition-separate-processes-parallelism-and-timing) and the **Build Logs** view (the profile editor shows **no** command preview): **Build** via UBT (per target), **Cook** via the editor commandlet (or `BuildCookRun -skipbuild -cook`), **Stage+Pak+Archive** via one `BuildCookRun -skipbuild -skipcook -stage -pak -archive`. **Copy Extras** and **Clean-up** are **app-owned tasks** (Rust `std::fs` / footprint module), not external commands - they expose an action + human-readable preview.
10. **Every phase is toggleable** (enabled by default) - the editor greys only **Pak and Archive when Stage is off** (both run inside the staged tree, so they need Stage; shown with a "needs Stage" hint). The arg builder emits a phase's command only when its `enabled` is set; the **Stage·Pak·Archive** unit is emitted only when Stage is on, including `-pak`/`-archive` per their toggles (and its graph node relabels, e.g. *Stage · Archive* when Pak is off).
11. **Per-phase additional-args (verbatim escape hatch):** each external phase carries an optional raw-args string appended to *its* command - **Build** → the UBT call; **Cook** → `-AdditionalCookerOptions="…"`; **Stage / Pak / Archive** → the **single** `BuildCookRun -skipbuild -skipcook -stage -pak -archive` call, so the three strings are **concatenated** into that one command line (in Stage→Pak→Archive order). Copy Extras / Clean-up are app tasks and have none.

These are exactly the inputs the [footprint feature](build-footprint.md#build-step--footprint-mapping) reads back: the verb set a profile ran ⇒ the folders that are safe to clean.

---

## 7. Gotchas & footguns

- **`-noP4` is mandatory without Perforce.** Omitting it makes UAT behave like a P4 build machine → errors/odd behavior. Always inject it.
- **`-build` is locked on - and that's safe.** Running `-build` when binaries are already current is **not a recompile**: UBT does an *up-to-date check* (parse target rules, hash/timestamp the module graph) and exits without compiling. That check is ~free for BP-only and seconds-tens-of-seconds on a big C++ project (many plugins) - never fatal, never a full rebuild. It does real work only when something actually changed (your code, a plugin, the engine) or a config was **never built** (e.g. first Shipping package on a source engine - required). So locking it on is correct: cheap when unneeded, essential when needed. The only thing forgone is shaving that up-to-date check off a rapid re-package - a negligible, deliberately-unexposed optimization.
- **`-clientconfig` vs `-configuration`.** Client packages use `-clientconfig` (servers: `-serverconfig`). `-configuration` is a different (UBT) concept and silently won't do what users expect.
- **`-nocompile` on the wrong engine type.** Installed engine ⇒ UAT can't self-compile ⇒ needs `-nocompile`. **Source build ⇒ may need `-compile`** or you run stale tooling. The reference engine is a **source build** - let RunUAT compile UAT; don't force `-nocompile`.
- **Iterative-cook staleness (a real footgun).** Legacy iterative cook (`-iterate` / `-iterativecooking`) is a *delta* cook against the previous output, and Epic confirms it **cannot detect: class schema changes (e.g. adding/removing/reordering a C++ `UPROPERTY`), "hidden" package dependencies (deps not saved as package imports), or dependencies on external files such as shaders.** The result is **stale cooked data or crashes** in the packaged build that don't reproduce in-editor (also documented: Blueprint mesh actors silently dropped). Epic's own guidance: *"try a full cook if any unexpected crashes happen after cooking iteratively."* So **never iterate for a release build; offer `-clean` to force a fresh cook.** Version note for the 5.5 target: the safer replacement, **Incremental Cooking** (Zen-snapshot based), is **not in 5.5** (experimental in 5.6, beta in 5.7), so legacy `-iterate` is the *only* iterative option on this engine - and it carries exactly these risks. Treat the profile's "iterative" toggle as dev-only with a staleness warning.
- **Archive dir mixing with sources.** Archiving *into* the project (SampleProject → `Build/`) intermingles the deliverable with packaging resources and inflates the tree - on top of the pipeline's ~4 copies. Default the output **outside** the project.
- **Shipping ≠ store-ready by itself.** Shipping config strips console/stats/profiling and is right for release, but some stores also want `-distribution` and `-prereqs`. For a typical **Steam Win64** build, Shipping + pak + prereqs is the norm; `-distribution` is more a console/mobile requirement (verify per store).
- **`-prereqs` matters when shipping to others.** Without it, end users missing the VC++ redist hit cryptic launch failures.
- **IoStore = `.utoc`/`.ucas`, not `.pak`.** With `-iostore` you get a small `.pak` **plus** `.ucas`/`.utoc` (+ `global.*`). The [footprint scanner](build-footprint.md) must recognize `.utoc`/`.ucas` as cooked output, not just `.pak`. Whether `-iostore` is on is governed by the `bUseIoStore` **project setting** - read it, don't assume.
- **UE5 platform folder rename.** UE5 dropped the `NoEditor` suffix: `Saved/Cooked/Windows/`, `Saved/StagedBuilds/Windows/` (UE4 was `WindowsNoEditor`). Old blogs showing `WindowsNoEditor` are UE4-era; structure is identical.

---

## 8. Phase decomposition: separate processes, parallelism, and timing

> Goal: run each phase as its own (separately-timed) step, overlap what can overlap, render the run as a **dynamic Jenkins-style graph**, and record **per-phase timing** the [Dashboard](user-experience.md#3-dashboard) can query. Phases come from an **extensible registry** (next) - the graph, executor, editor, history, and footprint map all derive from it, so adding a phase is a registry entry, not a rewrite. The feasibility check is the important part - the intuitive *"run build and cook in parallel"* needs one correction before it's right.

### The phase registry (extensibility seam)

The pipeline is **data, not hardcode.** The `pipeline` module holds a **registry** of phase definitions; the executor, the [Jenkins-style graph](user-experience.md#7-build-logs-window), the [profile editor](#5-what-to-expose-vs-auto-manage-vs-hide), the [timing record](data-storage.md#per-phase-timing-build_phases), and the [footprint map](build-footprint.md#build-step--footprint-mapping) all **derive from it**. Adding a phase = adding one entry; nothing else changes. Each entry declares roughly:

| Field | Purpose |
|---|---|
| `id` / `label` | stable key + display name (e.g. `cook` / "Cook") |
| `order` | canonical pipeline position (what the graph and record sort by - replaces a stored `ordinal`) |
| `dependsOn` | DAG edges (e.g. `stage → [build, cook]`). The editor build is an *implicit* prerequisite of Cook, enforced in the arg builder / planner - not a registry edge (Cook's `dependsOn` is empty). |
| `requiredness` | `always` \| `forCpp` \| `ifDependedOn` \| `optional` → drives the locked/greyed toggle |
| `kind` | `external` (spawns a child process: UBT / commandlet / BuildCookRun) or `app` (internal task: fs copy, footprint cleanup) |
| `build(profile, env)` | resolved command (external) or action + preview (app) |
| `configSchema` | the per-phase serde config type shown in the editor |
| `footprintPaths` | dirs this phase creates → feeds Clean-up and the footprint scanner |

**MVP registry:** `build` · `cook` · `stage` · `pak` · `archive` · `copyExtras` · `cleanup` (+ the implicit `editor-build` prerequisite). **Deferred** entries (added later with no executor/UI change): Turnkey SDK verify, dedicated-server, DLC/patch, encryption/signing, extra platforms. That is what *"expandable for future contribution"* means concretely.

**The two app-owned phases (MVP):**
- **Copy Extras** (`kind: app`) - copies a profile list of `{ from, to }` into the final build, where `from` is **project-relative** (file or folder) and `to` is **build-output-relative** (default the build root). E.g. `{ from: "steam_appid.txt", to: "." }` puts the repo-root file at the build root. Runs **after Archive** (the output exists). Plain Rust `std::fs`.
- **Clean-up** (`kind: app`) - the terminal phase: reclaim footprint by category (staging, cooked, intermediates, …) via the [build-footprint rules](build-footprint.md). **Runs on success by default** (keep artifacts to debug a failure); categories are a profile choice. This wires [R3](requirement.md#r3--footprint-management) cleanup into the pipeline.

### The real dependency graph

**Correction to "build ∥ cook":** cook does **not** depend on the game build - it depends on the **editor** build. Cooking *is* the editor running a commandlet (`UnrealEditor-Cmd -run=Cook`), so for a C++ project the **editor target (Win64 Development)** must be compiled first. The **game/client target** (e.g. `SampleProjectSteam` Shipping) is what gets **staged**, not what cooks. So:

```
BuildEditor (Win64 Dev) ──────────► Cook ─────────────┐
   (needed to cook; often already built)               │
                                                        ▼
BuildGameTarget ───────────────► Stage ─► Pak ─► Archive ─► Copy Extras ─► Clean-up
   (the exe that ships)          └─── one BuildCookRun ───┘    └─ app-owned tasks ─┘
```

- Edges: **BuildEditor → Cook** · **BuildGameTarget → Stage** · **Cook → Stage** · **Stage → Pak → Archive → Copy Extras → Clean-up**. Archive is required; Copy Extras and Clean-up are app-owned and optional.
- The parallelizable pair is therefore **{BuildGameTarget} ∥ {Cook}**, *both after the editor exists* - not "build ∥ cook" in the naive sense.

### What can actually run in parallel

| Pair | Parallel? | Why |
|---|---|---|
| **Build game target ∥ Cook** | ✅ the one worth doing | Disjoint outputs (`Binaries/` vs `Saved/Cooked/`). Compile is CPU-bound, cook is shader/GPU/IO-bound → genuine overlap. Both gated behind the editor build. |
| Build editor ∥ Build game target | ⚠️ marginal | Two UBT compiles in different intermediate dirs, but UBT already saturates all cores → oversubscription, little/negative gain on one box. |
| Stage ∥ Pak ∥ Archive | ❌ never | Pak is produced *inside* staging; archive copies the staged result. Strict chain. |
| Anything ∥ Cook, before the editor is built | ❌ | Cook can't start until the editor binary exists. |
| **Whole pipeline per platform/target** | ✅✅ the real win (future) | `Win64 ∥ Linux ∥ DedicatedServer` are independent DAGs - embarrassingly parallel modulo RAM/disk. MVP is Win64-only, so future scope, but it's where parallelism actually pays. |

**Optimal single-machine schedule:** ① Build editor (skip if already built - the common case, you work in it daily) → ② **Cook ∥ Build game target** → ③ Stage → ④ Pak → ⑤ Archive.

> Reality check for one workstation: the build∥cook overlap is real but **modest** - both phases are already multi-core-hungry, so concurrency mostly trades CPU for IO/RAM pressure. Expect a useful-but-not-dramatic wall-clock win, and make it a **toggle** (some machines thrash). The big multiplier is multi-platform pipelines, not intra-pipeline overlap.

### Parallelism you already get for free (don't re-orchestrate it)

Most CPU parallelism is **intra-phase** and automatic:
- **Build:** UBT compiles across all cores (+ XGE/IncrediBuild/UBA if present).
- **Cook:** spawns a ShaderCompileWorker pool; UE5 also supports **multi-process cook** (`-cookprocesscount=N` / MPCook) to parallelize the cook itself.
- **Pak/IoStore:** multithreaded.

So cross-phase orchestration only adds the single build∥cook overlap on top of parallelism already happening inside each phase.

### Executor: separate processes (the design you asked for)

The `runner` module drives the DAG, **spawning each phase as its own process and wall-clocking it** (`tokio::process` start/end per child - see [tech stack](../CLAUDE.md#tech-stack)). Every phase is also a node in the live **[Jenkins-style pipeline graph](user-experience.md#7-build-logs-window)** (status + elapsed, parallel branches for Build∥Cook), rendered dynamically from the registry:

- **Build** → call **UBT via `Build.bat`** (`<Target> <Platform> <Config> -WaitMutex`), once per needed target - the editor target first (always Win64 Development), then the game target at the profile's platform/config. Bypasses RunUAT's bootstrap → fast and cleanly timed.
- **Cook** → editor commandlet directly (`UnrealEditor-Cmd <proj> -run=Cook -targetplatform=Windows …`) or `BuildCookRun -skipbuild -cook -skipstage` (**`-skipstage` is required** - without it BuildCookRun falls through into Stage, which under `-skipbuild` and racing the concurrent Build unit dies with "Missing receipt …").
- **Stage + Pak + Archive** → **one** `BuildCookRun -skipbuild -skipcook -stage -pak -archive`, run and timed as a **single node** (the three registry phases collapse into one execution unit; no banner parsing). Pak isn't a separable step - UnrealPak runs *inside* staging (`-pak` changes what Stage emits, sharing the manifest + IoStore/Zen layout) - and the chain is strictly sequential, so a single process loses **no** parallelism while avoiding per-invocation re-bootstrap cost and the [IoStore/Zen flag-sync footgun](#7-gotchas--footguns). **Decided: all three run in this one process** (three registry phases → one execution unit). Archive is technically just a copy of the finalized `StagedBuilds/<Platform>/`, but keeping it in the same call is zero-risk and avoids extra orchestration. **Each of Stage/Pak/Archive also has a per-phase additional-args string; since they share this one command, the three strings are concatenated into it** (Stage→Pak→Archive order).
- **Copy Extras / Clean-up** → **app-owned tasks** (no child process): Copy Extras via Rust `std::fs` into the archive output; Clean-up via the footprint module. Still timed and shown as graph nodes like any phase.

**Source-engine caveat (your engine):** each separate `RunUAT` invocation re-bootstraps and may recompile UAT (see [§1](#1-the-toolchain-what-actually-runs)). Mitigate by compiling UAT once up front then passing `-nocompile`/`-nocompileuat`, and by calling UBT / the commandlet directly for Build/Cook (no RunUAT in the hot path). You also take on the skip-flag/path correctness UAT normally guarantees - encode it in the [arg-builder rules](#6-arg-builder-rules-for-the-unreal-module).

**Alternative executor (same data model):** the whole DAG can also run as a single integrated `BuildCookRun -build -cook -stage -pak -archive`, with per-phase timings parsed from UAT's banners (`********** … COMMAND STARTED|COMPLETED **********` + elapsed lines, via the [R4 log parser](requirement.md#r4--build-process--logs)). **MVP uses the separate-process executor above** (the Jenkins-style pipeline you want); the integrated mode is the fallback - and the Stage+Pak+Archive group is in fact already a single integrated `BuildCookRun` call (timed as one node, not banner-parsed). Either way the **timing data model is identical**, so the executor is swappable without touching the graph or Dashboard. *(Confirm the exact banner strings against a real SampleProject `Saved/Logs` capture.)*

### Per-phase timing → the record

Record one entry per **execution unit** the run emitted - **Build · Cook · Stage · Pak · Archive · Copy Extras · Clean-up** in the registry's `order`, except **Stage / Pak / Archive collapse into a single recorded entry** (`Stage · Pak · Archive`) because they run as one `BuildCookRun` unit - so the set is open-ended, not a fixed count. A phase the user **disabled** in the profile emits no unit and produces **no record** at all. Per unit record **start offset (s from build start) + duration (s) + status**; offset+duration reconstructs overlap (a Gantt view) and trends without a redundant end timestamp (mirrors how the main record keeps `duration`, not an end time). The `"Skipped"` status is recorded only for a phase that was **scheduled but bypassed by an upstream failure or cancel** - not for profile-disabled phases.

**Important:** with overlap, **wall-clock total ≠ Σ phase durations**. Store both - the wall-clock total on the build record, the breakdown per phase. Schema + queries → [data-storage.md](data-storage.md#per-phase-timing-build_phases).

---

## 9. Plugin packaging - `RunUAT BuildPlugin`

Packaging a **plugin** is a different command from packaging a game - not the `BuildCookRun` pipeline above, but a single standalone build:

```
<engine>\Engine\Build\BatchFiles\RunUAT.bat BuildPlugin ^
  -plugin="<path>\MyPlugin.uplugin" ^
  -package="<out>\MyPlugin" ^
  -rocket
```

This is what UnrealEasyPackage's **Actions → Package Plugin** runs (R7). Key facts (verified against Epic dev-community threads + the binary-plugin community references - sources below):

- **No host `.uproject` needed.** `BuildPlugin` compiles the plugin **standalone**: it spins up a *temporary throwaway host project*, compiles the plugin's modules against the chosen engine, and writes a full redistributable plugin (source + Binaries + Intermediate + content) to `-package`. The plugin does **not** have to sit inside a game project - only the `.uplugin` path + an engine are required. (Our reference plugin lives under a project's `Plugins/`, but `BuildPlugin` ignores that project.)
- **`-rocket` = installed-engine packaging mode.** "Rocket" is Epic's old codename for the binary/installed engine (the launcher build). `-rocket` forces the installed-engine packaging path - treat the engine as read-only, emit the redistributable/FAB-submittable shape (`"Installed": true`). It's effectively the older alias of `-installed`, and is emitted **regardless of source vs launcher engine** (our reference engines are source builds; `-rocket` still produces the clean redistributable). There is **no official UAT flag reference** - Epic publishes none - so precise effects come from community + engine automation source.
- **Build configurations are not a knob.** Unlike `BuildCookRun` you do **not** pass `-clientconfig`. `BuildPlugin` compiles a fixed set so the binary works in any consuming project: **Development + Shipping** for game/runtime modules, **DevelopmentEditor** for editor modules. (`DebugGame` is not produced and has no reliable BuildPlugin arg.) So our profile/config machinery does **not** apply here.
- **Dependencies resolve against the engine only.** A standalone build succeeds only if every module/plugin dependency is satisfied by the engine itself (engine modules, or plugins physically in `Engine/Plugins`). `BuildPlugin` does **not** pull dependencies from a host project. A plugin depending on a **non-engine** plugin is officially unsupported out of the box; the fix is to install the dependency into `Engine/Plugins` first. (Engine-shipped deps like `GameplayAbilities` resolve automatically.)
- **Output is engine-version-locked.** A binary plugin only loads on the engine version it was compiled with (point releases compatible) - which is why the engine picker matters and why one packages once per target UE version.
- **FAB strip.** FAB submission requires the uploaded plugin carry **no compiled output** - so the Package action optionally deletes `<package>/Binaries` and `<package>/Intermediate` after a successful build (default on, R7).

**Implementation:** the command is built by `unreal::args::build_plugin_command` (pure, tested); the engine list comes from `unreal::engine::enumerate_registry_engines`, and per-plugin machine-local settings (remembered engines + output folder + folder name) live in one plain JSON beside the `.uplugin` - `<plugin_root>/.uap/settings.json` (git-ignored, separate from the engine-side `.uep/`); the run reuses the M3 runner substrate (`runner::exec::spawn_plugin_package` - same `uep://run-*` stream, `ProcGroup` tree-kill, and Cancel as a build) but writes **no** build-history record and carries no `PhaseId` graph.

**Plugin sources** - [Epic Dev Community: *How to package a plugin dependent on another plugin?*](https://forums.unrealengine.com/t/how-to-package-a-plugin-dependent-on-another-plugin/438160) (temp dummy project; install deps into `Engine/Plugins`) · [*Plugin dependent on another plugin*](https://forums.unrealengine.com/t/plugin-dependent-on-another-plugin/387324) · [*How to package plugin for all build configurations?*](https://forums.unrealengine.com/t/how-to-package-plugin-for-all-build-configurations/1145730) (Dev+Shipping hardcoded) · [*What does -game and -rocket mean in UnrealBuildTool?*](https://forums.unrealengine.com/t/what-does-game-and-rocket-flag-means-in-unrealbuildtool-and-where-can-i-find-the-help-document/387374) · [Mercuna - Building binary plugins in UE4](https://mercuna.com/building-binary-plugins-in-unreal-engine-4/) (version coupling) · [Installed Build Reference Guide](https://dev.epicgames.com/documentation/unreal-engine/installed-build-reference-guide-for-unreal-engine) (installed = formerly "Rocket").

## 10. Verification & sources

**Ground truth for a specific engine:** `RunUAT BuildCookRun -help`. For UnrealEasyPackage, a sound approach is to shell that out once per detected engine and parse it - it's the only source guaranteed correct for that exact build (critical for the custom source engine). Where this doc and `-help` disagree, **`-help` wins**.

Cross-checked against (authority order):

- **Epic official docs** - [Build Operations: Cooking, Packaging, Deploying, Running](https://dev.epicgames.com/documentation/unreal-engine/build-operations-cooking-packaging-deploying-and-running-projects-in-unreal-engine) · [Packaging Projects](https://dev.epicgames.com/documentation/unreal-engine/packaging-your-project) · [Cooking Content](https://dev.epicgames.com/documentation/unreal-engine/cooking-content-in-unreal-engine) · [Build Configurations Reference](https://dev.epicgames.com/documentation/unreal-engine/build-configurations-reference-for-unreal-engine) · [Using the Project Launcher](https://dev.epicgames.com/documentation/unreal-engine/using-the-project-launcher-in-unreal-engine) · [Oodle Data](https://dev.epicgames.com/documentation/unreal-engine/oodle-data) · [Zen Storage as Cooked Output](https://dev.epicgames.com/documentation/unreal-engine/using-zen-storage-server-as-cooked-output-store-for-unreal-engine) · [Create a Patch](https://dev.epicgames.com/documentation/unreal-engine/how-to-create-a-patch-in-unreal-engine) · [UE5 Migration Guide](https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5-migration-guide).
- **Source-derived community references** - [botman99/ue4-unreal-automation-tool](https://github.com/botman99/ue4-unreal-automation-tool/blob/main/README.md) (mirrors `-help`/`ProjectParams.cs` descriptions; resolves the `Saved/Cooked` + `Saved/StagedBuilds` paths and removable `Manifest_*.txt`) · [ikrima Gamedev Guide - AutomationTool/UBT reference](https://ikrima.dev/ue4guide/build-guide/ubt/automationtool-exe-unrealbuildtool-exe-reference/) · [uetools UAT command docs](https://uetools.readthedocs.io/en/latest/commands/uat/uat.html) (current UE5 flag set incl. IoStore/Zen).
- **Iterative-cook limitations (Epic staff statement)** - [`-iterativecook` UPROPERTY issue](https://forums.unrealengine.com/t/iterativecook-uproperty-issue/2668727) - legacy iterative cook misses class-schema/`UPROPERTY` changes, hidden package deps, and shader/external-file deps; full cook is the fix. Confirms Incremental Cooking is 5.6 (experimental) / 5.7 (beta), **not 5.5**.
- **Real command examples** - [chen3feng/uct](https://github.com/chen3feng/uct) (verbatim UE5 editor/Turnkey-generated `BuildCookRun`) · [Life EXE - Unreal Engine CI](https://medium.com/@lifeexe/unreal-engine-ci-part-02-blueprint-game-build-3ffff59d8bee) (Shipping Win64) · the project's own `Build/build_worker.bat`.
- **No official exhaustive flag list exists** - see the Epic forum thread [RunUAT BuildCookRun full command line options documentation?](https://forums.unrealengine.com/t/runuat-buildcookrun-full-command-line-options-documentation/128809). Hence the `-help` rule.

**Flagged uncertainties (verify against `-help` for the target engine):** exact phase defaults; whether `-iostore`/Zen are on (read `ProjectPackagingSettings`); `-cookflavor` as a literal switch name (flavor is often encoded in the platform name); `-distribution` necessity for plain Steam Win64; several advanced DLC/patch/IoStore sub-flags.
