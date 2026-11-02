// The name this device will use when it starts or joins its first network (spec §6.1
// `Rename`). It starts as the host name; Welcome's Rename changes it for this tab (kept in
// sessionStorage, so a reload keeps it), and the CreateNetwork and JoinNetwork sheets read it.

import { live } from '../lib/live/store.svelte';

const KEY = 'kith.welcome-name';

function read(): string | null {
  try {
    return sessionStorage.getItem(KEY);
  } catch {
    return null;
  }
}

class Host {
  #chosen = $state<string | null>(read());

  get name(): string {
    return this.#chosen ?? live.status?.host.name ?? '';
  }
  set name(value: string) {
    this.#chosen = value;
    try {
      sessionStorage.setItem(KEY, value);
    } catch {
      // Storage is off; the name lasts until the page reloads.
    }
  }

  get os(): string {
    return live.status?.host.os ?? '';
  }
}

export const host = new Host();
