import { Box } from "@mantine/core";
import { IconAlertTriangle, IconCheck, IconX } from "@tabler/icons-react";
import type { ReactNode } from "react";
import { tokens } from "./tokens";

/** Footprint safety classification → badge. `slow-regen` for DDC (regenerates, but
 *  slowly); `output`/`never` retained for the protected/output cases the UI labels. */
export type SafeKind = "safe" | "slow-regen" | "output" | "never";

function Pill({
  bg,
  border,
  color,
  dashed,
  small,
  children,
}: {
  bg: string;
  border: string;
  color: string;
  dashed?: boolean;
  small?: boolean;
  children: ReactNode;
}) {
  return (
    <Box
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        height: small ? 16 : 22,
        padding: small ? "0 8px" : "0 10px",
        borderRadius: 11,
        background: bg,
        border: `1px solid ${border}`,
        borderStyle: dashed ? "dashed" : "solid",
        fontSize: small ? 10 : 11.5,
        fontWeight: 600,
        color,
        whiteSpace: "nowrap",
        lineHeight: 1,
      }}
    >
      {children}
    </Box>
  );
}

const Dot = ({ color }: { color: string }) => (
  <Box style={{ width: 7, height: 7, borderRadius: "50%", background: color }} />
);

export type StatusKind = "ready" | "success" | "failed" | "cancelled" | "engine-not-found" | "missing";

export function StatusBadge({ kind }: { kind: StatusKind }) {
  switch (kind) {
    case "ready":
      return (
        <Pill bg={tokens.successBg} border={tokens.successBorder} color={tokens.successText}>
          <Dot color={tokens.success} /> Ready
        </Pill>
      );
    case "success":
      return (
        <Pill bg={tokens.successBg} border={tokens.successBorder} color={tokens.successText}>
          <IconCheck size={12} stroke={2.5} /> Success
        </Pill>
      );
    case "failed":
      return (
        <Pill bg={tokens.dangerBg} border={tokens.dangerBorder} color={tokens.danger}>
          <IconX size={12} stroke={2} /> Failed
        </Pill>
      );
    case "cancelled":
      return (
        <Pill bg={tokens.warnBg} border={tokens.warnBorder} color={tokens.warn}>
          <IconX size={12} stroke={2} /> Cancelled
        </Pill>
      );
    case "engine-not-found":
      return (
        <Pill bg={tokens.warnBg} border={tokens.warnBorder} color={tokens.warn}>
          <IconAlertTriangle size={12} stroke={2} /> Engine not found
        </Pill>
      );
    case "missing":
      return (
        <Pill bg={tokens.neutralBadgeBg} border={tokens.neutralBadgeBorder} color={tokens.textMuted} dashed>
          Missing · auto-prune
        </Pill>
      );
  }
}

export function SafeBadge({ kind }: { kind: SafeKind }) {
  switch (kind) {
    case "safe":
      return (
        <Pill small bg={tokens.successBg} border={tokens.successBorder} color={tokens.successText}>
          ✓ safe
        </Pill>
      );
    case "slow-regen":
      return (
        <Pill small bg={tokens.warnBg} border={tokens.warnBorder} color={tokens.warn}>
          ✓ slow regen
        </Pill>
      );
    case "output":
      return (
        <Pill small bg={tokens.warnBg} border={tokens.warnBorder} color={tokens.warn}>
          ⚠ output
        </Pill>
      );
    case "never":
      return (
        <Pill small bg={tokens.cancelledBg} border={tokens.cancelledBorder} color={tokens.textMuted}>
          ✕ never
        </Pill>
      );
  }
}
