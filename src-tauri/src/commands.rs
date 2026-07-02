//! Thin `#[tauri::command]` wrappers - the only Tauri-facing layer. Each delegates
//! to `crate::unreal` / `crate::storage`; no business logic lives here.
//!
//! Flow (see `docs/user-experience.md` §1): `validate_project` is the cheap
//! pre-open check (project parses? engine resolves?). If the engine is missing
//! the UI calls `locate_engine` (which saves the chosen folder), then
//! `open_project` runs full detection and records the recent.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::footprint::{self, scan::FootprintReport};
use crate::history::{self, schema::BuildRecord};
use crate::pipeline::{self, PhaseInfo};
use crate::profiles::schema::BuildConfig;
use crate::profiles::{store, templates};
use crate::runner::{
    self, spawn_commandlet, spawn_plugin_package, spawn_run, CommandletInputs, LogLine,
    PluginPackageInputs, RunInputs, RunSnapshot,
};
use crate::settings::{self, AppSettings};
use crate::state::AppState;
use crate::storage::{self, RecentKind, RecentRecord};
use crate::unreal::args::{self, BuildEnv, PhaseCommand};
use crate::unreal::engine::{self, EngineInfo, EngineKind};
use crate::unreal::targets::{TargetInfo, TargetType};
use crate::unreal::{detect_plugin, detect_project, uplugin, uproject, DetectError, DetectedPlugin, DetectedProject};

/// Serializable error for the IPC boundary (the UI switches on `kind`).
#[derive(Debug, Serialize, specta::Type, thiserror::Error)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum AppError {
    #[error("{0}")]
    InvalidProject(String),
    // The `String` payload IS what the UI shows (serde `content = "message"`), so it
    // carries a human reason - not the raw path.
    #[error("{0}")]
    InvalidEngine(String),
    #[error("no engine found for association {0}")]
    EngineNotFound(String),
    /// Profile/template validation failed (one message per problem, newline-joined).
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Io(String),
}

impl From<DetectError> for AppError {
    fn from(e: DetectError) -> Self {
        match e {
            DetectError::EngineNotFound { association } => AppError::EngineNotFound(association),
            DetectError::Parse(p) => {
                AppError::InvalidProject(format!("could not parse {}", p.display()))
            }
            DetectError::Invalid(m) => AppError::InvalidProject(m),
            DetectError::Io { path, source } => {
                AppError::Io(format!("{}: {source}", path.display()))
            }
        }
    }
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(dir)
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// The open project's root, or `None` when none is open. Owned (guard dropped) so callers
/// can hold it across an `await`.
fn open_project_root(state: &State<AppState>) -> Option<String> {
    state.current.read().unwrap().as_ref().map(|p| p.project_root.clone())
}

/// Resolve a stored machine-local path to an absolute one against the project root. A
/// project-relative path (`./Tools/steamcmd/steamcmd.exe`) is joined under the root; an
/// absolute or empty path passes through unchanged. Mirrors the arg-builder's base-dir rule
/// so a path inside the project can be stored relative (portable if the project moves) yet
/// still resolves for spawning.
fn resolve_local_path(project_root: &str, stored: &str) -> String {
    let s = stored.trim();
    if s.is_empty() {
        return String::new();
    }
    // Reuse the arg-builder's project-relative anchoring (`resolve_under_root`) so a
    // machine-local path and the output base dir can never resolve by different rules.
    crate::unreal::args::resolve_under_root(Path::new(project_root), s).display().to_string()
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectValidation {
    pub name: String,
    pub engine_association: String,
    /// `None` ⇒ the engine could not be resolved; the UI must prompt Locate.
    pub engine: Option<EngineInfo>,
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    pub name: String,
    /// Descriptor path - a `.uproject` or `.uplugin` (see `kind`).
    pub path: String,
    /// Project vs plugin - drives the gate's PROJECT/PLUGIN tag + which open flow runs.
    pub kind: RecentKind,
    /// Pinned to the top of the recents list (persisted). The user toggles it via
    /// the star in the gate; sorts above unpinned entries regardless of recency.
    pub starred: bool,
    pub last_opened_ms: f64,
    /// The descriptor (`.uproject`/`.uplugin`) exists and parses.
    pub valid: bool,
    // ── project-only (all `None`/false for plugins) ──
    /// An engine was resolved + validated for this project's association.
    pub engine_valid: bool,
    pub engine_version: Option<String>,
    pub engine_kind: Option<EngineKind>,
    /// Resolved engine root, or the stale saved path when invalid (shown so the
    /// user can see what to fix). `None` when nothing is known.
    pub engine_path: Option<String>,
    pub engine_association: Option<String>,
    // ── plugin-only ──
    /// The `.uplugin`'s `VersionName` (e.g. `"1.2.0"`), shown in the gate; `None`
    /// for projects.
    pub version: Option<String>,
}

/// Cheap pre-open check: does the project parse, and can its engine be resolved?
#[tauri::command]
#[specta::specta]
pub fn validate_project(uproject_path: String) -> Result<ProjectValidation, AppError> {
    let path = Path::new(&uproject_path);
    let proj = uproject::parse_uproject(path)?;
    let saved = path.parent().and_then(storage::get_project_engine_path);
    let engine = engine::resolve_engine(&proj.engine_association, saved.as_deref());
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Project")
        .to_string();
    Ok(ProjectValidation {
        name,
        engine_association: proj.engine_association,
        engine,
    })
}

/// Validate a user-picked engine folder; on success, save it as this **project's**
/// engine override (`.uep/local.json`) - per-project, so it never bleeds into other
/// projects that happen to share the same `EngineAssociation`.
#[tauri::command]
#[specta::specta]
pub fn locate_engine(uproject_path: String, engine_dir: String) -> Result<EngineInfo, AppError> {
    let path = Path::new(&uproject_path);
    let proj = uproject::parse_uproject(path)?;
    let info = engine::validate_located_engine(Path::new(&engine_dir), &proj.engine_association)
        .ok_or_else(|| AppError::InvalidEngine("That folder isn't an Unreal Engine root. No Engine folder was found inside it.".into()))?;
    let root = path
        .parent()
        .ok_or_else(|| AppError::Io("project path has no parent directory".into()))?;
    storage::set_project_engine_path(root, &engine_dir).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(info)
}

/// Full detection + record the recent. Requires both the project and a valid
/// engine (saved or auto-resolved) - otherwise `EngineNotFound`.
#[tauri::command]
#[specta::specta]
pub fn open_project(
    app: AppHandle,
    state: State<AppState>,
    uproject_path: String,
) -> Result<DetectedProject, AppError> {
    let path = Path::new(&uproject_path);
    let dir = data_dir(&app)?;
    let saved = path.parent().and_then(storage::get_project_engine_path);
    let detected = detect_project(path, saved.as_deref())?;

    let _ = storage::upsert_recent(
        &dir,
        RecentRecord {
            name: detected.name.clone(),
            path: detected.uproject_path.clone(),
            kind: RecentKind::Project,
            last_opened_ms: now_ms(),
            // `upsert_recent` re-applies any existing pin for this path.
            starred: false,
        },
    );
    // Scaffold `.uep/.gitignore` so derived history/ + cache/ never leak into the
    // user's project repo (profiles/ stays committed). Best-effort, non-destructive.
    let _ = storage::ensure_uep_dir(Path::new(&detected.project_root));
    // Opening a project clears any previously-open plugin (one or the other).
    *state.current_plugin.write().unwrap() = None;
    *state.current.write().unwrap() = Some(detected.clone());
    Ok(detected)
}

/// Full plugin detection + record the recent. A plugin packages standalone, so this
/// needs no engine (the compile engine is chosen per-package in the Actions tab).
#[tauri::command]
#[specta::specta]
pub fn open_plugin(
    app: AppHandle,
    state: State<AppState>,
    plugin_path: String,
) -> Result<DetectedPlugin, AppError> {
    let dir = data_dir(&app)?;
    let detected = detect_plugin(Path::new(&plugin_path))?;
    let _ = storage::upsert_recent(
        &dir,
        RecentRecord {
            name: detected.name.clone(),
            path: detected.uplugin_path.clone(),
            kind: RecentKind::Plugin,
            last_opened_ms: now_ms(),
            starred: false,
        },
    );
    // Opening a plugin clears any previously-open project (one or the other).
    *state.current.write().unwrap() = None;
    *state.current_plugin.write().unwrap() = Some(detected.clone());
    Ok(detected)
}

/// The currently-open plugin (read-only snapshot of managed state), or `None`. Powers
/// the Actions tab's package action + engine picker.
#[tauri::command]
#[specta::specta]
pub fn current_plugin(state: State<AppState>) -> Result<Option<DetectedPlugin>, AppError> {
    Ok(state.current_plugin.read().unwrap().clone())
}

/// Recents with freshly-validated project + engine status (nothing cached).
/// Invalid entries are kept - the UI flags the offending part with a fix
/// affordance and offers Remove via the row menu; only an explicit Remove drops
/// a recent.
#[tauri::command]
#[specta::specta]
pub fn list_recents(app: AppHandle) -> Result<Vec<RecentEntry>, AppError> {
    let dir = data_dir(&app)?;
    let mut entries: Vec<RecentEntry> = storage::load_recents(&dir)
        .into_iter()
        .map(|r| match r.kind {
            RecentKind::Project => project_recent(r),
            RecentKind::Plugin => plugin_recent(r),
        })
        .collect();
    // Pinned-first; `sort_by_key` is stable, so recency order is preserved within
    // each group (load_recents is already most-recent-first).
    entries.sort_by_key(|e| !e.starred);
    Ok(entries)
}

/// A blank (all-`None`) entry carrying just the stored identity - the base every
/// branch fills in.
fn base_recent(r: &RecentRecord) -> RecentEntry {
    RecentEntry {
        name: r.name.clone(),
        path: r.path.clone(),
        kind: r.kind,
        starred: r.starred,
        last_opened_ms: r.last_opened_ms,
        valid: false,
        engine_valid: false,
        engine_version: None,
        engine_kind: None,
        engine_path: None,
        engine_association: None,
        version: None,
    }
}

/// Project recents: re-validate the descriptor + resolve its engine (nothing cached).
fn project_recent(r: RecentRecord) -> RecentEntry {
    let mut e = base_recent(&r);
    let Ok(proj) = uproject::parse_uproject(Path::new(&r.path)) else {
        return e;
    };
    let saved = Path::new(&r.path).parent().and_then(storage::get_project_engine_path);
    let engine = engine::resolve_engine(&proj.engine_association, saved.as_deref());
    e.valid = true;
    e.engine_valid = engine.is_some();
    e.engine_version = engine.as_ref().map(|x| x.version.short());
    e.engine_kind = engine.as_ref().map(|x| x.kind);
    e.engine_path = engine
        .as_ref()
        .map(|x| x.root.display().to_string())
        .or_else(|| saved.as_ref().map(|p| p.display().to_string()));
    e.engine_association = Some(proj.engine_association);
    e
}

/// Plugin recents: re-validate the descriptor + surface its version (no engine).
fn plugin_recent(r: RecentRecord) -> RecentEntry {
    let mut e = base_recent(&r);
    if let Ok(plugin) = uplugin::parse_uplugin(Path::new(&r.path)) {
        e.valid = true;
        e.version = Some(plugin.version_name).filter(|v| !v.trim().is_empty());
    }
    e
}

#[tauri::command]
#[specta::specta]
pub fn remove_recent(app: AppHandle, path: String) -> Result<(), AppError> {
    let dir = data_dir(&app)?;
    storage::remove_recent(&dir, &path).map_err(|e| AppError::Io(e.to_string()))
}

/// Pin/unpin a recent (persisted). Pinned entries sort to the top of the list.
#[tauri::command]
#[specta::specta]
pub fn set_recent_starred(app: AppHandle, path: String, starred: bool) -> Result<(), AppError> {
    let dir = data_dir(&app)?;
    storage::set_recent_starred(&dir, &path, starred).map_err(|e| AppError::Io(e.to_string()))
}

// ── M2: profiles, templates, arg-builder preview, phase registry ───────────────
//
// Profiles are project-local (`<project>/.uep/profiles/`, from the open project in
// managed state); templates are global (app-folder `templates/`). The pure logic
// lives in `crate::profiles` / `crate::pipeline` / `crate::unreal::args`.

/// `<project>/.uep/profiles` for the currently-open project (errors if none open).
fn profiles_dir(state: &State<AppState>) -> Result<PathBuf, AppError> {
    let guard = state.current.read().unwrap();
    let proj = guard
        .as_ref()
        .ok_or_else(|| AppError::Io("no project is open".into()))?;
    Ok(Path::new(&proj.project_root).join(".uep").join("profiles"))
}

/// App-folder `templates/` (global, project-agnostic).
fn templates_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(data_dir(app)?.join("templates"))
}

/// How a new profile is created - always a copy, never blank: `template` clones a
/// template, `clone` clones another profile. `sourceId` names that source.
#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CreateFromKind {
    Template,
    Clone,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    pub name: String,
    pub from: CreateFromKind,
    /// Template id (`template`) or source profile id (`clone`).
    pub source_id: String,
}

/// A new user template - created only by cloning an existing template (e.g. the
/// fixed `Default`); never blank.
#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateTemplateRequest {
    pub name: String,
    /// Source template id to clone (a fixed built-in, or any user template).
    pub source_id: String,
}

/// `name` → `slug-<base36 epoch nanos>`; unique enough for one user's files, and
/// readable in the committed `.uep/profiles/`.
fn gen_id(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", slugify(name), to_base36(nanos))
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        "profile".into()
    } else {
        s
    }
}

fn to_base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).unwrap()
}

#[tauri::command]
#[specta::specta]
pub fn list_profiles(state: State<AppState>) -> Result<Vec<BuildConfig>, AppError> {
    Ok(store::load_all(&profiles_dir(&state)?))
}

/// Create a profile by cloning a template or another profile (never blank), then
/// persist and return it (copy-on-create; no live link to the source).
#[tauri::command]
#[specta::specta]
pub fn create_profile(
    app: AppHandle,
    state: State<AppState>,
    req: CreateRequest,
) -> Result<BuildConfig, AppError> {
    let dir = profiles_dir(&state)?;
    let id = gen_id(&req.name);
    let cfg = match req.from {
        CreateFromKind::Template => {
            let tdir = templates_dir(&app)?;
            templates::ensure_builtins(&tdir).map_err(|e| AppError::Io(e.to_string()))?;
            let tmpl = store::load_one(&tdir, &req.source_id)
                .ok_or_else(|| AppError::Io(format!("template {} not found", req.source_id)))?;
            store::from_template(id, req.name, &tmpl)
        }
        CreateFromKind::Clone => {
            let src = store::load_one(&dir, &req.source_id)
                .ok_or_else(|| AppError::Io(format!("profile {} not found", req.source_id)))?;
            store::from_clone(id, req.name, &src)
        }
    };
    store::save(&dir, &cfg).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(cfg)
}

/// Duplicate a profile into a self-contained `<name> (copy)`.
#[tauri::command]
#[specta::specta]
pub fn duplicate_profile(state: State<AppState>, id: String) -> Result<BuildConfig, AppError> {
    let dir = profiles_dir(&state)?;
    let src = store::load_one(&dir, &id)
        .ok_or_else(|| AppError::Io(format!("profile {id} not found")))?;
    let dup = store::duplicate(gen_id(&format!("{}-copy", src.name)), &src);
    store::save(&dir, &dup).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(dup)
}

/// Validate (authoritative) then persist a profile.
#[tauri::command]
#[specta::specta]
pub fn save_profile(state: State<AppState>, profile: BuildConfig) -> Result<(), AppError> {
    profile
        .validate_profile()
        .map_err(|errs| AppError::Validation(errs.join("\n")))?;
    store::save(&profiles_dir(&state)?, &profile).map_err(|e| AppError::Io(e.to_string()))?;
    // When the Steam upload phase is on, (re)generate the committed VDFs from the profile's
    // managed fields, preserving any custom keys the user added. Best-effort - a VDF hiccup
    // must not fail the save (the profile JSON is the source of truth for the managed fields).
    if profile.phases.steam_upload.enabled {
        if let Some(root) = open_project_root(&state) {
            let _ = crate::steam::vdf::write_committed_vdf(Path::new(&root), &profile);
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn delete_profile(state: State<AppState>, id: String) -> Result<(), AppError> {
    store::delete(&profiles_dir(&state)?, &id).map_err(|e| AppError::Io(e.to_string()))
}

/// Global templates - the fixed built-ins (self-healed) plus user templates.
#[tauri::command]
#[specta::specta]
pub fn list_templates(app: AppHandle) -> Result<Vec<BuildConfig>, AppError> {
    let dir = templates_dir(&app)?;
    templates::ensure_builtins(&dir).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(store::load_all(&dir))
}

/// Create a user template by cloning an existing one (a fixed built-in, or any
/// user template) - never blank. Persist + return it.
#[tauri::command]
#[specta::specta]
pub fn create_template(
    app: AppHandle,
    req: CreateTemplateRequest,
) -> Result<BuildConfig, AppError> {
    let dir = templates_dir(&app)?;
    templates::ensure_builtins(&dir).map_err(|e| AppError::Io(e.to_string()))?;
    let src = store::load_one(&dir, &req.source_id)
        .ok_or_else(|| AppError::Io(format!("template {} not found", req.source_id)))?;
    let tmpl = store::from_clone(gen_id(&req.name), req.name, &src);
    store::save(&dir, &tmpl).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(tmpl)
}

/// Persist a user template. Built-ins are read-only; a template may omit the
/// output base dir (project-agnostic), so only id + name are required.
#[tauri::command]
#[specta::specta]
pub fn save_template(app: AppHandle, mut template: BuildConfig) -> Result<(), AppError> {
    if templates::is_builtin(&template.id) {
        return Err(AppError::Validation("built-in templates are read-only".into()));
    }
    if template.id.trim().is_empty() || template.name.trim().is_empty() {
        return Err(AppError::Validation("template id and name are required".into()));
    }
    template.builtin = false; // only the seeded built-ins are ever built-in
    store::save(&templates_dir(&app)?, &template).map_err(|e| AppError::Io(e.to_string()))
}

/// Delete a user template. The fixed built-in(s) cannot be deleted.
#[tauri::command]
#[specta::specta]
pub fn delete_template(app: AppHandle, id: String) -> Result<(), AppError> {
    if templates::is_builtin(&id) {
        return Err(AppError::Validation(
            "built-in templates cannot be deleted".into(),
        ));
    }
    store::delete(&templates_dir(&app)?, &id).map_err(|e| AppError::Io(e.to_string()))
}

/// Resolve a profile (possibly with unsaved edits) into the read-only per-phase
/// command preview, against the currently-open project. `{date}`/`{time}` resolve
/// to now (local).
#[tauri::command]
#[specta::specta]
pub fn preview_profile(
    state: State<AppState>,
    profile: BuildConfig,
) -> Result<Vec<PhaseCommand>, AppError> {
    let guard = state.current.read().unwrap();
    let proj = guard
        .as_ref()
        .ok_or_else(|| AppError::Io("no project is open".into()))?;

    // Resolve the game target: the profile's, else the single detected
    // packageable target (else the first - the UI forces a choice when >1).
    let target = profile.target.clone().unwrap_or_else(|| {
        let pkg: Vec<&TargetInfo> = proj.targets.iter().filter(|t| t.packageable()).collect();
        pkg.first().map(|t| t.name.clone()).unwrap_or_default()
    });
    let editor_target = proj
        .targets
        .iter()
        .find(|t| t.target_type == TargetType::Editor)
        .map(|t| t.name.clone());

    let now = chrono::Local::now();
    let date = now.format("%Y%m%d").to_string();
    let time = now.format("%H%M%S").to_string();

    let steam = storage::load_steam_local_settings(Path::new(&proj.project_root));
    let steamcmd = resolve_local_path(&proj.project_root, &steam.steamcmd_path);
    let env = BuildEnv {
        uproject_path: &proj.uproject_path,
        project_name: &proj.name,
        engine: &proj.engine,
        project_type: proj.project_type,
        target: &target,
        editor_target: editor_target.as_deref(),
        date: &date,
        time: &time,
        steamcmd_path: &steamcmd,
        steam_account: &steam.account,
    };
    Ok(args::build_commands(&profile, &env))
}

/// The currently-open detected project (read-only snapshot of managed state), or
/// `None` if none is open. Powers the Build Settings editor's target/map pickers
/// and output preview without re-running detection.
#[tauri::command]
#[specta::specta]
pub fn current_project(state: State<AppState>) -> Result<Option<DetectedProject>, AppError> {
    Ok(state.current.read().unwrap().clone())
}

/// The static phase registry (for the editor's phase-structured layout). Includes
/// each phase's `gated_by`, from which the editor derives locked/enabled itself.
#[tauri::command]
#[specta::specta]
pub fn phase_registry() -> Vec<PhaseInfo> {
    pipeline::registry_info()
}

// ── M3: runner (start / cancel / snapshot) ─────────────────────────────────────
//
// `start_build` resolves the profile into per-phase commands (same env as
// `preview_profile`) and spawns the separate-process executor (`crate::runner`),
// which streams `uep://run-*` events. The Build Logs window backfills via
// `active_run` then follows those events; `cancel_build` signals a clean kill.

/// Resolve a profile against the open project and launch its pipeline. Returns the
/// new run id; progress arrives via `uep://run-*` events. Refuses a second
/// concurrent run.
#[tauri::command]
#[specta::specta]
pub fn start_build(
    app: AppHandle,
    state: State<AppState>,
    profile: BuildConfig,
) -> Result<String, AppError> {
    if let Some(active) = state.run.lock().unwrap().as_ref() {
        if active.is_running() {
            return Err(AppError::Io("a build is already running".into()));
        }
    }
    // Authoritative validation before we spawn anything (same check as save).
    profile
        .validate_profile()
        .map_err(|errs| AppError::Validation(errs.join("\n")))?;

    // Resolve the per-phase commands + output dir against the open project - the
    // same resolution `preview_profile` does - capturing everything owned so the
    // spawned task never borrows managed state.
    let (units, output_dir, project, project_root, target, editor_target) = {
        let guard = state.current.read().unwrap();
        let proj = guard
            .as_ref()
            .ok_or_else(|| AppError::Io("no project is open".into()))?;

        let target = profile.target.clone().unwrap_or_else(|| {
            let pkg: Vec<&TargetInfo> = proj.targets.iter().filter(|t| t.packageable()).collect();
            pkg.first().map(|t| t.name.clone()).unwrap_or_default()
        });
        if target.trim().is_empty() {
            return Err(AppError::Validation(
                "no build target detected; set the profile's target".into(),
            ));
        }
        let editor_target = proj
            .targets
            .iter()
            .find(|t| t.target_type == TargetType::Editor)
            .map(|t| t.name.clone());

        let now = chrono::Local::now();
        let date = now.format("%Y%m%d").to_string();
        let time = now.format("%H%M%S").to_string();

        let steam = storage::load_steam_local_settings(Path::new(&proj.project_root));
        if profile.phases.steam_upload.enabled {
            if steam.steamcmd_path.trim().is_empty() {
                return Err(AppError::Validation(
                    "Steam upload is enabled but no steamcmd path is set - set it in the Steam upload settings.".into(),
                ));
            }
            if steam.account.trim().is_empty() {
                return Err(AppError::Validation(
                    "Steam upload is enabled but you're not logged in to Steam - log in first.".into(),
                ));
            }
        }
        let steamcmd = resolve_local_path(&proj.project_root, &steam.steamcmd_path);
        let env = BuildEnv {
            uproject_path: &proj.uproject_path,
            project_name: &proj.name,
            engine: &proj.engine,
            project_type: proj.project_type,
            target: &target,
            editor_target: editor_target.as_deref(),
            date: &date,
            time: &time,
            steamcmd_path: &steamcmd,
            steam_account: &steam.account,
        };
        (
            args::build_commands(&profile, &env),
            args::resolved_output_dir(&profile, &env),
            proj.name.clone(),
            proj.project_root.clone(),
            target,
            editor_target.clone(),
        )
    };

    let run_id = gen_id(&format!("build-{}", profile.name));
    let platform = profile.platform.uat().to_string();
    let configs = profile
        .staged_configs()
        .iter()
        .map(|c| c.as_str().to_string())
        .collect::<Vec<_>>();
    let active = spawn_run(
        app,
        RunInputs {
            run_id: run_id.clone(),
            units,
            profile,
            project_root,
            output_dir,
            project,
            platform,
            configs,
            target,
            editor_target,
            started_ms: now_ms(),
        },
    );
    *state.run.lock().unwrap() = Some(active);
    Ok(run_id)
}

/// Signal the active run to cancel (kills running children, settles the graph).
/// No-op when nothing is running.
#[tauri::command]
#[specta::specta]
pub fn cancel_build(state: State<AppState>) -> Result<(), AppError> {
    if let Some(active) = state.run.lock().unwrap().as_ref() {
        active.cancel();
    }
    Ok(())
}

/// Snapshot of the in-flight (or most recent) run - lets a freshly-opened Build
/// Logs window render the graph + backfill the console before following events.
#[tauri::command]
#[specta::specta]
pub fn active_run(state: State<AppState>) -> Result<Option<RunSnapshot>, AppError> {
    Ok(state.run.lock().unwrap().as_ref().map(|a| a.snapshot()))
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OutputCheck {
    pub path: String,
    pub exists: bool,
}

/// Resolve a profile's archive dir and report whether it already exists - drives
/// the Run-time "folder already exists, replace?" confirm (R1).
#[tauri::command]
#[specta::specta]
pub fn check_output(state: State<AppState>, profile: BuildConfig) -> Result<OutputCheck, AppError> {
    let guard = state.current.read().unwrap();
    let proj = guard
        .as_ref()
        .ok_or_else(|| AppError::Io("no project is open".into()))?;
    let target = profile.target.clone().unwrap_or_else(|| {
        proj.targets
            .iter()
            .find(|t| t.packageable())
            .map(|t| t.name.clone())
            .unwrap_or_default()
    });
    let editor_target = proj
        .targets
        .iter()
        .find(|t| t.target_type == TargetType::Editor)
        .map(|t| t.name.clone());
    let now = chrono::Local::now();
    let date = now.format("%Y%m%d").to_string();
    let time = now.format("%H%M%S").to_string();
    let env = BuildEnv {
        uproject_path: &proj.uproject_path,
        project_name: &proj.name,
        engine: &proj.engine,
        project_type: proj.project_type,
        target: &target,
        editor_target: editor_target.as_deref(),
        date: &date,
        time: &time,
        // Steam fields unused here (only resolved_output_dir is called, not build_commands).
        steamcmd_path: "",
        steam_account: "",
    };
    let path = args::resolved_output_dir(&profile, &env);
    let exists = Path::new(&path).exists();
    Ok(OutputCheck { path, exists })
}

// ── M4: build history (records under .uep/history/) ────────────────────────────

/// `<project>/.uep/history` for the open project (errors if none open).
fn history_dir(state: &State<AppState>) -> Result<PathBuf, AppError> {
    let guard = state.current.read().unwrap();
    let proj = guard
        .as_ref()
        .ok_or_else(|| AppError::Io("no project is open".into()))?;
    Ok(Path::new(&proj.project_root).join(".uep").join("history"))
}

/// All build records for the open project, newest first. (Powers the Dashboard,
/// which aggregates over the whole history; the Build tab pages via
/// [`list_history_page`].)
#[tauri::command]
#[specta::specta]
pub fn list_history(state: State<AppState>) -> Result<Vec<BuildRecord>, AppError> {
    Ok(history::store::load_all(&history_dir(&state)?))
}

#[derive(Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryFilter {
    pub platform: Option<String>,
    pub config: Option<String>,
    pub target: Option<String>,
    pub status: Option<String>,
}

#[derive(Default, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FilterOptions {
    pub platform: Vec<String>,
    pub config: Vec<String>,
    pub target: Vec<String>,
    pub status: Vec<String>,
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    /// The requested page of records (newest first).
    pub records: Vec<BuildRecord>,
    /// Records matching the filter - the page count divides this.
    pub filtered_total: f64,
    /// Records regardless of filter - the "x of y builds" line.
    pub grand_total: f64,
    /// Distinct tag values present, split by dimension (drives the filter menus).
    pub options: FilterOptions,
}

/// One filtered, newest-first **page** of history via the SQLite index (R5). Falls
/// back to a direct file scan if the index can't be opened/queried, so history never
/// disappears on a DB hiccup (the JSON records are the source of truth).
#[tauri::command]
#[specta::specta]
pub fn list_history_page(
    state: State<AppState>,
    offset: u32,
    limit: u32,
    filter: HistoryFilter,
) -> Result<HistoryPage, AppError> {
    let dir = history_dir(&state)?;
    Ok(page_via_index(&dir, offset, limit, &filter).unwrap_or_else(|| page_via_files(&dir, offset, limit, &filter)))
}

fn page_via_index(dir: &Path, offset: u32, limit: u32, filter: &HistoryFilter) -> Option<HistoryPage> {
    let conn = history::index::open_synced(dir).ok()?;
    let f = history::index::Filter {
        platform: filter.platform.clone(),
        config: filter.config.clone(),
        target: filter.target.clone(),
        status: filter.status.clone(),
    };
    let (records, filtered_total) = history::index::query_page(&conn, offset, limit, &f).ok()?;
    let grand_total = history::index::grand_total(&conn).ok()?;
    let options = options_from_tags(&history::index::distinct_tags(&conn).ok()?);
    Some(HistoryPage {
        records,
        filtered_total: filtered_total as f64,
        grand_total: grand_total as f64,
        options,
    })
}

fn page_via_files(dir: &Path, offset: u32, limit: u32, filter: &HistoryFilter) -> HistoryPage {
    let all = history::store::load_all(dir); // newest first
    let keep = |rec: &BuildRecord| {
        let has = |dim: &Option<String>| dim.as_ref().map_or(true, |v| rec.tags.iter().any(|t| t == v));
        has(&filter.platform) && has(&filter.config) && has(&filter.target) && has(&filter.status)
    };
    let filtered: Vec<&BuildRecord> = all.iter().filter(|r| keep(r)).collect();
    let grand_total = all.len() as f64;
    let filtered_total = filtered.len() as f64;
    let records = filtered
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .cloned()
        .collect();
    let tags: Vec<String> = all.iter().flat_map(|r| r.tags.clone()).collect();
    HistoryPage { records, filtered_total, grand_total, options: options_from_tags(&tags) }
}

/// Split distinct tag values into the four filter dimensions (via `history::tags`).
fn options_from_tags(tags: &[String]) -> FilterOptions {
    let mut o = FilterOptions::default();
    for t in tags {
        match history::tags::dimension_of(t) {
            "platform" => o.platform.push(t.clone()),
            "config" => o.config.push(t.clone()),
            "status" => o.status.push(t.clone()),
            _ => o.target.push(t.clone()),
        }
    }
    for list in [&mut o.platform, &mut o.config, &mut o.target, &mut o.status] {
        list.sort();
        list.dedup();
    }
    o
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDetail {
    pub record: BuildRecord,
    /// The saved log, re-classified for severity tinting on replay; each line's
    /// phase index is restored from the `build.idx` sidecar (0 for older records).
    pub lines: Vec<LogLine>,
}

/// A past build's record + its saved log (for Build Logs replay).
#[tauri::command]
#[specta::specta]
pub fn history_detail(
    state: State<AppState>,
    build_id: String,
) -> Result<Option<HistoryDetail>, AppError> {
    let dir = history_dir(&state)?;
    let Some(record) = history::store::load_one(&dir, &build_id) else {
        return Ok(None);
    };
    let log = history::store::load_log(&dir, &build_id).unwrap_or_default();
    let phase_idx = history::store::load_phase_idx(&dir, &build_id);
    let lines = log
        .lines()
        .enumerate()
        .map(|(i, text)| LogLine {
            seq: i as u32 + 1,
            phase_index: phase_idx.get(i).copied().unwrap_or(0),
            severity: runner::classify::classify_line(text),
            text: text.to_string(),
        })
        .collect();
    Ok(Some(HistoryDetail { record, lines }))
}

/// Delete one or more build records (folders). Idempotent.
#[tauri::command]
#[specta::specta]
pub fn delete_history(state: State<AppState>, ids: Vec<String>) -> Result<(), AppError> {
    let dir = history_dir(&state)?;
    for id in &ids {
        let _ = history::store::delete(&dir, id);
    }
    if let Ok(conn) = history::index::open(&dir) {
        let _ = history::index::remove(&conn, &ids);
    }
    Ok(())
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LocationCheck {
    pub path: String,
    pub exists: bool,
    /// The folder may no longer hold *this* build - a later build reused the same
    /// folder, or its mtime drifted from the record's.
    pub changed: bool,
}

/// Integrity check before "Open location" (R5): confirms the output folder still
/// holds *this* build. The primary signal is metadata, not the filesystem - a later
/// build that archived to the **same folder** overwrote this one's output, so opening
/// the older record would actually show the newer build. (The directory's own mtime
/// is unreliable here: NTFS doesn't bump a folder's mtime when only nested files are
/// rewritten, which is exactly the same-folder rebuild case.) mtime drift is kept as
/// a secondary signal for out-of-band edits. The UI opens the folder, or warns.
#[tauri::command]
#[specta::specta]
pub fn check_build_location(
    state: State<AppState>,
    build_id: String,
) -> Result<LocationCheck, AppError> {
    let dir = history_dir(&state)?;
    let all = history::store::load_all(&dir);
    let rec = all
        .iter()
        .find(|r| r.build_id == build_id)
        .ok_or_else(|| AppError::Io(format!("build record {build_id} not found")))?;
    let p = Path::new(&rec.output_path);
    let exists = p.exists();
    // A newer build that targeted the same folder has overwritten this one's output.
    let overwritten = all.iter().any(|r| {
        r.build_id != rec.build_id
            && r.output_path == rec.output_path
            && r.started_at_ms > rec.started_at_ms
    });
    // mtime is only the secondary drift signal - skip the stat when the path is gone
    // or already known overwritten.
    let mtime_drift =
        exists && !overwritten && (history::store::mtime_ms(p) - rec.output_mtime_ms).abs() > 2000.0;
    let changed = exists && (overwritten || mtime_drift);
    Ok(LocationCheck {
        path: rec.output_path.clone(),
        exists,
        changed,
    })
}

// ── M5: footprint (scan + cleanup) ─────────────────────────────────────────────
//
// The scan walks only the named category roots (`footprint::rules`) - never the
// whole project - and runs on a blocking thread so a multi-GB walk never stalls the
// UI (R3). The pure rules/scan/clean logic lives in `crate::footprint`.

/// Footprint target scope from a detected project: every non-editor (packageable)
/// target is "build", the editor target (+ engine tools) is "editor". Lets cleanup split
/// `Intermediate`/`Binaries` so cleaning the build never wipes the editor's compile cache.
fn footprint_scope(proj: &DetectedProject) -> footprint::rules::TargetScope {
    let build = proj
        .targets
        .iter()
        .filter(|t| t.packageable())
        .map(|t| t.name.clone())
        .collect();
    let editor = proj
        .targets
        .iter()
        .find(|t| t.target_type == TargetType::Editor)
        .map(|t| t.name.clone());
    footprint::rules::TargetScope::new(build, editor)
}

/// The open project's root + footprint scope (errors if none open). Read synchronously
/// so the managed-state guard is dropped before any `spawn_blocking`/await.
fn project_root_and_scope(
    state: &State<AppState>,
) -> Result<(PathBuf, footprint::rules::TargetScope), AppError> {
    let guard = state.current.read().unwrap();
    let proj = guard
        .as_ref()
        .ok_or_else(|| AppError::Io("no project is open".into()))?;
    Ok((PathBuf::from(&proj.project_root), footprint_scope(proj)))
}

/// Scan the open project's reclaimable footprint, bucketed by category (R3). Heavy walk
/// → off the UI thread via `spawn_blocking`.
#[tauri::command]
#[specta::specta]
pub async fn scan_footprint(state: State<'_, AppState>) -> Result<FootprintReport, AppError> {
    let (root, scope) = project_root_and_scope(&state)?;
    tauri::async_runtime::spawn_blocking(move || footprint::scan::scan(&root, &scope))
        .await
        .map_err(|e| AppError::Io(e.to_string()))
}

/// Delete the chosen Clean-tab nodes and report bytes reclaimed (R3). `node_ids` are leaf
/// ids from the scan (e.g. `intermediateGame:<target>`, `intermediateOther`, `binariesPlugin`).
/// The UI shows the confirm prompt that lists every folder *before* calling this; the
/// backend re-resolves the tree and re-validates each target against the guardrail. Heavy
/// delete → off the UI thread.
#[tauri::command]
#[specta::specta]
pub async fn clean_footprint(
    state: State<'_, AppState>,
    node_ids: Vec<String>,
) -> Result<footprint::clean::CleanOutcome, AppError> {
    let (root, scope) = project_root_and_scope(&state)?;
    tauri::async_runtime::spawn_blocking(move || footprint::clean::clean_by_ids(&root, &node_ids, &scope))
        .await
        .map_err(|e| AppError::Io(e.to_string()))
}

// ── M6: app settings (theme + notification prefs) ──────────────────────────────
//
// One app-folder JSON (`settings.json`), defaults self-healed on a missing/corrupt
// file. The Settings window loads + persists these; the running app applies the
// theme on every window mount (via `uep://settings-changed`) and the runner reads
// the notify prefs to decide whether to toast on finish.

/// Current app settings (defaults if never saved).
#[tauri::command]
#[specta::specta]
pub fn load_settings(app: AppHandle) -> Result<AppSettings, AppError> {
    Ok(settings::load(&data_dir(&app)?))
}

/// Persist app settings and broadcast `uep://settings-changed` so every open window
/// re-applies the theme live (each window is its own webview).
#[tauri::command]
#[specta::specta]
pub fn save_settings(app: AppHandle, settings: AppSettings) -> Result<(), AppError> {
    settings::save(&data_dir(&app)?, &settings).map_err(|e| AppError::Io(e.to_string()))?;
    let _ = app.emit("uep://settings-changed", &settings);
    Ok(())
}

/// The app version (from `Cargo.toml`) - for the Settings "About" line.
#[tauri::command]
#[specta::specta]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Hide the main window and reveal the tray icon - the "minimize to tray" choice
/// when the user closes the main window while a build is running. The build keeps
/// running (the window is hidden, not destroyed); the tray restores it. (Restore +
/// re-hide of the tray happens in the tray's own click/Show handlers in `lib.rs`.)
#[tauri::command]
#[specta::specta]
pub fn minimize_to_tray(app: AppHandle) -> Result<(), AppError> {
    if let Some(win) = app.get_webview_window("main") {
        win.hide().map_err(|e| AppError::Io(e.to_string()))?;
    }
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_visible(true).map_err(|e| AppError::Io(e.to_string()))?;
    }
    Ok(())
}

// ── Plugin packaging (RunUAT BuildPlugin) ───────────────────────────────────────
//
// A plugin opens standalone (no engine association); the user picks the compile
// engine here. `list_engines` enumerates the machine's registered engines plus the
// plugin's own remembered (browsed) ones, validating each and pruning stale entries;
// `add_custom_engine` validates a browsed folder and remembers it per-plugin under
// `<plugin_root>/.uep/local.json`. `start_plugin_package` resolves + launches a
// `BuildPlugin` run on the shared runner (streams `uep://run-*`, cancellable), with
// an optional post-package strip of `Binaries/`+`Intermediate/` for FAB submission.

/// Where an engine entry came from - a machine registration vs a folder the user
/// browsed for (remembered per-plugin). Display-only; both build identically.
#[derive(Debug, Clone, Copy, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum EngineSource {
    Registry,
    Custom,
}

/// One selectable engine in the plugin packaging picker.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EngineEntry {
    /// Engine root path - the stable id/value (unique even when two builds share a version).
    pub root: String,
    /// Short version (e.g. `"5.5"`).
    pub version: String,
    pub kind: EngineKind,
    /// Display label (`"UE 5.5"` for launcher installs, `"<folder> (5.5)"` for source).
    pub label: String,
    pub source: EngineSource,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginPackageRequest {
    pub plugin_path: String,
    pub engine_root: String,
    /// User-picked output folder (the package dir's parent).
    pub base_dir: String,
    /// Folder-name template (tokens `{plugin}` `{version}` `{engine}` `{date}` `{time}`).
    pub folder_template: String,
    /// Remove `<package>/Binaries` + `<package>/Intermediate` on success (FAB).
    pub strip_binaries_intermediate: bool,
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginPreview {
    /// Resolved `RunUAT BuildPlugin …` command line.
    pub command: String,
    /// Resolved `-package` directory (`base_dir / rendered folder`).
    pub package_dir: String,
    /// Whether that directory already exists (it'll be cleared before packaging).
    pub exists: bool,
}

/// Load the plugin's machine-local Actions settings (`<plugin_root>/.uap/settings.json`)
/// - remembered engines + last-used output folder + folder name.
#[tauri::command]
#[specta::specta]
pub fn load_plugin_settings(plugin_path: String) -> Result<storage::PluginSettings, AppError> {
    let root = plugin_root_of(&plugin_path)?;
    Ok(storage::load_plugin_settings(&root))
}

/// Persist the plugin's package output folder + folder-name template into its
/// `.uap/settings.json` (read-modify-write, so the remembered engines are untouched).
#[tauri::command]
#[specta::specta]
pub fn save_plugin_output(
    plugin_path: String,
    output_dir: String,
    folder_name: String,
) -> Result<(), AppError> {
    let root = plugin_root_of(&plugin_path)?;
    let mut s = storage::load_plugin_settings(&root);
    s.output_dir = output_dir;
    s.folder_name = folder_name;
    storage::save_plugin_settings(&root, &s).map_err(|e| AppError::Io(e.to_string()))
}

fn plugin_root_of(plugin_path: &str) -> Result<PathBuf, AppError> {
    Path::new(plugin_path)
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| AppError::Io("plugin path has no parent directory".into()))
}

fn engine_label(info: &EngineInfo) -> String {
    match info.kind {
        EngineKind::Launcher => format!("UE {}", info.version.short()),
        EngineKind::Source => {
            let folder = info
                .root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Engine");
            format!("{folder} ({})", info.version.short())
        }
    }
}

fn engine_entry(info: &EngineInfo, source: EngineSource) -> EngineEntry {
    EngineEntry {
        root: info.root.display().to_string(),
        version: info.version.short(),
        kind: info.kind,
        label: engine_label(info),
        source,
    }
}

/// Sort key - newest engine version first (numeric, so `5.10` > `5.7`).
fn ver_key(v: &str) -> (u32, u32) {
    let mut it = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Engines for the plugin packaging picker: every machine-registered engine plus the
/// plugin's remembered (browsed) ones, validated; stale remembered entries are pruned
/// from `.uep/local.json` as a side effect. Sorted newest-first.
#[tauri::command]
#[specta::specta]
pub fn list_engines(plugin_path: String) -> Result<Vec<EngineEntry>, AppError> {
    let registry = engine::enumerate_registry_engines();
    let mut seen: std::collections::HashSet<String> = registry
        .iter()
        .map(|e| e.root.display().to_string().to_lowercase())
        .collect();
    let mut entries: Vec<EngineEntry> =
        registry.iter().map(|e| engine_entry(e, EngineSource::Registry)).collect();

    // Remembered custom engines (from the plugin's .uap/settings.json) - validate, keep
    // the valid ones (pruning stale paths), and add any not already from the registry.
    let plugin_root = plugin_root_of(&plugin_path)?;
    let mut settings = storage::load_plugin_settings(&plugin_root);
    let before = settings.engines.len();
    let mut kept: Vec<String> = Vec::new();
    for path in &settings.engines {
        match engine::engine_at(Path::new(path)) {
            Some(info) => {
                kept.push(path.clone());
                if seen.insert(path.to_lowercase()) {
                    entries.push(engine_entry(&info, EngineSource::Custom));
                }
            }
            None => { /* no longer a valid engine → drop it from the list */ }
        }
    }
    if kept.len() != before {
        settings.engines = kept;
        let _ = storage::save_plugin_settings(&plugin_root, &settings);
    }

    entries.sort_by(|a, b| ver_key(&b.version).cmp(&ver_key(&a.version)).then(a.label.cmp(&b.label)));
    Ok(entries)
}

/// Validate a browsed engine folder and remember it for this plugin; returns its
/// picker entry. Errors if the folder isn't a valid Unreal Engine root.
#[tauri::command]
#[specta::specta]
pub fn add_custom_engine(plugin_path: String, engine_dir: String) -> Result<EngineEntry, AppError> {
    let info = engine::engine_at(Path::new(&engine_dir)).ok_or_else(|| {
        AppError::InvalidEngine(
            "That folder isn't an Unreal Engine root. No Engine/Build/Build.version was found inside it.".into(),
        )
    })?;
    let plugin_root = plugin_root_of(&plugin_path)?;
    let mut settings = storage::load_plugin_settings(&plugin_root);
    if !settings.engines.iter().any(|p| p.to_lowercase() == engine_dir.to_lowercase()) {
        settings.engines.push(engine_dir.clone());
        storage::save_plugin_settings(&plugin_root, &settings).map_err(|e| AppError::Io(e.to_string()))?;
    }
    Ok(engine_entry(&info, EngineSource::Custom))
}

/// Resolve a package request into its `BuildPlugin` command + output dir (shared by
/// preview + start). Validates the engine folder and parses the plugin for `{version}`.
fn resolve_plugin_package(req: &PluginPackageRequest) -> Result<(args::PluginCommand, String), AppError> {
    let engine_root = Path::new(&req.engine_root);
    let info = engine::engine_at(engine_root)
        .ok_or_else(|| AppError::InvalidEngine(format!("not a valid Unreal Engine root: {}", req.engine_root)))?;
    let plugin = uplugin::parse_uplugin(Path::new(&req.plugin_path))?;
    let plugin_name = Path::new(&req.plugin_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Plugin")
        .to_string();

    let now = chrono::Local::now();
    let date = now.format("%Y%m%d").to_string();
    let time = now.format("%H%M%S").to_string();
    let engine_short = info.version.short();
    let ctx = args::PluginTokenContext {
        plugin: &plugin_name,
        version: &plugin.version_name,
        engine: &engine_short,
        date: &date,
        time: &time,
    };
    let package_dir = args::resolved_plugin_output_dir(&req.base_dir, &req.folder_template, &ctx);
    let cmd = args::build_plugin_command(engine_root, &req.plugin_path, &package_dir);
    Ok((cmd, package_dir))
}

/// Resolve a package request into the read-only command preview + output dir (drives
/// the Actions tab's live command box + "folder exists" hint). `{date}`/`{time}`
/// resolve to now.
#[tauri::command]
#[specta::specta]
pub fn preview_plugin_package(req: PluginPackageRequest) -> Result<PluginPreview, AppError> {
    let (cmd, package_dir) = resolve_plugin_package(&req)?;
    let exists = Path::new(&package_dir).exists();
    Ok(PluginPreview { command: cmd.preview, package_dir, exists })
}

/// Resolve + launch a `BuildPlugin` run. Returns the run id; progress arrives via
/// `uep://run-*` (same as a build). Refuses a second concurrent run.
#[tauri::command]
#[specta::specta]
pub fn start_plugin_package(
    app: AppHandle,
    state: State<AppState>,
    req: PluginPackageRequest,
) -> Result<String, AppError> {
    if let Some(active) = state.run.lock().unwrap().as_ref() {
        if active.is_running() {
            return Err(AppError::Io("a build or package is already running".into()));
        }
    }
    if req.base_dir.trim().is_empty() {
        return Err(AppError::Validation("choose an output folder first".into()));
    }
    // Remember what we packaged with, so re-opening the plugin recalls it (best-effort).
    if let Ok(root) = plugin_root_of(&req.plugin_path) {
        let mut s = storage::load_plugin_settings(&root);
        s.output_dir = req.base_dir.clone();
        s.folder_name = req.folder_template.clone();
        let _ = storage::save_plugin_settings(&root, &s);
    }
    let (cmd, package_dir) = resolve_plugin_package(&req)?;
    let plugin_name = Path::new(&req.plugin_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Plugin")
        .to_string();

    let run_id = gen_id(&format!("plugin-{plugin_name}"));
    let active = spawn_plugin_package(
        app,
        PluginPackageInputs {
            run_id: run_id.clone(),
            plugin_name,
            program: cmd.program,
            args: cmd.args,
            preview: cmd.preview,
            package_dir,
            strip_build_artifacts: req.strip_binaries_intermediate,
            started_ms: now_ms(),
        },
    );
    *state.run.lock().unwrap() = Some(active);
    Ok(run_id)
}

// ── Editor commandlet tools (Resave / Validate) ─────────────────────────────────
//
// Project-side maintenance actions on the project Tools tab. Each runs the open
// project's own detected engine editor as a single commandlet child on the shared
// runner (streams `uep://run-*`, cancellable, no history). The `.uproject` + engine
// come from managed state; the frontend passes only the tool's options.

/// Options for the Resave tool (resaves the whole project).
#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResaveOptions {
    /// `-ProjectOnly` - skip the engine's own content (on by default in the UI).
    pub project_only: bool,
    pub fixup_redirectors: bool,
    pub skip_shader_compile: bool,
}

/// Resolve the open project + its engine, build the commandlet via `cmd_for`, and launch
/// it on the shared runner. Refuses a second concurrent run; errors if no project is open.
fn launch_commandlet(
    app: AppHandle,
    state: State<AppState>,
    title: &str,
    cmd_for: impl FnOnce(&EngineInfo, &str) -> args::CommandletCommand,
) -> Result<String, AppError> {
    if let Some(active) = state.run.lock().unwrap().as_ref() {
        if active.is_running() {
            return Err(AppError::Io("a build or tool is already running".into()));
        }
    }
    let (program, cmd_args, preview, project_name) = {
        let guard = state.current.read().unwrap();
        let proj = guard
            .as_ref()
            .ok_or_else(|| AppError::Io("no project is open".into()))?;
        let cmd = cmd_for(&proj.engine, proj.uproject_path.as_str());
        (cmd.program, cmd.args, cmd.preview, proj.name.clone())
    };
    let run_id = gen_id(&format!("tool-{title}"));
    let active = spawn_commandlet(
        app,
        CommandletInputs {
            run_id: run_id.clone(),
            title: title.to_string(),
            project_name,
            program,
            args: cmd_args,
            preview,
            started_ms: now_ms(),
        },
    );
    *state.run.lock().unwrap() = Some(active);
    Ok(run_id)
}

/// Launch a **Resave** run (`-run=ResavePackages`) on the open project - bakes in Core
/// Redirects, fixes up object redirectors, re-serializes assets. Returns the run id;
/// progress arrives via `uep://run-*`.
#[tauri::command]
#[specta::specta]
pub fn start_resave(
    app: AppHandle,
    state: State<AppState>,
    options: ResaveOptions,
) -> Result<String, AppError> {
    launch_commandlet(app, state, "Resave Assets", move |engine, uproject| {
        args::build_resave_command(
            engine,
            uproject,
            options.project_only,
            options.fixup_redirectors,
            options.skip_shader_compile,
        )
    })
}

/// Options for the Validate tool. `skip_engine_content` ⇒ the default project-only scope;
/// unticking it passes `-includeengine`. `asset_type` (empty ⇒ all) restricts validation to
/// one class and its subclasses (a short name or a full `/Script/...` path).
#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ValidateOptions {
    pub skip_engine_content: bool,
    pub asset_type: String,
}

/// Launch a **Validate** run (`-run=DataValidation`) on the open project - runs the enabled
/// asset validators. Returns the run id; progress arrives via `uep://run-*`.
#[tauri::command]
#[specta::specta]
pub fn start_validate(
    app: AppHandle,
    state: State<AppState>,
    options: ValidateOptions,
) -> Result<String, AppError> {
    launch_commandlet(app, state, "Validate Assets", move |engine, uproject| {
        args::build_validate_command(engine, uproject, !options.skip_engine_content, &options.asset_type)
    })
}

// ── Steam upload (steamcmd) ──────────────────────────────────────────────────────
//
// The Steam upload phase's machine-local settings (steamcmd path + build account) live in
// `<project>/.uep/steam-config/local.json` (git-ignored), separate from the committed
// profile/VDFs. `steam_login` runs steamcmd's interactive login once so it caches a session
// for non-interactive uploads; the password + Steam Guard code are transient (never stored).

/// The open project's machine-local Steam settings (steamcmd path + build account).
#[tauri::command]
#[specta::specta]
pub fn load_steam_settings(state: State<AppState>) -> Result<storage::SteamLocalSettings, AppError> {
    let root = open_project_root(&state).ok_or_else(|| AppError::Io("no project is open".into()))?;
    Ok(storage::load_steam_local_settings(Path::new(&root)))
}

/// Persist the open project's machine-local Steam settings (git-ignored `local.json`).
#[tauri::command]
#[specta::specta]
pub fn save_steam_settings(
    state: State<AppState>,
    settings: storage::SteamLocalSettings,
) -> Result<(), AppError> {
    let root = open_project_root(&state).ok_or_else(|| AppError::Io("no project is open".into()))?;
    storage::save_steam_local_settings(Path::new(&root), &settings).map_err(|e| AppError::Io(e.to_string()))
}

/// Setup status for the "Setup SteamCMD" modal: whether steamcmd is found at the saved path,
/// and whether it can sign in (a cached session for the saved account). The sign-in check runs
/// `steamcmd +login <account> +quit` (no password), so it can be slow on steamcmd's first run.
#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SteamStatus {
    pub steamcmd_found: bool,
    pub logged_in: bool,
    pub message: String,
}

#[tauri::command]
#[specta::specta]
pub async fn steam_status(state: State<'_, AppState>) -> Result<SteamStatus, AppError> {
    let root = open_project_root(&state).ok_or_else(|| AppError::Io("no project is open".into()))?;
    let steam = storage::load_steam_local_settings(Path::new(&root));
    let resolved = resolve_local_path(&root, &steam.steamcmd_path);
    let found = !resolved.trim().is_empty() && Path::new(&resolved).exists();
    if !found {
        let message = if steam.steamcmd_path.trim().is_empty() {
            "steamcmd path not set.".to_string()
        } else {
            format!("steamcmd.exe not found at {resolved}")
        };
        return Ok(SteamStatus { steamcmd_found: false, logged_in: false, message });
    }
    if steam.account.trim().is_empty() {
        return Ok(SteamStatus {
            steamcmd_found: true,
            logged_in: false,
            message: "Set the build account to check sign-in.".to_string(),
        });
    }
    let logged_in = crate::steam::login::verify(&resolved, steam.account.trim()).await.status
        == crate::steam::login::SteamLoginStatus::Success;
    // The modal composes "Signed in as <account>" from the account itself, so no message when OK.
    let message = if logged_in {
        String::new()
    } else {
        "You'll sign in at the build's Steam Login step.".to_string()
    };
    Ok(SteamStatus { steamcmd_found: true, logged_in, message })
}

/// Open steamcmd in its own console for an interactive sign-in - the Setup modal's "Try sign
/// in" link. Fire-and-forget; the user signs in there, then re-checks status.
#[tauri::command]
#[specta::specta]
pub fn steam_open_login_terminal(state: State<AppState>) -> Result<(), AppError> {
    let root = open_project_root(&state).ok_or_else(|| AppError::Io("no project is open".into()))?;
    let steam = storage::load_steam_local_settings(Path::new(&root));
    if steam.steamcmd_path.trim().is_empty() {
        return Err(AppError::Validation("Set the steamcmd path first.".into()));
    }
    let steamcmd = resolve_local_path(&root, &steam.steamcmd_path);
    crate::steam::login::open_login_terminal(&steamcmd, steam.account.trim())
        .map_err(|e| AppError::Io(e.to_string()))
}

// ── Remove UnrealEasyPackage from a project / plugin ─────────────────────────────
//
// Deletes everything UEP stores for the open project (its `<root>/.uep/`) or plugin
// (its `<root>/.uap/`), forgets it from the app-folder recents, and clears it from
// managed state. The user's actual project/plugin files are untouched. Destructive and
// irreversible (build history + logs are local-only) - the UI confirms first.

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemovedUep {
    /// The deleted data folder (`<project>/.uep` or `<plugin>/.uap`).
    pub path: String,
    /// Whether the folder existed (false ⇒ nothing to delete, but the recent is still cleared).
    pub existed: bool,
    pub freed_bytes: f64,
}

/// Delete the open project's `.uep/` (or open plugin's `.uap/`), forget its recent, and
/// clear it from managed state. The gate opens a project XOR a plugin; project takes
/// precedence. Heavy delete → off the UI thread.
#[tauri::command]
#[specta::specta]
pub async fn remove_uep_data(app: AppHandle, state: State<'_, AppState>) -> Result<RemovedUep, AppError> {
    // Resolve the target folder + descriptor, releasing the state guards before any await.
    let project = state
        .current
        .read()
        .unwrap()
        .as_ref()
        .map(|p| (p.project_root.clone(), p.uproject_path.clone()));
    let (folder, descriptor, is_project) = if let Some((root, uproject)) = project {
        (Path::new(&root).join(".uep"), uproject, true)
    } else {
        let plugin = state
            .current_plugin
            .read()
            .unwrap()
            .as_ref()
            .map(|p| (p.plugin_root.clone(), p.uplugin_path.clone()));
        let (root, uplugin) =
            plugin.ok_or_else(|| AppError::Io("no project or plugin is open".into()))?;
        (Path::new(&root).join(".uap"), uplugin, false)
    };

    // Size + delete off the UI thread (a project's .uep/history can be large).
    let target = folder.clone();
    let (existed, freed) = tauri::async_runtime::spawn_blocking(move || -> std::io::Result<(bool, u64)> {
        if !target.exists() {
            return Ok((false, 0));
        }
        let size = history::store::dir_size(&target);
        std::fs::remove_dir_all(&target)?;
        Ok((true, size))
    })
    .await
    .map_err(|e| AppError::Io(e.to_string()))?
    .map_err(|e| AppError::Io(e.to_string()))?;

    // Forget the descriptor from recents, then clear managed state (its data is gone).
    let dir = data_dir(&app)?;
    let _ = storage::remove_recent(&dir, &descriptor);
    if is_project {
        *state.current.write().unwrap() = None;
    } else {
        *state.current_plugin.write().unwrap() = None;
    }

    Ok(RemovedUep { path: folder.display().to_string(), existed, freed_bytes: freed as f64 })
}
