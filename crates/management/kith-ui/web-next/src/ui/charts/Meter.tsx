import { Meter as SdkMeter, MeterLabel, MeterTrack, MeterValue } from "@ui-sdk/react/meter";

type Props = { label: string; value: number; max?: number; segments?: number; danger?: boolean };

export function Meter({ label, value, max = 100, segments = 20, danger }: Props) {
  const top = max > 0 ? max : 100;
  const v = Number.isFinite(value) ? Math.min(Math.max(value, 0), top) : 0;
  const filled = v > 0 ? Math.max(1, Math.round((v / top) * segments)) : 0;
  return (
    <SdkMeter value={v} max={top} className="flex flex-col gap-2">
      <div className="flex items-baseline justify-between gap-3">
        <MeterLabel className="min-w-0 truncate font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">{label}</MeterLabel>
        <MeterValue className="shrink-0 whitespace-nowrap font-mono text-[13px] tabular-nums text-fg" />
      </div>
      <MeterTrack className="flex h-2 gap-[2px]">
        {Array.from({ length: segments }, (_, i) => (
          <span key={i} className={`flex-1 ${i < filled ? (danger ? "bg-hot" : "bg-fg") : "bg-ink-3"}`} />
        ))}
      </MeterTrack>
    </SdkMeter>
  );
}
