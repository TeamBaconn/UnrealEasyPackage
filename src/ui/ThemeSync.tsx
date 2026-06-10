import { useEffect, useReducer } from "react";
import { useMantineColorScheme } from "@mantine/core";
import { loadSettings } from "../ipc";
import { cacheTheme, onSettingsChanged } from "../settings";
import { applyScheme, notifyThemeChanged, subscribeTheme } from "./tokens";
import type { Theme } from "../bindings";

/** Apply `theme` everywhere: Mantine's color scheme (built-in dark surfaces + our
 *  `--uep-*` var overrides) AND the JS `tokens` palette (re-rendering consumers). */
function apply(theme: Theme, setColorScheme: (s: Theme) => void) {
  cacheTheme(theme);
  applyScheme(theme);
  setColorScheme(theme);
  notifyThemeChanged();
}

/** Drives the saved color scheme for this window on mount, and re-applies it live
 *  when another window changes it (`uep://settings-changed`). Renders nothing. */
export function ThemeSync() {
  const { setColorScheme } = useMantineColorScheme();

  useEffect(() => {
    let alive = true;
    loadSettings()
      .then((s) => {
        if (alive) apply(s.theme, setColorScheme);
      })
      .catch(() => {
        /* keep the cached/default scheme on a load failure */
      });

    const off = onSettingsChanged((s) => apply(s.theme, setColorScheme));
    return () => {
      alive = false;
      void off.then((fn) => fn());
    };
  }, [setColorScheme]);

  return null;
}

/** Force the calling component to re-render whenever the palette swaps. Call this
 *  in the component that *renders the app tree* (not a children-passthrough): on a
 *  swap it re-runs that render, creating fresh child elements so the whole subtree
 *  re-renders and every plain `tokens.*` read picks up the new scheme. State is
 *  preserved (same element types/positions - a re-render, not a remount).
 *
 *  A passthrough `({children}) => children` does NOT work here: `children` is built
 *  by the parent, so its element references are stable and React bails out of
 *  re-rendering them when only this node's state changes. */
export function useThemeRerender(): void {
  const [, bump] = useReducer((c: number) => c + 1, 0);
  useEffect(() => subscribeTheme(bump), []);
}
