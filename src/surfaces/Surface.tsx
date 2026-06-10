import { currentSurface } from "../ui/windows";
import { MainSurface } from "./MainSurface";
import { BuildSettingsWindow } from "./windows/BuildSettingsWindow";
import { BuildLogsWindow } from "./windows/BuildLogsWindow";
import { PluginLogsWindow } from "./windows/PluginLogsWindow";

/** Picks which surface this Tauri window renders, based on its `?w=` query. */
export function Surface() {
  switch (currentSurface()) {
    case "build-settings":
      return <BuildSettingsWindow />;
    case "build-logs":
      return <BuildLogsWindow />;
    case "plugin-logs":
      return <PluginLogsWindow />;
    default:
      return <MainSurface />;
  }
}
