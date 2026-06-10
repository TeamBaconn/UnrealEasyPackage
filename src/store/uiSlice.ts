import { createSlice, type PayloadAction } from "@reduxjs/toolkit";
import type { DetectedPlugin, DetectedProject } from "../bindings";

// Project tabs (dashboard/build/clean) and the plugin tab (actions) share one union;
// the shell renders only the tabs valid for the open `mode`.
export type NavTab = "dashboard" | "build" | "clean" | "tools" | "actions";
export type Screen = "gate" | "main";
/** What kind of thing is open - a project shell vs the plugin (Actions) shell. */
export type Mode = "project" | "plugin";

interface UiState {
  screen: Screen;
  mode: Mode;
  activeTab: NavTab;
  /** The opened project with its fresh detection result (engine, targets, maps, plugins). */
  currentProject: DetectedProject | null;
  /** The opened plugin (when a `.uplugin` was opened instead of a `.uproject`). */
  currentPlugin: DetectedPlugin | null;
  /** Settings modal visibility - opened from the gate and the shell header alike. */
  settingsOpen: boolean;
}

const initialState: UiState = {
  screen: "gate",
  mode: "project",
  activeTab: "dashboard",
  currentProject: null,
  currentPlugin: null,
  settingsOpen: false,
};

const uiSlice = createSlice({
  name: "ui",
  initialState,
  reducers: {
    // Gate → project shell, carrying the detected project from `open_project`.
    openProject(state, action: PayloadAction<DetectedProject>) {
      state.currentProject = action.payload;
      state.currentPlugin = null;
      state.mode = "project";
      state.screen = "main";
      state.activeTab = "dashboard";
    },
    // Gate → plugin shell, carrying the detected plugin from `open_plugin`.
    openPlugin(state, action: PayloadAction<DetectedPlugin>) {
      state.currentPlugin = action.payload;
      state.currentProject = null;
      state.mode = "plugin";
      state.screen = "main";
      state.activeTab = "actions";
    },
    switchProject(state) {
      state.screen = "gate";
    },
    setTab(state, action: PayloadAction<NavTab>) {
      state.activeTab = action.payload;
    },
    openSettings(state) {
      state.settingsOpen = true;
    },
    closeSettings(state) {
      state.settingsOpen = false;
    },
  },
});

export const { openProject, openPlugin, switchProject, setTab, openSettings, closeSettings } =
  uiSlice.actions;
export default uiSlice.reducer;
