import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import { Box, Button, Checkbox, Divider, Group, Modal, Paper, ScrollArea, Stack, Text, Tooltip } from "@mantine/core";
import { IconInfoCircle, IconRefresh, IconTrash } from "@tabler/icons-react";
import { cleanupCats, tokens } from "../../../ui/tokens";
import { dangerSolidButton } from "../../../ui/buttonStyles";
import { formatBytes as fmtSize } from "../../../ui/format";
import { IpcError, cleanFootprint, scanFootprint, type CleanupCategory, type FootprintNode, type FootprintReport } from "../../../ipc";

// Per-category display label + whether deleting costs a slow regen (shader/derived
// caches) vs a free rebuild. The bar/legend colors (selected fill + unselected tint)
// live in ui/tokens `cleanupCats` so they follow the scheme swap (dark mode).
const CAT: Record<CleanupCategory, { label: string; slowRegen?: boolean }> = {
  staged: { label: "Staged build" },
  cooked: { label: "Cooked content" },
  shader: { label: "Shader cache", slowRegen: true },
  binariesGame: { label: "Binaries · Game" },
  binariesPlugin: { label: "Binaries · Plugin" },
  intermediateGame: { label: "Intermediate · Game" },
  intermediateOther: { label: "Intermediate · Other", slowRegen: true },
  intermediatePlugin: { label: "Intermediate · Plugin" },
  derivedData: { label: "Derived data cache", slowRegen: true },
};
const CAT_ORDER: CleanupCategory[] = [
  "staged",
  "cooked",
  "shader",
  "binariesGame",
  "binariesPlugin",
  "intermediateGame",
  "intermediateOther",
  "intermediatePlugin",
  "derivedData",
];

// tree layout
const ROW_H = 34;
const INDENT = 22; // px per nesting level
const HALF = INDENT / 2;
const PAD = 6; // gap between the elbow and the row content
const cx = (col: number) => col * INDENT + HALF; // x of a guide column's vertical line

function leavesOf(node: FootprintNode): FootprintNode[] {
  return node.selectable ? [node] : node.children.flatMap(leavesOf);
}
function locationSummary(node: FootprintNode): string {
  if (node.locations.length === 0) return "";
  if (node.locations.length === 1) return node.locations[0].rel;
  return `${node.locations[0].rel}  +${node.locations.length - 1}`;
}

export function CleanTab() {
  // Connector-line color, read per-render so the tree follows the scheme swap.
  const LINE = tokens.borderStrong;
  // Solid "Stage ✕ failed" red for the destructive Delete buttons.
  const dangerSolid = dangerSolidButton();
  const [report, setReport] = useState<FootprintReport | null>(null);
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastScan, setLastScan] = useState<number | null>(null);
  const [reclaimed, setReclaimed] = useState<number | null>(null);
  const [checked, setChecked] = useState<Set<string>>(() => new Set());
  const [confirmOpen, setConfirmOpen] = useState(false);

  async function runScan() {
    setScanning(true);
    setError(null);
    setReclaimed(null);
    try {
      const r = await scanFootprint();
      setReport(r);
      setLastScan(Date.now());
      setChecked(new Set());
    } catch (e) {
      setError(e instanceof IpcError ? e.message : String(e));
    } finally {
      setScanning(false);
    }
  }

  // Auto-scan whenever the tab opens (the shell mounts this only when active).
  useEffect(() => {
    runScan();
  }, []);

  const allLeaves = useMemo(() => (report ? report.groups.flatMap(leavesOf) : []), [report]);

  // "Other" wipes the whole main Intermediate, so ticking it implies every game target is
  // wiped too - they render checked (and locked) and count toward the selection.
  const otherChecked = checked.has("intermediateOther");
  const isLeafChecked = (leaf: FootprintNode): boolean =>
    leaf.category === "intermediateGame" ? otherChecked || checked.has(leaf.id) : checked.has(leaf.id);

  // The leaves a Delete targets (for size + confirm). When Other is on it includes the
  // auto-ticked game targets, so the total reflects the whole wipe.
  const selectedNodes = useMemo(() => allLeaves.filter(isLeafChecked), [allLeaves, checked]); // eslint-disable-line react-hooks/exhaustive-deps

  const selectedBytes = selectedNodes.reduce((s, n) => s + (n.sizeBytes ?? 0), 0);
  // Delete request: when Other is on, drop the per-target game ids - Other's whole-dir wipe
  // subsumes them (the backend would otherwise skip them, but keep the request clean).
  const deleteIds = otherChecked
    ? selectedNodes.map((n) => n.id).filter((id) => !id.startsWith("intermediateGame:"))
    : selectedNodes.map((n) => n.id);

  const selectedCats = useMemo(() => {
    const s = new Set<CleanupCategory>();
    for (const n of selectedNodes) if (n.category) s.add(n.category);
    return s;
  }, [selectedNodes]);

  // Reclaimable composition by category (for the colored bar + legend).
  const catBytes = useMemo(() => {
    const m = new Map<CleanupCategory, number>();
    for (const leaf of allLeaves) {
      if (!leaf.category) continue;
      m.set(leaf.category, (m.get(leaf.category) ?? 0) + (leaf.sizeBytes ?? 0));
    }
    return m;
  }, [allLeaves]);
  const presentCats = CAT_ORDER.filter((c) => (catBytes.get(c) ?? 0) > 0);
  // Bar segments run largest → smallest, left to right.
  const barCats = [...presentCats].sort((a, b) => (catBytes.get(b) ?? 0) - (catBytes.get(a) ?? 0));

  const setIds = (ids: string[], on: boolean) =>
    setChecked((s) => {
      const n = new Set(s);
      for (const id of ids) (on ? n.add(id) : n.delete(id));
      return n;
    });
  const toggleLeaf = (leaf: FootprintNode) => setIds([leaf.id], !checked.has(leaf.id));
  const toggleCategory = (c: CleanupCategory) => {
    const ids = allLeaves.filter((l) => l.category === c).map((l) => l.id);
    const allOn = ids.length > 0 && ids.every((id) => checked.has(id));
    setIds(ids, !allOn);
  };

  async function confirmDelete() {
    setCleaning(true);
    setError(null);
    try {
      const out = await cleanFootprint(deleteIds);
      setConfirmOpen(false);
      setReclaimed(out.reclaimedBytes ?? 0);
      await runScan();
    } catch (e) {
      setConfirmOpen(false);
      setError(e instanceof IpcError ? e.message : String(e));
    } finally {
      setCleaning(false);
    }
  }

  // ── tree rendering: flat rows with computed connector lines (├ / └) ──
  // Each top group is a root (no lines); its subtree is walked into descriptors that
  // carry, per row, the ancestor trunks to draw and whether it's the last sibling (└).
  type RowDesc = { node: FootprintNode; level: number; ancestors: boolean[]; isLast: boolean; hasKids: boolean };

  function walk(node: FootprintNode, ancestors: boolean[], isLast: boolean, out: RowDesc[]) {
    const kids = node.selectable ? [] : node.children;
    out.push({ node, level: ancestors.length + 1, ancestors, isLast, hasKids: kids.length > 0 });
    const childAnc = [...ancestors, !isLast]; // this node's column keeps a trunk iff it has a younger sibling
    kids.forEach((c, i) => walk(c, childAnc, i === kids.length - 1, out));
  }

  function renderRow(desc: RowDesc): ReactNode {
    const { node, level, ancestors, isLast, hasKids } = desc;
    const ec = level - 1; // this row's elbow column
    const lines: ReactNode[] = [];
    ancestors.forEach((on, i) => {
      if (on) lines.push(<Box key={`a${i}`} style={{ position: "absolute", left: cx(i), top: 0, bottom: 0, width: 1, background: LINE }} />);
    });
    // elbow: vertical down to mid (└) or full height (├), plus the horizontal stub
    lines.push(<Box key="ev" style={{ position: "absolute", left: cx(ec), top: 0, height: isLast ? ROW_H / 2 : ROW_H, width: 1, background: LINE }} />);
    lines.push(<Box key="eh" style={{ position: "absolute", left: cx(ec), top: ROW_H / 2, width: HALF, height: 1, background: LINE }} />);
    // bridge down into this node's own children column so the first child connects up
    if (hasKids) lines.push(<Box key="cd" style={{ position: "absolute", left: cx(ec + 1), top: ROW_H / 2, bottom: 0, width: 1, background: LINE }} />);
    const content = node.selectable ? leafInner(node) : groupInner(node, false);
    return (
      <Box key={node.id || `g:${level}:${node.label}`} className="uep-hoverable" style={{ position: "relative", height: ROW_H, paddingLeft: level * INDENT + PAD, borderRadius: 6 }}>
        {lines}
        <Group wrap="nowrap" gap={10} style={{ height: "100%" }}>
          {content}
        </Group>
      </Box>
    );
  }

  function topGroupRow(node: FootprintNode): ReactNode {
    return (
      <Box key={node.label} className="uep-hoverable" style={{ position: "relative", height: ROW_H, paddingLeft: PAD, borderRadius: 6 }}>
        <Group wrap="nowrap" gap={10} style={{ height: "100%" }}>
          {groupInner(node, true)}
        </Group>
      </Box>
    );
  }

  function renderGroup(group: FootprintNode): ReactNode {
    const descs: RowDesc[] = [];
    group.children.forEach((c, i) => walk(c, [], i === group.children.length - 1, descs));
    return (
      <Fragment key={group.label}>
        {topGroupRow(group)}
        {descs.map(renderRow)}
      </Fragment>
    );
  }

  // ── row content (checkbox + columns) ──
  function leafInner(node: FootprintNode): ReactNode {
    const size = node.sizeBytes ?? 0;
    const empty = !!report && size <= 0;
    // Game targets are locked-on while Other (whole-Intermediate wipe) is selected.
    const disabled = cleaning || empty || (node.category === "intermediateGame" && otherChecked);
    const warn = node.category === "intermediateOther" ? "Wipes the whole Intermediate incl. the editor cache (full recompile next open)" : undefined;
    return (
      <>
        <Checkbox size="xs" checked={isLeafChecked(node)} disabled={disabled} onChange={() => toggleLeaf(node)} />
        {rowCols(
          <Text fz={12.5} c={tokens.text} title={warn}>
            {node.label}
          </Text>,
          size,
          { location: node, dim: empty }
        )}
      </>
    );
  }

  function groupInner(node: FootprintNode, bold: boolean): ReactNode {
    const leaves = leavesOf(node);
    // Nothing deletable in this category (no leaves, or every leaf is empty) ⇒ no
    // actionable tick - disable + dim, mirroring how empty leaves render.
    const groupBytes = leaves.reduce((s, l) => s + (l.sizeBytes ?? 0), 0);
    const empty = !!report && groupBytes <= 0;
    const checkedAll = leaves.length > 0 && leaves.every(isLeafChecked);
    const some = !checkedAll && leaves.some(isLeafChecked);
    const disabled = cleaning || empty;
    const toggle = () => setIds(leaves.map((l) => l.id), !checkedAll);
    return (
      <>
        <Checkbox size="xs" checked={checkedAll} indeterminate={some} disabled={disabled} onChange={toggle} />
        {rowCols(
          <Text fz={12.5} fw={bold ? 700 : 600} c={tokens.text}>
            {node.label}
          </Text>,
          node.sizeBytes,
          { dim: empty }
        )}
      </>
    );
  }

  function rowCols(label: ReactNode, sizeBytes: number | null | undefined, opts: { location?: FootprintNode; dim?: boolean }) {
    const size = sizeBytes ?? 0;
    return (
      <>
        <Box style={{ flex: 1, minWidth: 0, opacity: opts.dim ? 0.5 : 1 }}>{label}</Box>
        <Text ff="monospace" fz={10.5} c={tokens.textDim} title={opts.location?.locations.map((l) => l.path).join("\n")} style={{ width: 200, textAlign: "left", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {opts.location ? locationSummary(opts.location) : ""}
        </Text>
        <Text fz={12} fw={700} c={size > 0 ? tokens.ink : tokens.textDim} style={{ width: 88, textAlign: "right", paddingRight: 12 }}>
          {report ? fmtSize(size) : "-"}
        </Text>
      </>
    );
  }

  const total = report?.totalBytes ?? 0;

  return (
    <Box style={{ height: "calc(100dvh - var(--mantine-spacing-lg) * 2)", display: "flex", flexDirection: "column", minHeight: 0 }}>
      {/* summary on top: reclaimable total + bar + legend + selected */}
      <Paper withBorder radius="md" p="md" style={{ flexShrink: 0 }}>
        <Group justify="space-between" align="stretch" wrap="nowrap" gap="xl">
          <Box style={{ flexShrink: 0 }}>
            <Group gap={5} wrap="nowrap" align="center">
              <Text fz={11} c={tokens.textDim}>
                Reclaimable
              </Text>
              <Tooltip
                multiline
                w={250}
                withArrow
                label="Total size of build files that can be safely deleted. Removing them frees this disk space. They're regenerated on your next build (some caches are slower to rebuild)."
              >
                <IconInfoCircle size={13} color={tokens.textDim} style={{ cursor: "help", display: "block" }} />
              </Tooltip>
            </Group>
            <Text fz={27} fw={700} c={tokens.ink}>
              {fmtSize(total)}
            </Text>
          </Box>
          <Box style={{ flex: 1, minWidth: 0 }}>
            <Group gap={0} wrap="nowrap" style={{ height: 22, borderRadius: 4, overflow: "hidden", border: `1px solid ${tokens.borderStrong}`, background: tokens.surfaceAlt }}>
              {barCats.map((c) => (
                <Box
                  key={c}
                  className="uep-seg"
                  title={`${CAT[c].label} · ${fmtSize(catBytes.get(c))}`}
                  onClick={() => toggleCategory(c)}
                  style={{ flex: catBytes.get(c) ?? 0, height: "100%", background: selectedCats.has(c) ? cleanupCats[c].color : cleanupCats[c].light }}
                />
              ))}
            </Group>
            {/* legend - same largest→smallest order as the bar */}
            <Group gap={14} mt={10} wrap="wrap">
              {barCats.map((c) => {
                const on = selectedCats.has(c);
                return (
                  <Group key={c} gap={6} wrap="nowrap" onClick={() => toggleCategory(c)} style={{ cursor: "pointer", opacity: on ? 1 : 0.7 }}>
                    <Box style={{ width: 18, height: 12, borderRadius: 2, background: on ? cleanupCats[c].color : cleanupCats[c].light, border: `1px solid ${tokens.borderStrong}` }} />
                    <Text fz={11} c={tokens.textMuted}>
                      {CAT[c].label}
                    </Text>
                    <Text fz={11} fw={600} c={tokens.textDim}>
                      {fmtSize(catBytes.get(c))}
                    </Text>
                  </Group>
                );
              })}
            </Group>
          </Box>
          <Divider orientation="vertical" style={{ alignSelf: "stretch", height: "auto" }} />
          <Box style={{ flexShrink: 0, textAlign: "right" }}>
            <Text fz={11} c={tokens.textDim}>
              Selected to remove
            </Text>
            <Text fz={22} fw={700} c={selectedBytes > 0 ? tokens.ink : tokens.textDim}>
              {fmtSize(selectedBytes)}
            </Text>
          </Box>
        </Group>
      </Paper>

      {/* category tree (directory) - fills the remaining height; the list body scrolls */}
      <Paper withBorder radius="md" p={0} mt="md" style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        {/* column header - matches the Build tab's <Th> style (fz10 / fw700 / #8a8a92 / ls0.5),
            md padding, and a bottom rule separating the header from the list */}
        <Group wrap="nowrap" gap={10} px={16} style={{ height: 40, flexShrink: 0, borderBottom: `1px solid ${tokens.divider}` }}>
          <Text fz={10} fw={700} c={tokens.textMuted} style={{ flex: 1, letterSpacing: 0.5 }}>
            CATEGORY
          </Text>
          <Text fz={10} fw={700} c={tokens.textMuted} style={{ width: 200, textAlign: "left", letterSpacing: 0.5 }}>
            LOCATION
          </Text>
          <Text fz={10} fw={700} c={tokens.textMuted} style={{ width: 88, textAlign: "right", letterSpacing: 0.5, paddingRight: 12 }}>
            SIZE
          </Text>
        </Group>
        <Box px={16} py={8} style={{ flex: 1, minHeight: 0, overflowY: "auto" }}>
          {report ? report.groups.map(renderGroup) : null}
        </Box>
      </Paper>

      {/* bottom action bar - Re-scan (infrequent; also auto-runs on open + after a delete)
          + status on the left, destructive Delete on the right */}
      <Paper withBorder radius="md" p="md" mt="md" style={{ flexShrink: 0 }}>
        {error && (
          <Text fz={12.5} c={tokens.danger} mb={10}>
            {error}
          </Text>
        )}
        <Group justify="space-between" align="center" wrap="nowrap">
          <Group gap="md" wrap="nowrap" style={{ minWidth: 0 }}>
            <Button variant="default" leftSection={<IconRefresh size={16} />} loading={scanning} onClick={runScan}>
              Re-scan
            </Button>
            {lastScan != null && (
              <Text fz={12.5} c={tokens.textDim} truncate>
                Last scanned {new Date(lastScan).toLocaleString()}
                {reclaimed != null && (
                  <Text span c={tokens.successText}>
                    {" · "}Reclaimed {fmtSize(reclaimed)}
                  </Text>
                )}
              </Text>
            )}
          </Group>
          <Button
            leftSection={<IconTrash size={16} />}
            disabled={!report || deleteIds.length === 0 || scanning || cleaning}
            onClick={() => setConfirmOpen(true)}
            style={{ ...dangerSolid, flexShrink: 0 }}
          >
            Delete {selectedNodes.length} {selectedNodes.length === 1 ? "item" : "items"} · {fmtSize(selectedBytes)}
          </Button>
        </Group>
      </Paper>

      {/* confirm - lists every folder that will be removed (R3) */}
      <Modal
        opened={confirmOpen}
        onClose={() => !cleaning && setConfirmOpen(false)}
        title={`Delete ${selectedNodes.length} ${selectedNodes.length === 1 ? "item" : "items"}?`}
        centered
        size="lg"
      >
        <Text fz={13} c={tokens.textMuted}>
          These folders will be permanently deleted. They regenerate on the next build.
        </Text>
        <ScrollArea.Autosize mah={340} mt={14}>
          <Stack gap={14}>
            {selectedNodes.map((node) => (
              <Box key={node.id}>
                <Group justify="space-between" wrap="nowrap">
                  <Text fz={12} fw={600} c={tokens.text}>
                    {node.label}
                  </Text>
                  <Text fz={11} c={tokens.textDim} style={{ flexShrink: 0 }}>
                    {fmtSize(node.sizeBytes)}
                  </Text>
                </Group>
                <Stack gap={2} mt={6} style={{ background: tokens.surfaceAlt, border: `1px solid ${tokens.divider}`, borderRadius: 6, padding: "6px 10px" }}>
                  {node.locations.map((loc) => (
                    <Group key={loc.path} justify="space-between" wrap="nowrap" gap={16}>
                      <Text ff="monospace" fz={11} c={tokens.textMuted} title={loc.path} style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {loc.rel}
                      </Text>
                      <Text ff="monospace" fz={11} c={tokens.textDim} style={{ flexShrink: 0 }}>
                        {fmtSize(loc.sizeBytes)}
                      </Text>
                    </Group>
                  ))}
                </Stack>
              </Box>
            ))}
          </Stack>
        </ScrollArea.Autosize>
        <Divider my={14} />
        <Group justify="space-between">
          <Text fz={13} c={tokens.text}>
            Total reclaim
          </Text>
          <Text fz={15} fw={700} c={tokens.ink}>
            {fmtSize(selectedBytes)}
          </Text>
        </Group>
        <Group justify="flex-end" gap={8} mt={18}>
          <Button variant="default" onClick={() => setConfirmOpen(false)} disabled={cleaning}>
            Cancel
          </Button>
          <Button leftSection={<IconTrash size={16} />} loading={cleaning} onClick={confirmDelete} style={dangerSolid}>
            Delete permanently
          </Button>
        </Group>
      </Modal>
    </Box>
  );
}
