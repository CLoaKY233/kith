import { memo, useEffect, useMemo, useState } from "react";
import { clock } from "../format";
import { useCrosshair } from "./crosshair";
import { useWidth } from "./useWidth";

export type Point = { t: number; v: number };
export type Series = { id: string; label: string; points: Point[]; style?: "line" | "area"; tone?: 0 | 1 | 2 | 3 | 4 | 5 };

type Props = {
  series: Series[];
  format: (v: number) => string;
  height?: number;
  yMax?: number;
  sync?: boolean;
  /** Whole-number y ticks only (counts). */
  integer?: boolean;
  /** Keep the legend's row of height even with a single series, so panels side by side stay level. */
  reserveLegend?: boolean;
  /** Byte-style axis: ticks land on whole 1024-based units (1, 2, 3 MB/s), not on 1.91 MB/s. */
  binary?: boolean;
};

const TONES = ["#fafafa", "#a4a4a4", "#6f6f6f", "#4a4a4a", "#c8c8c8", "#868686"];
const PAD = { top: 8, right: 0, bottom: 26 };
// Geist Mono at 11px advances 0.6em per glyph.
const CHAR_W = 6.6;
const TIME_STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 180, 360, 720, 1440].map((m) => m * 60_000);

function niceStep(max: number, target: number) {
  const raw = max / target;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const f = raw / mag;
  return (f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10) * mag;
}

function nearest(points: Point[], t: number): Point | undefined {
  let lo = 0;
  let hi = points.length - 1;
  if (hi < 0) return undefined;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (points[mid].t < t) lo = mid + 1;
    else hi = mid;
  }
  const prev = points[lo - 1];
  return prev && t - prev.t < points[lo].t - t ? prev : points[lo];
}

function robustMax(values: number[]): { max: number; capped: boolean } {
  if (!values.length) return { max: 0, capped: false };
  const sorted = [...values].sort((a, b) => a - b);
  const max = sorted[sorted.length - 1];
  const p99 = sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * 0.95))];
  return p99 > 0 && max > 4 * p99 ? { max: p99 * 1.15, capped: true } : { max, capped: false };
}

export const TimeSeries = memo(function TimeSeries({ series, format, height = 200, yMax, sync = true, integer, reserveLegend, binary }: Props) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [shared, setShared] = useCrosshair();
  const [local, setLocal] = useState<number | null>(null);
  const hover = sync ? shared : local;
  const setHover = sync ? setShared : setLocal;

  useEffect(() => () => setHover(null), [setHover]);

  const clean = useMemo(
    () => series.map((s) => ({ ...s, points: s.points.filter((p) => Number.isFinite(p.t) && Number.isFinite(p.v)).sort((a, b) => a.t - b.t) })),
    [series],
  );
  const visible = useMemo(() => clean.filter((s) => !hidden.has(s.id) && s.points.length), [clean, hidden]);
  const empty = !clean.some((s) => s.points.length);
  const collecting = !empty && clean.every((s) => s.points.length < 2);
  const plotH = height - PAD.top - PAD.bottom;

  const geo = useMemo(() => {
    const all = visible.flatMap((s) => s.points);
    if (!all.length) return null;
    let t0 = Infinity;
    let t1 = -Infinity;
    for (const p of all) {
      if (p.t < t0) t0 = p.t;
      if (p.t > t1) t1 = p.t;
    }
    if (t1 === t0) t1 = t0 + 60_000;
    const robust = yMax == null ? robustMax(all.map((p) => p.v)) : { max: yMax, capped: false };
    const want = Math.max(2, Math.floor(plotH / 40));
    const unit = binary && robust.max >= 1024 ? 1024 ** Math.floor(Math.log(robust.max) / Math.log(1024)) : 1;
    const step = Math.max(integer ? 1 : 0, niceStep((robust.max || 1) / unit, want) * unit);
    const top = robust.capped ? robust.max : Math.ceil((robust.max || 1) / step) * step;
    const yTicks: number[] = [];
    for (let i = 0; i <= 50 && i * step <= top + step * 1e-9; i++) yTicks.push(i * step);
    const left = Math.ceil(Math.max(...yTicks.map((v) => format(v).length)) * CHAR_W) + 12;
    const plotW = width - left - PAD.right;
    if (plotW <= 0) return null;
    const tStep = TIME_STEPS.find((s) => (t1 - t0) / s <= Math.max(2, plotW / 90)) ?? TIME_STEPS[TIME_STEPS.length - 1];
    const xTicks: number[] = [];
    const tz = new Date(t0).getTimezoneOffset() * 60_000;
    for (let t = Math.ceil((t0 - tz) / tStep) * tStep + tz; t <= t1 && xTicks.length < 48; t += tStep) xTicks.push(t);
    const x = (t: number) => ((t - t0) / (t1 - t0)) * plotW;
    const y = (v: number) => plotH - (Math.max(0, Math.min(v, top)) / top) * plotH;
    return { t0, t1, top, yTicks, xTicks, x, y, left, plotW };
  }, [visible, width, plotH, yMax, format, integer, binary]);
  const plotW = geo?.plotW ?? 0;
  const left = geo?.left ?? 0;

  const paths = useMemo(() => {
    if (!geo) return [];
    return visible.map((s) => {
      const line = s.points.map((p, i) => `${i ? "L" : "M"}${geo.x(p.t).toFixed(1)},${geo.y(p.v).toFixed(1)}`).join("");
      const first = s.points[0];
      const last = s.points[s.points.length - 1];
      const area = s.style === "area" ? `${line}L${geo.x(last.t).toFixed(1)},${plotH}L${geo.x(first.t).toFixed(1)},${plotH}Z` : null;
      const clipped = s.points.filter((p) => p.v > geo.top);
      const single = s.points.length === 1 ? s.points[0] : null;
      return { s, line, area, clipped, single, color: TONES[s.tone ?? 0] };
    });
  }, [geo, visible, plotH]);

  const inRange = geo && hover != null && hover >= geo.t0 && hover <= geo.t1;
  const readout = inRange ? visible.map((s) => ({ s, p: nearest(s.points, hover) })) : [];
  const tipLeft = inRange && geo ? left + geo.x(hover) : 0;

  return (
    <div className="ui-chart">
      <div ref={ref} className="relative" style={{ height }}>
        {empty || collecting || !visible.length ? (
          <div className="flex h-full items-center justify-center px-4 text-center text-[13px] text-fg-3">
            {empty ? "No data in this range" : collecting ? "Collecting samples…" : "Every series is hidden"}
          </div>
        ) : (
          geo && (
            <svg
              width={width}
              height={height}
              className="block touch-pan-y overflow-visible"
              role="img"
              aria-label={`Chart: ${visible.map((s) => s.label).join(", ")}`}
              onPointerMove={(e) => {
                const px = e.clientX - e.currentTarget.getBoundingClientRect().left - left;
                setHover(px < 0 || px > plotW ? null : geo.t0 + (px / plotW) * (geo.t1 - geo.t0));
              }}
              onPointerLeave={() => setHover(null)}
              onPointerCancel={() => setHover(null)}
            >
              <g transform={`translate(${left},${PAD.top})`}>
                {geo.yTicks.map((v) => (
                  <g key={v} transform={`translate(0,${geo.y(v)})`}>
                    <line x2={plotW} stroke={v === 0 ? "#333" : "#1f1f1f"} shapeRendering="crispEdges" />
                    <text x={-10} dy="0.32em" textAnchor="end" className="fill-fg-3 font-mono text-[11px]">{format(v)}</text>
                  </g>
                ))}
                {geo.xTicks.map((t) => {
                  const x = geo.x(t);
                  const half = (clock(t).length * CHAR_W) / 2;
                  const anchor = x - half < -left ? "start" : x + half > plotW + PAD.right ? "end" : "middle";
                  const dx = anchor === "start" ? Math.max(-left, -x) : anchor === "end" ? Math.min(PAD.right, plotW + PAD.right - x) : 0;
                  return (
                    <g key={t} transform={`translate(${x},0)`}>
                      <line y2={plotH} stroke="#1f1f1f" shapeRendering="crispEdges" />
                      <text x={dx} y={plotH + 18} textAnchor={anchor} className="fill-fg-3 font-mono text-[11px]">{clock(t)}</text>
                    </g>
                  );
                })}
                {paths.map(({ s, line, area, clipped, single, color }) => (
                  <g key={s.id}>
                    {single && <rect x={geo.x(single.t) - 2.5} y={geo.y(single.v) - 2.5} width={5} height={5} fill={color} />}
                    {area && <path className="ui-path" d={area} fill={color} fillOpacity={0.08} />}
                    <path className="ui-path" d={line} fill="none" stroke={color} strokeWidth={1.25} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
                    {clipped.map((p, i) => (
                      <path key={i} d={`M${geo.x(p.t) - 3},0L${geo.x(p.t) + 3},0L${geo.x(p.t)},4Z`} fill={color}>
                        <title>{format(p.v)}</title>
                      </path>
                    ))}
                  </g>
                ))}
                {inRange && (
                  <g>
                    <line x1={geo.x(hover)} x2={geo.x(hover)} y2={plotH} stroke="#6f6f6f" shapeRendering="crispEdges" />
                    {readout.map(({ s, p }) =>
                      p ? <rect key={s.id} x={geo.x(p.t) - 2} y={geo.y(p.v) - 2} width={4} height={4} fill={TONES[s.tone ?? 0]} /> : null,
                    )}
                  </g>
                )}
              </g>
            </svg>
          )
        )}
        {inRange && readout.length > 0 && (
          <div
            className="pointer-events-none absolute top-2 z-10 min-w-36 border border-line-2 bg-ink-2 px-3 py-2 text-[12px]"
            style={{
              maxWidth: Math.max(144, Math.min(280, width - 16)),
              ...(tipLeft > width / 2 ? { right: Math.max(0, width - tipLeft + 12) } : { left: Math.min(tipLeft + 12, Math.max(0, width - 148)) }),
            }}
          >
            <div className="mb-1 font-mono text-[11px] text-fg-3">{clock(hover)}</div>
            {readout.map(({ s, p }) => (
              <div key={s.id} className="flex items-baseline justify-between gap-4">
                <span className="flex min-w-0 items-center gap-2 text-fg-2">
                  <span className="size-[7px] shrink-0" style={{ background: TONES[s.tone ?? 0] }} />
                  <span className="truncate" title={s.label}>{s.label}</span>
                </span>
                <span className="shrink-0 font-mono tabular-nums text-fg">{p ? format(p.v) : "—"}</span>
              </div>
            ))}
          </div>
        )}
      </div>
      {clean.length > 1 && !empty ? (
        <div className="mt-3 flex min-h-5 flex-wrap items-center gap-x-4 gap-y-1">
          {clean.map((s) => {
            const off = hidden.has(s.id);
            return (
              <button
                key={s.id}
                type="button"
                aria-pressed={!off}
                onClick={() =>
                  setHidden((h) => {
                    const next = new Set(h);
                    if (off) next.delete(s.id);
                    else next.add(s.id);
                    return next;
                  })
                }
                className={`flex max-w-full items-center gap-2 py-0.5 text-[12px] ${off ? "text-fg-3 line-through" : "text-fg-2 hover:text-fg"}`}
              >
                <span className="size-[7px] shrink-0" style={{ background: off ? "transparent" : TONES[s.tone ?? 0], border: `1px solid ${TONES[s.tone ?? 0]}` }} />
                <span className="truncate" title={s.label}>{s.label}</span>
              </button>
            );
          })}
        </div>
      ) : (
        (reserveLegend || clean.length > 1) && <div className="mt-3 h-5" aria-hidden="true" />
      )}
    </div>
  );
});
