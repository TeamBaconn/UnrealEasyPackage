import { Box } from "@mantine/core";
import type { ReactNode } from "react";
import { tokens } from "./tokens";

/** Small neutral pill for auto-derived build tags (platform / config / target). */
export function Tag({ children }: { children: ReactNode }) {
  return (
    <Box
      style={{
        display: "inline-flex",
        alignItems: "center",
        height: 18,
        padding: "0 9px",
        borderRadius: 9,
        background: tokens.neutralBadgeBg,
        border: `1px solid ${tokens.neutralBadgeBorder}`,
        fontSize: 10.5,
        color: tokens.textMuted,
        whiteSpace: "nowrap",
      }}
    >
      {children}
    </Box>
  );
}
