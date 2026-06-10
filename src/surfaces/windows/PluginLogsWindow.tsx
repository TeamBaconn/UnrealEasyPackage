import { useEffect, useMemo, useRef, useState } from "react";
import { ActionIcon, Box, Button, CopyButton, Group, Modal, Paper, Stack, Text, TextInput, Tooltip } from "@mantine/core";
import { useClipboard } from "@mantine/hooks";
import { IconCheck, IconChevronRight, IconCopy, IconFolderOpen, IconPlayerStopFilled, IconSearch, IconTerminal2 } from "@tabler/icons-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { tokens } from "../../ui/tokens";
import { dangerSolidButton } from "../../ui/buttonStyles";
import { openFolder } from "../../ipc";
import {
  activeRun,
  cancelBuild,
  onRunFinished,
  onRunLog,
  onRunStarted,
  type LogLine,
  type RunSnapshot,
  type RunStatus,
  type Severity,
} from "../../runner";

// The shared single-command Run Log window: used by plugin packaging AND the project
// Tools (Resave / Validate). Same streaming console + warning/error filter as Build
// Logs, but NO pipeline graph (a single command, not a DAG) and NO history - so closing
// it discards the log for good. The heading comes from the run snapshot's `title`.
// Reuses the shared `uep://run-*` stream + `active_run`/`cancel_build`.

const LINE_CAP = 6000;
type Filter = "all" | "warning" | "error";

// In-order append with seq-dedup (backfill ∪ live events), tail-capped.
function mergeBySeq(prev: LogLine[], incoming: LogLine[]): LogLine[] {
  if (incoming.length === 0) return prev;
  const seen = new Set(prev.map((l) => l.seq));
  const out = prev.slice();
  for (const l of incoming) {
    if (!seen.has(l.seq)) {
      out.push(l);
      seen.add(l.seq);
    }
  }
  if (out.length === prev.length) return prev;
  return out.length > LINE_CAP ? out.slice(-LINE_CAP) : out;
}

function fmtDur(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}

function fmtClock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour12: false });
}

export function PluginLogsWindow() {
  const [snap, setSnap] = useState<RunSnapshot | null>(null);
  const [lines, setLines] = useState<LogLine[]>([]);
  const [status, setStatus] = useState<RunStatus | "idle">("idle");
  const [finalDur, setFinalDur] = useState<number | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [search, setSearch] = useState("");
  const [now, setNow] = useState(() => Date.now());
  const [confirmClose, setConfirmClose] = useState(false);

  const runIdRef = useRef<string | null>(null);
  // Mirror of the running state for the close handler - only a still-running process
  // warns on close (closing it would stop the process; a finished/failed one is free
  // to close).
  const runningRef = useRef(false);

  useEffect(() => {
    let alive = true;

    const adopt = (s: RunSnapshot) => {
      runIdRef.current = s.runId;
      setSnap(s);
      setStatus(s.status);
      setFinalDur(null);
      setLines(s.lines);
    };

    // Backfill the run that was started just before this window opened.
    activeRun()
      .then((s) => {
        if (alive && s) adopt(s);
      })
      .catch(() => {});

    const subs = [
      onRunStarted((s) => {
        if (!alive) return;
        adopt(s);
      }),
      onRunLog((b) => {
        if (b.runId !== runIdRef.current) return;
        setLines((prev) => mergeBySeq(prev, b.lines));
      }),
      onRunFinished((f) => {
        if (f.runId !== runIdRef.current) return;
        setStatus(f.status);
        setFinalDur(f.durationMs);
      }),
    ];
    return () => {
      alive = false;
      subs.forEach((p) => p.then((un) => un()).catch(() => {}));
    };
  }, []);

  // Keep the close-handler's view of "running" current.
  useEffect(() => {
    runningRef.current = status === "running";
  }, [status]);

  // Tick the elapsed clock while running.
  useEffect(() => {
    if (status !== "running") return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [status]);

  // Close guard: only a still-running process warns (closing stops it). A finished or
  // failed run closes straight away.
  useEffect(() => {
    const off = getCurrentWindow().onCloseRequested((event) => {
      if (!runningRef.current) return; // done/failed/cancelled -> nothing to stop
      event.preventDefault();
      setConfirmClose(true);
    });
    return () => {
      void off.then((f) => f());
    };
  }, []);

  const counts = useMemo(() => {
    let w = 0;
    let e = 0;
    for (const l of lines) {
      if (l.severity === "warning") w++;
      else if (l.severity === "error") e++;
    }
    return { all: lines.length, warning: w, error: e };
  }, [lines]);

  const shown = useMemo(() => {
    const bySev = filter === "all" ? lines : lines.filter((l) => l.severity === filter);
    const q = search.trim().toLowerCase();
    return q ? bySev.filter((l) => l.text.toLowerCase().includes(q)) : bySev;
  }, [lines, filter, search]);

  const running = status === "running";
  const elapsed = snap ? (finalDur ?? now - (snap.startedMs ?? now)) : 0;
  const dangerSolid = dangerSolidButton();

  return (
    <Box style={{ height: "100vh", overflow: "hidden", background: tokens.page, display: "flex", flexDirection: "column" }}>
      {/* top bar */}
      <Group
        h={64}
        px={20}
        justify="space-between"
        wrap="nowrap"
        className="uep-chrome"
        style={{ background: tokens.surface, borderBottom: `1px solid ${tokens.border}`, flexShrink: 0 }}
      >
        <Group gap={14} wrap="nowrap" style={{ minWidth: 0 }}>
          <Box style={{ color: tokens.ink, display: "grid", placeItems: "center" }}>
            <IconTerminal2 size={22} stroke={1.8} />
          </Box>
          <Box style={{ minWidth: 0 }}>
            <Group gap={10} wrap="nowrap">
              <Text fw={700} fz={18} c={tokens.ink}>
                {snap?.title || "Run Log"}
              </Text>
              {status !== "idle" && <RunBadge status={status} />}
            </Group>
            <Text fz={12} c={tokens.textMuted} truncate>
              {snap
                ? `${snap.project}${snap.startedMs ? ` · started ${fmtClock(snap.startedMs)}` : ""} · ${
                    running ? `${fmtDur(elapsed)} elapsed` : `${fmtDur(elapsed)} total`
                  }`
                : "No active run"}
            </Text>
          </Box>
        </Group>
        <Group gap={10} wrap="nowrap">
          {status === "success" && snap?.outputDir && (
            <Button variant="default" leftSection={<IconFolderOpen size={16} />} onClick={() => void openFolder(snap.outputDir)}>
              Open output folder
            </Button>
          )}
          {running && (
            <Button leftSection={<IconPlayerStopFilled size={14} />} style={dangerSolid} onClick={() => void cancelBuild()}>
              Cancel
            </Button>
          )}
        </Group>
      </Group>

      {/* body */}
      <Box style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", padding: 16, gap: 16 }}>
        {!snap ? (
          <EmptyState />
        ) : (
          <>
            <Console lines={shown} counts={counts} filter={filter} onFilter={setFilter} running={running} search={search} onSearch={setSearch} />
            <CommandIsland command={snap.command} />
          </>
        )}
      </Box>

      <Modal opened={confirmClose} onClose={() => setConfirmClose(false)} title="Discard this process?" centered size="md">
        <Stack gap="lg">
          <Text size="sm">
            If you discard, the running process will stop. To keep it running, keep this window open.
          </Text>
          <Group justify="flex-end" gap="sm">
            <Button variant="default" onClick={() => setConfirmClose(false)}>
              Keep open
            </Button>
            <Button
              style={dangerSolid}
              onClick={() => void cancelBuild().catch(() => {}).finally(() => void getCurrentWindow().destroy())}
            >
              Discard &amp; close
            </Button>
          </Group>
        </Stack>
      </Modal>
    </Box>
  );
}

function EmptyState() {
  return (
    <Box style={{ flex: 1, display: "grid", placeItems: "center" }}>
      <Stack gap={6} align="center" maw={520}>
        <IconTerminal2 size={30} stroke={1.6} color={tokens.textDim} />
        <Text fz={14} fw={600} c={tokens.text}>
          No run active
        </Text>
        <Text fz={12.5} c={tokens.textDim} ta="center">
          Start a tool from the project's Tools tab, or a package from the plugin's Tools tab. The streaming log appears here.
        </Text>
      </Stack>
    </Box>
  );
}

function RunBadge({ status }: { status: RunStatus | "idle" }) {
  const map: Record<RunStatus | "idle", { label: string; bg: string; border: string; fg: string; dot: string }> = {
    idle: { label: "IDLE", bg: tokens.cancelledBg, border: tokens.cancelledBorder, fg: tokens.textMuted, dot: tokens.textDim },
    running: { label: "LIVE", bg: tokens.accentSoft, border: tokens.accentSoftBorder, fg: tokens.accentSoftText, dot: tokens.accent },
    success: { label: "DONE", bg: tokens.successBg, border: tokens.successBorder, fg: tokens.successText, dot: tokens.success },
    failed: { label: "FAILED", bg: tokens.dangerBg, border: tokens.dangerBorder, fg: tokens.danger, dot: tokens.danger },
    cancelled: { label: "CANCELLED", bg: tokens.warnBg, border: tokens.warnBorder, fg: tokens.warn, dot: tokens.warn },
  };
  const s = map[status];
  return (
    <Box
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        height: 22,
        padding: "0 10px",
        borderRadius: 11,
        background: s.bg,
        border: `1px solid ${s.border}`,
        fontSize: 11,
        fontWeight: 700,
        color: s.fg,
      }}
    >
      <Box style={{ width: 8, height: 8, borderRadius: "50%", background: s.dot }} />
      {s.label}
    </Box>
  );
}

function Console({
  lines,
  counts,
  filter,
  onFilter,
  running,
  search,
  onSearch,
}: {
  lines: LogLine[];
  counts: { all: number; warning: number; error: number };
  filter: Filter;
  onFilter: (f: Filter) => void;
  running: boolean;
  search: string;
  onSearch: (s: string) => void;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const clipboard = useClipboard({ timeout: 1200 });

  // Pin to the bottom while `stick` is true. Observing the content box (not the
  // viewport) catches new lines AND late height changes as `content-visibility`
  // resolves real (wrapped) line heights - so a freshly-opened window lands at the
  // true bottom and follows live output instead of stranding mid-log.
  useEffect(() => {
    const v = viewport.current;
    const c = content.current;
    if (!v || !c) return;
    const pin = () => {
      if (stick.current) v.scrollTop = v.scrollHeight;
    };
    pin();
    const ro = new ResizeObserver(pin);
    ro.observe(c);
    return () => ro.disconnect();
  }, []);

  const onScroll = () => {
    const v = viewport.current;
    if (!v) return;
    stick.current = v.scrollHeight - v.scrollTop - v.clientHeight < 60;
  };

  const sevTint: Record<Severity, string | undefined> = { info: undefined, warning: tokens.logWarnBg, error: tokens.logErrorBg };
  const sevFg: Record<Severity, string> = { info: tokens.textMuted, warning: tokens.warn, error: tokens.danger };

  return (
    <Paper withBorder radius="md" style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <Group justify="space-between" px="md" py={12} wrap="nowrap" gap={8} style={{ borderBottom: `1px solid ${tokens.divider}` }}>
        <Group gap={6} wrap="nowrap" style={{ flexShrink: 0 }}>
          <Text fw={600} fz={14} c={tokens.ink}>
            Console
          </Text>
          <Tooltip label={clipboard.copied ? "Copied" : "Copy log"} withArrow openDelay={300}>
            <ActionIcon
              size="sm"
              variant="subtle"
              color="gray"
              onClick={() => clipboard.copy(lines.map((l) => l.text).join("\n"))}
              disabled={lines.length === 0}
              aria-label="Copy log"
            >
              {clipboard.copied ? <IconCheck size={15} /> : <IconCopy size={15} />}
            </ActionIcon>
          </Tooltip>
        </Group>
        <Group gap={8} wrap="nowrap" style={{ minWidth: 0 }}>
          <TextInput
            value={search}
            onChange={(e) => onSearch(e.currentTarget.value)}
            placeholder="Search log…"
            leftSection={<IconSearch size={14} />}
            size="xs"
            style={{ width: 200 }}
            aria-label="Search log"
          />
          <Group gap={0} wrap="nowrap" style={{ border: `1px solid ${tokens.border}`, borderRadius: 6, overflow: "hidden", flexShrink: 0 }}>
            <FilterTab active={filter === "all"} onClick={() => onFilter("all")} label={`All · ${counts.all.toLocaleString()}`} />
            <FilterTab active={filter === "warning"} onClick={() => onFilter("warning")} label={`Warnings · ${counts.warning}`} dot={tokens.warn} />
            <FilterTab active={filter === "error"} onClick={() => onFilter("error")} label={`Errors · ${counts.error}`} dot={tokens.danger} />
          </Group>
        </Group>
      </Group>

      <Box ref={viewport} onScroll={onScroll} style={{ flex: 1, minHeight: 0, overflow: "auto", background: tokens.surfaceAlt }}>
        <Box ref={content} className="uep-selectable" style={{ padding: "8px 0", fontFamily: "var(--mantine-font-family-monospace)" }}>
          {lines.length === 0 ? (
            <Text fz={11.5} c={tokens.textDim} px="md" py={8}>
              {search.trim() ? "No lines match your search." : running ? "Waiting for output…" : "No lines for this filter."}
            </Text>
          ) : (
            lines.map((l) => (
              <Box key={l.seq} style={{ display: "flex", gap: 12, padding: "1px 16px", background: sevTint[l.severity], contentVisibility: "auto", containIntrinsicSize: "0 18px" }}>
                <Text component="span" fz={11} c={tokens.textDim} style={{ minWidth: 44, textAlign: "right", userSelect: "none", flexShrink: 0 }}>
                  {l.seq}
                </Text>
                <Text component="span" fz={11.5} c={sevFg[l.severity]} style={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
                  {l.text}
                </Text>
              </Box>
            ))
          )}
          {running && (
            <Box style={{ padding: "1px 16px 6px 72px" }}>
              <Box style={{ display: "inline-block", width: 8, height: 14, background: tokens.textMuted, animation: "uep-blink 1s steps(2) infinite", verticalAlign: "middle" }} />
            </Box>
          )}
        </Box>
      </Box>
    </Paper>
  );
}

// The resolved `RunUAT BuildPlugin …` command (same layout as Build Logs' command island).
function CommandIsland({ command }: { command: string }) {
  if (!command) return null;
  return (
    <Paper withBorder radius="md" p="md" style={{ flexShrink: 0 }}>
      <Group gap={8} wrap="nowrap" mb={10}>
        <IconChevronRight size={16} color={tokens.textMuted} />
        <Text fw={600} fz={14} c={tokens.ink}>
          Command
        </Text>
        <CopyButton value={command}>
          {({ copied, copy }) => (
            <Tooltip label={copied ? "Copied" : "Copy command"} withArrow openDelay={300}>
              <ActionIcon size="sm" variant="subtle" color="gray" onClick={copy} aria-label="Copy command">
                {copied ? <IconCheck size={15} /> : <IconCopy size={15} />}
              </ActionIcon>
            </Tooltip>
          )}
        </CopyButton>
      </Group>
      <Box
        className="uep-selectable"
        style={{
          background: tokens.surfaceAlt,
          border: `1px solid ${tokens.divider}`,
          borderRadius: 6,
          padding: "10px 14px",
          fontFamily: "var(--mantine-font-family-monospace)",
          fontSize: 11.5,
          color: tokens.textMuted,
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          minHeight: 36,
        }}
      >
        {command}
      </Box>
    </Paper>
  );
}

function FilterTab({ active, onClick, label, dot }: { active: boolean; onClick: () => void; label: string; dot?: string }) {
  return (
    <Box
      onClick={onClick}
      className={active ? undefined : "uep-hoverable"}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 7,
        padding: "5px 14px",
        cursor: "pointer",
        background: active ? tokens.accent : tokens.surface,
        color: active ? tokens.onAccent : tokens.text,
        fontSize: 12,
        fontWeight: active ? 600 : 500,
      }}
    >
      {dot && <Box style={{ width: 7, height: 7, borderRadius: "50%", background: dot }} />}
      {label}
    </Box>
  );
}
