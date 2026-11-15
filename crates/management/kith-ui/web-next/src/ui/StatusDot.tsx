export type Status = "online" | "offline" | "problem";

const LOOK: Record<Status, string> = { online: "dot-on", offline: "dot-off", problem: "dot-problem" };

export function StatusDot({ status, label }: { status: Status; label?: string }) {
  return <span role="img" aria-label={label ?? status} title={label ?? status} className={LOOK[status]} />;
}
