//! The async **DAG executor** (`docs/build-commands.md` §8, `docs/requirement.md`
//! R4). Walks the pure [`plan`] over the arg-builder's execution units, spawning
//! each `External` unit as its own child process (`tokio::process`) and running
//! the two `App` units (Copy Extras / Clean-up) in-process. It:
//!
//! - streams every child's stdout+stderr, classifying each line ([`classify`])
//!   and emitting batched `uep://run-log` events (custom log console);
//! - tracks per-phase status + start-offset/duration, emitting `uep://run-phase`
//!   on each transition (the live Jenkins graph);
//! - overlaps **Build (game) ∥ Cook** (the plan's one concurrent stage);
//! - supports a clean **Cancel** (kills running children, marks the rest);
//! - decides success/failed from the **process exit code**, never the log text.
//!
//! It also keeps a capped, shared [`RunSnapshot`] so a Build Logs window opened
//! *after* the run started can backfill via the `active_run` command, then follow
//! live events. Tauri-facing (emits events, holds `AppHandle`) ⇒ compiled out of
//! `cfg(test)`; the tested logic lives in the sibling pure modules.
#![allow(dead_code)]

use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

use crate::history;
use crate::pipeline::{PhaseId, PhaseKind};
use crate::profiles::schema::BuildConfig;
use crate::unreal::args::PhaseCommand;

use super::classify::{classify_line, Severity};
use super::plan;

// ── event channel names (the frontend listens on these) ───────────────────────
const EV_STARTED: &str = "uep://run-started";
const EV_LOG: &str = "uep://run-log";
const EV_PHASE: &str = "uep://run-phase";
const EV_FINISHED: &str = "uep://run-finished";

/// How many trailing log lines the snapshot retains for late-joining windows.
const LINE_BUF_CAP: usize = 8000;
/// Flush a phase's pending lines at least this often (keeps the stream "live").
const FLUSH_INTERVAL: Duration = Duration::from_millis(60);
/// …or sooner once this many lines have queued.
const FLUSH_LINES: usize = 256;

// ── serializable run model (also the IPC return / event payloads) ──────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum PhaseStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Success,
    Failed,
    Cancelled,
}

/// One graph node = one execution unit (the Stage·Pak·Archive unit is a single
/// node, matching the design). `level` is its scheduling stage = its graph column
/// (parallel siblings share a column).
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PhaseNode {
    pub index: u32,
    pub label: String,
    pub phase: PhaseId,
    pub kind: PhaseKind,
    pub level: u32,
    /// The resolved command line (`External`) or app-action preview (`App`) - shown
    /// read-only in the Build Logs "Command" island.
    pub command: String,
    pub status: PhaseStatus,
    pub start_offset_ms: Option<f64>,
    pub duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub seq: u32,
    pub phase_index: u32,
    pub severity: Severity,
    pub text: String,
}

/// The full live state of a run - returned by `active_run` so a freshly-opened
/// Build Logs window can render the graph + backfill the console before following
/// live events.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshot {
    pub run_id: String,
    pub project: String,
    pub platform: String,
    /// Every client config staged this run (one or more); shown as tags and recorded
    /// one-per-config in history. Empty for plugin/commandlet runs.
    pub configs: Vec<String>,
    pub target: String,
    pub output_dir: String,
    pub started_ms: f64,
    pub status: RunStatus,
    pub phases: Vec<PhaseNode>,
    pub lines: Vec<LogLine>,
    /// Single resolved command line - set for a plugin-package run or an editor-commandlet
    /// tool run (the Run Log window shows it). Empty for a build run, whose per-phase
    /// commands live on the `phases` nodes instead.
    pub command: String,
    /// Heading for the single-command Run Log window: "Package Plugin" for a plugin
    /// package, the tool name ("Resave Assets" / "Validate Assets") for a commandlet tool.
    /// Empty for a build run (the Build Logs window has its own heading).
    pub title: String,
}

// ── event payloads (emit-only; not part of any command signature) ──────────────

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogBatch {
    run_id: String,
    lines: Vec<LogLine>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PhaseUpdate {
    run_id: String,
    phase: PhaseNode,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunFinished {
    run_id: String,
    status: RunStatus,
    duration_ms: f64,
}

// ── process group: kill the whole build tree ───────────────────────────────────
//
// A build phase is `cmd /C RunUAT.bat …`, which fans out to AutomationTool → UBT →
// cl.exe / UnrealEditor-Cmd. TerminateProcess on the immediate child leaves that
// whole tree alive - which is why Cancel "didn't work" and why closing the app
// orphaned the build. On Windows we put every spawned child in one **Job Object**
// with KILL_ON_JOB_CLOSE: Cancel terminates the entire tree at once, and because the
// kernel closes the job handle when our process dies, quitting mid-build kills the
// orphans too - no reliance on Rust drop glue running. Other OSes get an inert stub
// (Windows-first MVP).
#[cfg(windows)]
mod procgroup {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    // windows-sys gates the `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` struct behind a
    // feature its functions don't pull in, so we declare the layout locally (stable
    // x64 ABI: 144 bytes, `limit_flags` at offset 16). We only zero it and set one
    // flag - KILL_ON_JOB_CLOSE must go through the *extended* class, not basic.
    #[allow(dead_code)]
    #[repr(C)]
    struct JobExtendedLimitInfo {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
        io_read_operation_count: u64,
        io_write_operation_count: u64,
        io_other_operation_count: u64,
        io_read_transfer_count: u64,
        io_write_transfer_count: u64,
        io_other_transfer_count: u64,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    /// Owns a Job Object handle (null ⇒ inert: creation failed, methods no-op).
    pub struct ProcGroup(HANDLE);
    // A job handle is process-global; sound to use from any thread.
    unsafe impl Send for ProcGroup {}
    unsafe impl Sync for ProcGroup {}

    impl ProcGroup {
        pub fn new() -> Self {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return ProcGroup(std::ptr::null_mut());
                }
                let mut info: JobExtendedLimitInfo = std::mem::zeroed();
                info.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info) as *const core::ffi::c_void,
                    std::mem::size_of::<JobExtendedLimitInfo>() as u32,
                ) == 0
                {
                    CloseHandle(job);
                    return ProcGroup(std::ptr::null_mut());
                }
                ProcGroup(job)
            }
        }

        /// Put a freshly-spawned child - and every process it spawns - in the job.
        pub fn adopt(&self, child: &tokio::process::Child) {
            if self.0.is_null() {
                return;
            }
            if let Some(h) = child.raw_handle() {
                unsafe { AssignProcessToJobObject(self.0, h as HANDLE) };
            }
        }

        /// Terminate every process in the job (the whole build tree).
        pub fn kill(&self) {
            if !self.0.is_null() {
                unsafe { TerminateJobObject(self.0, 1) };
            }
        }
    }

    impl Drop for ProcGroup {
        fn drop(&mut self) {
            // Closing the last handle fires KILL_ON_JOB_CLOSE for any survivors.
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }
}

#[cfg(not(windows))]
mod procgroup {
    /// Non-Windows stub - process-tree teardown is Windows-first for the MVP.
    pub struct ProcGroup;
    impl ProcGroup {
        pub fn new() -> Self {
            ProcGroup
        }
        pub fn adopt(&self, _child: &tokio::process::Child) {}
        pub fn kill(&self) {}
    }
}

use procgroup::ProcGroup;

// ── managed-state handle ───────────────────────────────────────────────────────

/// Stored in `AppState.run`. Holds the cancel signal, the process group, and the
/// live snapshot the `active_run`/`cancel_build` commands reach through.
pub struct ActiveRun {
    pub run_id: String,
    cancel: watch::Sender<bool>,
    snapshot: Arc<Mutex<RunSnapshot>>,
    proc_group: Arc<ProcGroup>,
}

impl ActiveRun {
    pub fn snapshot(&self) -> RunSnapshot {
        self.snapshot.lock().unwrap().clone()
    }
    pub fn is_running(&self) -> bool {
        self.snapshot.lock().unwrap().status == RunStatus::Running
    }
    pub fn cancel(&self) {
        let _ = self.cancel.send(true);
        // Kill the whole process tree - not just the immediate cmd.exe.
        self.proc_group.kill();
    }
}

/// Everything `start_build` resolves up front (synchronously, while it holds the
/// project state) and hands to the spawned executor - so the async task owns only
/// `'static` data, never a borrow of managed state.
pub struct RunInputs {
    pub run_id: String,
    pub units: Vec<PhaseCommand>,
    pub profile: BuildConfig,
    pub project_root: String,
    pub output_dir: String,
    pub project: String,
    pub platform: String,
    pub configs: Vec<String>,
    pub target: String,
    /// Editor target name (for the Clean-up phase's footprint scope), if detected.
    pub editor_target: Option<String>,
    pub started_ms: f64,
}

/// Build the initial snapshot, emit `run-started`, spawn the executor task, and
/// return the `ActiveRun` handle for the caller to store in managed state.
pub fn spawn_run(app: AppHandle, inputs: RunInputs) -> ActiveRun {
    let levels = plan::levels(&inputs.units);
    let phases: Vec<PhaseNode> = inputs
        .units
        .iter()
        .enumerate()
        .map(|(i, u)| PhaseNode {
            index: i as u32,
            label: u.label.clone(),
            phase: u.phase,
            kind: u.kind,
            level: levels[i],
            command: u.preview.clone(),
            status: PhaseStatus::Pending,
            start_offset_ms: None,
            duration_ms: None,
        })
        .collect();

    let snapshot = Arc::new(Mutex::new(RunSnapshot {
        run_id: inputs.run_id.clone(),
        project: inputs.project,
        platform: inputs.platform,
        configs: inputs.configs,
        target: inputs.target.clone(),
        output_dir: inputs.output_dir.clone(),
        started_ms: inputs.started_ms,
        status: RunStatus::Running,
        phases,
        lines: Vec::new(),
        command: String::new(), // build runs carry per-phase commands on `phases`
        title: String::new(),   // the Build Logs window has its own heading
    }));

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let proc_group = Arc::new(ProcGroup::new());
    let active = ActiveRun {
        run_id: inputs.run_id.clone(),
        cancel: cancel_tx,
        snapshot: snapshot.clone(),
        proc_group: proc_group.clone(),
    };

    let _ = app.emit(EV_STARTED, snapshot.lock().unwrap().clone());

    let stages: Vec<Vec<usize>> = plan::plan(&inputs.units).into_iter().map(|s| s.units).collect();
    let ctx = ExecCtx {
        app,
        snapshot: snapshot.clone(),
        seq: Arc::new(AtomicU32::new(1)),
        units: Arc::new(inputs.units),
        profile: inputs.profile,
        project_root: inputs.project_root,
        output_dir: inputs.output_dir,
        target: inputs.target,
        editor_target: inputs.editor_target,
        proc_group,
        warnings: Arc::new(AtomicU32::new(0)),
        errors: Arc::new(AtomicU32::new(0)),
    };
    tauri::async_runtime::spawn(execute(ctx, stages, cancel_rx));
    active
}

// ── plugin packaging run (RunUAT BuildPlugin) ───────────────────────────────────
//
// A standalone, non-pipeline run: one external `BuildPlugin` command, optionally
// followed by an app-owned strip of the packaged `Binaries/` + `Intermediate/` (the
// FAB-submission requirement). It reuses the build runner's whole live-run substrate
// - `ActiveRun`/`RunSnapshot` in `AppState.run`, the `uep://run-*` events, the
// `ProcGroup` tree-kill, the line pump + classifier - so the Actions-tab console,
// Cancel, and the close-to-tray guard all work unchanged. It deliberately does NOT
// write build history (packaging a plugin isn't a project build) and carries no
// `PhaseId` graph (the snapshot's `phases` stay empty).

/// Everything `start_plugin_package` resolves up front and hands to the executor.
pub struct PluginPackageInputs {
    pub run_id: String,
    /// Display name (the plugin) for the snapshot + finish toast.
    pub plugin_name: String,
    pub program: String,
    pub args: Vec<String>,
    /// Full resolved command line (echoed into the console).
    pub preview: String,
    /// The `-package` output directory (cleared before the run; stripped after).
    pub package_dir: String,
    /// Remove `<package>/Binaries` + `<package>/Intermediate` on success (FAB).
    pub strip_build_artifacts: bool,
    pub started_ms: f64,
}

/// Build the snapshot, emit `run-started`, spawn the package task, and return the
/// `ActiveRun` handle to store in managed state (same slot as a build run).
pub fn spawn_plugin_package(app: AppHandle, inputs: PluginPackageInputs) -> ActiveRun {
    let snapshot = Arc::new(Mutex::new(RunSnapshot {
        run_id: inputs.run_id.clone(),
        project: inputs.plugin_name,
        platform: String::new(),
        configs: Vec::new(),
        target: String::new(),
        output_dir: inputs.package_dir.clone(),
        started_ms: inputs.started_ms,
        status: RunStatus::Running,
        phases: Vec::new(),
        lines: Vec::new(),
        command: inputs.preview.clone(), // shown in the Run Log window's Command island
        title: "Package Plugin".to_string(),
    }));

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let proc_group = Arc::new(ProcGroup::new());
    let active = ActiveRun {
        run_id: inputs.run_id.clone(),
        cancel: cancel_tx,
        snapshot: snapshot.clone(),
        proc_group: proc_group.clone(),
    };

    let _ = app.emit(EV_STARTED, snapshot.lock().unwrap().clone());

    // The pump/emit helpers operate over an `ExecCtx`; a plugin run has no profile or
    // units, so we hand them inert defaults (those fields are never read on this path
    // - `run_external`/`run_app`/`write_history` aren't called).
    let ctx = ExecCtx {
        app,
        snapshot: snapshot.clone(),
        seq: Arc::new(AtomicU32::new(1)),
        units: Arc::new(Vec::new()),
        profile: BuildConfig::default(),
        project_root: String::new(),
        output_dir: inputs.package_dir.clone(),
        target: String::new(),
        editor_target: None,
        proc_group,
        warnings: Arc::new(AtomicU32::new(0)),
        errors: Arc::new(AtomicU32::new(0)),
    };
    tauri::async_runtime::spawn(execute_plugin(
        ctx,
        inputs.program,
        inputs.args,
        inputs.preview,
        inputs.package_dir,
        inputs.strip_build_artifacts,
        cancel_rx,
    ));
    active
}

async fn execute_plugin(
    ctx: ExecCtx,
    program: String,
    args: Vec<String>,
    preview: String,
    package_dir: String,
    strip: bool,
    mut cancel_rx: watch::Receiver<bool>,
) {
    let started = Instant::now();
    emit_line(&ctx, 0, Severity::Info, &format!("▶ {preview}"));

    // BuildPlugin writes into `-package` over the top and won't clear stale files, so
    // start pristine (the reference `.bat` does an `rmdir /s /q` first). Guarded: only
    // a path with a parent (never a drive root). Off-thread - the tree can be large.
    {
        let path = Path::new(&package_dir);
        if path.exists() && path.parent().is_some() {
            emit_line(&ctx, 0, Severity::Info, &format!("Clearing package dir: {package_dir}"));
            let dir = package_dir.clone();
            match tauri::async_runtime::spawn_blocking(move || std::fs::remove_dir_all(&dir)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => emit_line(&ctx, 0, Severity::Warning, &format!("could not clear package dir: {e}")),
                Err(e) => emit_line(&ctx, 0, Severity::Warning, &format!("package-dir clear task failed: {e}")),
            }
        }
    }

    let status = run_single_child(&ctx, &mut cancel_rx, &program, &args, "BuildPlugin").await;

    // Strip the build artifacts on success (FAB submission requires the packaged
    // plugin carry no compiled output - Binaries/ + Intermediate/).
    if status == RunStatus::Success && strip {
        emit_line(&ctx, 0, Severity::Info, "Stripping Binaries/ and Intermediate/ for FAB submission…");
        for sub in ["Binaries", "Intermediate"] {
            let target = Path::new(&package_dir).join(sub);
            if !target.exists() {
                continue;
            }
            let target_s = target.display().to_string();
            let target_c = target.clone();
            match tauri::async_runtime::spawn_blocking(move || std::fs::remove_dir_all(&target_c)).await {
                Ok(Ok(())) => emit_line(&ctx, 0, Severity::Info, &format!("  removed {sub}/")),
                Ok(Err(e)) => emit_line(&ctx, 0, Severity::Warning, &format!("  could not remove {target_s}: {e}")),
                Err(e) => emit_line(&ctx, 0, Severity::Warning, &format!("  strip task failed for {sub}/: {e}")),
            }
        }
    }

    let final_status = status;
    {
        let mut s = ctx.snapshot.lock().unwrap();
        s.status = final_status;
    }
    let elapsed = started.elapsed();
    let _ = ctx.app.emit(
        EV_FINISHED,
        RunFinished {
            run_id: ctx.run_id(),
            status: final_status,
            duration_ms: elapsed.as_millis() as f64,
        },
    );
    notify_finish(&ctx, final_status, elapsed.as_secs_f64(), "Package");
}

/// Spawn + stream a single child (a plugin package or an editor-commandlet tool),
/// deciding success/failed from its exit code (never the log text) and honoring cancel
/// - a trimmed `run_external`. `noun` labels the process in the exit-code line.
async fn run_single_child(
    ctx: &ExecCtx,
    cancel_rx: &mut watch::Receiver<bool>,
    program: &str,
    args: &[String],
    noun: &str,
) -> RunStatus {
    let mut cmd = build_command(program, args, "");
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_line(ctx, 0, Severity::Error, &format!("failed to launch {program}: {e}"));
            return RunStatus::Failed;
        }
    };
    ctx.proc_group.adopt(&child);

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let h1 = stdout.map(|s| tokio::spawn(pump(ctx.clone(), 0, s)));
    let h2 = stderr.map(|s| tokio::spawn(pump(ctx.clone(), 0, s)));

    let mut killed = false;
    let wait = tokio::select! {
        res = child.wait() => res,
        _ = cancel_rx.changed() => {
            killed = true;
            let _ = child.start_kill();
            child.wait().await
        }
    };
    if let Some(h) = h1 {
        let _ = h.await;
    }
    if let Some(h) = h2 {
        let _ = h.await;
    }

    if killed || *cancel_rx.borrow() {
        emit_line(ctx, 0, Severity::Warning, "cancelled");
        return RunStatus::Cancelled;
    }
    match wait {
        Ok(status) if status.success() => RunStatus::Success,
        Ok(status) => {
            let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "terminated".into());
            emit_line(ctx, 0, Severity::Error, &format!("{noun} exited with code {code}"));
            RunStatus::Failed
        }
        Err(e) => {
            emit_line(ctx, 0, Severity::Error, &format!("process error: {e}"));
            RunStatus::Failed
        }
    }
}

// ── editor commandlet tool run (UnrealEditor-Cmd -run=…) ─────────────────────────
//
// A project-side maintenance tool (Resave / Validate) is, like plugin packaging, a
// single external child with no pipeline graph and no history. It reuses the same
// live-run substrate (ActiveRun/RunSnapshot in AppState.run, the uep://run-* events,
// the ProcGroup tree-kill, the line pump + classifier), so the Tools-tab run buttons,
// the shared Run Log window, Cancel, and the close-to-tray guard all work unchanged.

/// Everything `start_resave`/`start_validate` resolve up front and hand to the executor.
pub struct CommandletInputs {
    pub run_id: String,
    /// Run Log heading + finish-toast noun ("Resave Assets" / "Validate Assets").
    pub title: String,
    /// The open project's name - the run subtitle + toast body.
    pub project_name: String,
    pub program: String,
    pub args: Vec<String>,
    /// Full resolved command line (echoed into the console + shown in the Command island).
    pub preview: String,
    pub started_ms: f64,
}

/// Build the snapshot, emit `run-started`, spawn the tool task, and return the
/// `ActiveRun` handle to store in managed state (same slot as a build/package run).
pub fn spawn_commandlet(app: AppHandle, inputs: CommandletInputs) -> ActiveRun {
    let snapshot = Arc::new(Mutex::new(RunSnapshot {
        run_id: inputs.run_id.clone(),
        project: inputs.project_name,
        platform: String::new(),
        configs: Vec::new(),
        target: String::new(),
        output_dir: String::new(), // a commandlet tool produces no output folder
        started_ms: inputs.started_ms,
        status: RunStatus::Running,
        phases: Vec::new(),
        lines: Vec::new(),
        command: inputs.preview.clone(),
        title: inputs.title.clone(),
    }));

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let proc_group = Arc::new(ProcGroup::new());
    let active = ActiveRun {
        run_id: inputs.run_id.clone(),
        cancel: cancel_tx,
        snapshot: snapshot.clone(),
        proc_group: proc_group.clone(),
    };

    let _ = app.emit(EV_STARTED, snapshot.lock().unwrap().clone());

    // Same inert ExecCtx defaults as the plugin path - only the pump/emit + notify_finish
    // helpers are exercised; no profile/units/history are read on this path.
    let ctx = ExecCtx {
        app,
        snapshot: snapshot.clone(),
        seq: Arc::new(AtomicU32::new(1)),
        units: Arc::new(Vec::new()),
        profile: BuildConfig::default(),
        project_root: String::new(),
        output_dir: String::new(),
        target: String::new(),
        editor_target: None,
        proc_group,
        warnings: Arc::new(AtomicU32::new(0)),
        errors: Arc::new(AtomicU32::new(0)),
    };
    tauri::async_runtime::spawn(execute_commandlet(
        ctx,
        inputs.program,
        inputs.args,
        inputs.preview,
        inputs.title,
        cancel_rx,
    ));
    active
}

async fn execute_commandlet(
    ctx: ExecCtx,
    program: String,
    args: Vec<String>,
    preview: String,
    title: String,
    mut cancel_rx: watch::Receiver<bool>,
) {
    let started = Instant::now();
    emit_line(&ctx, 0, Severity::Info, &format!("▶ {preview}"));

    let status = run_single_child(&ctx, &mut cancel_rx, &program, &args, &title).await;

    {
        let mut s = ctx.snapshot.lock().unwrap();
        s.status = status;
    }
    let elapsed = started.elapsed();
    let _ = ctx.app.emit(
        EV_FINISHED,
        RunFinished {
            run_id: ctx.run_id(),
            status,
            duration_ms: elapsed.as_millis() as f64,
        },
    );
    notify_finish(&ctx, status, elapsed.as_secs_f64(), &title);
}

/// Shared, cheaply-clonable context threaded through the executor tasks (the units
/// vec is immutable, shared by `Arc`).
#[derive(Clone)]
struct ExecCtx {
    app: AppHandle,
    snapshot: Arc<Mutex<RunSnapshot>>,
    seq: Arc<AtomicU32>,
    units: Arc<Vec<PhaseCommand>>,
    profile: BuildConfig,
    project_root: String,
    output_dir: String,
    /// The resolved game target + editor target - the Clean-up phase's footprint scope.
    target: String,
    editor_target: Option<String>,
    proc_group: Arc<ProcGroup>,
    /// Running tally of Warning/Error severities seen in streamed child output -
    /// folded into the build record at finalize (the Dashboard's health trend).
    warnings: Arc<AtomicU32>,
    errors: Arc<AtomicU32>,
}

impl ExecCtx {
    fn unit(&self, i: usize) -> &PhaseCommand {
        &self.units[i]
    }
}

async fn execute(ctx: ExecCtx, stages: Vec<Vec<usize>>, cancel_rx: watch::Receiver<bool>) {
    let started = Instant::now();
    let mut failed = false;
    let mut cancelled = false;

    for stage in &stages {
        if failed || cancelled || *cancel_rx.borrow() {
            break;
        }
        // Run the units in this stage concurrently (the one real overlap is
        // Build ∥ Cook); await all before the next stage (barrier).
        let mut handles = Vec::new();
        for &ui in stage {
            let ctx = ctx.clone();
            let cancel_rx = cancel_rx.clone();
            handles.push(tokio::spawn(run_phase(ctx, cancel_rx, ui as u32, started)));
        }
        for h in handles {
            match h.await {
                Ok(PhaseStatus::Failed) => failed = true,
                Ok(PhaseStatus::Cancelled) => cancelled = true,
                _ => {}
            }
        }
        if *cancel_rx.borrow() {
            cancelled = true;
        }
    }

    let final_status = if cancelled {
        RunStatus::Cancelled
    } else if failed {
        RunStatus::Failed
    } else {
        RunStatus::Success
    };

    // Settle any phase that never ran (downstream of a failure/cancel).
    let phases: Vec<PhaseNode> = {
        let mut s = ctx.snapshot.lock().unwrap();
        for p in &mut s.phases {
            if matches!(p.status, PhaseStatus::Pending | PhaseStatus::Running) {
                p.status = if cancelled {
                    PhaseStatus::Cancelled
                } else {
                    PhaseStatus::Skipped
                };
            }
        }
        s.status = final_status;
        s.phases.clone()
    };
    let run_id = ctx.run_id();
    for p in phases {
        let _ = ctx.app.emit(
            EV_PHASE,
            PhaseUpdate {
                run_id: run_id.clone(),
                phase: p,
            },
        );
    }
    // Persist the record *before* announcing completion: listeners refetch history
    // on EV_FINISHED, so the record must already be on disk. write_history walks the
    // whole output tree (dir_size) and is slow - emitting first caused a stale-list
    // race (the new build only appeared after a tab-switch remounted the view). One
    // `elapsed` so the recorded duration and the emitted duration_ms agree.
    let elapsed = started.elapsed();
    write_history(&ctx, elapsed.as_secs_f64(), final_status);
    let _ = ctx.app.emit(
        EV_FINISHED,
        RunFinished {
            run_id: ctx.run_id(),
            status: final_status,
            duration_ms: elapsed.as_millis() as f64,
        },
    );
    notify_finish(&ctx, final_status, elapsed.as_secs_f64(), "Build");
}

/// Fire an OS toast announcing the run result - gated on the saved notification
/// preference (M6). Fired from the backend so it works even when the log window is
/// closed. `noun` is the action label ("Build" for a project build, "Package" for a
/// plugin package) so the toast reads correctly for each. Best-effort: any failure
/// (no settings, toast denied) is silent.
fn notify_finish(ctx: &ExecCtx, status: RunStatus, duration_secs: f64, noun: &str) {
    use tauri::Manager;
    use tauri_plugin_notification::NotificationExt;

    let Ok(dir) = ctx.app.path().app_data_dir() else {
        return;
    };
    let prefs = crate::settings::load(&dir);
    if !prefs.notify_on_finish {
        return;
    }
    let (title, verb) = match status {
        RunStatus::Success => (format!("{noun} succeeded"), "finished"),
        RunStatus::Failed => (format!("{noun} failed"), "failed"),
        RunStatus::Cancelled => (format!("{noun} cancelled"), "was cancelled"),
        RunStatus::Running => return, // never called mid-run
    };
    let project = ctx.snapshot.lock().unwrap().project.clone();
    let secs = duration_secs.round() as u64;
    let elapsed = if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    };
    let body = format!("{project} {verb} in {elapsed}.");
    let mut builder = ctx.app.notification().builder().title(title).body(body);
    if prefs.notify_sound {
        builder = builder.sound("Default");
    }
    let _ = builder.show();
}

impl ExecCtx {
    fn run_id(&self) -> String {
        self.snapshot.lock().unwrap().run_id.clone()
    }
}

fn phase_status_label(s: PhaseStatus) -> &'static str {
    match s {
        PhaseStatus::Success => "Success",
        PhaseStatus::Failed => "Failed",
        PhaseStatus::Cancelled => "Cancelled",
        // Pending/Running shouldn't survive finalize, but record them as Skipped.
        _ => "Skipped",
    }
}

fn run_status_label(s: RunStatus) -> &'static str {
    match s {
        RunStatus::Success => "Success",
        RunStatus::Failed => "Failed",
        _ => "Cancelled",
    }
}

/// The folder a finished build actually produced - what "Open build folder" opens
/// and what we size/timestamp. Archive dir when the build archived; else the staged
/// tree when it only staged; else empty (build/cook-only ⇒ nothing to open). The
/// staged path assumes UE's `Saved/StagedBuilds/<platform>` layout (correct for the
/// reference Win64 game target), falling back to the parent `StagedBuilds` if that
/// subfolder is absent (server/client-split targets stage under a different name).
fn openable_output(ctx: &ExecCtx) -> String {
    let ph = &ctx.profile.phases;
    if ph.stage.enabled && ph.archive.enabled {
        ctx.output_dir.clone()
    } else if ph.stage.enabled {
        let staged = Path::new(&ctx.project_root)
            .join("Saved")
            .join("StagedBuilds")
            .join(ctx.profile.platform.folder());
        if staged.exists() {
            return staged.display().to_string();
        }
        let parent = Path::new(&ctx.project_root).join("Saved").join("StagedBuilds");
        if parent.exists() {
            parent.display().to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    }
}

/// Assemble the lean [`history::schema::BuildRecord`] from the final snapshot and
/// write it (+ the captured log) under `<project>/.uep/history/<buildId>/`.
fn write_history(ctx: &ExecCtx, total_secs: f64, final_status: RunStatus) {
    let open_path = openable_output(ctx);
    let output = Path::new(&open_path);
    let (record, log, phase_idx) = {
        let snap = ctx.snapshot.lock().unwrap();
        let phases = snap
            .phases
            .iter()
            .map(|p| history::schema::PhaseTiming {
                phase: p.label.clone(),
                start_offset: p.start_offset_ms.unwrap_or(0.0) / 1000.0,
                duration: p.duration_ms.unwrap_or(0.0) / 1000.0,
                status: phase_status_label(p.status).to_string(),
                kind: match p.kind {
                    PhaseKind::External => "external",
                    PhaseKind::App => "app",
                }
                .to_string(),
                command: p.command.clone(),
            })
            .collect();
        // Strip embedded newlines so build.log lines stay 1:1 with the phase-index
        // sidecar (build.idx) that re-attributes each line to its phase on replay.
        let log = snap
            .lines
            .iter()
            .map(|l| l.text.replace(['\n', '\r'], " "))
            .collect::<Vec<_>>()
            .join("\n");
        let phase_idx: Vec<u32> = snap.lines.iter().map(|l| l.phase_index).collect();
        let record = history::schema::BuildRecord {
            schema_version: history::schema::SCHEMA_VERSION,
            build_id: snap.run_id.clone(),
            started_at_ms: snap.started_ms,
            duration: total_secs,
            build_size: history::store::dir_size(output) as f64,
            warning_count: ctx.warnings.load(Ordering::Relaxed),
            error_count: ctx.errors.load(Ordering::Relaxed),
            output_path: open_path.clone(),
            output_mtime_ms: history::store::mtime_ms(output),
            phases,
            tags: history::tags::generate(&snap.platform, &snap.configs, &snap.target, run_status_label(final_status)),
        };
        (record, log, phase_idx)
    };
    let history_dir = Path::new(&ctx.project_root).join(".uep").join("history");
    let _ = history::store::write(&history_dir, &record, &log, &phase_idx);
    // Keep the derived SQLite index in step (best-effort; the JSON is the source of
    // truth, and a reader's open_synced would reconcile any miss anyway).
    if let Ok(conn) = history::index::open(&history_dir) {
        let _ = history::index::upsert(&conn, &record);
    }
}

async fn run_phase(ctx: ExecCtx, mut cancel_rx: watch::Receiver<bool>, index: u32, started: Instant) -> PhaseStatus {
    let offset = started.elapsed().as_millis() as f64;
    set_phase(&ctx, index, PhaseStatus::Running, Some(offset), None);

    let kind = ctx.unit(index as usize).kind;
    let status = match kind {
        PhaseKind::External => run_external(&ctx, &mut cancel_rx, index).await,
        PhaseKind::App => run_app(&ctx, &cancel_rx, index).await,
    };

    let dur = (started.elapsed().as_millis() as f64 - offset).max(0.0);
    set_phase(&ctx, index, status, Some(offset), Some(dur));
    status
}

fn set_phase(ctx: &ExecCtx, index: u32, status: PhaseStatus, start: Option<f64>, dur: Option<f64>) {
    let (node, run_id) = {
        let mut s = ctx.snapshot.lock().unwrap();
        let run_id = s.run_id.clone();
        let Some(p) = s.phases.iter_mut().find(|p| p.index == index) else {
            return;
        };
        p.status = status;
        if start.is_some() {
            p.start_offset_ms = start;
        }
        if dur.is_some() {
            p.duration_ms = dur;
        }
        (p.clone(), run_id)
    };
    let _ = ctx.app.emit(EV_PHASE, PhaseUpdate { run_id, phase: node });
}

async fn run_external(ctx: &ExecCtx, cancel_rx: &mut watch::Receiver<bool>, index: u32) -> PhaseStatus {
    let unit = ctx.unit(index as usize).clone();
    let program = unit.program.clone().unwrap_or_default();
    emit_line(ctx, index, Severity::Info, &format!("▶ {}", unit.preview));

    // Steam login preflight (emitted before Build when the upload phase is on): sign in up
    // front so an interactive login never interrupts a finished build. The account is the
    // `+login <account>` value in the args. If a session is already cached we're done; if not,
    // open steamcmd in its own console for the user to sign in (code / phone approval); it
    // `+quit`s and closes, then the build continues. The later upload phase assumes this ran.
    if unit.phase == PhaseId::SteamLogin {
        use crate::steam::login::SteamLoginStatus;
        let account = login_account(&unit.args);
        if account.is_empty() {
            emit_line(ctx, index, Severity::Error, "Steam upload needs an account name - set it in Setup SteamCMD.");
            return PhaseStatus::Failed;
        }
        emit_line(ctx, index, Severity::Info, "Checking Steam sign-in...");
        match steam_session_check(ctx, cancel_rx, &program, account).await {
            None => {
                emit_line(ctx, index, Severity::Warning, "cancelled");
                return PhaseStatus::Cancelled;
            }
            Some(r) if r.status == SteamLoginStatus::Success => {
                emit_line(ctx, index, Severity::Info, "Already signed in to Steam.");
                return PhaseStatus::Success;
            }
            Some(_) => {} // no cached session - open the interactive console below
        }
        emit_line(
            ctx,
            index,
            Severity::Info,
            "Not signed in - steamcmd is opening in its own window. Sign in there (enter the code, or approve on your phone); it closes automatically and the build continues.",
        );
        // steamcmd's console reports only its exit code, which is 0 even on an aborted/failed
        // login. Re-verify that a session was actually cached before letting the build (30+ min)
        // proceed - otherwise the piped upload phase would be the first to notice, far too late.
        let console = run_in_console(ctx, cancel_rx, index, &program, &unit.args).await;
        if console != PhaseStatus::Success {
            return console; // cancelled, or steamcmd failed to launch
        }
        emit_line(ctx, index, Severity::Info, "Confirming Steam sign-in...");
        return match steam_session_check(ctx, cancel_rx, &program, account).await {
            None => {
                emit_line(ctx, index, Severity::Warning, "cancelled");
                PhaseStatus::Cancelled
            }
            Some(r) if r.status == SteamLoginStatus::Success => {
                emit_line(ctx, index, Severity::Info, "Signed in to Steam.");
                PhaseStatus::Success
            }
            Some(r) => {
                emit_line(
                    ctx,
                    index,
                    Severity::Error,
                    &format!("Steam sign-in was not completed, so the build was stopped before packaging. {}", r.message.trim()),
                );
                PhaseStatus::Failed
            }
        };
    }

    // UAT's `-archive` copies files into the archive dir but never wipes it (no
    // clean-archive flag exists), so a build landing on an existing path would keep
    // stale files from a prior run. Clear it up front - but only for the unit that
    // actually archives (Stage·Pak·Archive with Archive on).
    if unit.phase == PhaseId::Stage && ctx.profile.phases.archive.enabled {
        clean_archive_dir(ctx, index).await;
    }

    // Steam upload's app-owned pre-step: materialize the resolved VDF (ContentRoot +
    // BuildOutput injected) into the scratch dir the steamcmd `+run_app_build` points at, then
    // upload **piped** (streamed to the Build Logs). Sign-in was handled up front by the Steam
    // Login preflight, so we do NOT open a console this late: if the session somehow isn't
    // valid here, steamcmd exits non-zero and the phase just fails.
    if unit.phase == PhaseId::SteamUpload {
        emit_line(ctx, index, Severity::Info, "Preparing Steam build scripts...");
        if let Err(e) = crate::steam::vdf::resolve_run_vdf(Path::new(&ctx.project_root), &ctx.profile, &ctx.output_dir) {
            emit_line(ctx, index, Severity::Error, &format!("could not prepare Steam build scripts: {e}"));
            return PhaseStatus::Failed;
        }
    }

    let mut cmd = build_command(&program, &unit.args, &ctx.project_root);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_line(ctx, index, Severity::Error, &format!("failed to launch {program}: {e}"));
            return PhaseStatus::Failed;
        }
    };
    // Group the child (and its UAT/UBT/cl.exe descendants) so Cancel / app-exit can
    // terminate the entire tree, not just this cmd.exe.
    ctx.proc_group.adopt(&child);

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let h1 = stdout.map(|s| tokio::spawn(pump(ctx.clone(), index, s)));
    let h2 = stderr.map(|s| tokio::spawn(pump(ctx.clone(), index, s)));

    let mut killed = false;
    let wait = tokio::select! {
        res = child.wait() => res,
        _ = cancel_rx.changed() => {
            killed = true;
            let _ = child.start_kill();
            child.wait().await
        }
    };

    if let Some(h) = h1 {
        let _ = h.await;
    }
    if let Some(h) = h2 {
        let _ = h.await;
    }

    if killed || *cancel_rx.borrow() {
        emit_line(ctx, index, Severity::Warning, "cancelled");
        return PhaseStatus::Cancelled;
    }
    match wait {
        Ok(status) if status.success() => PhaseStatus::Success,
        Ok(status) => {
            let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "terminated".into());
            emit_line(ctx, index, Severity::Error, &format!("phase exited with code {code}"));
            PhaseStatus::Failed
        }
        Err(e) => {
            emit_line(ctx, index, Severity::Error, &format!("process error: {e}"));
            PhaseStatus::Failed
        }
    }
}

/// The `+login <account>` value from a steamcmd arg vector (trimmed; empty if absent).
fn login_account(args: &[String]) -> &str {
    args.iter()
        .position(|a| a == "+login")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.trim())
        .unwrap_or("")
}

/// The Steam Login preflight's **cancellable** session check: spawn `steamcmd +login <account>
/// +quit` via the shared [`login::build_verify_command`], adopt it into the process group, and
/// race it against Cancel and a timeout, classifying the captured output. Returns `None` when
/// cancelled. Unlike the standalone `login::verify`, the child is adopted (so app-exit tears it
/// down) and interruptible - steamcmd's first run self-updates and can take minutes, and it must
/// never be left orphaned when the user cancels or closes the app.
async fn steam_session_check(
    ctx: &ExecCtx,
    cancel_rx: &mut watch::Receiver<bool>,
    program: &str,
    account: &str,
) -> Option<crate::steam::login::SteamLoginResult> {
    use crate::steam::login;
    let mut cmd = login::build_verify_command(program, account);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Some(login::SteamLoginResult {
                status: login::SteamLoginStatus::Failed,
                message: format!("could not launch steamcmd: {e}"),
            })
        }
    };
    // Adopt for app-exit teardown; `kill_on_drop` (set by build_verify_command) covers the
    // Cancel/timeout branches, where dropping the wait future drops the child.
    ctx.proc_group.adopt(&child);
    tokio::select! {
        out = child.wait_with_output() => match out {
            Ok(o) => Some(login::classify_output(&o.stdout, &o.stderr)),
            Err(e) => Some(login::SteamLoginResult {
                status: login::SteamLoginStatus::Failed,
                message: format!("steamcmd error: {e}"),
            }),
        },
        _ = cancel_rx.changed() => None,
        _ = tokio::time::sleep(login::LOGIN_TIMEOUT) => Some(login::SteamLoginResult {
            status: login::SteamLoginStatus::Failed,
            message: "steamcmd timed out".to_string(),
        }),
    }
}

/// Run a child (steamcmd) in its **own interactive console** so the user can sign in there
/// (password / Steam Guard / mobile-app confirmation) - used by the Steam Login preflight when
/// there's no cached session. Not piped, so its output shows in that window rather than the
/// Build Logs; we just wait for the exit code (honoring Cancel). Windows-first: other
/// platforms spawn without a dedicated console.
async fn run_in_console(ctx: &ExecCtx, cancel_rx: &mut watch::Receiver<bool>, index: u32, program: &str, args: &[String]) -> PhaseStatus {
    let mut cmd = tokio::process::Command::new(program);
    for a in args {
        cmd.arg(a);
    }
    if !ctx.project_root.is_empty() {
        cmd.current_dir(&ctx.project_root);
    }
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE - its own interactive window
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_line(ctx, index, Severity::Error, &format!("failed to launch steamcmd: {e}"));
            return PhaseStatus::Failed;
        }
    };
    // Adopt into the job so Cancel / app-exit tear down the login window too.
    ctx.proc_group.adopt(&child);
    let mut killed = false;
    let wait = tokio::select! {
        res = child.wait() => res,
        _ = cancel_rx.changed() => {
            killed = true;
            let _ = child.start_kill();
            child.wait().await
        }
    };
    if killed || *cancel_rx.borrow() {
        emit_line(ctx, index, Severity::Warning, "cancelled");
        return PhaseStatus::Cancelled;
    }
    match wait {
        Ok(s) if s.success() => {
            emit_line(ctx, index, Severity::Info, "steamcmd finished.");
            PhaseStatus::Success
        }
        Ok(s) => {
            let code = s.code().map(|c| c.to_string()).unwrap_or_else(|| "terminated".into());
            emit_line(ctx, index, Severity::Error, &format!("steamcmd exited with code {code}"));
            PhaseStatus::Failed
        }
        Err(e) => {
            emit_line(ctx, index, Severity::Error, &format!("process error: {e}"));
            PhaseStatus::Failed
        }
    }
}

/// Wipe the archive output dir before the Stage·Pak·Archive unit runs (UAT copies
/// in over the top and never cleans), so the archive starts pristine. `output_dir`
/// is `base_dir/<rendered folder>` and both are validated non-empty, so it's always
/// strictly below the base - never the bare base or a drive root (extra `parent()`
/// guard for belt-and-suspenders). Best-effort: a failure is a warning, not a
/// build-killer (UAT still overwrites same-named files). Off the async thread - the
/// tree can be GBs.
async fn clean_archive_dir(ctx: &ExecCtx, index: u32) {
    let dir = ctx.output_dir.clone();
    let path = Path::new(&dir);
    if !path.exists() || path.parent().is_none() {
        return;
    }
    emit_line(ctx, index, Severity::Info, &format!("Clearing archive dir (removing stale files): {dir}"));
    match tauri::async_runtime::spawn_blocking(move || std::fs::remove_dir_all(&dir)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => emit_line(ctx, index, Severity::Warning, &format!("could not clear archive dir, stale files may remain: {e}")),
        Err(e) => emit_line(ctx, index, Severity::Warning, &format!("archive-dir clear task failed: {e}")),
    }
}

async fn run_app(ctx: &ExecCtx, cancel_rx: &watch::Receiver<bool>, index: u32) -> PhaseStatus {
    if *cancel_rx.borrow() {
        return PhaseStatus::Cancelled;
    }
    let unit = ctx.unit(index as usize).clone();
    match unit.phase {
        PhaseId::CopyExtras => {
            let items = &ctx.profile.phases.copy_extras.items;
            emit_line(ctx, index, Severity::Info, &format!("Copy Extras: {} mapping(s) → {}", items.len(), ctx.output_dir));
            let mut ok = true;
            for it in items {
                let from = Path::new(&ctx.project_root).join(&it.from);
                let dest = if it.to == "." {
                    Path::new(&ctx.output_dir).to_path_buf()
                } else {
                    Path::new(&ctx.output_dir).join(&it.to)
                };
                match copy_into(&from, &dest) {
                    Ok(n) => emit_line(ctx, index, Severity::Info, &format!("  {} → {} ({n} file(s))", it.from, dest.display())),
                    Err(e) => {
                        ok = false;
                        emit_line(ctx, index, Severity::Error, &format!("  copy failed for {}: {e}", it.from));
                    }
                }
            }
            if ok {
                PhaseStatus::Success
            } else {
                PhaseStatus::Failed
            }
        }
        PhaseId::Cleanup => {
            // Reclaim the profile's chosen categories via the same guarded
            // `footprint::clean` the Footprint tab uses (R3, M5.4). No prompt here -
            // this is the automatic on-success phase. Cleanup is the terminal node,
            // and the executor breaks the stage loop on any failure, so it is only
            // ever reached after a successful build - honoring `only_on_success`
            // (the default) by position. (`only_on_success == false`, "clean even on
            // failure", is out of scope for the MVP executor.)
            let cats = ctx.profile.phases.cleanup.categories.clone();
            if cats.is_empty() {
                emit_line(ctx, index, Severity::Warning, "Clean-up enabled but no categories selected; nothing to reclaim.");
                return PhaseStatus::Success;
            }
            let names = cats.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
            emit_line(ctx, index, Severity::Info, &format!("Clean-up - reclaiming: {names}"));

            let root = ctx.project_root.clone();
            // Profile-specific scope: the build artifacts are this profile's one target;
            // editor artifacts (only if the profile explicitly selected them) map to the
            // detected editor target + engine tools.
            let scope = crate::footprint::rules::TargetScope::new(
                vec![ctx.target.clone()],
                ctx.editor_target.clone(),
            );
            let outcome = tauri::async_runtime::spawn_blocking(move || {
                crate::footprint::clean::clean_categories(Path::new(&root), &cats, &scope)
            })
            .await;
            match outcome {
                Ok(out) => {
                    for r in &out.removed {
                        if r.deleted {
                            emit_line(ctx, index, Severity::Info, &format!("  removed {} ({})", r.rel, fmt_bytes(r.size_bytes)));
                        } else {
                            emit_line(ctx, index, Severity::Warning, &format!("  skipped {} (guarded)", r.rel));
                        }
                    }
                    emit_line(ctx, index, Severity::Info, &format!("Reclaimed {}", fmt_bytes(out.reclaimed_bytes)));
                    PhaseStatus::Success
                }
                Err(e) => {
                    emit_line(ctx, index, Severity::Error, &format!("Clean-up failed: {e}"));
                    PhaseStatus::Failed
                }
            }
        }
        _ => PhaseStatus::Success,
    }
}

/// Stream one child pipe: read lines, classify, and flush batches (size- or
/// time-bounded) as `uep://run-log` events + into the snapshot buffer.
async fn pump<R>(ctx: ExecCtx, index: u32, stream: R)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(stream).lines();
    let mut batch: Vec<LogLine> = Vec::new();
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            maybe = lines.next_line() => match maybe {
                Ok(Some(text)) => {
                    let severity = classify_line(&text);
                    // Tally health counts off the real child stream (not emit_line chrome).
                    match severity {
                        Severity::Warning => { ctx.warnings.fetch_add(1, Ordering::Relaxed); }
                        Severity::Error => { ctx.errors.fetch_add(1, Ordering::Relaxed); }
                        Severity::Info => {}
                    }
                    let seq = ctx.seq.fetch_add(1, Ordering::Relaxed);
                    batch.push(LogLine { seq, phase_index: index, severity, text });
                    if batch.len() >= FLUSH_LINES {
                        flush(&ctx, &mut batch);
                    }
                }
                _ => break, // EOF or read error
            },
            _ = ticker.tick() => flush(&ctx, &mut batch),
        }
    }
    flush(&ctx, &mut batch);
}

fn flush(ctx: &ExecCtx, batch: &mut Vec<LogLine>) {
    if batch.is_empty() {
        return;
    }
    let lines = std::mem::take(batch);
    let run_id = {
        let mut s = ctx.snapshot.lock().unwrap();
        s.lines.extend(lines.iter().cloned());
        if s.lines.len() > LINE_BUF_CAP {
            let excess = s.lines.len() - LINE_BUF_CAP;
            s.lines.drain(0..excess);
        }
        s.run_id.clone()
    };
    let _ = ctx.app.emit(EV_LOG, LogBatch { run_id, lines });
}

/// Push a single executor-synthesized line (command echo, launch error, app-phase
/// progress) through the same path as streamed output.
fn emit_line(ctx: &ExecCtx, index: u32, severity: Severity, text: &str) {
    let seq = ctx.seq.fetch_add(1, Ordering::Relaxed);
    let line = LogLine { seq, phase_index: index, severity, text: text.to_string() };
    let run_id = {
        let mut s = ctx.snapshot.lock().unwrap();
        s.lines.push(line.clone());
        s.run_id.clone()
    };
    let _ = ctx.app.emit(EV_LOG, LogBatch { run_id, lines: vec![line] });
}

/// Compact byte formatter for Clean-up log lines (GB ≥ 1 GB, else MB).
fn fmt_bytes(b: f64) -> String {
    let gb = b / 1e9;
    if gb >= 1.0 {
        format!("{gb:.2} GB")
    } else {
        format!("{:.0} MB", b / 1e6)
    }
}

/// Windows `.bat`/`.cmd` must run via `cmd /C` (UBT's `Build.bat`, `RunUAT.bat`);
/// everything else is spawned directly. stdout/stderr piped, stdin null, killed on
/// drop.
fn build_command(program: &str, args: &[String], cwd: &str) -> tokio::process::Command {
    let lower = program.to_ascii_lowercase();
    let use_cmd = cfg!(windows) && (lower.ends_with(".bat") || lower.ends_with(".cmd"));
    let mut c = tokio::process::Command::new(if use_cmd { "cmd" } else { program });
    if use_cmd {
        c.arg("/C").arg(program);
    }
    for a in args {
        c.arg(a);
    }
    if !cwd.is_empty() {
        c.current_dir(cwd);
    }
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    c.kill_on_drop(true);
    // CREATE_NO_WINDOW (0x08000000): don't pop a console window for each cmd/UAT/UBT
    // child (they'd flash empty consoles); piped stdio still captures their output.
    #[cfg(windows)]
    c.creation_flags(0x0800_0000);
    c
}

/// Copy a file (or directory tree) *into* `dest` (created if needed), preserving
/// the source's own name under it. Returns the file count copied.
fn copy_into(from: &Path, dest: &Path) -> io::Result<u32> {
    let name = from
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "source has no file name"))?;
    if from.is_dir() {
        copy_dir(from, &dest.join(name))
    } else {
        std::fs::create_dir_all(dest)?;
        std::fs::copy(from, dest.join(name))?;
        Ok(1)
    }
}

fn copy_dir(src: &Path, dst: &Path) -> io::Result<u32> {
    std::fs::create_dir_all(dst)?;
    let mut count = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            count += copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
            count += 1;
        }
    }
    Ok(count)
}
