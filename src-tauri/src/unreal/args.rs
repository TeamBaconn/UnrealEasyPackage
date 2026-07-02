//! The **BuildCookRun arg builder** + output-folder token render.
//!
//! Turns a `BuildConfig` + detected environment into the **per-phase resolved
//! commands** the editor previews (read-only) and the runner (M3) spawns. The
//! pipeline is decomposed into separate processes (`docs/build-commands.md` §8):
//!
//! - **Build** → UBT via `Build.bat <Target> <Platform> <Config>`, once per
//!   needed target (editor first when C++, then the game target).
//! - **Cook** → `RunUAT BuildCookRun -skipbuild -cook …`.
//! - **Stage · Pak · Archive** → one `RunUAT BuildCookRun -skipbuild -skipcook
//!   -stage [-pak …] -archive` (three registry phases, one execution unit - Pak
//!   runs *inside* staging, the chain is strictly sequential).
//! - **Copy Extras / Clean-up** → app-owned tasks (no child process): an action
//!   + human-readable preview.
//!
//! Dependency/lock rules (§6) are encoded **here in code**, never trusted from
//! the UI toggles: I/O Store ⇒ Pak; boilerplate always emitted; `-clientconfig`
//! never `-configuration`; multi-values joined with `+`; source vs installed
//! engine decides the UAT compile flags.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::engine::{EngineInfo, EngineKind};
use super::uproject::ProjectType;
use crate::pipeline::{PhaseId, PhaseKind};
use crate::profiles::schema::{BuildConfig, CookMaps, IncrementalCookMode};

/// Detected environment the builder needs. Borrowed - the command layer owns the
/// detected project and supplies the resolved `target`/`editor_target` plus the
/// `date`/`time` token values (so the builder itself stays clock-free and
/// testable).
pub struct BuildEnv<'a> {
    pub uproject_path: &'a str,
    pub project_name: &'a str,
    pub engine: &'a EngineInfo,
    pub project_type: ProjectType,
    /// Resolved game target (profile's `target`, else the detected one).
    pub target: &'a str,
    /// Resolved editor target - drives the implicit editor build for C++ projects.
    pub editor_target: Option<&'a str>,
    /// `{date}` token, `YYYYMMDD` (resolved by the caller).
    pub date: &'a str,
    /// `{time}` token, `HHMMSS` (resolved by the caller).
    pub time: &'a str,
    /// Absolute path to the user's `steamcmd.exe` (machine-local; empty ⇒ the Steam upload
    /// phase can't run). Only the Steam upload phase reads this.
    pub steamcmd_path: &'a str,
    /// Steam build account for the upload's `+login` (machine-local; empty ⇒ not logged in).
    pub steam_account: &'a str,
}

/// One resolved execution unit. `phase` is its primary registry phase (the
/// Stage·Pak·Archive unit reports `Stage`); `External` units carry a `program` +
/// `args` to spawn, `App` units carry only the `preview` text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PhaseCommand {
    pub phase: PhaseId,
    pub label: String,
    pub kind: PhaseKind,
    /// External only - the executable/batch file to run.
    pub program: Option<String>,
    /// External only - the argument vector (excludes `program`).
    pub args: Vec<String>,
    /// The read-only display string: the full command line, or the app action.
    pub preview: String,
}

/// Token values for the output folder-name template.
pub struct TokenContext<'a> {
    pub project: &'a str,
    pub platform: &'a str,
    pub config: &'a str,
    pub profile: &'a str,
    pub target: &'a str,
    pub date: &'a str,
    pub time: &'a str,
}

/// Render an output folder name from the template
/// (`docs/requirement.md` R1): `{project} {platform} {config} {profile} {target}
/// {date} {time}`, rendered **lowercase**. Unknown tokens are left verbatim.
///
/// `{project}-{platform}-{config}-{date}` ⇒ `sampleproject-windows-development-20260603`.
pub fn render_folder_name(template: &str, ctx: &TokenContext) -> String {
    template
        .replace("{project}", ctx.project)
        .replace("{platform}", ctx.platform)
        .replace("{config}", ctx.config)
        .replace("{profile}", ctx.profile)
        .replace("{target}", ctx.target)
        .replace("{date}", ctx.date)
        .replace("{time}", ctx.time)
        .to_lowercase()
}

/// Resolve a stored path against `root`: an **absolute** path is used verbatim; a
/// **relative (local)** path has a leading `./` or `.\` stripped and is joined under
/// `root` (a bare `.` or an empty relative resolves to `root` itself). Shared by the
/// archive base-dir resolution here and the machine-local path resolver in `commands`,
/// so the two can never diverge on how a project-relative path is anchored.
pub fn resolve_under_root(root: &Path, stored: &str) -> std::path::PathBuf {
    let s = stored.trim();
    if Path::new(s).is_absolute() {
        return Path::new(s).to_path_buf();
    }
    let rel = s.strip_prefix("./").or_else(|| s.strip_prefix(".\\")).unwrap_or(s);
    if rel.is_empty() || rel == "." {
        root.to_path_buf()
    } else {
        root.join(rel)
    }
}

/// The full resolved archive directory: `<base> / <rendered folder>`. A **relative
/// (local)** base dir is resolved against the project root (the `.uproject`'s
/// directory); an **absolute (direct)** base dir is used verbatim.
pub fn resolved_output_dir(profile: &BuildConfig, env: &BuildEnv) -> String {
    // `{config}` joins every staged config (e.g. `development+shipping`), matching the
    // history tags and the `-clientconfig=A+B` list (one source of order - see
    // `BuildConfig::staged_config_join`).
    let config_join = profile.staged_config_join();
    let folder =
        render_folder_name(&profile.output.folder_template, &token_context(profile, env, &config_join));
    let root = Path::new(env.uproject_path).parent().unwrap_or_else(|| Path::new(""));
    let abs_base = resolve_under_root(root, &profile.output.base_dir);
    abs_base.join(folder).display().to_string()
}

fn token_context<'a>(profile: &'a BuildConfig, env: &'a BuildEnv<'a>, config: &'a str) -> TokenContext<'a> {
    TokenContext {
        project: env.project_name,
        platform: profile.platform.folder(),
        config,
        profile: &profile.name,
        target: env.target,
        date: env.date,
        time: env.time,
    }
}

// ── plugin packaging (RunUAT BuildPlugin) ──────────────────────────────────────
//
// A plugin is packaged standalone - no host `.uproject`, no BuildCookRun pipeline:
// one `RunUAT BuildPlugin -plugin=<.uplugin> -package=<dir> -rocket` against a
// chosen engine (`docs/build-commands.md` §9). `-rocket` forces the installed-engine
// packaging path (the redistributable/FAB shape) even on a source-built engine.

/// A resolved standalone command (program + argv + display preview). Plugin
/// packaging doesn't ride the `PhaseId` pipeline, so it carries no phase tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginCommand {
    pub program: String,
    pub args: Vec<String>,
    pub preview: String,
}

/// Token values for a plugin's output folder-name template.
pub struct PluginTokenContext<'a> {
    pub plugin: &'a str,
    pub version: &'a str,
    pub engine: &'a str,
    pub date: &'a str,
    pub time: &'a str,
}

/// Render a plugin package folder name from its template. Tokens: `{plugin}`
/// `{version}` `{engine}` `{date}` `{time}`. Unlike project archive folders this is
/// **case-preserving** - a plugin's distributable folder is conventionally its own
/// (often PascalCase) name (e.g. the reference `…/FAB/SamplePlugin`). Unknown tokens
/// are left verbatim; an empty template falls back to `{plugin}`.
pub fn render_plugin_folder(template: &str, ctx: &PluginTokenContext) -> String {
    let t = if template.trim().is_empty() { "{plugin}" } else { template };
    t.replace("{plugin}", ctx.plugin)
        .replace("{version}", ctx.version)
        .replace("{engine}", ctx.engine)
        .replace("{date}", ctx.date)
        .replace("{time}", ctx.time)
}

/// The full resolved package directory: `<base> / <rendered folder>`. `base_dir` is
/// the user-picked output folder (used verbatim); an empty base resolves to just the
/// rendered folder.
pub fn resolved_plugin_output_dir(base_dir: &str, template: &str, ctx: &PluginTokenContext) -> String {
    let folder = render_plugin_folder(template, ctx);
    let base = base_dir.trim();
    if base.is_empty() {
        folder
    } else {
        Path::new(base).join(folder).display().to_string()
    }
}

/// Build the standalone `RunUAT BuildPlugin` command for a plugin.
pub fn build_plugin_command(engine_root: &Path, uplugin_path: &str, package_dir: &str) -> PluginCommand {
    let program = batch(engine_root, "RunUAT");
    let args = vec![
        "BuildPlugin".to_string(),
        format!("-plugin={uplugin_path}"),
        format!("-package={package_dir}"),
        // Installed-engine packaging mode → the redistributable / FAB-submittable
        // shape. Emitted regardless of source vs launcher engine.
        "-rocket".to_string(),
    ];
    let preview = join_command(&program, &args);
    PluginCommand { program, args, preview }
}

// ── editor commandlet tools (UnrealEditor-Cmd -run=…) ───────────────────────────
//
// Project-side maintenance actions that ride the **editor commandlet** runner rather
// than UAT: **Resave** (`-run=ResavePackages` - bakes in the project's Core Redirects,
// fixes up object redirectors, re-serializes assets) and **Validate**
// (`-run=DataValidation` - runs the project's enabled asset validators). Each is a
// single child process - the project's own detected engine editor, no host pipeline,
// no history record. Confirmed flags only (Epic docs / engine source).

/// A resolved standalone editor-commandlet command (program + argv + preview). Like
/// [`PluginCommand`] it doesn't ride the `PhaseId` pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandletCommand {
    pub program: String,
    pub args: Vec<String>,
    pub preview: String,
}

/// Headless boilerplate every commandlet run carries: no dialogs / splash / end-of-run
/// pause, and the full log streamed to stdout so the runner's pump captures every line.
fn commandlet_boilerplate() -> Vec<String> {
    ["-unattended", "-nopause", "-nosplash", "-stdout", "-fullstdoutlogoutput", "-utf8output"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// The engine's editor **command-line** binary: `Engine/Binaries/<plat>/<stem>[.exe]`.
/// UE5 ships `UnrealEditor-Cmd`; UE4 shipped `UE4Editor-Cmd`. Windows-first (the MVP),
/// with Mac/Linux paths for completeness. A source build's Development editor uses these
/// same names (other configs get a suffix, but Development is the default built config).
fn editor_cmd(engine: &EngineInfo) -> String {
    let stem = if engine.version.major >= 5 { "UnrealEditor-Cmd" } else { "UE4Editor-Cmd" };
    let (sub, ext) = if cfg!(windows) {
        ("Win64", ".exe")
    } else if cfg!(target_os = "macos") {
        ("Mac", "")
    } else {
        ("Linux", "")
    };
    engine
        .root
        .join("Engine/Binaries")
        .join(sub)
        .join(format!("{stem}{ext}"))
        .display()
        .to_string()
}

/// `UnrealEditor-Cmd <uproject> -run=ResavePackages …`, resaving the whole project.
/// Perforce-free (`-IgnoreChangelist`, required off Perforce). `project_only` adds
/// `-ProjectOnly` (skip the engine's own content - `NORMALIZE_ExcludeEnginePackages`);
/// `fixup_redirectors` adds `-fixupredirects` (rewrite refs off object redirectors, then
/// delete them); `skip_shader_compile` adds `-NoShaderCompile` (much faster).
pub fn build_resave_command(
    engine: &EngineInfo,
    uproject_path: &str,
    project_only: bool,
    fixup_redirectors: bool,
    skip_shader_compile: bool,
) -> CommandletCommand {
    let program = editor_cmd(engine);
    let mut args = vec![
        uproject_path.to_string(),
        "-run=ResavePackages".to_string(),
        "-IgnoreChangelist".to_string(),
    ];
    if project_only {
        args.push("-ProjectOnly".to_string());
    }
    if fixup_redirectors {
        args.push("-fixupredirects".to_string());
    }
    if skip_shader_compile {
        args.push("-NoShaderCompile".to_string());
    }
    args.extend(commandlet_boilerplate());
    let preview = join_command(&program, &args);
    CommandletCommand { program, args, preview }
}

/// `UnrealEditor-Cmd <uproject> -run=DataValidation …` - runs the project's enabled asset
/// validators. Two confirmed commandlet knobs (from `DataValidationCommandlet.cpp`):
/// `include_engine` adds `-includeengine` (default is project-only - `/Engine` content is
/// stripped); a non-empty `asset_type` adds `-AssetType=<class>` (validate only that class
/// and its subclasses; a short name like `StaticMesh` or a full `/Script/...` path). The
/// commandlet has no path/folder filter.
pub fn build_validate_command(
    engine: &EngineInfo,
    uproject_path: &str,
    include_engine: bool,
    asset_type: &str,
) -> CommandletCommand {
    let program = editor_cmd(engine);
    let mut args = vec![uproject_path.to_string(), "-run=DataValidation".to_string()];
    if include_engine {
        args.push("-includeengine".to_string());
    }
    let asset_type = asset_type.trim();
    if !asset_type.is_empty() {
        args.push(format!("-AssetType={asset_type}"));
    }
    args.extend(commandlet_boilerplate());
    let preview = join_command(&program, &args);
    CommandletCommand { program, args, preview }
}

/// Build the ordered per-phase commands for a profile. The order mirrors the
/// pipeline registry: (editor build) → build → cook → stage·pak·archive →
/// (copy extras) → (clean-up).
pub fn build_commands(profile: &BuildConfig, env: &BuildEnv) -> Vec<PhaseCommand> {
    let mut out = Vec::new();
    let uat_exe = batch(&env.engine.root, "RunUAT");
    let build_exe = batch(&env.engine.root, "Build");
    let project_arg = format!("-Project={}", env.uproject_path);

    // Steam upload pushes the archived build, so it's gated on Archive (registry
    // gated_by = [Archive]); Archive only runs inside the Stage·Pak·Archive unit, so Stage
    // must be on too. Enforce that gate here so the upload (and its sign-in preflight) never
    // run against an output dir that was never archived (which would push a stale build).
    let steam_ready = profile.phases.steam_upload.enabled
        && profile.phases.stage.enabled
        && profile.phases.archive.enabled;

    // ── Steam login preflight - emitted first (before Build) when the upload phase is on, so an
    //    interactive sign-in (if needed) happens up front, not after a finished build. The
    //    runner checks the cached session and only opens a console when necessary; these args
    //    (`+login <account> +quit`) are the interactive-login command it runs in that case. ──
    if steam_ready {
        out.push(external(
            PhaseId::SteamLogin,
            "Steam Login",
            env.steamcmd_path,
            vec!["+login".to_string(), env.steam_account.to_string(), "+quit".to_string()],
        ));
    }

    // ── Build: editor first (C++ needs a built editor to cook with), then game.
    //    Skipped when the Build phase is off (downstream keeps -skipbuild and
    //    assumes current binaries). ──
    if profile.phases.build.enabled {
        if env.project_type == ProjectType::Cpp {
            if let Some(editor) = env.editor_target {
                out.push(external(
                    PhaseId::Build,
                    "Build (Editor)",
                    &build_exe,
                    vec![
                        editor.to_string(),
                        "Win64".to_string(), // the editor is always a Win64 Development host
                        "Development".to_string(),
                        project_arg.clone(),
                        "-WaitMutex".to_string(),
                    ],
                ));
            }
        }

        // One game build per config (primary + extras): each produces its own exe,
        // all later staged against the single cook. `-clean` / additional args are
        // shared, applied to each. Single-config keeps the bare "Build" label so the
        // emitted plan is byte-identical to before multi-config existed.
        let configs = profile.staged_configs();
        let multi = configs.len() > 1;
        for cfg in &configs {
            let mut game = vec![
                env.target.to_string(),
                profile.platform.uat().to_string(),
                cfg.as_str().to_string(),
                project_arg.clone(),
                "-WaitMutex".to_string(),
            ];
            if profile.phases.build.clean {
                game.push("-clean".to_string());
            }
            game.extend(split_args(&profile.phases.build.additional_args));
            let label = if multi {
                format!("Build ({})", cfg.as_str())
            } else {
                "Build".to_string()
            };
            out.push(external(PhaseId::Build, &label, &build_exe, game));
        }
    }

    // ── Cook (BuildCookRun -skipbuild -cook). Skipped when Cook is off; downstream
    //    keeps -skipcook and reuses the existing cook. ──
    if profile.phases.cook.enabled {
        let mut cook = vec![
            "BuildCookRun".to_string(),
            format!("-project={}", env.uproject_path),
            format!("-target={}", env.target),
            format!("-platform={}", profile.platform.uat()),
            // Cook is config-agnostic (one cook serves every staged config); pass the first.
            format!("-clientconfig={}", profile.staged_configs()[0].as_str()),
            "-skipbuild".to_string(),
            "-cook".to_string(),
            // Cook-only: without this BuildCookRun falls through into Stage, which
            // (under -skipbuild, and racing the concurrent Build unit) dies with
            // "Missing receipt …". Staging is the Stage·Pak·Archive unit's job.
            "-skipstage".to_string(),
        ];
        match &profile.phases.cook.maps {
            CookMaps::All => cook.push("-allmaps".to_string()),
            CookMaps::List(maps) if !maps.is_empty() => cook.push(format!("-map={}", maps.join("+"))),
            CookMaps::List(_) => {} // empty list ⇒ no map flag
        }
        if !profile.phases.cook.cultures.is_empty() {
            cook.push(format!("-cookcultures={}", profile.phases.cook.cultures.join("+")));
        }
        // Incremental cook - suppressed when a clean build is forced (mutually exclusive).
        if !profile.phases.build.clean {
            match profile.phases.cook.incremental {
                IncrementalCookMode::ModifiedOnly => cook.push("-iterativecooking".to_string()),
                IncrementalCookMode::ModifiedAndDependencies => cook.push("-cookincremental".to_string()),
                IncrementalCookMode::None => {}
            }
        }
        if profile.phases.cook.skip_editor_content {
            cook.push("-SkipCookingEditorContent".to_string());
        }
        let cooker_opts = profile.phases.cook.additional_options.trim();
        if !cooker_opts.is_empty() {
            cook.push(format!("-AdditionalCookerOptions={cooker_opts}"));
        }
        cook.extend(uat_compile_flags(env.engine.kind));
        cook.extend(nop4(profile));
        cook.extend(boilerplate());
        out.push(external(PhaseId::Cook, "Cook", &uat_exe, cook));
    }

    // Resolved at function scope: used by Stage·Pak·Archive (below) and Copy Extras.
    let out_dir = resolved_output_dir(profile, env);

    // ── Stage · Pak · Archive (one BuildCookRun). Stage gates the unit - Pak and
    //    Archive run inside the staged tree, so the unit is emitted only when Stage
    //    is on, with -pak/-archive included per their toggles. ──
    if profile.phases.stage.enabled {
        let mut spa = vec![
            "BuildCookRun".to_string(),
            format!("-project={}", env.uproject_path),
            format!("-target={}", env.target),
            format!("-platform={}", profile.platform.uat()),
            // Stage every built config against the single cook - UAT copies one exe per
            // config into the same package. Shares `staged_config_join` with the `{config}`
            // output-folder token, so the folder name can never drift from this list.
            format!("-clientconfig={}", profile.staged_config_join()),
            "-skipbuild".to_string(),
            "-skipcook".to_string(),
            "-stage".to_string(),
        ];
        let pak_on = profile.phases.pak.enabled;
        if pak_on {
            spa.push("-pak".to_string());
            if profile.phases.pak.io_store {
                spa.push("-iostore".to_string());
            }
            if profile.phases.pak.compressed {
                spa.push("-compressed".to_string());
            }
            if profile.phases.pak.package {
                spa.push("-package".to_string()); // native distributable
            }
        }
        if !profile.phases.stage.debug_symbols {
            spa.push("-nodebuginfo".to_string());
        }
        if profile.phases.stage.separate_debug_info {
            spa.push("-separatedebuginfo".to_string());
        }
        if profile.phases.stage.prereqs {
            spa.push("-prereqs".to_string());
        }
        if profile.phases.stage.for_distribution {
            spa.push("-distribution".to_string());
        }
        let archive_on = profile.phases.archive.enabled;
        if archive_on {
            spa.push("-archive".to_string());
            spa.push(format!("-archivedirectory={out_dir}"));
        }
        spa.extend(uat_compile_flags(env.engine.kind));
        spa.extend(nop4(profile));
        spa.extend(boilerplate());
        // Per-phase additional args concatenate into the one command (Stage→Pak→Archive).
        spa.extend(split_args(&profile.phases.stage.additional_args));
        if pak_on {
            spa.extend(split_args(&profile.phases.pak.additional_args));
        }
        if archive_on {
            spa.extend(split_args(&profile.phases.archive.additional_args));
        }
        // Label reflects which sub-phases are included.
        let mut parts = vec!["Stage"];
        if pak_on {
            parts.push("Pak");
        }
        if archive_on {
            parts.push("Archive");
        }
        out.push(external(PhaseId::Stage, &parts.join(" · "), &uat_exe, spa));
    }

    // ── Copy Extras (app-owned) ─────────────────────────────────────────────────
    if profile.phases.copy_extras.enabled {
        let items = &profile.phases.copy_extras.items;
        let mut preview = format!("Copy {} item(s) into {out_dir}:", items.len());
        for item in items {
            let dest = if item.to == "." {
                out_dir.clone()
            } else {
                Path::new(&out_dir).join(&item.to).display().to_string()
            };
            preview.push_str(&format!("\n  {} → {}", item.from, dest));
        }
        out.push(app(PhaseId::CopyExtras, "Copy Extras", preview));
    }

    // ── Upload to Steam (steamcmd child) ────────────────────────────────────────
    //   The VDF scripts are materialized into the scratch dir by the runner's pre-step
    //   (`runner::exec`, like Stage's archive-dir clean); this just resolves the steamcmd
    //   invocation. Runs after Copy Extras so any staged extras (e.g. steam_appid.txt) are
    //   in the archived tree before it uploads.
    if steam_ready {
        let project_root = Path::new(env.uproject_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let vdf = crate::steam::run_app_build_vdf_path(&project_root, &profile.id);
        let steam_args = vec![
            "+login".to_string(),
            env.steam_account.to_string(),
            "+run_app_build".to_string(),
            vdf.display().to_string(),
            "+quit".to_string(),
        ];
        out.push(external(PhaseId::SteamUpload, "Upload to Steam", env.steamcmd_path, steam_args));
    }

    // ── Clean-up (app-owned, terminal) ──────────────────────────────────────────
    if profile.phases.cleanup.enabled {
        let cats: Vec<&str> = profile
            .phases
            .cleanup
            .categories
            .iter()
            .map(|c| c.as_str())
            .collect();
        let when = if profile.phases.cleanup.only_on_success {
            "on success"
        } else {
            "always"
        };
        let cats_str = if cats.is_empty() {
            "(no categories selected)".to_string()
        } else {
            cats.join(", ")
        };
        out.push(app(
            PhaseId::Cleanup,
            "Clean-up",
            format!("Reclaim {cats_str} ({when})."),
        ));
    }

    out
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Always-emitted UAT boilerplate (`docs/build-commands.md` §6.8) - the user never
/// chooses these. (`-noP4` is now the Build phase's default-on toggle; see `nop4`.)
fn boilerplate() -> Vec<String> {
    ["-utf8output", "-unattended", "-nosplash"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// `-noP4` when the Build phase's No-Perforce toggle is on (default) - emitted on
/// every `BuildCookRun` (cook + stage/pak/archive). UBT (Build) doesn't take it.
fn nop4(profile: &BuildConfig) -> Vec<String> {
    if profile.phases.build.no_p4 {
        vec!["-noP4".to_string()]
    } else {
        Vec::new()
    }
}

/// Split a free-text additional-args string into argv tokens (whitespace-split;
/// verbatim escape hatch - the user owns quoting correctness).
fn split_args(s: &str) -> Vec<String> {
    s.split_whitespace().map(|t| t.to_string()).collect()
}

/// UAT self-compile control by engine type (§6.3): an installed engine can't
/// self-compile UAT (`-nocompile`); a source build is left to compile it.
fn uat_compile_flags(kind: EngineKind) -> Vec<String> {
    match kind {
        EngineKind::Launcher => vec!["-nocompile".to_string(), "-nocompileuat".to_string()],
        EngineKind::Source => Vec::new(),
    }
}

/// `<root>/Engine/Build/BatchFiles/<name>.{bat|sh}`.
fn batch(root: &Path, name: &str) -> String {
    let ext = if cfg!(windows) { "bat" } else { "sh" };
    root.join("Engine/Build/BatchFiles")
        .join(format!("{name}.{ext}"))
        .display()
        .to_string()
}

fn external(phase: PhaseId, label: &str, program: &str, args: Vec<String>) -> PhaseCommand {
    let preview = join_command(program, &args);
    PhaseCommand {
        phase,
        label: label.to_string(),
        kind: PhaseKind::External,
        program: Some(program.to_string()),
        args,
        preview,
    }
}

fn app(phase: PhaseId, label: &str, preview: String) -> PhaseCommand {
    PhaseCommand {
        phase,
        label: label.to_string(),
        kind: PhaseKind::App,
        program: None,
        args: Vec::new(),
        preview,
    }
}

/// Render a program + argv into a display string, quoting tokens with spaces.
fn join_command(program: &str, args: &[String]) -> String {
    let mut s = quote(program);
    for a in args {
        s.push(' ');
        s.push_str(&quote(a));
    }
    s
}

fn quote(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::schema::{Configuration, CookMaps, IncrementalCookMode, Platform};
    use std::path::PathBuf;

    fn engine(kind: EngineKind) -> EngineInfo {
        EngineInfo {
            root: PathBuf::from("C:/Engine/CustomEngine"),
            version: super::super::engine::EngineVersion { major: 5, minor: 5, patch: 0 },
            kind,
        }
    }

    fn env<'a>(eng: &'a EngineInfo) -> BuildEnv<'a> {
        BuildEnv {
            uproject_path: "C:/Projects/SampleProject/SampleProject.uproject",
            project_name: "SampleProject",
            engine: eng,
            project_type: ProjectType::Cpp,
            target: "SampleProjectSteam",
            editor_target: Some("SampleProjectEditor"),
            date: "20260603",
            time: "143501",
            steamcmd_path: "C:/steamcmd/steamcmd.exe",
            steam_account: "builder_bot",
        }
    }

    /// A profile resembling the reference `build_worker.bat` (Steam, Development,
    /// pak on).
    fn dev_profile() -> BuildConfig {
        let mut p = BuildConfig::default();
        p.id = "dev".into();
        p.name = "Development".into();
        p.platform = Platform::Win64;
        p.configs = vec![Configuration::Development];
        p.target = Some("SampleProjectSteam".into());
        p.output.base_dir = "C:/Builds".into();
        p
    }

    fn cmd<'a>(cmds: &'a [PhaseCommand], label: &str) -> &'a PhaseCommand {
        cmds.iter().find(|c| c.label == label).unwrap_or_else(|| panic!("no phase {label}"))
    }

    // ── token render ────────────────────────────────────────────────────────────

    #[test]
    fn renders_folder_lowercase_with_platform_folder_form() {
        let ctx = TokenContext {
            project: "SampleProject",
            platform: Platform::Win64.folder(), // "Windows"
            config: "Development",
            profile: "Dev",
            target: "SampleProjectSteam",
            date: "20260603",
            time: "143501",
        };
        assert_eq!(
            render_folder_name("{project}-{platform}-{config}-{date}", &ctx),
            "sampleproject-windows-development-20260603"
        );
    }

    // ── plugin packaging ─────────────────────────────────────────────────────────

    #[test]
    fn plugin_folder_is_case_preserving_with_tokens() {
        let ctx = PluginTokenContext {
            plugin: "SamplePlugin",
            version: "1.2.0",
            engine: "5.5",
            date: "20260603",
            time: "143501",
        };
        // Case preserved (unlike project archive folders).
        assert_eq!(render_plugin_folder("{plugin}", &ctx), "SamplePlugin");
        assert_eq!(
            render_plugin_folder("{plugin}-{version}-UE{engine}", &ctx),
            "SamplePlugin-1.2.0-UE5.5"
        );
        // Empty template falls back to {plugin}.
        assert_eq!(render_plugin_folder("  ", &ctx), "SamplePlugin");
    }

    #[test]
    fn resolved_plugin_dir_joins_base_and_folder() {
        let ctx = PluginTokenContext {
            plugin: "SamplePlugin", version: "1.2.0", engine: "5.5", date: "d", time: "t",
        };
        let out = resolved_plugin_output_dir("C:/FAB", "{plugin}", &ctx).replace('\\', "/");
        assert_eq!(out, "C:/FAB/SamplePlugin");
    }

    #[test]
    fn build_plugin_command_is_buildplugin_rocket() {
        let engine = PathBuf::from("C:/Engine/CustomEngine");
        let cmd = build_plugin_command(
            &engine,
            "C:/Plugins/SamplePlugin/SamplePlugin.uplugin",
            "C:/FAB/SamplePlugin",
        );
        assert!(cmd.program.replace('\\', "/").ends_with("Engine/Build/BatchFiles/RunUAT.bat") || cmd.program.replace('\\', "/").ends_with("Engine/Build/BatchFiles/RunUAT.sh"));
        assert_eq!(cmd.args[0], "BuildPlugin");
        assert!(cmd.args.contains(&"-plugin=C:/Plugins/SamplePlugin/SamplePlugin.uplugin".to_string()));
        assert!(cmd.args.contains(&"-package=C:/FAB/SamplePlugin".to_string()));
        assert!(cmd.args.contains(&"-rocket".to_string()));
        assert!(cmd.preview.contains("BuildPlugin"));
    }

    // ── editor commandlet tools ───────────────────────────────────────────────────

    #[test]
    fn resave_command_is_project_scoped_with_confirmed_flags() {
        let eng = engine(EngineKind::Source);
        let cmd = build_resave_command(&eng, "C:/Projects/SampleProject/SampleProject.uproject", true, true, true);
        let prog = cmd.program.replace('\\', "/");
        assert!(prog.contains("Engine/Binaries/") && prog.contains("UnrealEditor-Cmd"), "got {prog}");
        // the .uproject is the first positional token
        assert_eq!(cmd.args[0], "C:/Projects/SampleProject/SampleProject.uproject");
        for f in ["-run=ResavePackages", "-ProjectOnly", "-IgnoreChangelist", "-fixupredirects", "-NoShaderCompile", "-unattended", "-stdout"] {
            assert!(cmd.args.iter().any(|a| a == f), "resave missing {f}");
        }
        // no folder filter ⇒ whole project
        assert!(!cmd.args.iter().any(|a| a.starts_with("-PackageFolder")));
    }

    #[test]
    fn resave_toggles_off_drop_their_flags() {
        let eng = engine(EngineKind::Source);
        let cmd = build_resave_command(&eng, "C:/Projects/SampleProject/SampleProject.uproject", false, false, false);
        // every toggle off ⇒ none of the optional flags, but the run + changelist guard remain
        for f in ["-ProjectOnly", "-fixupredirects", "-NoShaderCompile"] {
            assert!(!cmd.args.iter().any(|a| a == f), "should have dropped {f}");
        }
        assert!(cmd.args.contains(&"-run=ResavePackages".to_string()));
        assert!(cmd.args.contains(&"-IgnoreChangelist".to_string()));
    }

    #[test]
    fn validate_command_runs_datavalidation() {
        let eng = engine(EngineKind::Source);
        // default: project-only (no -includeengine), all types (no -AssetType)
        let cmd = build_validate_command(&eng, "C:/Projects/SampleProject/SampleProject.uproject", false, "");
        assert_eq!(cmd.args[0], "C:/Projects/SampleProject/SampleProject.uproject");
        assert!(cmd.args.contains(&"-run=DataValidation".to_string()));
        assert!(cmd.args.contains(&"-unattended".to_string()));
        assert!(!cmd.args.iter().any(|a| a == "-includeengine"));
        assert!(!cmd.args.iter().any(|a| a.starts_with("-AssetType")));

        // both knobs on; a blank/whitespace asset type is ignored, a real one is trimmed
        let cmd = build_validate_command(&eng, "C:/Projects/SampleProject/SampleProject.uproject", true, "  StaticMesh ");
        assert!(cmd.args.contains(&"-includeengine".to_string()));
        assert!(cmd.args.contains(&"-AssetType=StaticMesh".to_string()));
    }

    #[test]
    fn editor_cmd_picks_ue4_binary_for_major_4() {
        let mut eng = engine(EngineKind::Source);
        eng.version = super::super::engine::EngineVersion { major: 4, minor: 27, patch: 2 };
        let cmd = build_validate_command(&eng, "C:/Projects/SampleProject/SampleProject.uproject", false, "");
        assert!(cmd.program.replace('\\', "/").contains("UE4Editor-Cmd"), "got {}", cmd.program);
    }

    #[test]
    fn unknown_token_is_left_verbatim() {
        let ctx = TokenContext {
            project: "P", platform: "Windows", config: "Development",
            profile: "x", target: "t", date: "d", time: "ti",
        };
        assert_eq!(render_folder_name("{project}-{unknown}", &ctx), "p-{unknown}");
    }

    #[test]
    fn resolved_output_dir_joins_base_and_folder() {
        let eng = engine(EngineKind::Source);
        let out = resolved_output_dir(&dev_profile(), &env(&eng));
        assert!(out.contains("sampleproject-windows-development-20260603"));
        assert!(out.starts_with("C:")); // absolute base dir, used as-is
    }

    #[test]
    fn relative_base_dir_resolves_under_the_project_root() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.output.base_dir = "Build".into(); // local/relative
        let out = resolved_output_dir(&p, &env(&eng)).replace('\\', "/");
        // env's uproject is C:/Projects/SampleProject/SampleProject.uproject
        assert!(out.contains("Projects/SampleProject/Build/"), "got {out}");
        assert!(out.contains("sampleproject-windows-development"));
    }

    // ── arg builder ──────────────────────────────────────────────────────────────

    #[test]
    fn emits_editor_then_game_build_for_cpp() {
        let eng = engine(EngineKind::Source);
        let cmds = build_commands(&dev_profile(), &env(&eng));
        let builds: Vec<&str> = cmds
            .iter()
            .filter(|c| c.phase == PhaseId::Build)
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(builds, vec!["Build (Editor)", "Build"]);
        // editor build targets the editor in Development; game build the profile target/config
        assert!(cmd(&cmds, "Build (Editor)").args.contains(&"SampleProjectEditor".to_string()));
        let game = cmd(&cmds, "Build");
        assert!(game.args.contains(&"SampleProjectSteam".to_string()));
        assert!(game.args.contains(&"Win64".to_string()));
        assert!(game.args.contains(&"Development".to_string()));
    }

    #[test]
    fn multi_config_builds_each_and_stages_the_joined_list() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.configs = vec![Configuration::DebugGame, Configuration::Shipping];
        p.phases.build.clean = true;
        p.phases.build.additional_args = "-extra".into();
        let cmds = build_commands(&p, &env(&eng));

        // One game build per config (canonical order), each per-config labeled, after editor.
        let builds: Vec<&str> = cmds
            .iter()
            .filter(|c| c.phase == PhaseId::Build)
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(builds, vec!["Build (Editor)", "Build (DebugGame)", "Build (Shipping)"]);

        // Each game build carries its own config positional + the shared clean/extra args.
        for (label, cfg) in [("Build (DebugGame)", "DebugGame"), ("Build (Shipping)", "Shipping")] {
            let g = cmd(&cmds, label);
            assert!(g.args.contains(&cfg.to_string()), "{label} missing {cfg}");
            assert!(g.args.contains(&"-clean".to_string()), "{label} missing -clean");
            assert!(g.args.contains(&"-extra".to_string()), "{label} missing additional arg");
        }

        // Stage joins the whole set; cook uses just the first; never -configuration.
        assert!(cmd(&cmds, "Stage · Pak · Archive")
            .args
            .contains(&"-clientconfig=DebugGame+Shipping".to_string()));
        assert!(cmd(&cmds, "Cook").args.contains(&"-clientconfig=DebugGame".to_string()));
        assert!(!cmds.iter().flat_map(|c| &c.args).any(|a| a == "-configuration"));
    }

    #[test]
    fn staged_config_dedup_in_args() {
        // Out-of-order + duplicate configs collapse to a canonical, deduped set.
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.configs = vec![
            Configuration::Shipping,
            Configuration::Development,
            Configuration::Shipping,
        ];
        let cmds = build_commands(&p, &env(&eng));
        let builds: Vec<&str> = cmds
            .iter()
            .filter(|c| c.phase == PhaseId::Build && c.label != "Build (Editor)")
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(builds, vec!["Build (Development)", "Build (Shipping)"]);
        assert!(cmd(&cmds, "Stage · Pak · Archive")
            .args
            .contains(&"-clientconfig=Development+Shipping".to_string()));
    }

    #[test]
    fn blueprint_project_skips_editor_build() {
        let eng = engine(EngineKind::Source);
        let mut e = env(&eng);
        e.project_type = ProjectType::Blueprint;
        let cmds = build_commands(&dev_profile(), &e);
        assert!(!cmds.iter().any(|c| c.label == "Build (Editor)"));
    }

    #[test]
    fn cook_uses_skipbuild_and_allmaps() {
        let eng = engine(EngineKind::Source);
        let cmds = build_commands(&dev_profile(), &env(&eng));
        let cook = cmd(&cmds, "Cook");
        assert!(cook.args.contains(&"-skipbuild".to_string()));
        assert!(cook.args.contains(&"-cook".to_string()));
        // cook-only: must stop BuildCookRun from continuing into Stage
        assert!(cook.args.contains(&"-skipstage".to_string()));
        assert!(cook.args.contains(&"-allmaps".to_string()));
        assert!(cook.args.contains(&"-clientconfig=Development".to_string()));
        assert!(!cook.args.iter().any(|a| a == "-configuration"), "must never emit -configuration");
    }

    #[test]
    fn map_list_joins_with_plus() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.phases.cook.maps = CookMaps::List(vec!["Entry".into(), "Arena".into()]);
        let cmds = build_commands(&p, &env(&eng));
        assert!(cmd(&cmds, "Cook").args.contains(&"-map=Entry+Arena".to_string()));
        assert!(!cmd(&cmds, "Cook").args.contains(&"-allmaps".to_string()));
    }

    #[test]
    fn incremental_cook_modes_emit_their_flags_and_clean_suppresses_them() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.phases.cook.incremental = IncrementalCookMode::ModifiedOnly;
        let cmds = build_commands(&p, &env(&eng));
        assert!(cmd(&cmds, "Cook").args.contains(&"-iterativecooking".to_string()));

        p.phases.cook.incremental = IncrementalCookMode::ModifiedAndDependencies;
        let cmds = build_commands(&p, &env(&eng));
        assert!(cmd(&cmds, "Cook").args.contains(&"-cookincremental".to_string()));

        // a clean build forces a fresh cook → incremental is dropped, -clean is on the build
        p.phases.build.clean = true;
        let cmds = build_commands(&p, &env(&eng));
        assert!(!cmd(&cmds, "Cook").args.iter().any(|a| a == "-cookincremental"));
        assert!(cmd(&cmds, "Build").args.contains(&"-clean".to_string()));
    }

    #[test]
    fn distribution_separate_debug_and_native_package_flags() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.phases.stage.for_distribution = true;
        p.phases.stage.separate_debug_info = true;
        p.phases.pak.package = true;
        let cmds = build_commands(&p, &env(&eng));
        let spa = cmd(&cmds, "Stage · Pak · Archive");
        for f in ["-distribution", "-separatedebuginfo", "-package"] {
            assert!(spa.args.iter().any(|a| a == f), "stage group missing {f}");
        }
    }

    #[test]
    fn no_perforce_toggle_off_omits_nop4() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.phases.build.no_p4 = false;
        let cmds = build_commands(&p, &env(&eng));
        assert!(!cmd(&cmds, "Cook").args.iter().any(|a| a == "-noP4"));
        assert!(!cmd(&cmds, "Stage · Pak · Archive").args.iter().any(|a| a == "-noP4"));
    }

    #[test]
    fn stage_group_packages_and_archives() {
        let eng = engine(EngineKind::Source);
        let cmds = build_commands(&dev_profile(), &env(&eng));
        let spa = cmd(&cmds, "Stage · Pak · Archive");
        assert_eq!(spa.phase, PhaseId::Stage);
        for f in ["-skipbuild", "-skipcook", "-stage", "-pak", "-iostore", "-compressed", "-archive"] {
            assert!(spa.args.iter().any(|a| a == f), "stage group missing {f}");
        }
        assert!(spa.args.iter().any(|a| a.starts_with("-archivedirectory=")));
        // Single-config (no extras): the stage list is just the primary - unchanged.
        assert!(spa.args.contains(&"-clientconfig=Development".to_string()));
    }

    #[test]
    fn pak_disabled_omits_pak_and_relabels() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.phases.pak.enabled = false;
        p.phases.pak.io_store = true; // io_store is a sub-option - no effect when Pak is off
        let cmds = build_commands(&p, &env(&eng));
        // the stage unit drops Pak from its label + flags (loose files)
        let spa = cmd(&cmds, "Stage · Archive");
        assert!(!spa.args.iter().any(|a| a == "-pak"));
        assert!(!spa.args.iter().any(|a| a == "-iostore"));
        assert!(spa.args.contains(&"-stage".to_string()));
        assert!(spa.args.contains(&"-archive".to_string()));
    }

    #[test]
    fn toggling_phases_off_drops_their_units() {
        let eng = engine(EngineKind::Source);

        // Build off ⇒ no Build unit (cook/stage keep -skipbuild)
        let mut p = dev_profile();
        p.phases.build.enabled = false;
        let cmds = build_commands(&p, &env(&eng));
        assert!(!cmds.iter().any(|c| c.phase == PhaseId::Build));
        assert!(cmds.iter().any(|c| c.phase == PhaseId::Cook));

        // Cook off ⇒ no Cook unit
        let mut p = dev_profile();
        p.phases.cook.enabled = false;
        let cmds = build_commands(&p, &env(&eng));
        assert!(!cmds.iter().any(|c| c.phase == PhaseId::Cook));

        // Stage off ⇒ no Stage·Pak·Archive unit at all (Pak/Archive need Stage)
        let mut p = dev_profile();
        p.phases.stage.enabled = false;
        let cmds = build_commands(&p, &env(&eng));
        assert!(!cmds.iter().any(|c| c.phase == PhaseId::Stage));

        // Archive off (Stage on) ⇒ stage unit runs without -archive
        let mut p = dev_profile();
        p.phases.archive.enabled = false;
        let cmds = build_commands(&p, &env(&eng));
        let spa = cmd(&cmds, "Stage · Pak");
        assert!(spa.args.contains(&"-stage".to_string()));
        assert!(!spa.args.iter().any(|a| a == "-archive"));
        assert!(!spa.args.iter().any(|a| a.starts_with("-archivedirectory")));
    }

    #[test]
    fn shipping_drops_debug_info() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.configs = vec![Configuration::Shipping];
        p.phases.stage.debug_symbols = false;
        let cmds = build_commands(&p, &env(&eng));
        assert!(cmd(&cmds, "Stage · Pak · Archive").args.contains(&"-nodebuginfo".to_string()));
    }

    #[test]
    fn boilerplate_always_present() {
        let eng = engine(EngineKind::Source);
        let cmds = build_commands(&dev_profile(), &env(&eng));
        for c in cmds.iter().filter(|c| c.kind == PhaseKind::External && c.program.as_deref().unwrap().contains("RunUAT")) {
            for f in ["-noP4", "-utf8output", "-unattended", "-nosplash"] {
                assert!(c.args.iter().any(|a| a == f), "{} missing {f}", c.label);
            }
        }
    }

    #[test]
    fn source_engine_omits_nocompile_installed_adds_it() {
        let src = engine(EngineKind::Source);
        let cmds = build_commands(&dev_profile(), &env(&src));
        assert!(!cmd(&cmds, "Cook").args.iter().any(|a| a == "-nocompile"));

        let inst = engine(EngineKind::Launcher);
        let cmds = build_commands(&dev_profile(), &env(&inst));
        assert!(cmd(&cmds, "Cook").args.contains(&"-nocompile".to_string()));
        assert!(cmd(&cmds, "Cook").args.contains(&"-nocompileuat".to_string()));
    }

    #[test]
    fn per_phase_additional_args_route_to_the_right_command() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.phases.build.additional_args = "-NoCodeSign".into();
        p.phases.cook.additional_options = "-CookPartialGC".into();
        p.phases.stage.additional_args = "-applocaldirectory=Redist".into();
        p.phases.pak.additional_args = "-compressionformats=Oodle".into();
        p.phases.archive.additional_args = "-CrashReporter".into();
        let cmds = build_commands(&p, &env(&eng));

        assert!(cmd(&cmds, "Build").args.contains(&"-NoCodeSign".to_string()));
        // cook options are wrapped, not raw-appended
        assert!(cmd(&cmds, "Cook").args.contains(&"-AdditionalCookerOptions=-CookPartialGC".to_string()));
        // stage/pak/archive strings merge into the one BuildCookRun, in order
        let spa = cmd(&cmds, "Stage · Pak · Archive");
        for f in ["-applocaldirectory=Redist", "-compressionformats=Oodle", "-CrashReporter"] {
            assert!(spa.args.iter().any(|a| a == f), "stage group missing {f}");
        }
    }

    #[test]
    fn steam_login_preflight_emitted_before_build() {
        let eng = engine(EngineKind::Source);
        // disabled ⇒ no preflight
        assert!(!build_commands(&dev_profile(), &env(&eng)).iter().any(|c| c.phase == PhaseId::SteamLogin));

        let mut p = dev_profile();
        p.phases.steam_upload.enabled = true;
        p.phases.steam_upload.app_id = "480".into();
        p.phases.steam_upload.depots =
            vec![crate::profiles::schema::DepotItem { depot_id: "481".into(), path: ".".into() }];
        let cmds = build_commands(&p, &env(&eng));
        // the very first emitted unit is the Steam Login preflight
        assert_eq!(cmds[0].phase, PhaseId::SteamLogin);
        assert_eq!(cmds[0].label, "Steam Login");
        for f in ["+login", "builder_bot", "+quit"] {
            assert!(cmds[0].args.iter().any(|a| a == f), "preflight missing {f}");
        }
        assert!(!cmds[0].args.iter().any(|a| a == "+run_app_build"), "preflight must not upload");
        // and the real upload phase is still emitted (later)
        assert!(cmds.iter().any(|c| c.phase == PhaseId::SteamUpload));
    }

    #[test]
    fn steam_upload_emits_steamcmd_run_app_build() {
        let eng = engine(EngineKind::Source);
        // off by default ⇒ no unit
        let cmds = build_commands(&dev_profile(), &env(&eng));
        assert!(!cmds.iter().any(|c| c.phase == PhaseId::SteamUpload));

        let mut p = dev_profile();
        p.phases.steam_upload.enabled = true;
        p.phases.steam_upload.app_id = "480".into();
        p.phases.steam_upload.depots =
            vec![crate::profiles::schema::DepotItem { depot_id: "481".into(), path: ".".into() }];
        let cmds = build_commands(&p, &env(&eng));
        let steam = cmd(&cmds, "Upload to Steam");
        assert_eq!(steam.phase, PhaseId::SteamUpload);
        assert_eq!(steam.kind, PhaseKind::External);
        assert_eq!(steam.program.as_deref(), Some("C:/steamcmd/steamcmd.exe"));
        assert!(steam.args.contains(&"+login".to_string()));
        assert!(steam.args.contains(&"builder_bot".to_string()));
        assert!(steam.args.contains(&"+run_app_build".to_string()));
        assert!(steam.args.contains(&"+quit".to_string()));
        // the run VDF sits in the git-ignored scratch dir, under the project root
        assert!(steam.args.iter().any(|a| a.replace('\\', "/").contains(".uep/steam-build-output/dev/app_build.vdf")));
    }

    #[test]
    fn steam_upload_gated_off_when_archive_disabled() {
        let eng = engine(EngineKind::Source);
        let mut p = dev_profile();
        p.phases.steam_upload.enabled = true;
        p.phases.steam_upload.app_id = "480".into();
        p.phases.steam_upload.depots =
            vec![crate::profiles::schema::DepotItem { depot_id: "481".into(), path: ".".into() }];
        // Archive off ⇒ nothing fresh to push ⇒ neither the preflight nor the upload is emitted.
        p.phases.archive.enabled = false;
        let cmds = build_commands(&p, &env(&eng));
        assert!(!cmds.iter().any(|c| c.phase == PhaseId::SteamLogin));
        assert!(!cmds.iter().any(|c| c.phase == PhaseId::SteamUpload));
    }

    #[test]
    fn app_phases_emitted_only_when_enabled() {
        let eng = engine(EngineKind::Source);
        // disabled by default
        let cmds = build_commands(&dev_profile(), &env(&eng));
        assert!(!cmds.iter().any(|c| c.phase == PhaseId::CopyExtras));
        assert!(!cmds.iter().any(|c| c.phase == PhaseId::Cleanup));

        let mut p = dev_profile();
        p.phases.copy_extras.enabled = true;
        p.phases.copy_extras.items = vec![super::super::super::profiles::schema::CopyItem {
            from: "steam_appid.txt".into(),
            to: ".".into(),
        }];
        p.phases.cleanup.enabled = true;
        p.phases.cleanup.categories =
            vec![crate::profiles::schema::CleanupCategory::Staged, crate::profiles::schema::CleanupCategory::Cooked];
        let cmds = build_commands(&p, &env(&eng));

        let copy = cmd(&cmds, "Copy Extras");
        assert_eq!(copy.kind, PhaseKind::App);
        assert!(copy.program.is_none());
        assert!(copy.preview.contains("steam_appid.txt"));

        let clean = cmd(&cmds, "Clean-up");
        assert!(clean.preview.contains("staged, cooked"));
        assert!(clean.preview.contains("on success"));
    }
}
