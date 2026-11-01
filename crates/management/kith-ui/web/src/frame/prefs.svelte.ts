// Per-viewer preferences (spec §6.8): theme, compact rows, single-key shortcuts, the
// notification switch, the last network and hidden needs. This is the only writer of
// `kith.prefs`. Kept in localStorage, every access wrapped in try/catch: a private window
// or blocked storage just means defaults. App.svelte applies the theme and density to
// <html>; Settings writes through `prefs`.

import { IGNORE_BLOCKED_MS, NOTIFY_PROMPT_SNOOZE_MS } from '../lib/live/timing';
import type { Density, Theme } from '../lib/types';

const KEY = 'kith.prefs';
const PROMPT_KEY = 'kith.notify-prompt-until';

type Stored = {
  theme: Theme;
  density: Density;
  shortcuts: boolean;
  notifications: boolean;
  lastNetwork: string | null;
  /** Need id → when it was hidden, in ms: ignored blocked groups, dismissed warnings, settled joins. */
  ignored: Record<string, number>;
};

const DEFAULTS: Stored = {
  theme: 'system',
  density: 'comfortable',
  shortcuts: true,
  notifications: false,
  lastNetwork: null,
  ignored: {},
};

const THEMES: readonly Theme[] = ['system', 'light', 'dark'];
const DENSITIES: readonly Density[] = ['comfortable', 'compact'];

function read(): Stored {
  let parsed: Partial<Record<keyof Stored, unknown>>;
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { ...DEFAULTS };
    parsed = JSON.parse(raw) as Partial<Record<keyof Stored, unknown>>;
  } catch {
    return { ...DEFAULTS };
  }
  const ignored: Record<string, number> = {};
  if (parsed.ignored && typeof parsed.ignored === 'object') {
    for (const [id, at] of Object.entries(parsed.ignored as Record<string, unknown>)) {
      if (typeof at === 'number') ignored[id] = at;
    }
  }
  return {
    theme: THEMES.find((t) => t === parsed.theme) ?? DEFAULTS.theme,
    density: DENSITIES.find((d) => d === parsed.density) ?? DEFAULTS.density,
    shortcuts: typeof parsed.shortcuts === 'boolean' ? parsed.shortcuts : DEFAULTS.shortcuts,
    notifications:
      typeof parsed.notifications === 'boolean' ? parsed.notifications : DEFAULTS.notifications,
    lastNetwork: typeof parsed.lastNetwork === 'string' ? parsed.lastNetwork : null,
    ignored,
  };
}

class Prefs {
  #state = $state<Stored>(read());

  get theme(): Theme {
    return this.#state.theme;
  }
  set theme(value: Theme) {
    this.#write({ theme: value });
  }
  get density(): Density {
    return this.#state.density;
  }
  set density(value: Density) {
    this.#write({ density: value });
  }
  /** Single-key shortcuts (`/`, `?`, `g o`, `A`, `D`); ⌘K and Esc always work. */
  get shortcuts(): boolean {
    return this.#state.shortcuts;
  }
  set shortcuts(value: boolean) {
    this.#write({ shortcuts: value });
  }
  get notifications(): boolean {
    return this.#state.notifications;
  }
  set notifications(value: boolean) {
    this.#write({ notifications: value });
  }
  get lastNetwork(): string | null {
    return this.#state.lastNetwork;
  }
  set lastNetwork(value: string | null) {
    if (value !== this.#state.lastNetwork) this.#write({ lastNetwork: value });
  }

  /** Whether a need was hidden less than a day ago (§7.5 `Hidden for a day`). */
  ignoredAt(id: string, nowMs: number): boolean {
    const at = this.#state.ignored[id];
    return at !== undefined && nowMs - at < IGNORE_BLOCKED_MS;
  }

  ignore(id: string, nowMs: number): void {
    const ignored: Record<string, number> = {};
    for (const [key, at] of Object.entries(this.#state.ignored)) {
      if (nowMs - at < IGNORE_BLOCKED_MS) ignored[key] = at;
    }
    ignored[id] = nowMs;
    this.#write({ ignored });
  }

  unignore(id: string): void {
    const ignored = Object.fromEntries(
      Object.entries(this.#state.ignored).filter(([key]) => key !== id),
    );
    this.#write({ ignored });
  }

  #write(change: Partial<Stored>): void {
    this.#state = { ...this.#state, ...change };
    try {
      localStorage.setItem(KEY, JSON.stringify(this.#state));
    } catch {
      // Storage is off; the choice lasts until the tab closes.
    }
  }
}

export const prefs = new Prefs();

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

/** Whether `Not now` on the notification card still holds. */
export function promptSnoozed(nowMs: number): boolean {
  try {
    const until = Number(localStorage.getItem(PROMPT_KEY));
    return Number.isFinite(until) && until > nowMs;
  } catch {
    return false;
  }
}

/** `Not now`: hides the notification card for NOTIFY_PROMPT_SNOOZE_MS. */
export function snoozePrompt(nowMs: number): void {
  try {
    localStorage.setItem(PROMPT_KEY, String(nowMs + NOTIFY_PROMPT_SNOOZE_MS));
  } catch {
    // Hidden for this page only.
  }
}
