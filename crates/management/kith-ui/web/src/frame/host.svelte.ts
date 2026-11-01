// The name this device will use when it starts or joins its first network (spec §6.1
// `Rename`). It starts as the host name; Welcome's Rename changes it for this visit, and
// the CreateNetwork and JoinNetwork sheets read it.

import { live } from '../lib/live/store.svelte';

class Host {
  #chosen = $state<string | null>(null);

  get name(): string {
    return this.#chosen ?? live.status?.host.name ?? '';
  }
  set name(value: string) {
    this.#chosen = value;
  }

  get os(): string {
    return live.status?.host.os ?? '';
  }
}

export const host = new Host();
