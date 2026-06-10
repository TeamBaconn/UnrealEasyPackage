import { useEffect, useState } from "react";
import { Badge, Box, Group, LoadingOverlay, Modal, Paper, Stack, Switch, Text } from "@mantine/core";
import { IconCheck, IconSettings } from "@tabler/icons-react";
import { Brand } from "../ui/Brand";
import { tokens } from "../ui/tokens";
import { appVersion, loadSettings, saveSettings } from "../ipc";
import type { AppSettings, Theme } from "../bindings";

/** App preferences as a modal over the main window (theme · notifications · about).
 *  Changes apply immediately: each toggle persists and `save_settings` broadcasts
 *  `uep://settings-changed`, so every window re-applies the theme live via ThemeSync. */
export function SettingsModal({ opened, onClose }: { opened: boolean; onClose: () => void }) {
  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title={
        <Group gap={10} wrap="nowrap" align="center">
          <IconSettings size={20} stroke={1.8} color={tokens.ink} />
          <Text fw={700} fz={18} c={tokens.ink}>
            Settings
          </Text>
        </Group>
      }
      centered
      size="lg"
      radius="md"
    >
      <SettingsBody />
    </Modal>
  );
}

function SettingsBody() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    loadSettings()
      .then(setSettings)
      .catch(() => setSettings({ theme: "light", notifyOnFinish: true, notifySound: true }));
    appVersion().then(setVersion).catch(() => setVersion(null));
  }, []);

  function patch(next: Partial<AppSettings>) {
    setSettings((cur) => {
      if (!cur) return cur;
      const merged = { ...cur, ...next };
      void saveSettings(merged).catch(() => {});
      return merged;
    });
  }

  if (!settings) {
    return (
      <Box pos="relative" style={{ minHeight: 280 }}>
        <LoadingOverlay visible overlayProps={{ blur: 1 }} />
      </Box>
    );
  }

  return (
    <Stack gap={18}>
      <SectionCard title="Appearance">
        <Group gap={16} grow align="stretch">
          <ThemeTile
            scheme="light"
            label="Light"
            selected={settings.theme === "light"}
            onSelect={() => patch({ theme: "light" })}
          />
          <ThemeTile
            scheme="dark"
            label="Dark"
            selected={settings.theme === "dark"}
            onSelect={() => patch({ theme: "dark" })}
          />
        </Group>
      </SectionCard>

      <SectionCard title="Notifications">
        <Stack gap={0}>
          <ToggleRow
            label="Desktop notification on build finish"
            checked={settings.notifyOnFinish}
            onChange={(v) => patch({ notifyOnFinish: v })}
          />
          <Box style={{ borderTop: `1px solid ${tokens.divider}` }} />
          <ToggleRow
            label="Play sound on finish"
            checked={settings.notifySound}
            disabled={!settings.notifyOnFinish}
            onChange={(v) => patch({ notifySound: v })}
          />
        </Stack>
      </SectionCard>

      <SectionCard title="About">
        <Stack gap={12}>
          <Group justify="space-between" wrap="nowrap">
            <Brand size={22} />
            <Badge variant="default" radius="sm" tt="none" ff="monospace" c={tokens.textMuted}>
              Version {version ?? "-"}
            </Badge>
          </Group>
          <Group justify="space-between" wrap="nowrap">
            <Text fw={700} fz="md" c={tokens.ink}>
              Author
            </Text>
            <Badge variant="default" radius="sm" tt="none" ff="monospace" c={tokens.textMuted}>
              Team Bacon
            </Badge>
          </Group>
        </Stack>
      </SectionCard>
    </Stack>
  );
}

function SectionCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <Paper
      withBorder
      radius="md"
      p={20}
      style={{ background: tokens.surface, borderColor: tokens.border }}
    >
      <Text fw={600} fz={14} c={tokens.text} mb={16}>
        {title}
      </Text>
      {children}
    </Paper>
  );
}

function ToggleRow({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <Group justify="space-between" wrap="nowrap" py={12} style={{ opacity: disabled ? 0.5 : 1 }}>
      <Text fw={500} fz={14} c={tokens.text}>
        {label}
      </Text>
      <Switch
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.currentTarget.checked)}
      />
    </Group>
  );
}

// Illustrative mini-window per scheme - like the design's previews, these are
// literal swatches (not theme-driven chrome), so the Dark tile shows dark colors
// even while the app is still in Light.
const PREVIEW = {
  light: { win: "#ffffff", bar: "#eef3f2", line: "#e6ecea", nav: "#e4eae8", navActive: "#a2d729", bars: "#dae1df" },
  dark: { win: "#342e37", bar: "#211d24", line: "#4d4653", nav: "#4d4653", navActive: "#a8da2e", bars: "#5e5666" },
} as const;

function ThemeTile({
  scheme,
  label,
  selected,
  onSelect,
}: {
  scheme: Theme;
  label: string;
  selected: boolean;
  onSelect: () => void;
}) {
  const p = PREVIEW[scheme];
  return (
    <Paper
      withBorder
      radius="md"
      p={14}
      onClick={onSelect}
      role="radio"
      aria-checked={selected}
      style={{
        position: "relative",
        cursor: "pointer",
        background: tokens.surfaceAlt,
        borderColor: selected ? tokens.accent : tokens.border,
        borderWidth: selected ? 2 : 1,
        // Keep the inner geometry steady when the 1→2px border grows.
        padding: selected ? 13 : 14,
      }}
    >
      {selected && (
        <Box
          style={{
            position: "absolute",
            top: 10,
            right: 10,
            width: 22,
            height: 22,
            borderRadius: "50%",
            background: tokens.accent,
            display: "grid",
            placeItems: "center",
          }}
        >
          <IconCheck size={13} color={tokens.onAccent} stroke={3} />
        </Box>
      )}
      {/* mini app-window mock */}
      <Box style={{ background: p.win, border: `1px solid ${p.line}`, borderRadius: 6, overflow: "hidden" }}>
        <Box style={{ height: 14, background: p.bar, borderBottom: `1px solid ${p.line}` }} />
        <Group gap={0} wrap="nowrap" align="stretch" style={{ height: 74 }}>
          <Stack gap={6} p={8} style={{ width: 64, borderRight: `1px solid ${p.line}` }}>
            <Box style={{ height: 8, borderRadius: 3, background: p.navActive }} />
            <Box style={{ height: 8, borderRadius: 3, background: p.nav }} />
            <Box style={{ height: 8, borderRadius: 3, background: p.nav }} />
          </Stack>
          <Stack gap={7} p={10} style={{ flex: 1 }}>
            <Box style={{ height: 7, width: "55%", borderRadius: 3, background: p.bars }} />
            <Box style={{ height: 7, width: "85%", borderRadius: 3, background: p.bars }} />
            <Box style={{ height: 7, width: "70%", borderRadius: 3, background: p.bars }} />
          </Stack>
        </Group>
      </Box>
      <Text ta="center" fw={selected ? 600 : 500} fz={13} mt={10} c={tokens.text}>
        {label}
      </Text>
    </Paper>
  );
}
