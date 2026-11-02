// Browser notifications (spec §7.12), the opt-in half. Off by default. When something that
// would have notified arrives while the tab is hidden, Activity shows an inline card
// asking to turn them on; `Not now` hides it for 7 days. Settings has the master switch.

import { Delivery, type Notice, type NotifyEnv, type Permission } from './notify';
import { NOTIFY_PROMPT_SNOOZE_MS, NOTIFY_REPLAY_CUTOFF_MS } from './timing';

const ENABLED_KEY = 'kith.notifications';
const SNOOZE_KEY = 'kith.notifications.prompt-snoozed-until';
// Where earlier versions of the page kept the switch and the snooze; read once.
const OLD_PREFS_KEY = 'kith.prefs';
const OLD_SNOOZE_KEY = 'kith.notify-prompt-until';

function oldSwitch(raw: string | null): boolean {
  try {
    const prefs: unknown = raw ? JSON.parse(raw) : null;
    return typeof prefs === 'object' && prefs !== null && (prefs as Record<string, unknown>)['notifications'] === true;
  } catch {
    return false;
  }
}

/** The opt-in state and delivery. Settings binds `on`; Activity shows `prompt`. */
export class Notifications {
  /** The person turned them on (stored per browser). */
  enabled = $state(false);
  permission = $state<Permission>('default');
  /** Something would have notified while the tab was hidden. */
  missed = $state(false);
  /** Server ms until which the Activity card stays hidden. */
  snoozedUntil = $state(0);

  private readonly env: NotifyEnv;
  private readonly delivery: Delivery;

  constructor(env: NotifyEnv) {
    this.env = env;
    this.delivery = new Delivery(env);
    const stored = env.load(ENABLED_KEY);
    this.enabled = stored === null ? oldSwitch(env.load(OLD_PREFS_KEY)) : stored === 'on';
    if (stored === null && this.enabled) env.save(ENABLED_KEY, 'on');
    let snooze = env.load(SNOOZE_KEY);
    if (snooze === null) {
      snooze = env.load(OLD_SNOOZE_KEY);
      if (snooze !== null) env.save(SNOOZE_KEY, snooze);
    }
    this.snoozedUntil = Number(snooze ?? 0) || 0;
    this.permission = env.permission();
  }

  /** On, and the browser allows it. */
  get on(): boolean {
    return this.enabled && this.permission === 'granted';
  }

  /** The browser can show notifications at all. */
  get supported(): boolean {
    return this.permission !== 'unsupported';
  }

  /** The card may be offered: off, the browser could still allow it, not snoozed. */
  get offerable(): boolean {
    return (
      !this.on &&
      this.permission !== 'denied' &&
      this.permission !== 'unsupported' &&
      this.env.now() >= this.snoozedUntil
    );
  }

  /** §7.12: show the `Get told when something needs you?` card in Activity. */
  get prompt(): boolean {
    return this.missed && this.offerable;
  }

  /** Turns notifications on or off. Turning on asks the browser; false if it refused. */
  async setEnabled(on: boolean): Promise<boolean> {
    if (!on) {
      this.enabled = false;
      this.env.save(ENABLED_KEY, 'off');
      return true;
    }
    let permission = this.env.permission();
    if (permission === 'default') permission = await this.env.requestPermission();
    this.permission = permission;
    this.enabled = permission === 'granted';
    this.env.save(ENABLED_KEY, this.enabled ? 'on' : 'off');
    if (this.enabled) this.missed = false;
    return this.enabled;
  }

  /** The card's `Turn on notifications`. */
  turnOn(): Promise<boolean> {
    return this.setEnabled(true);
  }

  /** The card's `Not now`: hidden for NOTIFY_PROMPT_SNOOZE_MS. */
  notNow(): void {
    this.snoozedUntil = this.env.now() + NOTIFY_PROMPT_SNOOZE_MS;
    this.env.save(SNOOZE_KEY, String(this.snoozedUntil));
    this.missed = false;
  }

  /** Delivers a notice if the rules allow it, or remembers it for the prompt. */
  deliver(notice: Notice): void {
    const now = this.env.now();
    if (now - notice.atMs > NOTIFY_REPLAY_CUTOFF_MS) return;
    if (this.env.focused()) return;
    this.permission = this.env.permission();
    if (!this.on) {
      if (this.env.hidden()) this.missed = true;
      return;
    }
    this.delivery.show(notice);
  }
}
