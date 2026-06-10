# Design system - color & states

> Canonical UI palette for UnrealEasyPackage. The `ui-implementation` skill points here for color. Implemented in `src/styles.css` (`:root --uep-*` CSS variables - the source of truth) and mirrored as JS values in `src/ui/tokens.ts`; components use Mantine props/theme + these tokens, never one-off hexes.

## Intent
A **neutral, professional desktop-tool** look in the spirit of **Unity Hub / Unreal Editor**, recolored to the project **brand palette**: layered Porcelain / Shadow-Grey surfaces that clearly separate, a **Yellow Green** interactive accent for primary/active (pure black/white **ink** is reserved for heading emphasis), and semantic colors (green / Coral / red) used *only* on status. Text stays **pure neutral grayscale** (never brand-tinted) for contrast. Light scheme by default; a token-driven **dark scheme** (M6, below) flips alongside it. **Bias toward visible contrast** - surfaces, borders, and especially hover/active states must read clearly, never blend.

## Palette (light)

| Token (`--uep-*` / `tokens.*`) | Hex | Role |
|---|---|---|
| `bg` / `page` | `#eef3f2` | App canvas - Porcelain (makes white surfaces pop) |
| `surface` | `#ffffff` | Cards, bars, table body |
| `surface-sunken` / `surfaceAlt`,`headerRow` | `#eef3f2` | Table headers, inset/nested panels |
| `border` | `#dae1df` | Default card/table borders - clearly visible |
| `border-strong` / `borderStrong` | `#c4cfcc` | Inputs, emphasized separation |
| `divider` | `#e6ecea` | In-card row dividers |
| **`hover`** | **`#edf7cf`** | **Hover fill on rows / interactive entries (lime soft)** |
| **`active`** | **`#d4ec8f`** | Selected / pressed entry (stronger lime) |
| `accent` | `#a2d729` | **Yellow Green** - primary buttons, active nav, the hover **accent bar** |
| `ink` | `#000000` | Pure black - heading / emphasis text only |
| `secondary` | `#342e37` | Shadow Grey - secondary (`variant="default"`) button fill |
| `text` | `#1a1a1a` | Primary text |
| `text-muted` / `textMuted` | `#474747` | Secondary text, mono paths |
| `text-dim` / `textDim` | `#6e6e6e` | De-emphasized / placeholder |

**Status** (semantic - only on badges / glyphs / destructive actions, never as surface fills): success **green**, warning **Coral** (orange), danger **red** - applied via the status token families in `tokens.ts` (`success*` / `warn* `/ `danger*`), with `color="red"` for destructive menu items/buttons. The **Yellow Green (`lime`) ramp is Mantine `primaryColor`** (`primaryShade { light: 6, dark: 5 }`, `autoContrast` picking pure-black text on the bright lime fill); secondary buttons use a Shadow-Grey fill via `--uep-secondary*`.

## Palette (dark) - M6

Same layered surface language on a **Shadow-Grey near-black** base, contrast preserved. Selected via the Settings **modal** (theme pref persisted in `settings.json`, applied per-window on mount). **`ink` flips to pure white** (`#ffffff`) so heading emphasis reads on dark, and the Yellow Green accent brightens to `#a8da2e`; Mantine `primaryShade` is `{ light: 6, dark: 5 }` so filled buttons stay visible.

| Token | Dark hex | | Token | Dark hex |
|---|---|---|---|---|
| `bg`/`page` | `#3f3944` | | `active` | `#4f5e1d` |
| `surface` | `#342e37` | | `accent` | `#a8da2e` |
| `surface-sunken` | `#3f3944` | | `ink` | `#ffffff` |
| `border` | `#4d4653` | | `text` | `#ebebeb` |
| `border-strong` | `#5e5666` | | `text-muted` | `#c4c4c4` |
| `divider` | `#3a343f` | | `text-dim` | `#9a9a9a` |
| `hover` | `#2b3310` | | `secondary` | `#4a4350` |

**Two color paths (keep both in sync):** raw CSS reads `--uep-*`, overridden under `[data-mantine-color-scheme="dark"]` in `styles.css`. JS reads the `tokens` object (incl. SVG `fill=`/`stroke=` *attributes* in charts, where `var()` does **not** resolve) - so dark is a runtime **palette swap** (`applyScheme` in `tokens.ts`), re-rendered via the theme bus (`subscribeTheme`/`notifyThemeChanged` in `tokens.ts`; `ThemeSync` applies the scheme and the `useThemeRerender` hook force-re-renders the app-tree root). Status `*Bg`/`*Border` and the cleanup-category chart hues also have dark variants in `tokens.ts`.

## Interaction states (must be unmistakable)
- **Hover (selectable row / entry):** fill `--uep-hover` **plus** a 3px **Yellow Green (accent) left-accent bar** (`box-shadow: inset 3px 0 0 var(--uep-accent)` on the first cell). Apply via the `.uep-row` class (tables) or `.uep-hoverable` (other entries) in `styles.css`. This is the high-contrast cue - a bare background tint alone reads as "blended."
- **Active / selected:** fill `--uep-active` + the same accent bar (persisted).
- **Pressed:** Mantine default (`active` darken) is fine for buttons.
- **Disabled:** Mantine default (reduced opacity).
- **Focus:** Mantine default focus ring (keep - accessibility).

## Rules
- Color comes from these tokens / the Mantine theme - **no one-off literal hexes** pasted into components.
- Keep `styles.css :root` and `tokens.ts` in sync with this table; if you add a token, add it here first.
- New surfaces use `surface` on `bg`, `border` for edges, `surface-sunken` for headers/insets, and the `.uep-row`/`.uep-hoverable` hover treatment for anything clickable.
