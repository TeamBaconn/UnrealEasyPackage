import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { Box, Group, Paper, Select, SimpleGrid, Text } from "@mantine/core";
import { tokens } from "../../../ui/tokens";
import { formatBytes as fmtSize, bytesToGiB } from "../../../ui/format";
import { listHistory, type BuildRecord } from "../../../ipc";
import { onRunFinished } from "../../../runner";
import { LineChart, type LineSeries, type XPoint } from "../charts";

// Tag vocabularies - mirror src-tauri/src/history/tags.rs (platform/config/status
// are closed sets; the leftover tag is the target). A build's cohort = platform ·
// config · target; trends only compare like with like (a line mixing Development
// and Shipping would be noise).
const PLATFORMS = ["Win64", "Linux", "Mac"];
const CONFIGS = ["Debug", "DebugGame", "Development", "Test", "Shipping"];
const STATUSES = ["Success", "Failed", "Cancelled"];
const find = (tags: string[], vocab: string[]) => tags.find((t) => vocab.includes(t));
const platformOf = (t: string[]) => find(t, PLATFORMS) ?? "-";
const configOf = (t: string[]) => find(t, CONFIGS) ?? "-";
// A multi-config build tags every staged config (tags.rs), so a build can carry
// more than one config; `configsOf` returns them all (`configOf` keeps the first
// for the default cohort selection).
const configsOf = (t: string[]) => t.filter((x) => CONFIGS.includes(x));
const statusOf = (t: string[]) => find(t, STATUSES) ?? "Success";
const targetOf = (t: string[]) =>
  t.find((x) => !PLATFORMS.includes(x) && !CONFIGS.includes(x) && !STATUSES.includes(x)) ?? "-";

interface Cohort {
  platform: string;
  config: string;
  target: string;
}
const cohortOf = (r: BuildRecord): Cohort => ({
  platform: platformOf(r.tags),
  config: configOf(r.tags),
  target: targetOf(r.tags),
});
// A build joins a cohort when platform + target match and ANY of its config tags is
// the cohort's config - so a multi-config build (e.g. Development AND Shipping)
// appears in every one of those config cohorts, not just its first.
const matchesCohort = (r: BuildRecord, c: Cohort) =>
  platformOf(r.tags) === c.platform &&
  targetOf(r.tags) === c.target &&
  configsOf(r.tags).includes(c.config);

function fmtSecs(s: number): string {
  const x = Math.round(s);
  return x < 60 ? `${x}s` : `${Math.floor(x / 60)}m ${String(x % 60).padStart(2, "0")}s`;
}
function fmtDate(ms: number | null | undefined): string {
  return new Date(ms ?? 0).toLocaleDateString();
}
const xPoint = (ms: number | null | undefined): XPoint => {
  const d = new Date(ms ?? 0);
  return { tick: d.toLocaleDateString(undefined, { month: "short", day: "numeric" }), full: d.toLocaleString() };
};
type DeltaTone = "up" | "down" | "flat";
/** "▲ 6% vs previous" within the cohort + its tone (for both size and duration, an
 *  increase is worse), or null when there's no comparable prior. */
function deltaNote(
  curr: number | null | undefined,
  prev: number | null | undefined
): { text: string; tone: DeltaTone } | null {
  if (curr == null || prev == null || prev <= 0) return null;
  const pct = ((curr - prev) / prev) * 100;
  if (!isFinite(pct)) return null;
  const tone: DeltaTone = pct > 0.5 ? "up" : pct < -0.5 ? "down" : "flat";
  const arrow = tone === "up" ? "▲" : tone === "down" ? "▼" : "▬";
  return { text: `${arrow} ${Math.abs(pct).toFixed(0)}% vs previous`, tone };
}
/** Note color: an increase (▲) is negative → red; a decrease (▼) → green; else neutral. */
function toneColor(tone: DeltaTone | undefined): string {
  return tone === "up" ? tokens.dangerText : tone === "down" ? tokens.successText : tokens.textDim;
}

// Static legends so the line meaning shows even before any data is captured.
const STATUS_LEGEND: LineSeries[] = [
  { label: "Succeeded", values: [], color: tokens.success },
  { label: "Failed", values: [], color: tokens.danger },
];
const WE_LEGEND: LineSeries[] = [
  { label: "Warnings", values: [], color: tokens.warn },
  { label: "Errors", values: [], color: tokens.danger },
];

function Swatch({ color, label }: { color: string; label: string }) {
  return (
    <Group gap={6} wrap="nowrap">
      <Box style={{ width: 14, height: 11, background: color, borderRadius: 2 }} />
      <Text fz={11} c={tokens.textDim}>
        {label}
      </Text>
    </Group>
  );
}
const Legend = ({ series }: { series: LineSeries[] }) => (
  <Group gap={18}>
    {series.map((s) => (
      <Swatch key={s.label} color={s.color} label={s.label} />
    ))}
  </Group>
);

function ChartCard({ title, sub, children, legend }: { title: string; sub: string; children: ReactNode; legend?: ReactNode }) {
  return (
    <Paper withBorder radius="md" p="md">
      <Text fw={600} fz={14} c={tokens.ink}>
        {title}
      </Text>
      <Text fz={11} c={tokens.textDim}>
        {sub}
      </Text>
      <Box mt={10}>{children}</Box>
      {legend && <Box mt={10}>{legend}</Box>}
    </Paper>
  );
}

export function DashboardTab() {
  const [records, setRecords] = useState<BuildRecord[]>([]);
  const [cohort, setCohort] = useState<Cohort | null>(null);

  useEffect(() => {
    const load = () => listHistory().then(setRecords).catch(() => {});
    load();
    const un = onRunFinished(() => load());
    return () => {
      un.then((u) => u()).catch(() => {});
    };
  }, []);

  const options = useMemo(() => {
    const uniq = (vals: string[]) => [...new Set(vals)].filter((v) => v && v !== "-");
    return {
      platform: uniq(records.map((r) => platformOf(r.tags))),
      config: uniq(records.flatMap((r) => configsOf(r.tags))),
      target: uniq(records.map((r) => targetOf(r.tags))),
    };
  }, [records]);

  // Default to the most-recent build's cohort; reconcile if the selection falls out
  // of range (e.g. after switching projects). The memoized default keeps a stable
  // ref so this can't loop.
  const defaultCohort = useMemo(() => (records.length ? cohortOf(records[0]) : null), [records]);
  useEffect(() => {
    setCohort((prev) => {
      if (!records.length) return null;
      const ok =
        prev &&
        options.platform.includes(prev.platform) &&
        options.config.includes(prev.config) &&
        options.target.includes(prev.target);
      return ok ? prev : defaultCohort;
    });
  }, [records, options, defaultCohort]);

  const view = useMemo(() => {
    const inCohort = cohort ? records.filter((r) => matchesCohort(r, cohort)) : []; // newest-first
    const chrono = [...inCohort].reverse(); // oldest-first for the time series

    // Build size - archived builds only (a 0 = "never archived" would falsely dip).
    const sized = chrono.filter((r) => (r.buildSize ?? 0) > 0);
    const sizeSeries: LineSeries[] = sized.length
      ? [{ label: "Build size", values: sized.map((r) => bytesToGiB(r.buildSize ?? 0)), color: tokens.accent }]
      : [];
    const sizeX = sized.map((r) => xPoint(r.startedAtMs));

    // Build status - cumulative passed vs failed (two colour-coded lines).
    let p = 0;
    let f = 0;
    const passed: number[] = [];
    const failed: number[] = [];
    for (const r of chrono) {
      const s = statusOf(r.tags);
      if (s === "Success") p++;
      else if (s === "Failed") f++;
      passed.push(p);
      failed.push(f);
    }
    const statusSeries: LineSeries[] = chrono.length
      ? [
          { label: "Succeeded", values: passed, color: tokens.success },
          { label: "Failed", values: failed, color: tokens.danger },
        ]
      : [];
    const statusX = chrono.map((r) => xPoint(r.startedAtMs));

    // Warnings / errors - only once some build actually recorded counts (forward-only;
    // pre-feature records have none, so we don't fake a flat zero line).
    const hasWE = chrono.some((r) => r.warningCount != null || r.errorCount != null);
    const weSeries: LineSeries[] = hasWE
      ? [
          { label: "Warnings", values: chrono.map((r) => r.warningCount ?? 0), color: tokens.warn },
          { label: "Errors", values: chrono.map((r) => r.errorCount ?? 0), color: tokens.danger },
        ]
      : [];
    const weX = hasWE ? chrono.map((r) => xPoint(r.startedAtMs)) : [];

    const latest = inCohort[0];
    const prev = inCohort[1];
    const total = inCohort.length;
    const success = inCohort.filter((r) => statusOf(r.tags) === "Success").length;
    const sizeDelta = latest ? deltaNote(latest.buildSize, prev?.buildSize) : null;
    const durDelta = latest ? deltaNote(latest.duration, prev?.duration) : null;
    const kpis: { label: string; value: string; note: string; noteTone?: DeltaTone }[] = [
      {
        label: "Latest build size",
        value: latest ? fmtSize(latest.buildSize) : "-",
        note: sizeDelta?.text ?? (latest ? fmtDate(latest.startedAtMs) : "no builds yet"),
        noteTone: sizeDelta?.tone,
      },
      {
        label: "Latest duration",
        value: latest ? fmtSecs(latest.duration ?? 0) : "-",
        note: durDelta?.text ?? (latest ? "first build" : "-"),
        noteTone: durDelta?.tone,
      },
      {
        label: "Success rate",
        value: total ? `${Math.round((success / total) * 100)}%` : "-",
        note: `${success} / ${total} builds`,
      },
      {
        label: "Total builds",
        value: String(total),
        note: cohort ? `${cohort.config} · ${cohort.platform}` : "-",
      },
    ];

    return { sizeSeries, sizeX, statusSeries, statusX, weSeries, weX, kpis, sizedCount: sized.length, total };
  }, [records, cohort]);

  const noHistory = !records.length;

  return (
    <Box>
      <Paper withBorder radius="md" p="md" mb="md">
        <Group gap="sm" align="flex-end">
          <Select
            label="Platform"
            data={options.platform}
            value={cohort?.platform ?? null}
            onChange={(v) => v && setCohort((c) => (c ? { ...c, platform: v } : c))}
            allowDeselect={false}
            disabled={noHistory}
            w={140}
            size="sm"
          />
          <Select
            label="Target"
            data={options.target}
            value={cohort?.target ?? null}
            onChange={(v) => v && setCohort((c) => (c ? { ...c, target: v } : c))}
            allowDeselect={false}
            disabled={noHistory}
            w={220}
            size="sm"
          />
          <Select
            label="Configuration"
            data={options.config}
            value={cohort?.config ?? null}
            onChange={(v) => v && setCohort((c) => (c ? { ...c, config: v } : c))}
            allowDeselect={false}
            disabled={noHistory}
            w={170}
            size="sm"
          />
        </Group>
      </Paper>

      <SimpleGrid cols={{ base: 2, lg: 4 }} spacing="md">
        {view.kpis.map((k) => (
          <Paper key={k.label} withBorder radius="md" p="md">
            <Text fz={11} c={tokens.textDim}>
              {k.label}
            </Text>
            <Text fz={23} fw={700} c={tokens.ink} mt={6}>
              {k.value}
            </Text>
            <Text fz={11} c={toneColor(k.noteTone)} mt={4}>
              {k.note}
            </Text>
          </Paper>
        ))}
      </SimpleGrid>

      <SimpleGrid cols={{ base: 1, md: 2 }} spacing="md" mt="md">
        <ChartCard title="Build size over time" sub={`${view.sizedCount} archived build(s) · hover a point for details`}>
          <LineChart series={view.sizeSeries} x={view.sizeX} format={(n) => `${n.toFixed(2)} GB`} />
        </ChartCard>
        <ChartCard title="Warnings & errors over time" sub="per build · new builds onward" legend={<Legend series={WE_LEGEND} />}>
          <LineChart series={view.weSeries} x={view.weX} />
        </ChartCard>
      </SimpleGrid>

      <Box mt="md">
        <ChartCard title="Build status" sub="cumulative · passed vs failed" legend={<Legend series={STATUS_LEGEND} />}>
          <LineChart series={view.statusSeries} x={view.statusX} />
        </ChartCard>
      </Box>
    </Box>
  );
}
