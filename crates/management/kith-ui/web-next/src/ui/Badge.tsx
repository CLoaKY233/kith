import type { ReactNode } from "react";

type Tone = "neutral" | "strong" | "danger";

const TONE: Record<Tone, string> = {
  neutral: "border-line-2 text-fg-2",
  strong: "border-fg text-fg",
  danger: "border-hot text-hot",
};

export function Badge({ tone = "neutral", title, children }: { tone?: Tone; title?: string; children: ReactNode }) {
  return (
    <span title={title} className={`inline-flex h-5 max-w-full shrink-0 items-center leading-none whitespace-nowrap border px-1.5 font-mono text-[11px] uppercase tracking-[0.08em] ${TONE[tone]}`}>
      <span className="min-w-0 truncate">{children}</span>
    </span>
  );
}
