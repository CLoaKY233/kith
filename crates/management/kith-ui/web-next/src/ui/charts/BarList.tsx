import type { ReactNode } from "react";

export type BarItem = { id: string; label: ReactNode; value: number };

export function BarList({ items, format, limit }: { items: BarItem[]; format: (v: number) => string; limit?: number }) {
  const rows = items
    .filter((r) => Number.isFinite(r.value))
    .sort((a, b) => b.value - a.value)
    .slice(0, limit);
  const max = rows[0]?.value > 0 ? rows[0].value : 1;
  return (
    <ol role="list" className="flex list-none flex-col gap-3 p-0">
      {rows.map((r) => (
        <li key={r.id} className="grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-3 gap-y-1">
          <span className="min-w-0 truncate text-[13px] text-fg" title={typeof r.label === "string" ? r.label : undefined}>{r.label}</span>
          <span className="whitespace-nowrap text-right font-mono text-[12px] tabular-nums text-fg-2">{format(r.value)}</span>
          <span className="col-span-2 h-1 bg-ink-3" aria-hidden="true">
            <span className="ui-bar block h-full bg-fg-2" style={{ width: `${(Math.max(0, r.value) / max) * 100}%` }} />
          </span>
        </li>
      ))}
    </ol>
  );
}
