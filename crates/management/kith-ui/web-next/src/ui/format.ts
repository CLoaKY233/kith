const UNITS = ["B", "KB", "MB", "GB", "TB"];
const DASH = "—";

// Rounds to 3 significant digits, so 99.96 reads "100" rather than "100.0".
function sig(v: number): string {
  const d = v >= 100 ? 0 : v >= 10 ? 1 : 2;
  const r = Number(v.toFixed(d));
  return r >= 100 ? r.toFixed(0) : r >= 10 ? r.toFixed(Math.min(d, 1)) : r.toFixed(d);
}

function scaled(n: number, units: string[], step: number): string {
  if (!Number.isFinite(n)) return DASH;
  let i = 0;
  let v = Math.abs(n);
  while (Math.round(v) >= step && i < units.length - 1) {
    v /= step;
    i++;
  }
  const out = i === 0 ? String(Math.round(v)) : sig(v);
  return `${n < 0 && Number(out) !== 0 ? "-" : ""}${out} ${units[i]}`;
}

export const bytes = (n: number) => scaled(n, UNITS, 1024);
export const rate = (n: number) => (Number.isFinite(n) ? `${bytes(n)}/s` : DASH);
export const ms = (n: number) => {
  if (!Number.isFinite(n)) return DASH;
  if (n >= 999.5) return `${(n / 1000).toFixed(n >= 9950 ? 0 : 1)} s`;
  return n < 9.95 ? `${n.toFixed(1)} ms` : `${Math.round(n)} ms`;
};
export const percent = (n: number) => {
  if (!Number.isFinite(n)) return DASH;
  if (n !== 0 && Math.abs(n) < 0.05) return "<0.1%";
  return n !== 0 && Math.abs(n) < 9.95 ? `${n.toFixed(1)}%` : `${Math.round(n)}%`;
};
export const count = (n: number) => (!Number.isFinite(n) ? DASH : Number.isInteger(n) ? n.toLocaleString() : n.toFixed(1));

const pad = (n: number) => String(n).padStart(2, "0");
export const clock = (t: number) => {
  const d = new Date(t);
  if (Number.isNaN(d.getTime())) return DASH;
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

export const absolute = (t: number) =>
  Number.isFinite(t) ? new Date(t).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "medium" }) : DASH;

export function relative(t: number, now = Date.now()): string {
  if (!Number.isFinite(t)) return DASH;
  const s = Math.round((now - t) / 1000);
  if (s < 10) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}
