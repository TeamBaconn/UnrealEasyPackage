import { Button, Modal, createTheme, type MantineColorsTuple } from "@mantine/core";

// Yellow-Green brand ramp - the interactive accent (filled buttons, active controls).
// primaryShade lands on ~#a2d729 (brand Yellow Green); `autoContrast` + a low
// `luminanceThreshold` make Mantine pick pure-black text on the bright lime fills
// (white-on-lime is unreadable, black passes ~12:1).
const lime: MantineColorsTuple = [
  "#f4fbdf",
  "#e6f5b8",
  "#d3ec85",
  "#c0e455",
  "#b1de37",
  "#a8da2e",
  "#a2d729",
  "#88bb1e",
  "#6b9417",
  "#4f6e10",
];

export const theme = createTheme({
  primaryColor: "lime",
  primaryShade: { light: 6, dark: 5 },
  autoContrast: true,
  luminanceThreshold: 0.2,
  colors: { lime },
  fontFamily: "'Segoe UI', Inter, -apple-system, BlinkMacSystemFont, system-ui, sans-serif",
  fontFamilyMonospace: "'Consolas', 'Courier New', monospace",
  defaultRadius: "md",
  cursorType: "pointer",
  headings: {
    fontWeight: "700",
  },
  components: {
    // Secondary = branded Shadow-Grey fill. Every non-primary button uses
    // variant="default"; the built-in default variant resolves to a neutral grey, so we
    // re-point its inline --button-* vars at --uep-secondary* (scheme-aware in CSS).
    // Primary (filled lime) and destructive (color="red") buttons are untouched.
    Button: Button.extend({
      vars: (_theme, props) => {
        if (props.variant === "default") {
          return {
            root: {
              "--button-bg": "var(--uep-secondary)",
              "--button-hover": "var(--uep-secondary-hover)",
              "--button-color": "var(--uep-secondary-fg)",
              "--button-bd": "1px solid var(--uep-secondary-border)",
            },
          };
        }
        return { root: {} };
      },
    }),
    // All modal titles render bold.
    Modal: Modal.extend({
      styles: { title: { fontWeight: 700 } },
    }),
  },
});
