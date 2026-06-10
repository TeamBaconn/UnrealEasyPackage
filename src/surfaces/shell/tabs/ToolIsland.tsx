import { useState, type ComponentType, type ReactNode } from "react";
import { Box, Button, Collapse, Group, Paper, Stack, Text } from "@mantine/core";
import { IconChevronDown, type IconProps } from "@tabler/icons-react";
import { tokens } from "../../../ui/tokens";
import { dangerSolidButton } from "../../../ui/buttonStyles";

/** One collapsible tool island - the shared chrome (header toggle + collapse + bottom
 *  action bar) for the project and plugin Tools tabs. `running` disables the button (a run
 *  in flight); `busy` swaps the label to a running state; `danger` styles the button red for
 *  a destructive action. */
export function ToolIsland({
  icon: Icon,
  title,
  runLabel,
  runIcon,
  running,
  busy,
  danger = false,
  onRun,
  children,
}: {
  icon: ComponentType<IconProps>;
  title: string;
  runLabel: string;
  runIcon: ReactNode;
  running: boolean;
  busy: boolean;
  danger?: boolean;
  onRun: () => void;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const dangerSolid = dangerSolidButton();
  return (
    <Paper withBorder radius="md" style={{ display: "flex", flexDirection: "column" }}>
      {/* header - click to collapse / expand */}
      <Group
        gap={10}
        px="md"
        py={12}
        wrap="nowrap"
        justify="space-between"
        className="uep-hoverable"
        style={{ cursor: "pointer", borderRadius: 8, borderBottom: open ? `1px solid ${tokens.divider}` : "none" }}
        onClick={() => setOpen((o) => !o)}
      >
        <Group gap={10} wrap="nowrap">
          <Icon size={18} color={tokens.ink} />
          <Text fw={600} fz={15} c={tokens.ink}>
            {title}
          </Text>
        </Group>
        <IconChevronDown
          size={18}
          color={tokens.textMuted}
          style={{ transform: open ? "rotate(180deg)" : "none", transition: "transform 120ms" }}
        />
      </Group>

      <Collapse expanded={open}>
        <Box style={{ padding: 16 }}>
          <Stack gap="md">{children}</Stack>
        </Box>

        {/* bottom bar - action button (collapses with the island) */}
        <Group justify="flex-end" px="md" py={12} style={{ borderTop: `1px solid ${tokens.divider}` }}>
          <Button leftSection={runIcon} disabled={running} onClick={onRun} style={danger ? dangerSolid : undefined}>
            {busy ? "Running… (see log window)" : runLabel}
          </Button>
        </Group>
      </Collapse>
    </Paper>
  );
}
