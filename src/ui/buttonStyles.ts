import type { CSSProperties } from "react";
import { tokens } from "./tokens";

/** Solid "Stage ✕ failed" red for destructive primary buttons (Cancel build, Delete,
 *  Discard, Confirm…). Overrides Mantine's Button CSS vars so the fill matches the
 *  palette-preview chip (#d6342a fill / #b22a21 border / white text, hover darkens to
 *  the border). Call in render so it follows the scheme swap. Apply to a filled Button
 *  (no `variant`/`color`) - `variant="default"` would re-point these same vars. */
export function dangerSolidButton(): CSSProperties {
  return {
    "--button-bg": tokens.dangerSolidBg,
    "--button-hover": tokens.dangerSolidBorder,
    "--button-color": tokens.dangerSolidFg,
    "--button-bd": `1px solid ${tokens.dangerSolidBorder}`,
  } as CSSProperties;
}
