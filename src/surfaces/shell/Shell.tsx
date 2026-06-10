import { AppShell, Box, Text, Tooltip } from "@mantine/core";
import { IconBox, IconLayoutDashboard, IconSparkles, IconTerminal2 } from "@tabler/icons-react";
import { tokens } from "../../ui/tokens";
import { openFolder } from "../../ipc";
import { useAppSelector } from "../../store/hooks";
import { Sidebar, type NavItem } from "./Sidebar";
import { DashboardTab } from "./tabs/DashboardTab";
import { BuildTab } from "./tabs/BuildTab";
import { CleanTab } from "./tabs/CleanTab";
import { ToolsTab } from "./tabs/ToolsTab";

const NAV: NavItem[] = [
  { tab: "dashboard", label: "Dashboard", icon: IconLayoutDashboard },
  { tab: "build", label: "Build", icon: IconBox },
  { tab: "clean", label: "Clean", icon: IconSparkles },
  { tab: "tools", label: "Tools", icon: IconTerminal2 },
];

function baseName(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}

/** Project identity block under the brand - name + engine line, each opening its folder. */
function ProjectInfo() {
  const project = useAppSelector((s) => s.ui.currentProject);
  if (!project) return null;
  const engineLine = `UE ${project.engine.version.major}.${project.engine.version.minor} · ${
    project.engine.kind === "source" ? `${baseName(project.engine.root)} (source)` : "Launcher install"
  }`;
  return (
    <Box px={6} pt={6} pb={2} style={{ minWidth: 0 }}>
      <Tooltip label={`Open ${project.projectRoot}`} withArrow openDelay={300} position="bottom">
        <Text
          fw={600}
          fz={14}
          c={tokens.ink}
          ta="center"
          truncate
          style={{ cursor: "pointer", transition: "color 90ms ease" }}
          onMouseEnter={(e) => (e.currentTarget.style.color = tokens.accent)}
          onMouseLeave={(e) => (e.currentTarget.style.color = tokens.ink)}
          onClick={() => void openFolder(project.projectRoot)}
        >
          {project.name}
        </Text>
      </Tooltip>
      <Tooltip label={`Open ${project.engine.root}`} withArrow openDelay={300} position="bottom">
        <Text
          fz={11.5}
          c={tokens.textMuted}
          ta="center"
          truncate
          style={{ cursor: "pointer", transition: "color 90ms ease" }}
          onMouseEnter={(e) => (e.currentTarget.style.color = tokens.accent)}
          onMouseLeave={(e) => (e.currentTarget.style.color = tokens.textMuted)}
          onClick={() => void openFolder(project.engine.root)}
        >
          {engineLine}
        </Text>
      </Tooltip>
    </Box>
  );
}

export function Shell() {
  const tab = useAppSelector((s) => s.ui.activeTab);

  return (
    <AppShell layout="alt" navbar={{ width: 230, breakpoint: "sm" }} padding="lg">
      <AppShell.Navbar style={{ background: tokens.surface }}>
        <Sidebar nav={NAV} info={<ProjectInfo />} />
      </AppShell.Navbar>
      <AppShell.Main style={{ background: tokens.page }}>
        {tab === "dashboard" && <DashboardTab />}
        {tab === "build" && <BuildTab />}
        {tab === "clean" && <CleanTab />}
        {tab === "tools" && <ToolsTab />}
      </AppShell.Main>
    </AppShell>
  );
}
