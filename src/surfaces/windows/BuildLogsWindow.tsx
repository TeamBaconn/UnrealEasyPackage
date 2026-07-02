import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { ActionIcon, Box, Button, CopyButton, Group, Paper, Stack, Text, TextInput, Tooltip } from "@mantine/core";
import { useClipboard } from "@mantine/hooks";
import { IconChevronRight, IconCopy, IconCheck, IconPlayerStopFilled, IconSearch, IconTerminal2 } from "@tabler/icons-react";
import { listen } from "@tauri-apps/api/event";
import { tokens } from "../../ui/tokens";
import { dangerSolidButton } from "../../ui/buttonStyles";
import { initialBuildParam } from "../../ui/windows";
import { historyDetail, type HistoryDetail, type PhaseId, type PhaseTiming } from "../../ipc";
import {
  activeRun,
  cancelBuild,
  onRunFinished,
  onRunLog,
  onRunPhase,
  onRunStarted,
  type LogLine,
  type PhaseNode,
  type PhaseStatus,
  type RunSnapshot,
  type RunStatus,
  type Severity,
} from "../../runner";

// Status palette for the pipeline graph nodes (done/green, running/orange-accent,
// pending/grey, failed/red, cancelled, skipped). Built in-render (see `nodeStyles`
// in GraphNode and the severity maps in Console) so they follow the scheme swap -
// module-level consts would freeze at the light values and never flip to dark.
function nodeStyles(): Record<PhaseStatus, { bg: string; border: string; fg: string; sub: string }> {
  return {
    success: { bg: tokens.successSolidBg, border: tokens.successSolidBorder, fg: tokens.successSolidFg, sub: tokens.successSolidSub },
    running: { bg: tokens.accent, border: tokens.runningBorder, fg: tokens.onAccent, sub: tokens.runningSub },
    pending: { bg: tokens.surfaceAlt, border: tokens.border, fg: tokens.textMuted, sub: tokens.textDim },
    failed: { bg: tokens.dangerSolidBg, border: tokens.dangerSolidBorder, fg: tokens.dangerSolidFg, sub: tokens.dangerSolidSub },
    cancelled: { bg: tokens.surfaceAlt, border: tokens.border, fg: tokens.textMuted, sub: tokens.textDim },
    skipped: { bg: tokens.surfaceAlt, border: tokens.border, fg: tokens.textDim, sub: tokens.textDim },
  };
}

function fmtDur(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}

function fmtClock(ms: number): string {
  const d = new Date(ms);
  return d.toLocaleTimeString(undefined, { hour12: false });
}

// Keep the live console bounded; the backend streams every line, a long build can
// emit a lot.
const LINE_CAP = 6000;

// Merge streamed batches with the backfilled snapshot by unique, increasing `seq`
// - dedups the overlap between `active_run`'s buffer and the live events (so no
// duplicate React keys), tolerates out-of-order arrival, and tail-caps.
function mergeBySeq(prev: LogLine[], incoming: LogLine[]): LogLine[] {
  if (incoming.length === 0) return prev;
  if (prev.length === 0) return incoming.slice(-LINE_CAP);
  const seen = new Set<number>();
  for (const l of prev) seen.add(l.seq);
  const last = prev[prev.length - 1].seq;
  let outOfOrder = false;
  const out = prev.slice();
  for (const l of incoming) {
    if (seen.has(l.seq)) continue;
    if (l.seq < last) outOfOrder = true;
    out.push(l);
    seen.add(l.seq);
  }
  if (out.length === prev.length) return prev;
  if (outOfOrder) out.sort((a, b) => a.seq - b.seq);
  return out.length > LINE_CAP ? out.slice(-LINE_CAP) : out;
}

// ── past-build replay: synthesize a live-shaped snapshot from a saved record ──
const PLAT = ["Win64", "Linux", "Mac"];
const CONF = ["Debug", "DebugGame", "Development", "Test", "Shipping"];
const STAT = ["Success", "Failed", "Cancelled"];

function reverseTags(tags: string[]) {
  return {
    platform: tags.find((t) => PLAT.includes(t)),
    configs: tags.filter((t) => CONF.includes(t)),
    status: tags.find((t) => STAT.includes(t)),
    target: tags.find((t) => !PLAT.includes(t) && !CONF.includes(t) && !STAT.includes(t)),
  };
}
function statusToRun(s?: string): RunStatus {
  return s === "Failed" ? "failed" : s === "Cancelled" ? "cancelled" : "success";
}
function statusFromString(s: string): PhaseStatus {
  switch (s) {
    case "Success":
      return "success";
    case "Failed":
      return "failed";
    case "Cancelled":
      return "cancelled";
    case "Skipped":
      return "skipped";
    default:
      return "pending";
  }
}
function phaseIdFromLabel(l: string): PhaseId {
  // Steam Login is the preflight, Upload to Steam the post-archive tail - match them
  // before Build so neither falls through to the "build" catch-all.
  if (/steam login/i.test(l)) return "steamLogin";
  if (/upload to steam/i.test(l)) return "steamUpload";
  // Match Build first: multi-config emits "Build (DebugGame)", "Build (Shipping)", etc.
  if (/^build/i.test(l)) return "build";
  if (/cook/i.test(l)) return "cook";
  if (/stage|pak|archive/i.test(l)) return "stage";
  if (/copy/i.test(l)) return "copyExtras";
  if (/clean/i.test(l)) return "cleanup";
  return "build";
}
// Reconstruct the graph column from the phase label (the live plan's levels).
function levelFromLabel(l: string): number {
  // Steam Login preflight is the very first column (before the editor build).
  if (/steam login/i.test(l)) return -1;
  if (/editor/i.test(l)) return 0;
  // All game builds ("Build", "Build (DebugGame)", …) share the build∥cook column.
  if (/cook/i.test(l) || (/^build/i.test(l) && !/editor/i.test(l))) return 1;
  if (/stage|pak|archive/i.test(l)) return 2;
  if (/copy/i.test(l)) return 3;
  // Upload to Steam is the tail stage after Copy Extras, before Clean-up.
  if (/upload to steam/i.test(l)) return 4;
  if (/clean/i.test(l)) return 5;
  return 1;
}
function timingToNode(t: PhaseTiming, index: number): PhaseNode {
  return {
    index,
    label: t.phase,
    phase: phaseIdFromLabel(t.phase),
    // Persisted kind/command (new records); fall back to label inference for older
    // ones that predate those fields.
    kind: t.kind === "app" ? "app" : t.kind === "external" ? "external" : /copy|clean/i.test(t.phase) ? "app" : "external",
    level: levelFromLabel(t.phase),
    command: t.command ?? "",
    status: statusFromString(t.status),
    startOffsetMs: (t.startOffset ?? 0) * 1000,
    durationMs: (t.duration ?? 0) * 1000,
  };
}
function recordToSnap(detail: HistoryDetail): RunSnapshot {
  const r = detail.record;
  const rev = reverseTags(r.tags);
  return {
    runId: r.buildId,
    project: "",
    platform: rev.platform ?? "",
    configs: rev.configs,
    target: rev.target ?? "",
    outputDir: r.outputPath,
    startedMs: r.startedAtMs,
    status: statusToRun(rev.status),
    phases: r.phases.map((t, i) => timingToNode(t, i)),
    lines: detail.lines,
    command: "", // build replay shows per-phase commands on the graph nodes
    title: "", // the Build Logs window has its own heading
  };
}

type Filter = "all" | "warning" | "error";

export function BuildLogsWindow() {
  const [snap, setSnap] = useState<RunSnapshot | null>(null);
  const [lines, setLines] = useState<LogLine[]>([]);
  const [phases, setPhases] = useState<PhaseNode[]>([]);
  const [status, setStatus] = useState<RunStatus>("running");
  const [finalDur, setFinalDur] = useState<number | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<number | null>(null);
  const [now, setNow] = useState<number>(() => Date.now());
  const [missing, setMissing] = useState(false); // a past build was opened but its record is gone
  const [loading, setLoading] = useState(true); // initial backfill in flight - suppress chrome to avoid a default-UI flash

  const runId = snap?.runId ?? null;
  const runIdRef = useRef<string | null>(null);
  runIdRef.current = runId;
  const modeRef = useRef<"live" | "past">("live");

  // ── live run (backfill + events) or past-build replay (?build= / show-build) ─
  useEffect(() => {
    let alive = true;

    // `reset` = a brand-new run (seq restarts at 1) ⇒ replace; else merge.
    const adoptLive = (s: RunSnapshot, reset: boolean) => {
      if (!alive) return;
      modeRef.current = "live";
      runIdRef.current = s.runId;
      setSnap(s);
      setPhases(s.phases);
      setStatus(s.status);
      setFinalDur(null);
      if (reset) {
        setLines(s.lines);
        setSelected(firstActive(s.phases));
      } else {
        setLines((prev) => mergeBySeq(prev, s.lines));
        setSelected((cur) => cur ?? firstActive(s.phases));
      }
    };

    const goLive = () => {
      modeRef.current = "live";
      setMissing(false);
      setFinalDur(null);
      activeRun()
        .then((s) => {
          if (!alive) return;
          if (s) adoptLive(s, true);
          else {
            setSnap(null);
            setLines([]);
            setPhases([]);
          }
        })
        .catch(() => {})
        .finally(() => alive && setLoading(false));
    };

    const goPast = (id: string) => {
      modeRef.current = "past";
      setMissing(false);
      historyDetail(id)
        .then((d) => {
          if (!alive) return;
          if (!d) {
            // The record was deleted while the list still showed it. Don't fall back
            // to the live-run chrome ("LIVE" badge + Cancel build) - say it's gone.
            setMissing(true);
            setSnap(null);
            setLines([]);
            setPhases([]);
            setStatus("cancelled"); // anything but "running" so the timer/Cancel stop
            setFinalDur(null);
            return;
          }
          const s = recordToSnap(d);
          runIdRef.current = s.runId;
          setSnap(s);
          setPhases(s.phases);
          setStatus(s.status);
          setFinalDur((d.record.duration ?? 0) * 1000);
          setLines(s.lines);
          setSelected(firstActive(s.phases));
        })
        .catch(() => {})
        .finally(() => alive && setLoading(false));
    };

    const initial = initialBuildParam();
    if (initial) goPast(initial);
    else goLive();

    const unlisteners = [
      listen<{ buildId: string | null }>("uep://show-build", (e) => {
        const id = e.payload?.buildId ?? null;
        if (id) goPast(id);
        else goLive();
      }),
      onRunStarted((s) => {
        if (modeRef.current === "live") adoptLive(s, true);
      }),
      onRunLog((b) => {
        if (modeRef.current !== "live" || b.runId !== runIdRef.current) return;
        setLines((prev) => mergeBySeq(prev, b.lines));
      }),
      onRunPhase((u) => {
        if (modeRef.current !== "live" || u.runId !== runIdRef.current) return;
        setPhases((prev) => prev.map((p) => (p.index === u.phase.index ? u.phase : p)));
      }),
      onRunFinished((f) => {
        if (modeRef.current !== "live" || f.runId !== runIdRef.current) return;
        setStatus(f.status);
        setFinalDur(f.durationMs);
      }),
    ];
    return () => {
      alive = false;
      unlisteners.forEach((p) => p.then((un) => un()).catch(() => {}));
    };
  }, []);

  // tick the elapsed clock while running
  useEffect(() => {
    if (status !== "running") return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [status]);

  // Lines carry a per-phase index - live, and persisted for replay via build.idx -
  // so the console follows the selected node. Older records have no attribution
  // (every line phase 0); detect that and show the whole log rather than filter it
  // to nothing.
  const hasPhaseAttribution = useMemo(() => lines.some((l) => l.phaseIndex !== 0), [lines]);
  const phaseLines = useMemo(
    () => (!hasPhaseAttribution || selected == null ? lines : lines.filter((l) => l.phaseIndex === selected)),
    [lines, selected, hasPhaseAttribution]
  );
  const counts = useMemo(() => {
    let w = 0;
    let e = 0;
    for (const l of phaseLines) {
      if (l.severity === "warning") w++;
      else if (l.severity === "error") e++;
    }
    return { all: phaseLines.length, warning: w, error: e };
  }, [phaseLines]);

  const shown = useMemo(() => {
    const bySev = filter === "all" ? phaseLines : phaseLines.filter((l) => l.severity === filter);
    const q = search.trim().toLowerCase();
    return q ? bySev.filter((l) => l.text.toLowerCase().includes(q)) : bySev;
  }, [phaseLines, filter, search]);

  const elapsed = snap ? (finalDur ?? now - (snap.startedMs ?? now)) : 0;
  const running = status === "running";
  const selectedNode = phases.find((p) => p.index === selected) ?? null;
  // Solid "Stage ✕ failed" red for the destructive Cancel button (matches the Clean tab's Delete).
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
                Build Logs
              </Text>
              {!loading && !missing && <RunBadge status={status} />}
            </Group>
            <Text fz={12} c={tokens.textMuted} truncate>
              {loading
                ? ""
                : missing
                ? "This build is no longer in history"
                : snap
                ? [snap.project, snap.configs.join(", "), snap.platform, snap.target].filter(Boolean).join(" · ") +
                  ` · started ${fmtClock(snap.startedMs ?? Date.now())}`
                : "No active build"}
            </Text>
          </Box>
        </Group>
        <Group gap={10} wrap="nowrap">
          {!loading && running && (
            <Button
              leftSection={<IconPlayerStopFilled size={14} />}
              style={dangerSolid}
              onClick={() => void cancelBuild()}
            >
              Cancel build
            </Button>
          )}
        </Group>
      </Group>

      {/* body */}
      <Box style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", padding: 16, gap: 16 }}>
        {loading ? null : missing ? (
          <MissingState />
        ) : !snap ? (
          <EmptyState />
        ) : (
          <>
            <PipelineGraph
              phases={phases}
              elapsed={elapsed}
              running={running}
              selected={selected}
              onSelect={setSelected}
            />
            <Console
              lines={shown}
              counts={counts}
              filter={filter}
              onFilter={setFilter}
              running={running}
              search={search}
              onSearch={setSearch}
            />
            <CommandIsland node={selectedNode} />
          </>
        )}
      </Box>
    </Box>
  );
}

function firstActive(phases: PhaseNode[]): number | null {
  const running = phases.find((p) => p.status === "running");
  if (running) return running.index;
  return phases[0]?.index ?? null;
}

function RunBadge({ status }: { status: RunStatus }) {
  const map: Record<RunStatus, { label: string; bg: string; border: string; fg: string; dot: string }> = {
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

function EmptyState() {
  return (
    <Box style={{ flex: 1, display: "grid", placeItems: "center" }}>
      <Stack gap={6} align="center" maw={520}>
        <IconTerminal2 size={30} stroke={1.6} color={tokens.textDim} />
        <Text fz={14} fw={600} c={tokens.text}>
          No build running
        </Text>
        <Text fz={12.5} c={tokens.textDim} ta="center">
          Launch a profile from the Build tab. The live pipeline graph and streaming console appear here. Reading a
          past build's saved log arrives with build history (M4).
        </Text>
      </Stack>
    </Box>
  );
}

function MissingState() {
  return (
    <Box style={{ flex: 1, display: "grid", placeItems: "center" }}>
      <Stack gap={6} align="center" maw={520}>
        <IconTerminal2 size={30} stroke={1.6} color={tokens.textDim} />
        <Text fz={14} fw={600} c={tokens.text}>
          Build no longer exists
        </Text>
        <Text fz={12.5} c={tokens.textDim} ta="center">
          This build's record was deleted. There's nothing to replay.
        </Text>
      </Stack>
    </Box>
  );
}

// ── pipeline graph ────────────────────────────────────────────────────────────
// Fixed node geometry so the SVG connectors line up with the HTML nodes without
// measuring: every node is NODE_H tall with NODE_GAP between stacked siblings, and
// each column is vertically centred within the tallest column's height.
const NODE_W = 196;
const NODE_H = 48;
const NODE_GAP = 12;
const CONN_W = 44;

function PipelineGraph({
  phases,
  elapsed,
  running,
  selected,
  onSelect,
}: {
  phases: PhaseNode[];
  elapsed: number;
  running: boolean;
  selected: number | null;
  onSelect: (i: number) => void;
}) {
  // columns by scheduling level (parallel siblings - e.g. Build ∥ Cook - share one)
  const columns = useMemo(() => {
    const byLevel = new Map<number, PhaseNode[]>();
    for (const p of phases) {
      const arr = byLevel.get(p.level) ?? [];
      arr.push(p);
      byLevel.set(p.level, arr);
    }
    return [...byLevel.entries()].sort((a, b) => a[0] - b[0]).map(([, ps]) => ps);
  }, [phases]);

  const maxNodes = Math.max(1, ...columns.map((c) => c.length));
  const graphH = maxNodes * NODE_H + (maxNodes - 1) * NODE_GAP;
  // Vertically-centred node centres for a column of n nodes, in graph coordinates.
  const centersFor = (n: number): number[] => {
    const top = (graphH - (n * NODE_H + (n - 1) * NODE_GAP)) / 2;
    return Array.from({ length: n }, (_, i) => top + i * (NODE_H + NODE_GAP) + NODE_H / 2);
  };

  return (
    <Paper withBorder radius="md" p="md" style={{ flexShrink: 0 }}>
      <Group justify="space-between" mb={14} wrap="nowrap">
        <Group gap={12} wrap="nowrap">
          <Text fw={600} fz={14} c={tokens.ink}>
            Pipeline
          </Text>
          <Text fz={13} c={tokens.textMuted}>
            {running ? `Running · ${fmtDur(elapsed)} elapsed` : `${fmtDur(elapsed)} total`}
          </Text>
        </Group>
        <Group gap={16} wrap="nowrap">
          <Legend color={tokens.success} label="done" />
          <Legend color={tokens.accent} label="running" />
          <Legend color={tokens.border} label="pending" outline />
          <Legend color={tokens.dangerSolidBg} label="failed" />
        </Group>
      </Group>

      {/* horizontal scroll for a wide graph; vertical padding leaves room for the
          selection outline so it isn't clipped at the panel edge */}
      <Box style={{ overflowX: "auto", overflowY: "hidden", paddingBottom: 4 }}>
        <Group gap={0} align="center" wrap="nowrap" style={{ width: "max-content", padding: "8px 6px" }}>
          <StartDot graphH={graphH} />
          {columns.map((col, ci) => (
            <Fragment key={ci}>
              <Stack gap={NODE_GAP} style={{ height: graphH, justifyContent: "center", flexShrink: 0 }}>
                {col.map((p) => (
                  <GraphNode key={p.index} node={p} selected={p.index === selected} onSelect={() => onSelect(p.index)} />
                ))}
              </Stack>
              {ci < columns.length - 1 && (
                <Connector graphH={graphH} left={centersFor(col.length)} right={centersFor(columns[ci + 1].length)} />
              )}
            </Fragment>
          ))}
        </Group>
      </Box>
    </Paper>
  );
}

// Entry dot + orthogonal fork/merge connectors as SVG, lined up with the fixed node
// geometry: a vertical bus at mid-gap with a
// horizontal stub into each node - straight for 1→1, a fork for 1→N, a merge for N→1.
function StartDot({ graphH }: { graphH: number }) {
  const cy = graphH / 2;
  const w = 16;
  return (
    <svg width={w} height={graphH} style={{ flexShrink: 0, display: "block" }} aria-hidden>
      <circle cx={5} cy={cy} r={4.5} fill={tokens.textDim} />
      <path d={`M9 ${cy} H${w}`} stroke={tokens.borderStrong} strokeWidth={1.6} fill="none" />
    </svg>
  );
}

function Connector({ graphH, left, right }: { graphH: number; left: number[]; right: number[] }) {
  const midX = CONN_W / 2;
  const all = [...left, ...right];
  const top = Math.min(...all);
  const bottom = Math.max(...all);
  return (
    <svg width={CONN_W} height={graphH} style={{ flexShrink: 0, display: "block" }} aria-hidden>
      <g stroke={tokens.borderStrong} strokeWidth={1.6} fill="none">
        {bottom - top > 0.5 && <path d={`M${midX} ${top} V${bottom}`} />}
        {left.map((y, i) => (
          <path key={`l${i}`} d={`M0 ${y} H${midX}`} />
        ))}
        {right.map((y, j) => (
          <path key={`r${j}`} d={`M${midX} ${y} H${CONN_W}`} />
        ))}
      </g>
    </svg>
  );
}

function Legend({ color, label, outline }: { color: string; label: string; outline?: boolean }) {
  return (
    <Group gap={6} wrap="nowrap">
      <Box
        style={{
          width: 11,
          height: 11,
          borderRadius: 2,
          background: outline ? tokens.surfaceAlt : color,
          border: outline ? `1px solid ${color}` : "none",
        }}
      />
      <Text fz={11} c={tokens.textDim}>
        {label}
      </Text>
    </Group>
  );
}

function GraphNode({ node, selected, onSelect }: { node: PhaseNode; selected: boolean; onSelect: () => void }) {
  const s = nodeStyles()[node.status];
  const detail =
    node.status === "success"
      ? `done · ${fmtDur(node.durationMs ?? 0)}`
      : node.status === "running"
        ? "running…"
        : node.status === "failed"
          ? node.durationMs != null
            ? `failed · ${fmtDur(node.durationMs)}`
            : "failed"
          : node.status;
  return (
    <Box
      onClick={onSelect}
      className="uep-hoverable"
      style={{
        position: "relative",
        width: NODE_W,
        height: NODE_H,
        display: "flex",
        flexDirection: "column",
        justifyContent: "center",
        padding: "0 14px",
        borderRadius: 9,
        background: s.bg,
        border: `1px solid ${s.border}`,
        outline: selected ? `2px solid ${tokens.accent}` : "none",
        outlineOffset: 2,
        cursor: "pointer",
        flexShrink: 0,
      }}
    >
      <Text fz={12} fw={600} c={s.fg} truncate>
        {node.label}
      </Text>
      <Text fz={10.5} c={s.sub} truncate>
        {detail}
      </Text>
    </Box>
  );
}

// ── console ───────────────────────────────────────────────────────────────────
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

  // Severity row tint + text color, read per-render so they follow the scheme swap.
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
            <FilterTab
              active={filter === "warning"}
              onClick={() => onFilter("warning")}
              label={`Warnings · ${counts.warning}`}
              dot={tokens.warn}
            />
            <FilterTab
              active={filter === "error"}
              onClick={() => onFilter("error")}
              label={`Errors · ${counts.error}`}
              dot={tokens.danger}
            />
          </Group>
        </Group>
      </Group>

      <Box
        ref={viewport}
        onScroll={onScroll}
        style={{ flex: 1, minHeight: 0, overflow: "auto", background: tokens.surfaceAlt }}
      >
        <Box ref={content} className="uep-selectable" style={{ padding: "8px 0", fontFamily: "var(--mantine-font-family-monospace)" }}>
          {lines.length === 0 ? (
            <Text fz={11.5} c={tokens.textDim} px="md" py={8}>
              {search.trim() ? "No lines match your search." : running ? "Waiting for output…" : "No lines for this filter."}
            </Text>
          ) : (
            lines.map((l) => (
              <Box
                key={l.seq}
                style={{
                  display: "flex",
                  gap: 12,
                  padding: "1px 16px",
                  background: sevTint[l.severity],
                  contentVisibility: "auto",
                  containIntrinsicSize: "0 18px",
                }}
              >
                <Text
                  component="span"
                  fz={11}
                  c={tokens.textDim}
                  style={{ minWidth: 44, textAlign: "right", userSelect: "none", flexShrink: 0 }}
                >
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
              <Box
                style={{
                  display: "inline-block",
                  width: 8,
                  height: 14,
                  background: tokens.textMuted,
                  animation: "uep-blink 1s steps(2) infinite",
                  verticalAlign: "middle",
                }}
              />
            </Box>
          )}
        </Box>
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

// ── command island ────────────────────────────────────────────────────────────
function CommandIsland({ node }: { node: PhaseNode | null }) {
  const isApp = node?.kind === "app";
  const command = node?.command ?? "";
  return (
    <Paper withBorder radius="md" p="md" style={{ flexShrink: 0 }}>
      <Group gap={8} wrap="nowrap" mb={10}>
        <IconChevronRight size={16} color={tokens.textMuted} />
        <Text fw={600} fz={14} c={tokens.ink}>
          Command
        </Text>
        {!isApp && command && (
          <CopyButton value={command}>
            {({ copied, copy }) => (
              <Tooltip label={copied ? "Copied" : "Copy command"} withArrow openDelay={300}>
                <ActionIcon size="sm" variant="subtle" color="gray" onClick={copy} aria-label="Copy command">
                  {copied ? <IconCheck size={15} /> : <IconCopy size={15} />}
                </ActionIcon>
              </Tooltip>
            )}
          </CopyButton>
        )}
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
        {!node
          ? "Select a pipeline node to see its command."
          : isApp
            ? "App-owned task. Runs in-process (no external command)."
            : command}
      </Box>
    </Paper>
  );
}
