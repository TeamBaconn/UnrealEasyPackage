// Typed IPC layer. Wraps the tauri-specta `commands` (generated in bindings.ts)
// so callers get plain promises that throw `IpcError` on a backend `AppError`,
// plus the native file/folder dialogs used by the open + locate-engine flows.

import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { commands } from "./bindings";
import type { AppError, AppSettings, BuildConfig, CreateRequest, HistoryFilter, PluginPackageRequest, ResaveOptions, SteamLocalSettings, ValidateOptions } from "./bindings";

export type {
  AppError,
  DetectedProject,
  DetectedPlugin,
  EngineInfo,
  EngineKind,
  EngineVersion,
  MapInventory,
  ProjectType,
  ProjectValidation,
  RecentEntry,
  RecentKind,
  TargetInfo,
  TargetType,
  // M7 - plugin packaging (RunUAT BuildPlugin)
  EngineEntry,
  EngineSource,
  PluginPackageRequest,
  PluginPreview,
  PluginSettings,
  ResaveOptions,
  ValidateOptions,
  RemovedUep,
  // M2 - profiles / templates / phase registry
  BuildConfig,
  Phases,
  Platform,
  Configuration,
  BuildCfg,
  ArchiveCfg,
  CookCfg,
  CookMaps,
  IncrementalCookMode,
  StageCfg,
  PakCfg,
  CopyExtrasCfg,
  CopyItem,
  CleanupCfg,
  CleanupCategory,
  // Steam upload phase
  SteamUploadCfg,
  DepotItem,
  SteamLocalSettings,
  SteamStatus,
  Output,
  PhaseInfo,
  PhaseId,
  PhaseKind,
  Requiredness,
  CreateRequest,
  CreateFromKind,
  // M4 - build history
  BuildRecord,
  PhaseTiming,
  HistoryDetail,
  OutputCheck,
  LocationCheck,
  HistoryFilter,
  HistoryPage,
  FilterOptions,
  // M5 - footprint scan + cleanup
  FootprintReport,
  FootprintNode,
  FootprintLocation,
  CleanOutcome,
  Removed,
  // M6 - app settings
  AppSettings,
  Theme,
} from "./bindings";

export class IpcError extends Error {
  constructor(public readonly appError: AppError) {
    super(appError.message);
    this.name = "IpcError";
  }
}

export type Result<T> = { status: "ok"; data: T } | { status: "error"; error: AppError };

export async function unwrap<T>(p: Promise<Result<T>>): Promise<T> {
  const r = await p;
  if (r.status === "error") throw new IpcError(r.error);
  return r.data;
}

export const listRecents = () => unwrap(commands.listRecents());
export const validateProject = (uprojectPath: string) => unwrap(commands.validateProject(uprojectPath));
export const openProject = (uprojectPath: string) => unwrap(commands.openProject(uprojectPath));
/** Open a `.uplugin` (no engine needed - chosen per-package in the Actions tab). */
export const openPlugin = (pluginPath: string) => unwrap(commands.openPlugin(pluginPath));
/** The currently-open plugin (managed-state snapshot), or null. */
export const currentPlugin = () => unwrap(commands.currentPlugin());
/** Save a per-project engine override (`.uep/local.json`) and return its info. */
export const locateEngine = (uprojectPath: string, engineDir: string) =>
  unwrap(commands.locateEngine(uprojectPath, engineDir));
/** Remove a recent by its descriptor path (`.uproject` or `.uplugin`). */
export const removeRecent = (path: string) => unwrap(commands.removeRecent(path));
export const setRecentStarred = (path: string, starred: boolean) =>
  unwrap(commands.setRecentStarred(path, starred));

// ── M7: plugin packaging (RunUAT BuildPlugin) ─────────────────────────────────
/** Engines for the package picker (registry + remembered, validated, newest-first). */
export const listEngines = (pluginPath: string) => unwrap(commands.listEngines(pluginPath));
/** Validate + remember a browsed engine folder for this plugin; returns its entry. */
export const addCustomEngine = (pluginPath: string, engineDir: string) =>
  unwrap(commands.addCustomEngine(pluginPath, engineDir));
/** Resolve a package request into the command preview + output dir (live preview). */
export const previewPluginPackage = (req: PluginPackageRequest) =>
  unwrap(commands.previewPluginPackage(req));
/** Resolve + launch a BuildPlugin run; returns the run id (progress via uep://run-*). */
export const startPluginPackage = (req: PluginPackageRequest) =>
  unwrap(commands.startPluginPackage(req));
/** Load this plugin's machine-local Actions settings (`.uap/settings.json`). */
export const loadPluginSettings = (pluginPath: string) => unwrap(commands.loadPluginSettings(pluginPath));
/** Persist this plugin's output folder + folder name into `.uap/settings.json`. */
export const savePluginOutput = (pluginPath: string, outputDir: string, folderName: string) =>
  unwrap(commands.savePluginOutput(pluginPath, outputDir, folderName));

// ── Editor commandlet tools (project Tools tab): Resave / Validate ────────────
/** Launch a Resave (ResavePackages) run on the open project; returns the run id. */
export const startResave = (options: ResaveOptions) => unwrap(commands.startResave(options));
/** Launch a Validate (DataValidation) run on the open project; returns the run id. */
export const startValidate = (options: ValidateOptions) => unwrap(commands.startValidate(options));
/** Delete UEP's per-project (.uep) or per-plugin (.uap) data, forget the recent, clear state. */
export const removeUepData = () => unwrap(commands.removeUepData());

// ── Steam upload phase: machine-local settings + one-time steamcmd login ──────
/** The open project's machine-local Steam settings (steamcmd path + build account). */
export const loadSteamSettings = () => unwrap(commands.loadSteamSettings());
/** Persist the open project's machine-local Steam settings (git-ignored). */
export const saveSteamSettings = (settings: SteamLocalSettings) => unwrap(commands.saveSteamSettings(settings));
/** Setup status for the Setup SteamCMD modal: steamcmd found + can-sign-in (runs a check). */
export const steamStatus = () => unwrap(commands.steamStatus());
/** Open steamcmd in its own console for an interactive sign-in (the "Try sign in" link). */
export const steamOpenLoginTerminal = () => unwrap(commands.steamOpenLoginTerminal());

// ── M2: profiles, templates, phase registry ──────────────────────────────────
// Profiles are project-local (read against the open project in backend state);
// templates are global. The phase registry + per-profile states drive the
// editor's locked/greyed toggles.
export const currentProject = () => unwrap(commands.currentProject());
export const listProfiles = () => unwrap(commands.listProfiles());
export const createProfile = (req: CreateRequest) => unwrap(commands.createProfile(req));
export const duplicateProfile = (id: string) => unwrap(commands.duplicateProfile(id));
export const saveProfile = (profile: BuildConfig) => unwrap(commands.saveProfile(profile));
export const deleteProfile = (id: string) => unwrap(commands.deleteProfile(id));
export const listTemplates = () => unwrap(commands.listTemplates());
export const saveTemplate = (template: BuildConfig) => unwrap(commands.saveTemplate(template));
export const deleteTemplate = (id: string) => unwrap(commands.deleteTemplate(id));
/** Static phase registry (order/deps/requiredness/kind/gatedBy) - no Result wrapper. */
export const phaseRegistry = () => commands.phaseRegistry();

// ── M4: build history + run-time output check ─────────────────────────────────
export const checkOutput = (profile: BuildConfig) => unwrap(commands.checkOutput(profile));
export const listHistory = () => unwrap(commands.listHistory());
export const listHistoryPage = (offset: number, limit: number, filter: HistoryFilter) =>
  unwrap(commands.listHistoryPage(offset, limit, filter));
export const historyDetail = (buildId: string) => unwrap(commands.historyDetail(buildId));
export const deleteHistory = (ids: string[]) => unwrap(commands.deleteHistory(ids));
export const checkBuildLocation = (buildId: string) => unwrap(commands.checkBuildLocation(buildId));

// ── M5: footprint (scan + cleanup) ────────────────────────────────────────────
// Scan resolves the Clean-tab node tree (off-thread); clean is allow-list gated in
// Rust. The confirm-prompt that lists every folder lives in the Clean tab.
export const scanFootprint = () => unwrap(commands.scanFootprint());
/** Delete the selected Clean-tab nodes (leaf ids, e.g. `intermediateGame:<target>` / `intermediateOther`). */
export const cleanFootprint = (nodeIds: string[]) => unwrap(commands.cleanFootprint(nodeIds));

// ── M6: app settings (theme + notification prefs) + about version ─────────────
// One app-folder JSON. saveSettings broadcasts `uep://settings-changed`; the theme
// is applied per-window via the settings store (see settings.ts).
export const loadSettings = () => unwrap(commands.loadSettings());
export const saveSettings = (settings: AppSettings) => unwrap(commands.saveSettings(settings));
/** App version string (no Result wrapper). */
export const appVersion = () => commands.appVersion();
/** Hide the main window into the tray (build keeps running; tray restores it). */
export const minimizeToTray = () => unwrap(commands.minimizeToTray());

/** Native picker for a `.uproject` file. Returns the path, or null if cancelled. */
export async function pickUproject(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Unreal Project", extensions: ["uproject"] }],
  });
  return typeof selected === "string" ? selected : null;
}

/** Native picker for a `.uplugin` file. Returns the path, or null if cancelled. */
export async function pickUplugin(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Unreal Plugin", extensions: ["uplugin"] }],
  });
  return typeof selected === "string" ? selected : null;
}

/** Native picker for an engine root folder. Returns the path, or null if cancelled. */
export async function pickEngineFolder(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: true,
    title: "Locate the Unreal Engine folder",
  });
  return typeof selected === "string" ? selected : null;
}

/** Native picker for a single directory (e.g. the archive base dir). `defaultPath`
 * is where the dialog opens (e.g. the project root). */
export async function pickDirectory(title?: string, defaultPath?: string): Promise<string | null> {
  const selected = await open({ multiple: false, directory: true, title, defaultPath });
  return typeof selected === "string" ? selected : null;
}

/** Native picker for a single file (e.g. a Copy Extras source). */
export async function pickFile(title?: string, defaultPath?: string): Promise<string | null> {
  const selected = await open({ multiple: false, directory: false, title, defaultPath });
  return typeof selected === "string" ? selected : null;
}

/** Open a folder in the OS file manager. */
export const openFolder = (folderPath: string) => openPath(folderPath);

/** Directory containing a file path (for "open project folder"). */
export function parentDir(filePath: string): string {
  return filePath.replace(/[\\/][^\\/]*$/, "");
}
