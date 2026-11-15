import type { ReactNode } from "react";

type PanelProps = {
  label?: ReactNode;
  right?: ReactNode;
  span?: 3 | 4 | 6 | 8 | 12;
  className?: string;
  children?: ReactNode;
};

const SPAN = { 3: "lg:col-span-3", 4: "lg:col-span-4", 6: "lg:col-span-6", 8: "lg:col-span-8", 12: "lg:col-span-12" };

export function Panel({ label, right, span, className = "", children }: PanelProps) {
  return (
    <div className={`ui-panel min-w-0 border border-line ${span ? `col-span-12 ${SPAN[span]}` : ""} ${className}`}>
      {(label || right) && (
        <div className="flex min-h-11 items-center justify-between gap-3 border-b border-line px-4 py-1">
          <span title={typeof label === "string" ? label : undefined} className="min-w-0 truncate font-mono text-[11px] leading-4 uppercase tracking-[0.08em] text-fg-3">{label}</span>
          {right && <div className="flex shrink-0 items-center gap-2 font-mono text-[13px] tabular-nums text-fg">{right}</div>}
        </div>
      )}
      <div className="min-w-0 p-4">{children}</div>
    </div>
  );
}

export function PanelGrid({ children }: { children: ReactNode }) {
  return <div className="ui-panel-grid grid grid-cols-12">{children}</div>;
}
