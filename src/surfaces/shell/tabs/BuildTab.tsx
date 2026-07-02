import { useEffect, useState, type CSSProperties } from "react";
import { ActionIcon, Box, Button, Checkbox, Group, Menu, Modal, NumberInput, Pagination, Paper, Select, Stack, Table, Text, Tooltip } from "@mantine/core";
import { IconFolder, IconPlayerPlayFilled, IconTool, IconTrash } from "@tabler/icons-react";
import { Th } from "../../../ui/layout";
import { tokens } from "../../../ui/tokens";
import { formatBytes as fmtSize } from "../../../ui/format";
import { CodeBox } from "../../../ui/CodeBox";
import { dangerSolidButton } from "../../../ui/buttonStyles";
import { StatusBadge, type StatusKind } from "../../../ui/StatusBadge";
import { Tag } from "../../../ui/Tag";
import { openAuxWindow, openBuildLogs, onProfilesChanged } from "../../../ui/windows";
import {
  IpcError,
  checkBuildLocation,
  checkOutput,
  deleteHistory,
  listHistoryPage,
  listProfiles,
  openFolder,
  type BuildConfig,
  type BuildRecord,
  type FilterOptions,
} from "../../../ipc";
import { activeRun, onRunFinished, onRunStarted, startBuild, type RunSnapshot, type RunStatus } from "../../../runner";

// Status vocabulary (mirrors history/tags.rs) for the badge + splitting a record's
// flat tags into status vs the platform/config/target pills. Filtering and the filter
// option lists are computed server-side now (the SQLite index).
const STATUSES = ["Success", "Failed", "Cancelled"];
const isStatus = (t: string) => STATUSES.includes(t);
const STATUS_KIND: Record<string, StatusKind> = { Success: "success", Failed: "failed", Cancelled: "cancelled" };

function statusOf(tags: string[]): string {
  return tags.find(isStatus) ?? "Success";
}

function fmtSecs(secs: number | null): string {
  const s = Math.round(secs ?? 0);
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}
function fmtElapsed(ms: number): string {
  return fmtSecs(ms / 1000);
}

type LocWarn = { kind: "missing" | "changed"; path: string };

export function BuildTab() {
  const [realProfiles, setRealProfiles] = useState<BuildConfig[]>([]);
  const [profile, setProfile] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);
  const [replaceConfirm, setReplaceConfirm] = useState<{ path: string } | null>(null);

  const [records, setRecords] = useState<BuildRecord[]>([]);
  const [filteredTotal, setFilteredTotal] = useState(0);
  const [grandTotal, setGrandTotal] = useState(0);
  const [options, setOptions] = useState<FilterOptions>({ platform: [], config: [], target: [], status: [] });
  const [reloadKey, setReloadKey] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [filters, setFilters] = useState<{ platform: string | null; config: string | null; target: string | null; status: string | null }>({
    platform: null,
    config: null,
    target: null,
    status: null,
  });
  const [locWarn, setLocWarn] = useState<LocWarn | null>(null);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);

  const [live, setLive] = useState<RunSnapshot | null>(null);
  const [liveStatus, setLiveStatus] = useState<RunStatus>("running");
  const [now, setNow] = useState(() => Date.now());
  const showLive = live != null && liveStatus === "running";

  // Re-read profiles from disk. Runs on mount AND whenever the separate Build
  // Settings window signals a change (uep://profiles-changed), so the list - and the
  // profile handed to start_build - stay current. Keeps the current selection if it
  // still exists, else falls back to the first.
  const reloadProfiles = () =>
    listProfiles()
      .then((ps) => {
        setRealProfiles(ps);
        setProfile((cur) => (cur && ps.some((p) => p.id === cur) ? cur : ps[0]?.id ?? null));
      })
      .catch(() => {});

  useEffect(() => {
    reloadProfiles();
  }, []);

  useEffect(() => {
    let alive = true;
    activeRun()
      .then((s) => {
        if (alive && s) {
          setLive(s);
          setLiveStatus(s.status);
        }
      })
      .catch(() => {});
    const uns = [
      onRunStarted((s) => {
        setLive(s);
        setLiveStatus("running");
      }),
      onRunFinished((f) => {
        setLiveStatus(f.status);
        setReloadKey((k) => k + 1); // the finished build now has a record - reload the page
      }),
      onProfilesChanged(reloadProfiles), // edits from the Build Settings window
    ];
    return () => {
      alive = false;
      uns.forEach((p) => p.then((u) => u()).catch(() => {}));
    };
  }, []);

  useEffect(() => {
    if (liveStatus !== "running") return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [liveStatus]);

  // Server-side paging via the SQLite index: fetch just this page (filter + sort done
  // in SQL), re-running when the page, page size, filters, or a finished/deleted build
  // (reloadKey) change. The `alive` guard drops a stale response.
  useEffect(() => {
    let alive = true;
    listHistoryPage((page - 1) * pageSize, pageSize, filters)
      .then((p) => {
        if (!alive) return;
        setRecords(p.records);
        setFilteredTotal(p.filteredTotal ?? 0);
        setGrandTotal(p.grandTotal ?? 0);
        setOptions(p.options);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [page, pageSize, filters, reloadKey]);

  const pageCount = Math.max(1, Math.ceil(filteredTotal / pageSize));
  const currentPage = Math.min(page, pageCount);
  // A delete (or filter) can shrink the result below the current page - pull back in.
  useEffect(() => {
    if (page > pageCount) setPage(pageCount);
  }, [page, pageCount]);

  // Selection is page-scoped: with server paging the client only holds the current
  // page, so the master checkbox + delete act on the rows in view.
  const allSelected = records.length > 0 && records.every((r) => selected.has(r.buildId));
  const someSelected = selected.size > 0 && !allSelected;

  async function attemptRun() {
    setRunError(null);
    // Re-read from disk first: the profile can be edited in the separate Build
    // Settings window, so this window's cached copy may lag (e.g. a base dir set
    // after the list loaded). Running the stale copy would fail start_build's
    // validation even though the saved profile is valid.
    const fresh = await listProfiles().catch(() => realProfiles);
    setRealProfiles(fresh);
    const p = fresh.find((x) => x.id === profile);
    if (!p) {
      setRunError("That profile no longer exists. Pick another.");
      return;
    }
    try {
      const out = await checkOutput(p);
      if (out.exists) {
        setReplaceConfirm({ path: out.path });
        return;
      }
      await doRun(p);
    } catch (e) {
      setRunError(e instanceof IpcError ? e.message : String(e));
    }
  }

  async function doRun(p: BuildConfig) {
    setReplaceConfirm(null);
    setStarting(true);
    setRunError(null);
    try {
      await startBuild(p);
      await openBuildLogs();
    } catch (e) {
      setRunError(e instanceof IpcError ? e.message : String(e));
    } finally {
      setStarting(false);
    }
  }

  async function openLocation(buildId: string) {
    try {
      const r = await checkBuildLocation(buildId);
      if (!r.exists) return setLocWarn({ kind: "missing", path: r.path });
      if (r.changed) return setLocWarn({ kind: "changed", path: r.path });
      await openFolder(r.path);
    } catch (e) {
      console.error("open build folder failed", e);
    }
  }

  const toggleAll = () =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (allSelected) records.forEach((r) => next.delete(r.buildId));
      else records.forEach((r) => next.add(r.buildId));
      return next;
    });
  const toggle = (id: string) =>
    setSelected((s) => {
      const n = new Set(s);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });
  const deleteSelected = async () => {
    await deleteHistory([...selected]).catch(() => {});
    setSelected(new Set());
    setReloadKey((k) => k + 1);
  };

  // Changing a filter resets to the first page so you don't land on an empty page.
  const updateFilter = (patch: Partial<typeof filters>) => {
    setFilters((f) => ({ ...f, ...patch }));
    setPage(1);
  };

  const selectedProfile = realProfiles.find((x) => x.id === profile);

  return (
    <Box style={{ height: "calc(100dvh - var(--mantine-spacing-lg) * 2)", display: "flex", flexDirection: "column", minHeight: 0 }}>
      {/* run bar */}
      <Paper withBorder radius="md" p="md">
        <Group justify="space-between" wrap="nowrap">
          <Group wrap="nowrap" style={{ flex: 1, minWidth: 0 }}>
            <Text fz={13} c={tokens.text} fw={500}>
              Profile
            </Text>
            <Select
              w={404}
              value={profile}
              onChange={setProfile}
              allowDeselect={false}
              placeholder={realProfiles.length ? "Pick a profile" : "No profiles. Add one in Build Settings"}
              data={realProfiles.map((p) => ({
                value: p.id,
                label: `${p.name} · ${p.platform ?? "Win64"} · ${p.target ?? "auto-target"}`,
              }))}
            />
            <Tooltip label="Build Settings" withArrow openDelay={300}>
              <ActionIcon
                variant="default"
                size={36}
                aria-label="Build Settings"
                onClick={() => openAuxWindow("build-settings")}
                // Match the branded "secondary" (Shadow-Grey) fill the default Buttons use -
                // the theme's Button extension doesn't reach ActionIcon, so re-point its vars.
                style={{
                  "--ai-bg": "var(--uep-secondary)",
                  "--ai-hover": "var(--uep-secondary-hover)",
                  "--ai-color": "var(--uep-secondary-fg)",
                  "--ai-bd": "1px solid var(--uep-secondary-border)",
                } as CSSProperties}
              >
                <IconTool size={18} />
              </ActionIcon>
            </Tooltip>
          </Group>
          <Button
            size="md"
            leftSection={<IconPlayerPlayFilled size={16} />}
            disabled={!profile || starting}
            loading={starting}
            onClick={attemptRun}
          >
            Run
          </Button>
        </Group>
        {runError && (
          <Text fz={12.5} c={tokens.danger} mt={10}>
            {runError}
          </Text>
        )}
      </Paper>

      {/* history */}
      <Paper withBorder radius="md" p="md" mt="md" style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <Text fw={600} fz={14} c={tokens.ink}>
          Build history
        </Text>

        <Group gap={8} mt={12} wrap="nowrap">
          <Text fz={11} c={tokens.textDim}>
            Filter
          </Text>
          <FilterChip label="Platform" value={filters.platform} options={options.platform} onChange={(v) => updateFilter({ platform: v })} />
          <FilterChip label="Config" value={filters.config} options={options.config} onChange={(v) => updateFilter({ config: v })} />
          <FilterChip label="Target" value={filters.target} options={options.target} onChange={(v) => updateFilter({ target: v })} />
          <FilterChip label="Status" value={filters.status} options={options.status} onChange={(v) => updateFilter({ status: v })} />
        </Group>

        {/* selection toolbar - master checkbox aligned to the row-checkbox column
            (pl matches the table's horizontalSpacing); build counts share the row */}
        <Group justify="space-between" align="center" mt={14} mb={2} h={28} wrap="nowrap">
          <Group gap={12} pl="md" wrap="nowrap">
            <Checkbox size="xs" checked={allSelected} indeterminate={someSelected} onChange={toggleAll} aria-label="Select all" />
            {selected.size > 0 && (
              <Tooltip label={`Delete ${selected.size} selected`} withArrow>
                <ActionIcon variant="subtle" color="red" onClick={deleteSelected} aria-label="Delete selected">
                  <IconTrash size={16} />
                </ActionIcon>
              </Tooltip>
            )}
          </Group>
          <Text fz={11} c={tokens.textDim} pr="md">
            {filteredTotal} of {grandTotal} builds · {showLive ? 1 : 0} running · {selected.size} selected
          </Text>
        </Group>

        {/* Column header kept OUT of the scroll region so the scrollbar starts at the
            first row, not beside the titles. Both tables share HistoryCols (table-layout
            fixed) so columns line up; pr={10} here + scrollbar-gutter on the body reserve
            the same 10px, keeping the header aligned with the scrolling rows. */}
        <Box pr={10}>
          <Table verticalSpacing="sm" horizontalSpacing="md" layout="fixed">
            <HistoryCols />
            <Table.Thead>
              <Table.Tr>
                <Table.Th />
                <Th>BUILD TIME</Th>
                <Th>SIZE</Th>
                <Th>DURATION</Th>
                <Th>STATUS</Th>
                <Th>TAGS</Th>
                <Table.Th />
              </Table.Tr>
            </Table.Thead>
          </Table>
        </Box>

        <Box style={{ flex: 1, minHeight: 0, overflowY: "auto", overflowX: "hidden", scrollbarGutter: "stable" }}>
        <Table verticalSpacing="sm" horizontalSpacing="md" layout="fixed">
          <HistoryCols />
          <Table.Tbody>
            {showLive && live && currentPage === 1 && (
              <Table.Tr style={{ background: tokens.active }}>
                <Table.Td style={{ boxShadow: `inset 3px 0 0 0 ${tokens.accent}` }} />
                <Table.Td>
                  <Text fz={13}>{new Date(live.startedMs ?? now).toLocaleString()}</Text>
                </Table.Td>
                <Table.Td>
                  <Text fz={13} c={tokens.textDim}>
                    -
                  </Text>
                </Table.Td>
                <Table.Td>
                  <Text fz={13}>{fmtElapsed(now - (live.startedMs ?? now))}…</Text>
                </Table.Td>
                <Table.Td>
                  <LiveBadge />
                </Table.Td>
                <Table.Td>
                  <Group gap={6}>
                    <Tag>{live.platform}</Tag>
                    {live.configs.map((c) => (
                      <Tag key={c}>{c}</Tag>
                    ))}
                    <Tag>{live.target}</Tag>
                  </Group>
                </Table.Td>
                <Table.Td>
                  <Group justify="flex-end">
                    <Button size="compact-sm" onClick={() => openBuildLogs()}>
                      Detail
                    </Button>
                  </Group>
                </Table.Td>
              </Table.Tr>
            )}

            {records.map((b) => {
              const pills = b.tags.filter((t) => !isStatus(t));
              const status = statusOf(b.tags);
              return (
                <Table.Tr key={b.buildId} className={selected.has(b.buildId) ? "uep-row uep-row--selected" : "uep-row"}>
                  <Table.Td>
                    <Checkbox size="xs" checked={selected.has(b.buildId)} onChange={() => toggle(b.buildId)} />
                  </Table.Td>
                  <Table.Td>
                    <Text fz={13}>{new Date(b.startedAtMs ?? 0).toLocaleString()}</Text>
                  </Table.Td>
                  <Table.Td>
                    <Text fz={13} c={fmtSize(b.buildSize) === "-" ? tokens.textDim : undefined}>
                      {fmtSize(b.buildSize)}
                    </Text>
                  </Table.Td>
                  <Table.Td>
                    <Text fz={13}>{fmtSecs(b.duration)}</Text>
                  </Table.Td>
                  <Table.Td>
                    <StatusBadge kind={STATUS_KIND[status] ?? "cancelled"} />
                  </Table.Td>
                  <Table.Td>
                    <Group gap={6}>
                      {pills.map((t) => (
                        <Tag key={t}>{t}</Tag>
                      ))}
                    </Group>
                  </Table.Td>
                  <Table.Td>
                    <Group justify="flex-end" gap={6} wrap="nowrap">
                      {status === "Success" && !!b.outputPath && (
                        <Tooltip label="Open build folder" withArrow>
                          <ActionIcon variant="subtle" color="gray" onClick={() => openLocation(b.buildId)} aria-label="Open location">
                            <IconFolder size={16} />
                          </ActionIcon>
                        </Tooltip>
                      )}
                      <Button variant="default" size="compact-sm" onClick={() => openBuildLogs(b.buildId)}>
                        Detail
                      </Button>
                    </Group>
                  </Table.Td>
                </Table.Tr>
              );
            })}
          </Table.Tbody>
        </Table>
        </Box>

        {grandTotal === 0 && !showLive && (
          <Text fz={12.5} c={tokens.textDim} ta="center" py={28}>
            No builds yet. Pick a profile and hit Run. Finished builds appear here.
          </Text>
        )}

        {filteredTotal > 0 && (
          <Group justify="space-between" align="center" mt="md">
            <Group gap={8} wrap="nowrap" align="center">
              <NumberInput
                size="xs"
                w={76}
                value={pageSize}
                onChange={(v) => {
                  const n = typeof v === "number" ? v : Number(v);
                  if (Number.isFinite(n) && n >= 1) {
                    setPageSize(Math.floor(n));
                    setPage(1);
                  }
                }}
                min={1}
                max={1000}
                step={10}
                clampBehavior="blur"
                allowDecimal={false}
                allowNegative={false}
                aria-label="Builds per page"
              />
              <Text fz={12} c={tokens.textDim}>
                per page
              </Text>
            </Group>
            {pageCount > 1 && <Pagination size="sm" value={currentPage} onChange={setPage} total={pageCount} />}
          </Group>
        )}
      </Paper>

      {/* confirm-replace at Run */}
      <Modal opened={replaceConfirm != null} onClose={() => setReplaceConfirm(null)} title="Output folder already exists" centered>
        <Text fz={13} c={tokens.textMuted}>
          A build already exists at:
        </Text>
        <CodeBox mt={8}>{replaceConfirm?.path}</CodeBox>
        <Text fz={13} c={tokens.textMuted} mt={10}>
          Running will overwrite it. Continue?
        </Text>
        <Group justify="flex-end" gap={8} mt={18}>
          <Button variant="default" onClick={() => setReplaceConfirm(null)}>
            Cancel
          </Button>
          <Button style={dangerSolidButton()} onClick={() => selectedProfile && doRun(selectedProfile)}>
            Replace & run
          </Button>
        </Group>
      </Modal>

      {/* open-location integrity warning */}
      <Modal opened={locWarn != null} onClose={() => setLocWarn(null)} title={locWarn?.kind === "missing" ? "Build folder missing" : "Build folder changed"} centered>
        <Stack gap={10}>
          <Text fz={13} c={tokens.warnText}>
            {locWarn?.kind === "missing"
              ? "The output folder for this build no longer exists."
              : "The output folder no longer matches this build. A later build reused the same folder, so it may now contain different output."}
          </Text>
          <CodeBox>{locWarn?.path}</CodeBox>
          <Group justify="flex-end" gap={8} mt={6}>
            <Button variant="default" onClick={() => setLocWarn(null)}>
              Close
            </Button>
            {locWarn?.kind === "changed" && (
              <Button
                onClick={() => {
                  const p = locWarn.path;
                  setLocWarn(null);
                  void openFolder(p);
                }}
              >
                Open anyway
              </Button>
            )}
          </Group>
        </Stack>
      </Modal>
    </Box>
  );
}

function HistoryCols() {
  // Shared fixed column widths so the out-of-scroll header table and the scrolling body
  // table line up (table-layout: fixed reads these). TAGS has no width → takes the rest.
  return (
    <colgroup>
      <col style={{ width: 46 }} />
      <col style={{ width: 184 }} />
      <col style={{ width: 88 }} />
      <col style={{ width: 96 }} />
      <col style={{ width: 128 }} />
      <col />
      <col style={{ width: 132 }} />
    </colgroup>
  );
}

function FilterChip({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string | null;
  options: string[];
  onChange: (v: string | null) => void;
}) {
  return (
    <Menu position="bottom-start" withinPortal shadow="md">
      <Menu.Target>
        <Box
          className="uep-hoverable"
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 6,
            height: 24,
            padding: "0 10px",
            borderRadius: 6,
            background: value ? tokens.active : tokens.surfaceAlt,
            border: `1px solid ${tokens.border}`,
            fontSize: 11,
            color: tokens.text,
            cursor: "pointer",
            whiteSpace: "nowrap",
          }}
        >
          {label}: {value ?? "All"} ▾
        </Box>
      </Menu.Target>
      <Menu.Dropdown>
        <Menu.Item onClick={() => onChange(null)}>All</Menu.Item>
        {options.map((o) => (
          <Menu.Item key={o} onClick={() => onChange(o)}>
            {o}
          </Menu.Item>
        ))}
      </Menu.Dropdown>
    </Menu>
  );
}

function LiveBadge() {
  return (
    <Box
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 7,
        height: 22,
        padding: "0 10px",
        borderRadius: 11,
        background: tokens.accentSoft,
        border: `1px solid ${tokens.accentSoftBorder}`,
        fontSize: 11,
        fontWeight: 600,
        color: tokens.accentSoftText,
      }}
    >
      <Box style={{ width: 8, height: 8, borderRadius: "50%", background: tokens.accent, animation: "uep-blink 1s steps(2) infinite" }} />
      Live
    </Box>
  );
}
