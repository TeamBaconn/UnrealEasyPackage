// JS mirror of the color system in docs/design-system.md. The `--uep-*` vars in
// styles.css are the source of truth for raw CSS; these mirror them for the many
// places that consume colors as JS values - including SVG `fill=`/`stroke=`
// *attributes* (the dashboard/log charts), where `var()` does NOT resolve. That's
// why dark mode is a runtime palette *swap* here (not a `var()` flip): `applyScheme`
// overwrites the live `tokens` object in place, and `notifyThemeChanged` re-renders
// the tree (see ui/ThemeSync). Keep these in sync with the styles.css var blocks.
//
// Brand: Bright Ocean / Shadow Grey / Yellow Green / Porcelain / Coral Glow, with a
// matched semantic red (error). Yellow Green is the interactive accent, a deeper green
// is success, Coral is warning, the dark scheme is a Shadow-Grey near-black, Porcelain
// is the light page. Blue/teal/violet live in the cleanup-category hues. `ink` is
// strong heading text (pure black / white); `accent` is the interactive yellow-green.
// Text stays PURE neutral grayscale (never brand-tinted) for max contrast.

export type Palette = {
  page: string;
  surface: string;
  surfaceAlt: string;
  headerRow: string;
  border: string;
  borderStrong: string;
  divider: string;
  hover: string;
  active: string;

  ink: string; // strong heading text emphasis (pure black light / pure white dark)
  text: string;
  textMuted: string;
  textDim: string;

  // interactive accent (Yellow Green)
  accent: string;
  onAccent: string; // text/icon on an accent fill (pure black - lime is bright)
  accentSoft: string;
  accentSoftBorder: string;
  accentSoftText: string;
  accentWash: string; // faint accent row-highlight (live build row)

  // status - success (deeper green, distinct from the lime accent)
  success: string;
  successBg: string;
  successBorder: string;
  successText: string;
  successSolidBg: string;
  successSolidBorder: string;
  successSolidFg: string;
  successSolidSub: string;

  // status - warning (Coral Glow)
  warn: string;
  warnBg: string;
  warnBorder: string;
  warnText: string;
  logWarnBg: string;

  // status - error/failed (matched red)
  danger: string;
  dangerBg: string;
  dangerBorder: string;
  dangerText: string;
  dangerSub: string;
  // solid "Stage ✕ failed" red - scheme-independent fill (mirrors the successSolid* family)
  dangerSolidBg: string;
  dangerSolidBorder: string;
  dangerSolidFg: string;
  dangerSolidSub: string;
  logErrorBg: string;

  // status - running (yellow-green accent fill); fg = onAccent, bg = accent
  runningBorder: string;
  runningSub: string;

  // status - cancelled / neutral pill
  cancelledBg: string;
  cancelledBorder: string;
  neutralBadgeBg: string;
  neutralBadgeBorder: string;

  // dependency-free charts
  chartNeutral: string;
  chartStroke: string;
};

// Light scheme - Porcelain surfaces, pure-grayscale text, Yellow Green accent.
const light: Palette = {
  page: "#eef3f2", // screen canvas - same value as surfaceAlt, kept as an independent literal (not wired to surfaceAlt)
  surface: "#ffffff", // White cards / panels
  surfaceAlt: "#eef3f2", // nested panels, inset bodies, table headers, input tracks
  headerRow: "#eef3f2",
  border: "#dae1df",
  borderStrong: "#c4cfcc",
  divider: "#e6ecea",

  hover: "#edf7cf",
  active: "#d4ec8f",

  ink: "#000000", // pure black - headings / emphasis
  text: "#1a1a1a",
  textMuted: "#474747",
  textDim: "#6e6e6e",

  accent: "#a2d729", // Yellow Green
  onAccent: "#000000",
  accentSoft: "#edf7cf",
  accentSoftBorder: "#d4ec8f",
  accentSoftText: "#5a7d10",
  accentWash: "#f4fae0",

  success: "#3a8b2a",
  successBg: "#e2f0dc",
  successBorder: "#b8dba8",
  successText: "#2a6b1e",
  successSolidBg: "#3a8b2a",
  successSolidBorder: "#2f6b22",
  successSolidFg: "#ffffff",
  successSolidSub: "#c8e6bd",

  warn: "#d96a2e",
  warnBg: "#fde8dc",
  warnBorder: "#f6c5a8",
  warnText: "#b35421",
  logWarnBg: "#fdefe6",

  danger: "#d6342a",
  dangerBg: "#f9e3e1",
  dangerBorder: "#efb8b3",
  dangerText: "#a82319",
  dangerSub: "#c4685f",
  dangerSolidBg: "#d6342a",
  dangerSolidBorder: "#b22a21",
  dangerSolidFg: "#ffffff",
  dangerSolidSub: "#f3c2ba",
  logErrorBg: "#fbe9e7",

  runningBorder: "#88bb1e",
  runningSub: "#3d5410",

  cancelledBg: "#eceeed",
  cancelledBorder: "#d4d8d7",
  neutralBadgeBg: "#eceeed",
  neutralBadgeBorder: "#d4d8d7",

  chartNeutral: "#d8dedd",
  chartStroke: "#b6c0be",
};

// Dark scheme - Shadow-Grey near-black base, bright Yellow Green accent.
const dark: Palette = {
  page: "#3f3944", // screen canvas - same value as surfaceAlt, kept as an independent literal (not wired to surfaceAlt)
  surface: "#342e37", // Shadow Grey
  surfaceAlt: "#3f3944",
  headerRow: "#3f3944",
  border: "#4d4653",
  borderStrong: "#5e5666",
  divider: "#3a343f",

  hover: "#2b3310",
  active: "#4f5e1d",

  ink: "#ffffff", // pure white - headings on dark
  text: "#ebebeb",
  textMuted: "#c4c4c4",
  textDim: "#9a9a9a",

  accent: "#a8da2e",
  onAccent: "#000000",
  accentSoft: "#2b3310",
  accentSoftBorder: "#4f5e1d",
  accentSoftText: "#c2e86a",
  accentWash: "#1f2410",

  success: "#5fae3a",
  successBg: "#1f2e16",
  successBorder: "#3a5926",
  successText: "#9ed178",
  successSolidBg: "#3a8b2a",
  successSolidBorder: "#2f6b22",
  successSolidFg: "#ffffff",
  successSolidSub: "#c8e6bd",

  warn: "#fa824c",
  warnBg: "#38241a",
  warnBorder: "#5e3a26",
  warnText: "#f6a877",
  logWarnBg: "#38241a",

  danger: "#e8635a",
  dangerBg: "#3a1d1b",
  dangerBorder: "#5e2e2a",
  dangerText: "#f3938b",
  dangerSub: "#cf837c",
  dangerSolidBg: "#d6342a",
  dangerSolidBorder: "#b22a21",
  dangerSolidFg: "#ffffff",
  dangerSolidSub: "#f3c2ba",
  logErrorBg: "#3a1d1b",

  runningBorder: "#88bb1e",
  runningSub: "#2f4108",

  cancelledBg: "#3a343f",
  cancelledBorder: "#4d4653",
  neutralBadgeBg: "#3a343f",
  neutralBadgeBorder: "#4d4653",

  chartNeutral: "#3f3944",
  chartStroke: "#5e5666",
};

/** Live palette - mutated in place by `applyScheme` so every consumer that reads a
 *  field on its next render gets the active scheme's value. */
export const tokens: Palette = { ...light };

// Cleanup-category colors (Clean tab bar + legend). Categorical hues drawn from the
// ocean / lime / coral / teal / violet / grey families; swapped per scheme like
// `phaseShades` so the bar follows dark mode. `color` = selected fill, `light` =
// unselected tint. Keyed by CleanupCategory (kept as plain strings so tokens stays
// ipc-agnostic).
export type CleanupCatColor = { color: string; light: string };
const cleanupCatsLight: Record<string, CleanupCatColor> = {
  staged: { color: "#3c91e6", light: "#dceafa" },
  cooked: { color: "#86a52a", light: "#eef6d6" },
  shader: { color: "#fa824c", light: "#fde8dc" },
  binariesGame: { color: "#8b6fb8", light: "#e6e1fb" },
  binariesPlugin: { color: "#2fb0a0", light: "#d6f0ec" },
  intermediateGame: { color: "#245f9e", light: "#d9e6f3" },
  intermediateOther: { color: "#5a5560", light: "#e6e4e8" },
  intermediatePlugin: { color: "#9bbf2c", light: "#eef3d0" },
  derivedData: { color: "#c4571f", light: "#f6e0d2" },
};
const cleanupCatsDark: Record<string, CleanupCatColor> = {
  staged: { color: "#4f9eed", light: "#1c3147" },
  cooked: { color: "#9bcf2c", light: "#26300f" },
  shader: { color: "#fa824c", light: "#38241a" },
  binariesGame: { color: "#9a82c8", light: "#2a2342" },
  binariesPlugin: { color: "#3fc0ae", light: "#0f2e2a" },
  intermediateGame: { color: "#3f78c0", light: "#16263a" },
  intermediateOther: { color: "#7a7280", light: "#2e2a33" },
  intermediatePlugin: { color: "#aacf3a", light: "#262e10" },
  derivedData: { color: "#d86a2f", light: "#341f12" },
};
export const cleanupCats: Record<string, CleanupCatColor> = { ...cleanupCatsLight };

export type Scheme = "light" | "dark";

/** Swap the live palette to `scheme`. Call `notifyThemeChanged()` afterwards to
 *  re-render consumers (ThemeSync does both). */
export function applyScheme(scheme: Scheme): void {
  Object.assign(tokens, scheme === "dark" ? dark : light);
  Object.assign(cleanupCats, scheme === "dark" ? cleanupCatsDark : cleanupCatsLight);
}

// ── theme re-render bus ────────────────────────────────────────────────────────
// `tokens` is read as plain JS at render time, so swapping it doesn't by itself
// re-render anyone. A boundary near the root subscribes and force-updates, which
// re-renders the tree so every `tokens.*` / `phaseShades[*]` read is fresh.
const listeners = new Set<() => void>();
export function subscribeTheme(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}
export function notifyThemeChanged(): void {
  listeners.forEach((l) => l());
}
