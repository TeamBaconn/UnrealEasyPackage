import { useCallback, useEffect, useRef, useState } from "react";
import {
  ActionIcon,
  Box,
  Button,
  Checkbox,
  Collapse,
  Group,
  Paper,
  Select,
  Stack,
  Text,
  TextInput,
  Tooltip,
} from "@mantine/core";
import { IconBox, IconChevronDown, IconFolderOpen, IconRefresh } from "@tabler/icons-react";
import { tokens } from "../../../ui/tokens";
import { useAppSelector } from "../../../store/hooks";
import {
  IpcError,
  addCustomEngine,
  listEngines,
  loadPluginSettings,
  pickDirectory,
  pickEngineFolder,
  savePluginOutput,
  startPluginPackage,
  type EngineEntry,
} from "../../../ipc";
import { activeRun, onRunFinished, onRunStarted } from "../../../runner";
import { openRunLogs } from "../../../ui/windows";
import { RemoveUepIsland } from "./RemoveUepIsland";

const DEFAULT_TEMPLATE = "{plugin}-{version}";
// Fixed width for the engine version column so every entry's path starts at the same x.
const ENGINE_LABEL_W = 168;

function messageOf(e: unknown): string {
  if (e instanceof IpcError) return e.appError.message;
  return e instanceof Error ? e.message : String(e);
}

export function ActionsTab() {
  const plugin = useAppSelector((s) => s.ui.currentPlugin);

  const [engines, setEngines] = useState<EngineEntry[] | null>(null);
  const [engineRoot, setEngineRoot] = useState<string | null>(null);
  const [baseDir, setBaseDir] = useState("");
  const [folderTemplate, setFolderTemplate] = useState(DEFAULT_TEMPLATE);
  const [strip, setStrip] = useState(true);
  const [engineError, setEngineError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [actionsOpen, setActionsOpen] = useState(false);
  const prefsLoaded = useRef(false);

  const upluginPath = plugin?.upluginPath ?? null;

  // Load the engine list (registry + remembered, validated) when the plugin opens.
  const refreshEngines = useCallback(
    async (selectRoot?: string) => {
      if (!upluginPath) return;
      try {
        const list = await listEngines(upluginPath);
        setEngines(list);
        setEngineRoot((cur) => {
          if (selectRoot && list.some((e) => e.root === selectRoot)) return selectRoot;
          if (cur && list.some((e) => e.root === cur)) return cur;
          // Prefer an engine matching the .uplugin's EngineVersion, else the newest.
          const match = plugin?.engineVersion ? list.find((e) => e.version === plugin.engineVersion) : undefined;
          return match?.root ?? list[0]?.root ?? null;
        });
      } catch (e) {
        setEngineError(messageOf(e));
      }
    },
    [upluginPath, plugin?.engineVersion]
  );

  useEffect(() => {
    void refreshEngines();
  }, [refreshEngines]);

  // Recall this plugin's saved output folder + folder name (from .uap/settings.json).
  useEffect(() => {
    if (!upluginPath) return;
    let alive = true;
    prefsLoaded.current = false;
    loadPluginSettings(upluginPath)
      .then((s) => {
        if (!alive) return;
        if (s.outputDir) setBaseDir(s.outputDir);
        if (s.folderName) setFolderTemplate(s.folderName);
      })
      .catch(() => {})
      .finally(() => {
        if (alive) prefsLoaded.current = true;
      });
    return () => {
      alive = false;
    };
  }, [upluginPath]);

  // Persist output folder + folder name as they change (debounced), once loaded.
  useEffect(() => {
    if (!upluginPath || !prefsLoaded.current) return;
    const id = setTimeout(() => {
      void savePluginOutput(upluginPath, baseDir, folderTemplate);
    }, 400);
    return () => clearTimeout(id);
  }, [upluginPath, baseDir, folderTemplate]);

  // Track whether a run is in flight (the streaming log + resolved command live in their
  // own window) so the Package button reflects/disables it.
  useEffect(() => {
    let alive = true;
    activeRun()
      .then((s) => alive && s && setRunning(s.status === "running"))
      .catch(() => {});
    const subs = [
      onRunStarted(() => alive && setRunning(true)),
      onRunFinished(() => alive && setRunning(false)),
    ];
    return () => {
      alive = false;
      subs.forEach((p) => p.then((un) => un()).catch(() => {}));
    };
  }, []);

  const onBrowseOutput = useCallback(async () => {
    const dir = await pickDirectory("Choose the package output folder", baseDir || undefined);
    if (dir) setBaseDir(dir);
  }, [baseDir]);

  const onBrowseEngine = useCallback(async () => {
    if (!upluginPath) return;
    const dir = await pickEngineFolder();
    if (!dir) return;
    setEngineError(null);
    try {
      const entry = await addCustomEngine(upluginPath, dir);
      await refreshEngines(entry.root);
    } catch (e) {
      setEngineError(messageOf(e));
    }
  }, [upluginPath, refreshEngines]);

  const onPackage = useCallback(async () => {
    if (!upluginPath) return;
    setFormError(null);
    if (!engineRoot) {
      setFormError("Select an Unreal Engine to compile with.");
      return;
    }
    if (!baseDir.trim()) {
      setFormError("Choose an output folder.");
      return;
    }
    try {
      await startPluginPackage({
        pluginPath: upluginPath,
        engineRoot,
        baseDir,
        folderTemplate,
        stripBinariesIntermediate: strip,
      });
      setRunning(true);
      // The streaming log + resolved command open in their own window (no inline console).
      await openRunLogs();
    } catch (e) {
      setFormError(messageOf(e));
    }
  }, [upluginPath, engineRoot, baseDir, folderTemplate, strip]);

  if (!plugin) return null;

  const byRoot = new Map((engines ?? []).map((e) => [e.root, e]));
  const engineData = (engines ?? []).map((e) => ({
    value: e.root,
    label: `${e.label}${e.source === "custom" ? " (custom)" : ""}`,
  }));

  return (
    <Box style={{ height: "100%", overflowY: "auto" }}>
      <Stack gap="lg">
      <Paper withBorder radius="md" style={{ display: "flex", flexDirection: "column" }}>
        {/* header - click to collapse / expand */}
        <Group
          gap={10}
          px="md"
          py={12}
          wrap="nowrap"
          justify="space-between"
          className="uep-hoverable"
          style={{ cursor: "pointer", borderRadius: 8, borderBottom: actionsOpen ? `1px solid ${tokens.divider}` : "none" }}
          onClick={() => setActionsOpen((o) => !o)}
        >
          <Group gap={10} wrap="nowrap">
            <IconBox size={18} color={tokens.ink} />
            <Text fw={600} fz={15} c={tokens.ink}>
              Package Plugin
            </Text>
          </Group>
          <IconChevronDown
            size={18}
            color={tokens.textMuted}
            style={{ transform: actionsOpen ? "rotate(180deg)" : "none", transition: "transform 120ms" }}
          />
        </Group>

        <Collapse expanded={actionsOpen}>
          <Box style={{ padding: 16 }}>
            <Stack gap="md">
              {/* engine */}
              <Box>
                <Group gap={6} mb={6} wrap="nowrap">
                  <Text fz={13} fw={600} c={tokens.text}>
                    Unreal Engine
                  </Text>
                  <Tooltip label="Rescan installed engines" withArrow openDelay={300}>
                    <ActionIcon
                      size="sm"
                      variant="subtle"
                      color="gray"
                      aria-label="Rescan installed engines"
                      onClick={() => void refreshEngines()}
                    >
                      <IconRefresh size={14} />
                    </ActionIcon>
                  </Tooltip>
                </Group>
                <Group gap={8} wrap="nowrap" align="flex-start">
                  <Select
                    style={{ flex: 1 }}
                    data={engineData}
                    value={engineRoot}
                    onChange={setEngineRoot}
                    placeholder={engines === null ? "Detecting engines…" : engineData.length ? "Select an engine" : "No engines found - Browse…"}
                    nothingFoundMessage="No engines - use Browse…"
                    checkIconPosition="right"
                    comboboxProps={{ withinPortal: true }}
                    renderOption={({ option }) => {
                      const e = byRoot.get(option.value);
                      return (
                        <Group gap={12} wrap="nowrap" style={{ width: "100%", minWidth: 0 }}>
                          <Text fw={600} fz={13} c={tokens.text} style={{ width: ENGINE_LABEL_W, flexShrink: 0 }} truncate>
                            {option.label}
                          </Text>
                          <Text fz={12} c={tokens.textDim} ff="monospace" truncate style={{ minWidth: 0 }}>
                            {e?.root ?? ""}
                          </Text>
                        </Group>
                      );
                    }}
                  />
                  <Button variant="default" leftSection={<IconFolderOpen size={16} />} onClick={onBrowseEngine}>
                    Browse…
                  </Button>
                </Group>
                {engineError && (
                  <Text fz={12} c={tokens.danger} mt={6}>
                    {engineError}
                  </Text>
                )}
              </Box>

              {/* output folder */}
              <Box>
                <Text fz={13} fw={600} c={tokens.text} mb={6}>
                  Output folder
                </Text>
                <Group gap={8} wrap="nowrap">
                  <TextInput
                    style={{ flex: 1 }}
                    value={baseDir}
                    onChange={(e) => setBaseDir(e.currentTarget.value)}
                    placeholder="Where to write the packaged plugin"
                  />
                  <Button variant="default" leftSection={<IconFolderOpen size={16} />} onClick={onBrowseOutput}>
                    Browse…
                  </Button>
                </Group>
              </Box>

              {/* folder name (tokens) */}
              <Box>
                <Text fz={13} fw={600} c={tokens.text} mb={6}>
                  Folder name
                </Text>
                <TextInput
                  value={folderTemplate}
                  onChange={(e) => setFolderTemplate(e.currentTarget.value)}
                  placeholder={DEFAULT_TEMPLATE}
                />
                <Text fz={11} c={tokens.textDim} mt={4}>
                  Tokens: <Text span ff="monospace" fz={11}>{"{plugin} {version} {engine} {date} {time}"}</Text>
                </Text>
              </Box>

              {/* FAB strip */}
              <Checkbox
                checked={strip}
                onChange={(e) => setStrip(e.currentTarget.checked)}
                label={
                  <Box>
                    <Text fz={13} c={tokens.text}>
                      Delete Binaries and Intermediate after packaging
                    </Text>
                    <Text fz={11.5} c={tokens.textDim}>
                      Required for FAB submission - the uploaded plugin must carry no compiled output.
                    </Text>
                  </Box>
                }
              />

              {formError && (
                <Text fz={12.5} c={tokens.danger}>
                  {formError}
                </Text>
              )}
            </Stack>
          </Box>

          {/* bottom bar - Package button (collapses with the island) */}
          <Group justify="flex-end" px="md" py={12} style={{ borderTop: `1px solid ${tokens.divider}` }}>
            <Button
              leftSection={<IconBox size={16} />}
              disabled={running || !engineRoot || !baseDir.trim()}
              onClick={() => void onPackage()}
            >
              {running ? "Packaging… (see log window)" : "Package Plugin"}
            </Button>
          </Group>
        </Collapse>
      </Paper>
      <RemoveUepIsland running={running} />
      </Stack>
    </Box>
  );
}
