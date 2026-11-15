import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import type { Boot, Tick } from '@kith/types';
import { api, errorMessage, ApiError } from './api';

type Live = {
  boot: Boot | null;
  error: string | null;
  /** The session is gone: `/api/boot` answered 401. */
  signedOut: boolean;
  refresh: () => void;
  /** Link ticks seen since the page opened (one per poll, newest last, at most an hour). */
  ticks: readonly Tick[];
};
const LiveContext = createContext<Live>({ boot: null, error: null, signedOut: false, refresh: () => {}, ticks: [] });

const TICKS_KEPT = 1800;

/** Keeps a fresh snapshot of the network: status, peers, traffic, pending, every 2 s. */
export function LiveProvider({ children }: { children: ReactNode }) {
  const [boot, setBoot] = useState<Boot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [signedOut, setSignedOut] = useState(false);
  const [tick, setTick] = useState(0);
  const [ticks, setTicks] = useState<readonly Tick[]>([]);
  useEffect(() => {
    let alive = true;
    let seq = 0;
    let shown = 0;
    const load = () => {
      if (document.hidden) return;
      const mine = ++seq;
      api
        .get<Boot>('/api/boot')
        .then((b) => {
          // A slow answer must not overwrite a newer one.
          if (!alive || mine < shown) return;
          shown = mine;
          setBoot(b);
          setError(null);
          const latest = b.last_tick;
          if (latest)
            setTicks((kept) =>
              kept.at(-1)?.at_ms === latest.at_ms ? kept : [...kept.slice(-(TICKS_KEPT - 1)), latest],
            );
        })
        .catch((e: unknown) => {
          if (!alive || mine < shown) return;
          if (e instanceof ApiError && e.status === 401) {
            setSignedOut(true);
            alive = false;
            clearInterval(timer);
          }
          setError(errorMessage(e));
        });
    };
    load();
    const timer = setInterval(load, 2000);
    const wake = () => {
      if (!document.hidden) load();
    };
    document.addEventListener('visibilitychange', wake);
    return () => {
      alive = false;
      clearInterval(timer);
      document.removeEventListener('visibilitychange', wake);
    };
  }, [tick]);
  const refresh = useCallback(() => setTick((t) => t + 1), []);
  const value = useMemo(() => ({ boot, error, signedOut, ticks, refresh }), [boot, error, signedOut, ticks, refresh]);
  return <LiveContext.Provider value={value}>{children}</LiveContext.Provider>;
}

export const useLive = () => useContext(LiveContext);

/** One network's id, name and peers from the snapshot (the first network by default). */
export function useNetwork() {
  const { boot } = useLive();
  return useMemo(() => {
    const network = boot?.status.networks[0] ?? null;
    const peers = network ? (boot?.peers[network.id] ?? []) : [];
    return { boot, network, peers };
  }, [boot]);
}

/** A GET that reloads on demand. Data from another path never shows under this one. */
export function useFetch<T>(path: string | null, every = 0) {
  const [state, setState] = useState<{ path: string | null; data: T | null; error: string | null }>({
    path,
    data: null,
    error: null,
  });
  const [n, setN] = useState(0);
  const latest = useRef(0);
  useEffect(() => {
    if (!path) return;
    let alive = true;
    const load = () => {
      const mine = ++latest.current;
      api
        .get<T>(path)
        .then((d) => alive && mine === latest.current && setState({ path, data: d, error: null }))
        .catch((e: unknown) => alive && mine === latest.current && setState((s) => ({ path, data: s.path === path ? s.data : null, error: errorMessage(e) })));
    };
    load();
    const timer = every ? setInterval(() => !document.hidden && load(), every) : undefined;
    const wake = () => {
      if (!document.hidden) load();
    };
    if (every) document.addEventListener('visibilitychange', wake);
    return () => {
      alive = false;
      if (timer) clearInterval(timer);
      document.removeEventListener('visibilitychange', wake);
    };
  }, [path, every, n]);
  const current = state.path === path;
  const reload = useCallback(() => setN((x) => x + 1), []);
  return { data: current ? state.data : null, error: current ? state.error : null, reload };
}
