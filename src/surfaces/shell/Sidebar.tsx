import { Box, Divider, Stack, Text, UnstyledButton } from "@mantine/core";
import { IconArrowsLeftRight, IconSettings, type IconProps } from "@tabler/icons-react";
import type { ComponentType, ReactNode } from "react";
import { Brand } from "../../ui/Brand";
import { tokens } from "../../ui/tokens";
import { useAppDispatch, useAppSelector } from "../../store/hooks";
import { openSettings, setTab, switchProject, type NavTab } from "../../store/uiSlice";

/** One sidebar nav entry - the project shell and the plugin shell each pass their own set. */
export interface NavItem {
  tab: NavTab;
  label: string;
  icon: ComponentType<IconProps>;
}

function NavButton({
  icon: Icon,
  label,
  on,
  onClick,
}: {
  icon: ComponentType<IconProps>;
  label: string;
  on: boolean;
  onClick: () => void;
}) {
  return (
    <UnstyledButton
      onClick={onClick}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        height: 42,
        padding: "0 14px",
        borderRadius: 8,
        background: on ? tokens.accent : "transparent",
        transition: "background 120ms",
      }}
      onMouseEnter={(e) => {
        if (!on) e.currentTarget.style.background = tokens.hover;
      }}
      onMouseLeave={(e) => {
        if (!on) e.currentTarget.style.background = "transparent";
      }}
    >
      <Icon size={18} stroke={1.8} color={on ? tokens.onAccent : tokens.textMuted} />
      <Text fz={14} fw={on ? 600 : 500} c={on ? tokens.onAccent : tokens.text}>
        {label}
      </Text>
    </UnstyledButton>
  );
}

/** Shared shell sidebar: brand + a caller-supplied `info` block + the caller's nav,
 *  then the constant Settings / Change Project footer. Used by both the project shell
 *  and the plugin shell so the layout stays identical (only the nav + info differ). */
export function Sidebar({ nav, info }: { nav: NavItem[]; info: ReactNode }) {
  const active = useAppSelector((s) => s.ui.activeTab);
  const dispatch = useAppDispatch();

  return (
    <Box className="uep-chrome" p={16} h="100%" style={{ display: "flex", flexDirection: "column" }}>
      <Box px={6} pt={6} pb={2} style={{ display: "flex", justifyContent: "center" }}>
        <Brand size={96} wordmark={false} />
      </Box>
      {info}
      <Divider my={14} color={tokens.divider} />
      <Stack gap={6}>
        {nav.map(({ tab, label, icon }) => (
          <NavButton key={tab} icon={icon} label={label} on={tab === active} onClick={() => dispatch(setTab(tab))} />
        ))}
      </Stack>
      <Box style={{ flex: 1 }} />
      <Divider my={14} color={tokens.divider} />
      <Stack gap={6}>
        <NavButton icon={IconSettings} label="Settings" on={false} onClick={() => dispatch(openSettings())} />
        <NavButton icon={IconArrowsLeftRight} label="Change Project" on={false} onClick={() => dispatch(switchProject())} />
      </Stack>
    </Box>
  );
}

export { NavButton };
