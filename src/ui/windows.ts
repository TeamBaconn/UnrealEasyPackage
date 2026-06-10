import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { emit, listen } from "@tauri-apps/api/event";

// Each Tauri window is its own webview/JS context. We render one bundle and pick
// the surface from the `?w=` query the window was opened with (label matches it).
// Cross-window state will flow through the Rust backend in later milestones; for
// the M0 shell every window stands alone with mock data.

export type SurfaceKey = "main" | "build-settings" | "build-logs" | "plugin-logs";
type AuxKey = Exclude<SurfaceKey, "main">;

const AUX: Record<AuxKey, { title: string; width: number; height: number }> = {
  "build-settings": { title: "Build Settings - UnrealEasyPackage", width: 1180, height: 820 },
  "build-logs": { title: "Build Logs - UnrealEasyPackage", width: 1180, height: 820 },
  "plugin-logs": { title: "Run Log - UnrealEasyPackage", width: 1100, height: 760 },
};

export function currentSurface(): SurfaceKey {
  const w = new URLSearchParams(window.location.search).get("w");
  if (w === "build-settings" || w === "build-logs" || w === "plugin-logs") return w;
  return "main";
}

/** Open the shared single-command Run Log window - used by plugin packaging and the
 *  project Tools (Resave / Validate). Focus if already open; it adopts the new run via
 *  the `uep://run-*` events. No history - closing it discards the log. */
export const openRunLogs = () => openAuxWindow("plugin-logs");

/** Open an auxiliary window, focusing it if already open. */
export async function openAuxWindow(key: AuxKey): Promise<void> {
  const existing = await WebviewWindow.getByLabel(key);
  if (existing) {
    await existing.setFocus();
    return;
  }
  const cfg = AUX[key];
  const win = new WebviewWindow(key, {
    url: `index.html?w=${key}`,
    title: cfg.title,
    width: cfg.width,
    height: cfg.height,
    minWidth: 720,
    minHeight: 540,
    center: true,
    resizable: true,
  });
  win.once("tauri://error", (e) => console.error(`failed to open ${key} window`, e));
}

/**
 * Open the Build Logs window. With no `buildId` it shows the live run; with one it
 * replays that past build. If the window is already open, focus it and tell it
 * which build to show (live = null) via the `uep://show-build` event.
 */
export async function openBuildLogs(buildId?: string): Promise<void> {
  const payload = { buildId: buildId ?? null };
  const existing = await WebviewWindow.getByLabel("build-logs");
  if (existing) {
    await existing.setFocus();
    await emit("uep://show-build", payload);
    return;
  }
  const cfg = AUX["build-logs"];
  const url = `index.html?w=build-logs${buildId ? `&build=${encodeURIComponent(buildId)}` : ""}`;
  const win = new WebviewWindow("build-logs", {
    url,
    title: cfg.title,
    width: cfg.width,
    height: cfg.height,
    minWidth: 720,
    minHeight: 540,
    center: true,
    resizable: true,
  });
  win.once("tauri://error", (e) => console.error("failed to open build-logs window", e));
}

/** The `?build=` query the Build Logs window was opened with (past-build mode). */
export function initialBuildParam(): string | null {
  return new URLSearchParams(window.location.search).get("build");
}

// Cross-window profile-set sync. The Build Settings window mutates profiles in its
// own JS context (separate webview); emitting this lets the main window (Build tab)
// re-fetch so its list - and the profile it hands to `start_build` - never go stale.
export function emitProfilesChanged(): Promise<void> {
  return emit("uep://profiles-changed");
}

export function onProfilesChanged(cb: () => void): Promise<() => void> {
  return listen("uep://profiles-changed", () => cb());
}
