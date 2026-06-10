import { AppShell, Box, Text, Tooltip } from "@mantine/core";
import { IconTerminal2 } from "@tabler/icons-react";
import { tokens } from "../../ui/tokens";
import { openFolder } from "../../ipc";
import { useAppSelector } from "../../store/hooks";
import { Sidebar, type NavItem } from "./Sidebar";
import { ActionsTab } from "./tabs/ActionsTab";

const NAV: NavItem[] = [{ tab: "actions", label: "Tools", icon: IconTerminal2 }];

/** Plugin identity block under the brand - friendly name + version, opening the
 *  plugin folder on click (mirrors the project shell's ProjectInfo). */
function PluginInfo() {
  const plugin = useAppSelector((s) => s.ui.currentPlugin);
  if (!plugin) return null;
  const sub = [plugin.versionName ? `v${plugin.versionName}` : null, "plugin"].filter(Boolean).join(" · ");
  return (
    <Box px={6} pt={6} pb={2} style={{ minWidth: 0 }}>
      <Tooltip label={`Open ${plugin.pluginRoot}`} withArrow openDelay={300} position="bottom">
        <Text
          fw={600}
          fz={14}
          c={tokens.ink}
          ta="center"
          truncate
          style={{ cursor: "pointer", transition: "color 90ms ease" }}
          onMouseEnter={(e) => (e.currentTarget.style.color = tokens.accent)}
          onMouseLeave={(e) => (e.currentTarget.style.color = tokens.ink)}
          onClick={() => void openFolder(plugin.pluginRoot)}
        >
          {plugin.friendlyName}
        </Text>
      </Tooltip>
      <Text fz={11.5} c={tokens.textMuted} ta="center" truncate>
        {sub}
      </Text>
    </Box>
  );
}

/** The plugin shell - same sidebar+main layout as the project shell, but the only
 *  tab is Tools (the packaging action). */
export function PluginShell() {
  const tab = useAppSelector((s) => s.ui.activeTab);
  return (
    <AppShell layout="alt" navbar={{ width: 230, breakpoint: "sm" }} padding="lg">
      <AppShell.Navbar style={{ background: tokens.surface }}>
        <Sidebar nav={NAV} info={<PluginInfo />} />
      </AppShell.Navbar>
      <AppShell.Main style={{ background: tokens.page }}>
        {tab === "actions" && <ActionsTab />}
      </AppShell.Main>
    </AppShell>
  );
}
