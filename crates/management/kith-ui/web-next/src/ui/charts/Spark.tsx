type Props = { values: number[]; width?: number; height?: number; label?: string };

export function Spark({ values, width = 80, height = 20, label }: Props) {
  const vs = values.map((v) => (Number.isFinite(v) ? Math.max(0, v) : 0));
  const a11y = label ? { role: "img" as const, "aria-label": label } : { "aria-hidden": true as const };
  const box = { width, height, viewBox: `0 0 ${width} ${height}`, preserveAspectRatio: "none" as const, style: { maxWidth: "100%", height: "auto", aspectRatio: `${width} / ${height}` } };
  if (vs.length < 2) return <svg {...box} className="block shrink-0" {...a11y} />;
  const max = vs.reduce((m, v) => Math.max(m, v), 0) || 1;
  const step = width / (vs.length - 1);
  const d = vs.map((v, i) => `${i ? "L" : "M"}${(i * step).toFixed(1)},${(height - 1 - (v / max) * (height - 2)).toFixed(1)}`).join("");
  return (
    <svg {...box} className="block shrink-0" {...a11y}>
      <path d={d} fill="none" stroke="var(--color-fg-2)" strokeWidth={1} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}
