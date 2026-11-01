// Browser notification rules (spec §7.12); notifications.svelte.ts holds the opt-in.
//
// Notifies: a join request (admins only), the exit going offline, a recently-used device
// going offline, the daemon stopped for more than 10 s. Never: path changes, records,
// logs, online events. Rules: one tag per kind and subject; more than 3 in 30 s coalesce
// into one; never while the tab is focused; never for entries older than 30 s.

import { copy } from '../copy';
import type { Entry } from '../types.gen';
import { NOTIFY_COALESCE_COUNT, NOTIFY_COALESCE_WINDOW_MS } from './timing';

export type NoticeKind = 'join' | 'exit_offline' | 'device_offline' | 'daemon_stopped';

/** One notification the page would show. */
export type Notice = {
  readonly kind: NoticeKind;
  /** `kith:<kind>:<subject>`: a newer notice about the same thing replaces the older. */
  readonly tag: string;
  readonly title: string;
  readonly body: string;
  /** Server time of the event, in ms. */
  readonly atMs: number;
  /** For a join: the network and the Needs-you id that a click opens at `?need=`. */
  readonly need: { readonly network: string; readonly id: string } | null;
};

/** What the rules read from the store. */
export interface NoticeContext {
  /** The network's name, for copy and URLs. */
  networkName(network: string): string | null;
  /** This device is an admin of the network. */
  isAdmin(network: string): boolean;
  /** This device's exit as `<network name>/<device name>` (`StatusView.exit_node`). */
  exitNode(): string | null;
  /** The device moved data within RECENTLY_USED_MS before `atMs`. */
  usedRecently(network: string, node: string, atMs: number): boolean;
}

/** §7.12: the notice an entry would give, or null. Pure; the age and focus rules are applied on delivery. */
export function noticeFor(entry: Entry, ctx: NoticeContext): Notice | null {
  switch (entry.kind) {
    case 'join_requested': {
      if (!ctx.isAdmin(entry.network)) return null;
      const network = ctx.networkName(entry.network) ?? entry.network;
      return {
        kind: 'join',
        tag: `kith:join:${entry.network}:${entry.node}`,
        title: copy.notify.join(entry.name, network),
        body: copy.notify.joinBody,
        atMs: entry.at_ms,
        // The server names a join's Needs-you item `join:<network>:<node>`.
        need: { network: entry.network, id: `join:${entry.network}:${entry.node}` },
      };
    }
    case 'peer_offline': {
      const network = ctx.networkName(entry.network);
      const exit = ctx.exitNode();
      const isExit =
        exit !== null &&
        ((network !== null && exit === `${network}/${entry.name}`) ||
          exit === `${entry.network}/${entry.node}`);
      if (isExit) {
        return {
          kind: 'exit_offline',
          tag: `kith:exit_offline:${entry.network}:${entry.node}`,
          title: copy.notify.exitOffline(entry.name),
          body: '',
          atMs: entry.at_ms,
          need: null,
        };
      }
      if (!ctx.usedRecently(entry.network, entry.node, entry.at_ms)) return null;
      return {
        kind: 'device_offline',
        tag: `kith:device_offline:${entry.network}:${entry.node}`,
        title: copy.notify.deviceOffline(entry.name),
        body: '',
        atMs: entry.at_ms,
        need: null,
      };
    }
    default:
      return null;
  }
}

/** §7.12: the daemon has been stopped for DAEMON_STOPPED_NOTIFY_MS. */
export function daemonNotice(atMs: number): Notice {
  return {
    kind: 'daemon_stopped',
    tag: 'kith:daemon_stopped',
    title: copy.notify.daemonStopped,
    body: '',
    atMs,
    need: null,
  };
}

const COALESCED_TAG = 'kith:coalesced';

export type Permission = NotificationPermission | 'unsupported';

/** A shown notification, as far as the rules need it. */
export interface Shown {
  close(): void;
}

/** The browser, injectable so the rules run in tests. */
export interface NotifyEnv {
  /** Server time now, in ms. */
  now(): number;
  /** The tab is hidden (another tab, minimised). */
  hidden(): boolean;
  /** The tab is visible and has focus. */
  focused(): boolean;
  permission(): Permission;
  requestPermission(): Promise<Permission>;
  show(notice: Pick<Notice, 'title' | 'body' | 'tag'>, onClick: () => void): Shown | null;
  /** Brings the page forward, at the Needs-you item when there is one. */
  open(need: Notice['need']): void;
  load(key: string): string | null;
  save(key: string, value: string): void;
}

/** Coalescing and the shown notifications by tag (§7.12 rules); no reactive state. */
export class Delivery {
  private readonly env: NotifyEnv;
  private recent: number[] = [];
  private shown = new Map<string, Shown>();

  constructor(env: NotifyEnv) {
    this.env = env;
  }

  /** Shows a notice, or one coalesced notice once more than 3 arrive within 30 s. */
  show(notice: Notice): void {
    const now = this.env.now();
    this.recent = this.recent.filter((at) => now - at < NOTIFY_COALESCE_WINDOW_MS);
    this.recent.push(now);
    if (this.recent.length > NOTIFY_COALESCE_COUNT) {
      for (const [tag, shown] of this.shown) {
        if (tag !== COALESCED_TAG) {
          shown.close();
          this.shown.delete(tag);
        }
      }
      this.put({ title: copy.notify.coalesced(this.recent.length), body: '', tag: COALESCED_TAG }, null);
      return;
    }
    this.put(notice, notice.need);
  }

  private put(notice: Pick<Notice, 'title' | 'body' | 'tag'>, need: Notice['need']): void {
    this.shown.get(notice.tag)?.close();
    const shown = this.env.show(notice, () => {
      this.shown.delete(notice.tag);
      this.env.open(need);
    });
    if (shown) this.shown.set(notice.tag, shown);
    else this.shown.delete(notice.tag);
  }
}

/** The real browser, for the page. `now` is server time; `open` routes inside the app. */
export function browserEnv(now: () => number, open: (need: Notice['need']) => void): NotifyEnv {
  const supported = typeof window !== 'undefined' && 'Notification' in window;
  return {
    now,
    hidden: () => typeof document !== 'undefined' && document.visibilityState === 'hidden',
    focused: () =>
      typeof document !== 'undefined' && document.visibilityState === 'visible' && document.hasFocus(),
    permission: () => (supported ? Notification.permission : 'unsupported'),
    requestPermission: async () => {
      if (!supported) return 'unsupported';
      try {
        return await Notification.requestPermission();
      } catch {
        return Notification.permission;
      }
    },
    show: (notice, onClick) => {
      if (!supported) return null;
      try {
        const shown = new Notification(notice.title, {
          body: notice.body,
          tag: notice.tag,
        });
        shown.onclick = () => {
          window.focus();
          shown.close();
          onClick();
        };
        return shown;
      } catch {
        // Some browsers (Chrome on Android) only allow notifications from a service worker.
        return null;
      }
    },
    open,
    load: (key) => {
      try {
        return localStorage.getItem(key);
      } catch {
        return null;
      }
    },
    save: (key, value) => {
      try {
        localStorage.setItem(key, value);
      } catch {
        // Private windows and blocked storage: the choice lasts for this page only.
      }
    },
  };
}
