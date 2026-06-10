// Lightweight, dependency-free grayscale chart for the Dashboard. Prop-driven,
// fixed-viewBox SVG that scales to its container.

import { useState } from "react";
import type { MouseEvent, ReactNode } from "react";
import { tokens } from "../../ui/tokens";

const svgProps = (w: number, h: number) => ({
  viewBox: `0 0 ${w} ${h}`,
  width: "100%",
  height: "auto",
  style: { display: "block" as const },
  preserveAspectRatio: "xMidYMid meet",
});

// ── Interactive multi-series line chart (size / status / warnings · errors) ──────

export interface LineSeries {
  label: string;
  values: number[];
  color: string;
}
export interface XPoint {
  /** Short axis label (e.g. "Jun 4"). */
  tick: string;
  /** Full hover label (e.g. "6/4/2026, 2:31:00 PM"). */
  full: string;
}

/** Smallest "nice" step ≥ raw, floored at 1 so axis labels stay whole numbers. */
function niceStep(raw: number): number {
  const r = Math.max(raw, 1e-9);
  const nice = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000];
  for (const s of nice) if (s >= r) return s;
  return 1000 * Math.ceil(r / 1000);
}

/**
 * Whole-number, evenly-spaced y-axis ticks covering [min, max] - labels are always
 * integers (no decimals); the tooltip keeps full precision via the `format` prop.
 */
function niceTicks(min: number, max: number): { lo: number; hi: number; ticks: number[] } {
  if (!isFinite(min) || !isFinite(max)) return { lo: 0, hi: 1, ticks: [0, 1] };
  const step = niceStep((max - min) / 4);
  const lo = Math.floor(min / step) * step;
  let hi = Math.ceil(max / step) * step;
  if (hi <= lo) hi = lo + step;
  const ticks: number[] = [];
  for (let v = lo; v <= hi + step * 1e-6; v += step) ticks.push(Math.round(v));
  return { lo, hi, ticks };
}

/**
 * Multi-series line chart with y-axis value ticks, x-axis labels, a dot per point,
 * and a hover callout (vertical guide + every series' value at that x). Series are
 * told apart by **color** (no dashes). Scales to its container via a fixed viewBox.
 */
export function LineChart({
  series,
  x,
  unit = "",
  format = (n: number) => `${Math.round(n)}`,
  vh = 260,
}: {
  series: LineSeries[];
  x: XPoint[];
  unit?: string;
  format?: (n: number) => string;
  /** viewBox height - lower = a wider, shorter chart (full-width size strip). */
  vh?: number;
}) {
  const W = 680;
  const H = vh;
  const left = 48;
  const right = 16;
  const top = 18;
  const bottom = 30;
  const plotW = W - left - right;
  const plotH = H - top - bottom;
  const [hover, setHover] = useState<number | null>(null);

  const n = x.length;
  const allVals = series.flatMap((s) => s.values);
  if (!allVals.length || !n) {
    return (
      <svg {...svgProps(W, H)}>
        <text x={W / 2} y={H / 2} textAnchor="middle" fontSize={12} fill={tokens.textDim}>
          No data yet
        </text>
      </svg>
    );
  }

  const { lo, hi, ticks } = niceTicks(Math.min(...allVals), Math.max(...allVals));
  const span = hi - lo || 1;
  const px = (i: number) => (n <= 1 ? left + plotW / 2 : left + (plotW * i) / (n - 1));
  const py = (v: number) => top + plotH - (plotH * (v - lo)) / span;
  const step = Math.max(1, Math.ceil(n / 7));

  const onMove = (e: MouseEvent<SVGSVGElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const vx = ((e.clientX - r.left) / r.width) * W; // client px → viewBox x
    const idx = Math.round(((vx - left) / plotW) * (n - 1));
    setHover(Math.min(n - 1, Math.max(0, idx)));
  };

  let callout: ReactNode = null;
  if (hover != null && hover < n) {
    const rows = [
      { text: x[hover]?.full ?? x[hover]?.tick ?? "", color: null as string | null },
      ...series.map((s) => ({
        text: `${s.label}: ${format(s.values[hover] ?? 0)}${unit ? ` ${unit}` : ""}`,
        color: s.color,
      })),
    ];
    const boxW = Math.max(...rows.map((r) => r.text.length)) * 5.7 + 26;
    const boxH = rows.length * 16 + 8;
    const bx = Math.min(W - right - boxW, Math.max(left, px(hover) + 10));
    const by = top + 4;
    callout = (
      <g pointerEvents="none">
        <rect x={bx} y={by} width={boxW} height={boxH} rx={6} fill={tokens.surface} stroke={tokens.border} opacity={0.97} />
        {rows.map((r, i) => {
          const ty = by + 17 + i * 16;
          return (
            <g key={i}>
              {r.color && <circle cx={bx + 12} cy={ty - 4} r={3} fill={r.color} />}
              <text
                x={r.color ? bx + 22 : bx + 12}
                y={ty}
                fontSize={11}
                fontWeight={r.color ? 400 : 600}
                fill={r.color ? tokens.text : tokens.textMuted}
              >
                {r.text}
              </text>
            </g>
          );
        })}
      </g>
    );
  }

  return (
    <svg {...svgProps(W, H)} onMouseMove={onMove} onMouseLeave={() => setHover(null)}>
      {ticks.map((t, i) => (
        <g key={i}>
          <line x1={left} x2={W - right} y1={py(t)} y2={py(t)} stroke={tokens.divider} />
          <text x={left - 8} y={py(t) + 3} textAnchor="end" fontSize={10} fill={tokens.textDim}>
            {t}
          </text>
        </g>
      ))}
      <line x1={left} y1={top} x2={left} y2={top + plotH} stroke={tokens.borderStrong} />
      <line x1={left} y1={top + plotH} x2={W - right} y2={top + plotH} stroke={tokens.borderStrong} />
      {unit && (
        <text x={left - 8} y={top - 5} textAnchor="end" fontSize={10} fill={tokens.textDim}>
          {unit}
        </text>
      )}
      {x.map((p, i) =>
        i % step === 0 || i === n - 1 ? (
          <text key={i} x={px(i)} y={H - 9} textAnchor="middle" fontSize={10} fill={tokens.textDim}>
            {p.tick}
          </text>
        ) : null,
      )}
      {hover != null && (
        <line x1={px(hover)} y1={top} x2={px(hover)} y2={top + plotH} stroke={tokens.borderStrong} strokeDasharray="3 3" />
      )}
      {series.map((s, si) => (
        <g key={si}>
          {s.values.length > 1 && (
            <polyline
              fill="none"
              stroke={s.color}
              strokeWidth={2}
              points={s.values.map((v, i) => `${px(i).toFixed(1)},${py(v).toFixed(1)}`).join(" ")}
            />
          )}
          {s.values.map((v, i) => (
            <circle
              key={i}
              cx={px(i)}
              cy={py(v)}
              r={hover === i ? 4.5 : 2.4}
              fill={s.color}
              stroke={tokens.surface}
              strokeWidth={hover === i ? 1.3 : 0.8}
            >
              <title>{`${x[i]?.full ?? ""} · ${s.label}: ${format(v)}${unit ? ` ${unit}` : ""}`}</title>
            </circle>
          ))}
        </g>
      ))}
      {callout}
    </svg>
  );
}
