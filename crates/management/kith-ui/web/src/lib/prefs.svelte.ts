// Per-viewer preferences (spec §6.8): theme, compact rows, keyboard shortcuts and the
// notification switch, kept in localStorage. Storage can be missing or throw (private
// windows, blocked site data), so every read and write is guarded and the page works
// without it. The theme and density apply as `data-theme` and `data-density` on <html>,
// which tokens.css reads; importing this module applies them once.

import type { Density, Prefs, Theme } from './types';

const KEY = 'kith.prefs';

const DEFAULTS: Prefs = {
  theme: 'system',
  density: 'comfortable',
  shortcuts: true,
  notifications: false,
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
    notifications:
      typeof parsed.notifications === 'boolean' ? parsed.notifications : DEFAULTS.notifications,
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
  get notifications() {
    return current.notifications;
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
