import { Box, Paper, type PaperProps, Table, Text, Title } from "@mantine/core";
import type { ReactNode } from "react";
import { tokens } from "./tokens";

export function PageHeading({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <Box mb={18}>
      <Title order={2} fz={22}>
        {title}
      </Title>
      {subtitle && (
        <Text c={tokens.textMuted} fz={12.5} mt={5}>
          {subtitle}
        </Text>
      )}
    </Box>
  );
}

export function Card({ children, ...props }: { children: ReactNode } & PaperProps) {
  return (
    <Paper withBorder radius="md" {...props}>
      {children}
    </Paper>
  );
}

/** Compact uppercase column header for Mantine tables. */
export function Th({ children, ta }: { children: ReactNode; ta?: "left" | "right" | "center" }) {
  return (
    <Table.Th ta={ta}>
      <Text component="span" fz={10} fw={700} c={tokens.textMuted} style={{ letterSpacing: 0.5 }}>
        {children}
      </Text>
    </Table.Th>
  );
}
