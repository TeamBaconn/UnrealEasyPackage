// Frontend side of app settings (M6). The backend `settings.json` is the source of
// truth; each window (its own webview) applies the theme on mount and re-applies on
// the `uep://settings-changed` broadcast that `save_settings` emits. We also mirror
// the theme into localStorage so the very first paint matches (no light→dark flash).

import { listen } from "@tauri-apps/api/event";
import type { AppSettings, Theme } from "./bindings";

const THEME_KEY = "uep-theme";

/** Last-applied theme from localStorage - used as the MantineProvider default so
 *  the first paint already matches the saved scheme. */
export function cachedTheme(): Theme {
  return localStorage.getItem(THEME_KEY) === "dark" ? "dark" : "light";
}

export function cacheTheme(theme: Theme): void {
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    /* private mode / storage disabled - non-fatal, we just lose the flash-free paint */
  }
}

/** Subscribe to backend settings changes (theme + notif prefs change live). */
export function onSettingsChanged(cb: (s: AppSettings) => void): Promise<() => void> {
  return listen<AppSettings>("uep://settings-changed", (e) => cb(e.payload));
}
