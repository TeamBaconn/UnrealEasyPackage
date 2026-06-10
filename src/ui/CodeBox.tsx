import { Box } from "@mantine/core";
import type { ReactNode } from "react";
import { tokens } from "./tokens";

/** Inset monospace box for a folder path or command/script - surface-sunken fill +
 *  border (the "code box" used in modals and previews). Selectable text. */
export function CodeBox({ children, mt, fz = 12 }: { children: ReactNode; mt?: number; fz?: number }) {
  return (
    <Box
      className="uep-selectable"
      style={{
        background: tokens.surfaceAlt,
        border: `1px solid ${tokens.divider}`,
        borderRadius: 6,
        padding: "8px 12px",
        marginTop: mt,
        fontFamily: "var(--mantine-font-family-monospace)",
        fontSize: fz,
        color: tokens.text,
        wordBreak: "break-all",
        lineHeight: 1.5,
      }}
    >
      {children}
    </Box>
  );
}
