// Per-viewer preferences (spec §6.8): theme, compact rows and keyboard shortcuts, kept in
// localStorage; the notification switch lives in lib/live/notifications.svelte.ts.
// Storage can be missing or throw (private windows, blocked site data), so every read and
// write is guarded and the page works without it. The theme and density apply as `data-theme` and `data-density` on <html>,
// which tokens.css reads; importing this module applies them once.

import type { Density, Prefs, Theme } from './types';

const KEY = 'kith.prefs';

const DEFAULTS: Prefs = {
  theme: 'system',
  density: 'comfortable',
  shortcuts: true,
  lastNetwork: null,
};

const THEMES: readonly Theme[] = ['system', 'light', 'dark'];
const DENSITIES: readonly Density[] = ['comfortable', 'compact'];

function load(): Prefs {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(KEY);
  } catch {
    return DEFAULTS;
  }
  if (raw === null) return DEFAULTS;
  let parsed: Partial<Record<keyof Prefs, unknown>>;
  try {
    parsed = JSON.parse(raw) as Partial<Record<keyof Prefs, unknown>>;
  } catch {
    return DEFAULTS;
  }
  return {
    theme: THEMES.find((t) => t === parsed.theme) ?? DEFAULTS.theme,
    density: DENSITIES.find((d) => d === parsed.density) ?? DEFAULTS.density,
    shortcuts: typeof parsed.shortcuts === 'boolean' ? parsed.shortcuts : DEFAULTS.shortcuts,
    lastNetwork: typeof parsed.lastNetwork === 'string' ? parsed.lastNetwork : null,
  };
}

function save(value: Prefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(value));
  } catch {
    // Kept for this page only.
  }
}

let current = $state<Prefs>(load());

/** The viewer's preferences, reactive. */
export const prefs: Readonly<Prefs> = {
  get theme() {
    return current.theme;
  },
  get density() {
    return current.density;
  },
  get shortcuts() {
    return current.shortcuts;
  },
  get lastNetwork() {
    return current.lastNetwork;
  },
};

/** Sets `data-theme` and `data-density` on <html> from the preferences. */
export function applyPrefs(): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  if (current.theme === 'system') delete root.dataset['theme'];
  else root.dataset['theme'] = current.theme;
  if (current.density === 'compact') root.dataset['density'] = 'compact';
  else delete root.dataset['density'];
}

/** Changes one preference, stores it and applies it. */
export function setPref<K extends keyof Prefs>(key: K, value: Prefs[K]): void {
  current = { ...current, [key]: value };
  save(current);
  applyPrefs();
}

applyPrefs();

// §7.12: the Activity card asks about notifications only after something that would have
// notified arrived while the tab was hidden, so the page remembers when it was hidden.
const hiddenSpans: { from: number; to: number | null }[] = [];

if (typeof document !== 'undefined') {
  const track = (): void => {
    const now = Date.now();
    const open = hiddenSpans[hiddenSpans.length - 1];
    if (document.visibilityState === 'hidden') {
      if (!open || open.to !== null) hiddenSpans.push({ from: now, to: null });
    } else if (open && open.to === null) open.to = now;
  };
  track();
  document.addEventListener('visibilitychange', track);
}

/** Whether the tab was hidden at `localMs` (this clock, not the server's). */
export function wasHiddenAt(localMs: number): boolean {
  return hiddenSpans.some((span) => localMs >= span.from && (span.to === null || localMs <= span.to));
}
