// Typed runner IPC (M3): start / cancel / snapshot commands plus the live
// `uep://run-*` event listeners the Build Logs window subscribes to. Backend:
// `src-tauri/src/runner`. Events are global (emitted with `app.emit`), so they
// reach the separate Build Logs webview window.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { commands } from "./bindings";
import { unwrap } from "./ipc";
import type { BuildConfig, LogLine, PhaseNode, RunSnapshot, RunStatus } from "./bindings";

export type { RunSnapshot, PhaseNode, LogLine, RunStatus, PhaseStatus, Severity } from "./bindings";

/** Resolve + launch the profile's pipeline. Returns the new run id. */
export const startBuild = (profile: BuildConfig) => unwrap(commands.startBuild(profile));
/** Signal the active run to cancel (clean child kill). */
export const cancelBuild = () => unwrap(commands.cancelBuild());
/** Snapshot of the in-flight (or most recent) run, or null. */
export const activeRun = () => unwrap(commands.activeRun());

// ── live event payloads (mirror the Rust emit structs) ────────────────────────
export interface LogBatch {
  runId: string;
  lines: LogLine[];
}
export interface PhaseUpdate {
  runId: string;
  phase: PhaseNode;
}
export interface RunFinished {
  runId: string;
  status: RunStatus;
  durationMs: number;
}

/** Full snapshot when a run starts (lets an already-open window adopt it). */
export const onRunStarted = (cb: (s: RunSnapshot) => void): Promise<UnlistenFn> =>
  listen<RunSnapshot>("uep://run-started", (e) => cb(e.payload));
/** A batch of new classified console lines. */
export const onRunLog = (cb: (b: LogBatch) => void): Promise<UnlistenFn> =>
  listen<LogBatch>("uep://run-log", (e) => cb(e.payload));
/** A phase changed status / gained timing (drives the live graph). */
export const onRunPhase = (cb: (p: PhaseUpdate) => void): Promise<UnlistenFn> =>
  listen<PhaseUpdate>("uep://run-phase", (e) => cb(e.payload));
/** The run settled (success / failed / cancelled). */
export const onRunFinished = (cb: (f: RunFinished) => void): Promise<UnlistenFn> =>
  listen<RunFinished>("uep://run-finished", (e) => cb(e.payload));
