// What a device's chosen settings mean on screen: whether it has picked them up yet, the
// tags the list shows, and who may change them.

import type { DeviceSettingsInfo, ExitView, PersonView } from '../types.gen';

export type Applied =
  | { kind: 'own' }
  | { kind: 'paused' }
  | { kind: 'using'; exit: string }
  | { kind: 'offline' }
  | { kind: 'waiting' }
  | { kind: 'set'; exit: string };

export type AppliedInput = {
  settings: DeviceSettingsInfo;
  /** The device's name in the network. */
  name: string;
  online: boolean;
  /** The network's exits, as the exit side reports who uses them. */
  exits: readonly ExitView[];
  /** For the Console's own device: `exit_node` from the status (`network/device`), or null. */
  usedNow?: string | null;
  network?: string;
  /** The device that serves this page: only an exit it is can report who uses it. */
  observer?: string | null;
};

/**
 * Honest progress: `using` only once the exit reports the device among its users (or, for
 * the Console's own device, once the status names that exit); until then `offline` or
 * `waiting`, never a guess.
 */
export function appliedOf(input: AppliedInput): Applied {
  const { settings, exits, name, online } = input;
  const chosen = settings.exit === null ? null : (exits.find((e) => e.node === settings.exit) ?? null);
  if (chosen !== null && !settings.paused) {
    const reported =
      input.usedNow !== undefined
        ? input.usedNow === `${input.network ?? ''}/${chosen.name}`
        : chosen.in_use_by.includes(name);
    if (reported) return { kind: 'using', exit: chosen.name };
  }
  if (!online) return { kind: 'offline' };
  if (settings.paused) return { kind: 'paused' };
  if (settings.exit === null) return { kind: 'own' };
  // Another device's exit traffic is invisible here: say it is set, not that it waits.
  if (chosen !== null && input.usedNow === undefined && chosen.node !== input.observer) {
    return { kind: 'set', exit: chosen.name };
  }
  return { kind: 'waiting' };
}

/** Whether the saved settings say anything worth a tag. */
export function isDefault(settings: DeviceSettingsInfo | null | undefined): boolean {
  return !settings || (settings.exit === null && !settings.exit_locked && !settings.paused);
}

/** The exits a device is offered: allowed ones that are not the device itself, plus the one already chosen. */
export function choosable(exits: readonly ExitView[], node: string, chosen: string | null): ExitView[] {
  return exits
    .filter((e) => e.node !== node && (e.allowed || e.node === chosen))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** Whether `node` is one of the signed-in person's devices, by the people list. */
export function owns(people: readonly PersonView[] | null, node: string): boolean {
  return people?.some((p) => p.me && p.devices.includes(node)) ?? false;
}

export function same(a: DeviceSettingsInfo, b: DeviceSettingsInfo): boolean {
  return a.exit === b.exit && a.exit_locked === b.exit_locked && a.paused === b.paused;
}
