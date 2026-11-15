import type { ReactNode } from "react";

type Props = { label: ReactNode; value: ReactNode; unit?: ReactNode; secondary?: ReactNode; title?: string };

export function Stat({ label, value, unit, secondary, title }: Props) {
  return (
    <div className="min-w-0" title={title ?? (typeof value === "string" ? value : undefined)}>
      <div className="truncate font-mono text-[11px] leading-4 uppercase tracking-[0.08em] text-fg-3">{label}</div>
      <div className="mt-2 flex items-baseline gap-1.5 font-mono tabular-nums text-fg">
        <span className="min-w-0 truncate text-[clamp(22px,7vw,32px)] leading-none tracking-[-0.03em] md:text-[36px]">{value}</span>
        {unit && <span className="shrink-0 text-[13px] text-fg-2">{unit}</span>}
      </div>
      {secondary && <div className="mt-2 text-[13px] leading-5 text-fg-2 [overflow-wrap:anywhere]">{secondary}</div>}
    </div>
  );
}
