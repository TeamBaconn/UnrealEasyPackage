import { useCallback, useEffect, useMemo, useState, type MouseEvent, type ReactNode } from "react";
import {
  ActionIcon,
  Box,
  Button,
  Code,
  Group,
  Loader,
  LoadingOverlay,
  Menu,
  Modal,
  Paper,
  Stack,
  Table,
  Text,
  TextInput,
  Title,
} from "@mantine/core";
import {
  IconBox,
  IconDotsVertical,
  IconFolderOpen,
  IconPencil,
  IconPlus,
  IconPuzzle,
  IconSearch,
  IconSettings,
  IconStar,
  IconStarFilled,
  IconTrash,
} from "@tabler/icons-react";
import { Brand } from "../../ui/Brand";
import { Tag } from "../../ui/Tag";
import { Th } from "../../ui/layout";
import { CodeBox } from "../../ui/CodeBox";
import { tokens } from "../../ui/tokens";
import { useAppDispatch } from "../../store/hooks";
import {
  openPlugin as openPluginAction,
  openProject as openProjectAction,
  openSettings,
} from "../../store/uiSlice";
import {
  appVersion,
  IpcError,
  listRecents,
  locateEngine,
  openFolder,
  openPlugin as openPluginCmd,
  openProject as openProjectCmd,
  parentDir,
  pickEngineFolder,
  pickUplugin,
  pickUproject,
  removeRecent,
  setRecentStarred,
  validateProject,
  type RecentEntry,
} from "../../ipc";

function fmtLastOpened(ms: number | null): string {
  if (ms == null) return "-";
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function messageOf(e: unknown): string {
  if (e instanceof IpcError) return e.appError.message;
  return e instanceof Error ? e.message : String(e);
}

export function ProjectSelection() {
  const dispatch = useAppDispatch();
  const [recents, setRecents] = useState<RecentEntry[] | null>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Engine unresolved for a project the user is opening → explain + offer Locate
  // (instead of silently popping a native folder picker mid-open).
  const [pendingEngine, setPendingEngine] = useState<{
    uprojectPath: string;
    name: string;
    association: string;
  } | null>(null);
  const [locating, setLocating] = useState(false);
  // A picked folder that wasn't an engine root → shown in an overlapping modal (the
  // path), not inline in the Locate modal or the gate.
  const [badEngineDir, setBadEngineDir] = useState<string | null>(null);
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    appVersion().then(setVersion).catch(() => setVersion(null));
  }, []);

  const refresh = useCallback(async () => {
    try {
      setRecents(await listRecents());
    } catch (e) {
      setError(messageOf(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Project open flow: validate → (engine missing? hand off to the Locate modal) →
  // open → project shell.
  const openFlow = useCallback(
    async (uprojectPath: string) => {
      setBusy(true);
      setError(null);
      try {
        const validation = await validateProject(uprojectPath);
        if (!validation.engine) {
          // Don't pop a contextless native picker - explain first via the modal.
          setBadEngineDir(null);
          setPendingEngine({
            uprojectPath,
            name: validation.name,
            association: validation.engineAssociation,
          });
          return;
        }
        dispatch(openProjectAction(await openProjectCmd(uprojectPath)));
      } catch (e) {
        setError(messageOf(e));
        void refresh();
      } finally {
        setBusy(false);
      }
    },
    [dispatch, refresh]
  );

  // Plugin open flow: no engine resolution (the compile engine is chosen per-package
  // in the Actions tab) - open straight into the plugin shell.
  const openPluginFlow = useCallback(
    async (pluginPath: string) => {
      setBusy(true);
      setError(null);
      try {
        dispatch(openPluginAction(await openPluginCmd(pluginPath)));
      } catch (e) {
        setError(messageOf(e));
        void refresh();
      } finally {
        setBusy(false);
      }
    },
    [dispatch, refresh]
  );

  // Modal "Locate" → native folder picker → validate+save → resume the open.
  // Cancelling the native picker keeps the modal up so the user can retry.
  const onLocateConfirm = useCallback(async () => {
    if (!pendingEngine) return;
    const dir = await pickEngineFolder();
    if (!dir) return;
    setLocating(true);
    setBadEngineDir(null);
    try {
      await locateEngine(pendingEngine.uprojectPath, dir);
      const path = pendingEngine.uprojectPath;
      setPendingEngine(null);
      dispatch(openProjectAction(await openProjectCmd(path)));
    } catch {
      // Wrong folder (no engine there) → surface the path in the overlap modal; keep
      // the Locate modal open so the user can immediately pick again.
      setBadEngineDir(dir);
    } finally {
      setLocating(false);
    }
  }, [pendingEngine, dispatch]);

  const onLocateCancel = useCallback(() => {
    setPendingEngine(null);
    setBadEngineDir(null);
  }, []);

  const onOpenProject = useCallback(async () => {
    const path = await pickUproject();
    if (path) void openFlow(path);
  }, [openFlow]);

  const onOpenPlugin = useCallback(async () => {
    const path = await pickUplugin();
    if (path) void openPluginFlow(path);
  }, [openPluginFlow]);

  // Re-pick a descriptor (the one at the recorded path is unreadable) - by kind.
  const onFix = useCallback(
    async (entry: RecentEntry) => {
      const path = entry.kind === "plugin" ? await pickUplugin() : await pickUproject();
      if (!path) return;
      if (entry.kind === "plugin") void openPluginFlow(path);
      else void openFlow(path);
    },
    [openFlow, openPluginFlow]
  );

  const onFixEngine = useCallback(
    async (entry: RecentEntry) => {
      const dir = await pickEngineFolder();
      if (!dir) return;
      setBusy(true);
      setBadEngineDir(null);
      try {
        await locateEngine(entry.path, dir);
        await refresh();
      } catch {
        // Wrong folder → the overlap modal with the path, not an error under the table.
        setBadEngineDir(dir);
      } finally {
        setBusy(false);
      }
    },
    [refresh]
  );

  const onRemove = useCallback(
    async (entry: RecentEntry) => {
      try {
        await removeRecent(entry.path);
        await refresh();
      } catch (e) {
        setError(messageOf(e));
      }
    },
    [refresh]
  );

  const onStar = useCallback(
    async (entry: RecentEntry) => {
      try {
        await setRecentStarred(entry.path, !entry.starred);
        await refresh();
      } catch (e) {
        setError(messageOf(e));
      }
    },
    [refresh]
  );

  const onRowOpen = useCallback(
    (entry: RecentEntry) => {
      if (!entry.valid) return void onFix(entry);
      if (entry.kind === "plugin") return void openPluginFlow(entry.path);
      return void openFlow(entry.path);
    },
    [onFix, openFlow, openPluginFlow]
  );

  const filtered = useMemo(() => {
    if (!recents) return null;
    const q = query.trim().toLowerCase();
    if (!q) return recents;
    return recents.filter(
      (r) => r.name.toLowerCase().includes(q) || r.path.toLowerCase().includes(q)
    );
  }, [recents, query]);

  return (
    <Box mih="100vh" style={{ display: "flex", flexDirection: "column" }}>
      {/* content */}
      <Box px="xl" py="xl" maw={1320} mx="auto" w="100%" style={{ flex: 1 }}>
        <Group justify="space-between" align="flex-end" mb="md" px="md">
          <Title order={2}>Recent</Title>
          <Group gap="sm">
            <TextInput
              w={300}
              leftSection={<IconSearch size={16} />}
              placeholder="Search recent projects & plugins"
              value={query}
              onChange={(e) => setQuery(e.currentTarget.value)}
            />
            <Menu position="bottom-end" shadow="md" width={240} withinPortal>
              <Menu.Target>
                <Button leftSection={<IconPlus size={16} />}>Open</Button>
              </Menu.Target>
              <Menu.Dropdown>
                <Menu.Label>Open something to work on</Menu.Label>
                <Menu.Item leftSection={<IconBox size={16} />} onClick={onOpenProject}>
                  Project <Code>.uproject</Code>
                </Menu.Item>
                <Menu.Item leftSection={<IconPuzzle size={16} />} onClick={onOpenPlugin}>
                  Plugin <Code>.uplugin</Code>
                </Menu.Item>
              </Menu.Dropdown>
            </Menu>
          </Group>
        </Group>

        {error && (
          <Text c="red.7" fz="sm" mb="sm">
            {error}
          </Text>
        )}

        <Paper withBorder radius="md" pos="relative" style={{ overflow: "hidden" }}>
          <LoadingOverlay visible={busy} zIndex={5} />
          <Table horizontalSpacing="md" verticalSpacing="sm" layout="fixed">
            <Table.Thead>
              <Table.Tr>
                <Th>NAME</Th>
                <Th>ENGINE</Th>
                <Table.Th w={170}>
                  <HeadLabel>LAST OPENED</HeadLabel>
                </Table.Th>
                <Table.Th w={56} />
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {filtered === null ? (
                <CenterRow>
                  <Loader size="sm" color="dark" />
                </CenterRow>
              ) : filtered.length === 0 ? (
                <CenterRow>
                  <Text c="dimmed" fz="sm">
                    {recents && recents.length > 0
                      ? "No matches."
                      : "Nothing yet. Use Open to pick a .uproject or .uplugin."}
                  </Text>
                </CenterRow>
              ) : (
                filtered.map((entry) => (
                  <RecentRow
                    key={entry.path}
                    entry={entry}
                    onOpen={() => onRowOpen(entry)}
                    onStar={() => onStar(entry)}
                    onFix={() => onFix(entry)}
                    onFixEngine={() => onFixEngine(entry)}
                    onRemove={() => onRemove(entry)}
                  />
                ))
              )}
            </Table.Tbody>
          </Table>
        </Paper>
      </Box>

      {/* bottom bar - brand + settings as a footer */}
      <Group
        h={64}
        px="xl"
        justify="space-between"
        className="uep-chrome"
        bg="var(--uep-surface)"
        style={{ borderTop: "1px solid var(--uep-border)" }}
      >
        <Group gap={12} wrap="nowrap">
          <Brand />
          {version && (
            <Text fz={12} c={tokens.textMuted}>
              v{version}
            </Text>
          )}
        </Group>
        <Button variant="default" px={10} onClick={() => dispatch(openSettings())} aria-label="Settings">
          <IconSettings size={18} />
        </Button>
      </Group>

      <Modal opened={pendingEngine !== null} onClose={onLocateCancel} title="Engine not found" centered>
        <Stack gap="md">
          <Text fz="sm" c={tokens.warn}>
            Couldn&apos;t resolve the Engine path for{" "}
            <Text span fw={600}>
              {pendingEngine?.name}
            </Text>
            . Help us find the engine folder first to continue.
          </Text>
          {pendingEngine?.association ? (
            <Text fz="xs" c="dimmed">
              Engine association: <Code>{pendingEngine.association}</Code>
            </Text>
          ) : null}
          <Group justify="flex-end" gap="sm">
            <Button variant="default" onClick={onLocateCancel} disabled={locating}>
              Cancel
            </Button>
            <Button leftSection={<IconFolderOpen size={16} />} onClick={() => void onLocateConfirm()} loading={locating}>
              Locate Engine Folder
            </Button>
          </Group>
        </Stack>
      </Modal>

      {/* Overlaps the Locate modal when the picked folder isn't an engine root. */}
      <Modal
        opened={badEngineDir !== null}
        onClose={() => setBadEngineDir(null)}
        title="Not an Unreal Engine root"
        centered
        zIndex={500}
      >
        <Stack gap="md">
          <Text fz="sm" c="red.7">
            The following folder isn&apos;t an Unreal Engine root:
          </Text>
          <CodeBox>{badEngineDir}</CodeBox>
          <Group justify="flex-end">
            <Button onClick={() => setBadEngineDir(null)}>OK</Button>
          </Group>
        </Stack>
      </Modal>
    </Box>
  );
}

function HeadLabel({ children }: { children: ReactNode }) {
  return (
    <Text component="span" fz={10} fw={700} c="var(--uep-text-muted)" style={{ letterSpacing: 0.5 }}>
      {children}
    </Text>
  );
}

function CenterRow({ children }: { children: ReactNode }) {
  return (
    <Table.Tr>
      <Table.Td colSpan={4}>
        <Group justify="center" py={40}>
          {children}
        </Group>
      </Table.Td>
    </Table.Tr>
  );
}

/** PROJECT / PLUGIN pill - the gate's type tag (replaces the old Ready/Invalid status). */
// Monospace path that opens its folder on click (underline-on-hover affordance).
// Non-clickable when `onOpen` is omitted (e.g. an invalid/unresolved path).
function PathLink({ text, invalid, onOpen }: { text: string; invalid?: boolean; onOpen?: () => void }) {
  return (
    <Text
      ff="monospace"
      fz="xs"
      c={invalid ? "red.7" : "dimmed"}
      truncate
      className={onOpen ? "uep-path-link" : undefined}
      onClick={
        onOpen
          ? (e) => {
              e.stopPropagation();
              onOpen();
            }
          : undefined
      }
    >
      {text}
    </Text>
  );
}

function RecentRow({
  entry,
  onOpen,
  onStar,
  onFix,
  onFixEngine,
  onRemove,
}: {
  entry: RecentEntry;
  onOpen: () => void;
  onStar: () => void;
  onFix: () => void;
  onFixEngine: () => void;
  onRemove: () => void;
}) {
  const isPlugin = entry.kind === "plugin";
  const stop = (e: MouseEvent) => e.stopPropagation();
  const [starHover, setStarHover] = useState(false);
  const starFilled = entry.starred || starHover;
  const starColor = entry.starred ? "var(--uep-accent)" : "var(--uep-active)";

  return (
    <Table.Tr className="uep-row" style={{ cursor: "pointer" }} onClick={onOpen}>
      {/* NAME */}
      <Table.Td>
        <Group gap="sm" wrap="nowrap">
          <ActionIcon
            variant="transparent"
            size="lg"
            aria-label={entry.starred ? "Unpin" : "Pin to top"}
            onMouseEnter={() => setStarHover(true)}
            onMouseLeave={() => setStarHover(false)}
            onClick={(e) => {
              stop(e);
              onStar();
            }}
          >
            {starFilled ? (
              <IconStarFilled size={18} style={{ color: starColor }} />
            ) : (
              <IconStar size={18} stroke={1.6} style={{ color: "var(--uep-text-muted)" }} />
            )}
          </ActionIcon>
          <Box style={{ minWidth: 0 }}>
            <Group gap={8} wrap="nowrap">
              <Text fw={600} fz="sm" truncate>
                {entry.name}
              </Text>
              {isPlugin && <Tag>Plugin</Tag>}
            </Group>
            <Group gap={4} wrap="nowrap">
              <PathLink text={entry.path} invalid={!entry.valid} />
            </Group>
          </Box>
        </Group>
      </Table.Td>

      {/* ENGINE (projects only - a plugin's engine is chosen per-package) */}
      <Table.Td>
        {isPlugin ? (
          <Text c="dimmed" fz="sm">
            -
          </Text>
        ) : entry.valid ? (
          <Box style={{ minWidth: 0 }}>
            <Text fw={600} fz="sm">
              {entry.engineVersion ? `UE ${entry.engineVersion}` : "-"}
            </Text>
            <Group gap={4} wrap="nowrap">
              {entry.engineValid && entry.enginePath ? (
                <PathLink text={entry.enginePath} />
              ) : (
                <PathLink text={entry.enginePath ?? "association unresolved"} invalid />
              )}
            </Group>
          </Box>
        ) : (
          <Text c="dimmed" fz="sm">
            -
          </Text>
        )}
      </Table.Td>

      {/* LAST OPENED */}
      <Table.Td>
        <Text fz="sm" c="dimmed">
          {fmtLastOpened(entry.lastOpenedMs)}
        </Text>
      </Table.Td>

      {/* kebab */}
      <Table.Td onClick={stop}>
        <Menu position="bottom-end" shadow="md" width={210} withinPortal>
          <Menu.Target>
            <ActionIcon variant="subtle" color="gray" aria-label="Row actions">
              <IconDotsVertical size={18} />
            </ActionIcon>
          </Menu.Target>
          <Menu.Dropdown>
            <Menu.Item leftSection={<IconFolderOpen size={16} />} onClick={() => void openFolder(parentDir(entry.path))}>
              Open {isPlugin ? "plugin" : "project"} folder
            </Menu.Item>
            {!isPlugin && (
              <Menu.Item
                leftSection={<IconFolderOpen size={16} />}
                disabled={!entry.enginePath}
                onClick={() => entry.enginePath && void openFolder(entry.enginePath)}
              >
                Open engine folder
              </Menu.Item>
            )}
            <Menu.Divider />
            <Menu.Item leftSection={<IconPencil size={16} />} onClick={onFix}>
              Change {isPlugin ? "plugin" : "project"} path
            </Menu.Item>
            {!isPlugin && (
              <Menu.Item leftSection={<IconPencil size={16} />} onClick={onFixEngine}>
                Change engine path
              </Menu.Item>
            )}
            <Menu.Divider />
            <Menu.Item
              className="uep-menu-item-danger"
              leftSection={<IconTrash size={16} color="currentColor" />}
              onClick={onRemove}
            >
              Remove from recents
            </Menu.Item>
          </Menu.Dropdown>
        </Menu>
      </Table.Td>
    </Table.Tr>
  );
}
