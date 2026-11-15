import { useSyncExternalStore } from "react";

let time: number | null = null;
const listeners = new Set<() => void>();

function subscribe(fn: () => void) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function setCrosshair(t: number | null) {
  if (t === time) return;
  time = t;
  listeners.forEach((fn) => fn());
}

export function useCrosshair(): [number | null, (t: number | null) => void] {
  return [useSyncExternalStore(subscribe, () => time), setCrosshair];
}
