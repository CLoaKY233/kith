// What the page shows about its connection (spec §8.5). The store's `live.conn` changes
// at once; people see it only after the spec's delays: `Reconnecting…` after 1 s of
// catching up (never on first load), stale and daemon-down after 3 more seconds. Rows
// read `connection.stale` to show `—` and hollow dots; actions read `connection.frozen`.

import { untrack } from 'svelte';
import { announce } from '../lib/announce';
import { copy } from '../lib/copy';
import { clock } from '../lib/format';
import { live } from '../lib/live/store.svelte';
import { BANNER_DELAY_MS, CATCHING_UP_QUIET_MS } from '../lib/live/timing';
import { toast } from '../lib/ui/Toast.svelte';

export type Shown = 'live' | 'reconnecting' | 'stale' | 'daemon_down' | 'ui_gone' | 'unauth';

class Connection {
  shown = $state<Shown>('live');
  /** Server time of the last tick before things went quiet, for `as of 14:02`. */
  asOf = $state(0);

  /** Values are last known: rates read `—`, dots go hollow. */
  get stale(): boolean {
    return this.shown === 'stale' || this.shown === 'daemon_down' || this.shown === 'ui_gone';
  }

  /** Nothing can change: actions are aria-disabled with `Can't change anything…`. */
  get frozen(): boolean {
    return this.stale || this.shown === 'unauth';
  }

  /** Call once from App.svelte's setup; it owns the timers and announcements. */
  track(): void {
    let wasLive = false;
    let lastBoot = '';

    $effect(() => {
      const conn = live.conn;
      if (conn === 'live') wasLive = true;
      const show = (next: Shown, after: number): (() => void) | undefined => {
        if (after <= 0) {
          this.shown = next;
          return undefined;
        }
        const timer = setTimeout(() => (this.shown = next), after);
        return () => clearTimeout(timer);
      };
      switch (conn) {
        case 'live':
          return show('live', 0);
        case 'catching_up':
          return wasLive ? show('reconnecting', CATCHING_UP_QUIET_MS) : undefined;
        case 'stale':
          this.asOf = untrack(() => live.lastTickMs);
          return show('stale', BANNER_DELAY_MS);
        case 'daemon_down':
          this.asOf = untrack(() => live.lastTickMs);
          return show('daemon_down', BANNER_DELAY_MS);
        case 'ui_gone':
          return show('ui_gone', 0);
        case 'unauth':
          return show('unauth', 0);
      }
    });

    let down = false;
    $effect(() => {
      const now = this.shown;
      if (now === 'daemon_down' && !down) {
        down = true;
        announce(copy.conn.daemonLost, 'assertive');
      } else if (now === 'live' && down) {
        down = false;
        announce(copy.conn.daemonBack, 'assertive');
      }
    });

    $effect(() => {
      const boot = live.boot;
      if (lastBoot !== '' && boot !== '' && boot !== lastBoot) {
        untrack(() => toast.done(copy.conn.reset(clock(live.now()))));
      }
      if (boot !== '') lastBoot = boot;
    });
  }
}

export const connection = new Connection();
