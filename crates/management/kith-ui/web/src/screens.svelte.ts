// Screens and sheets outside the first load (spec §1.4). Each loads when first shown and
// stays loaded; `loaded` holds it from then on, so later renders are synchronous. The
// screen a page opens on is fetched before the first paint, so it renders whole and
// nothing shifts while its code arrives (§11.9).

import type { Boot } from './lib/types.gen';

const loaders = {
  access: () => import('./routes/Access.svelte'),
  account: () => import('./frame/Account.svelte'),
  login: () => import('./routes/Login.svelte'),
  people: () => import('./routes/People.svelte'),
  activity: () => import('./routes/Activity.svelte'),
  devices: () => import('./routes/Devices.svelte'),
  exits: () => import('./routes/Exits.svelte'),
  names: () => import('./routes/Names.svelte'),
  settings: () => import('./routes/Settings.svelte'),
  this: () => import('./routes/ThisDevice.svelte'),
  welcome: () => import('./routes/Welcome.svelte'),
  addDevice: () => import('./sheets/AddDevice.svelte'),
  addPhone: () => import('./sheets/AddPhone.svelte'),
  invite: () => import('./sheets/Invite.svelte'),
  remove: () => import('./sheets/RemoveDevice.svelte'),
  need: () => import('./frame/NeedSheet.svelte'),
  needsCard: () => import('./lib/kith/NeedsYouCard.svelte'),
} as const;

export type ScreenKey = keyof typeof loaders;
export type ScreenModule<K extends ScreenKey> = Awaited<ReturnType<(typeof loaders)[K]>>;
type Loaded = { readonly [K in ScreenKey]?: ScreenModule<K> };

class Screens {
  /** Every module loaded so far. */
  all = $state.raw<Loaded>({});
}

export const loaded = new Screens();
/** Loads started, so each module loads once; not state, nothing renders from it. */
const loading: Partial<Record<ScreenKey, Promise<unknown>>> = {};

/** Loads one screen or sheet once; later calls share the first load. */
export function load<K extends ScreenKey>(key: K): Promise<ScreenModule<K>> {
  let promise = loading[key];
  if (!promise) {
    promise = loaders[key]().then((module) => {
      loaded.all = { ...loaded.all, [key]: module };
      return module;
    });
    loading[key] = promise;
    // A failed load may be retried, after a reconnect say.
    promise.catch(() => {
      loading[key] = undefined;
    });
  }
  return promise as Promise<ScreenModule<K>>;
}

const PAGES = ['access', 'activity', 'devices', 'exits', 'names', 'people', 'settings'] as const;

/** Fetches what the first screen at `pathname` shows; never fails. */
export function preload(pathname: string, boot: Boot | null): Promise<void> {
  const [first, , page] = pathname.split('/').filter(Boolean);
  const loads: Promise<unknown>[] = [];
  const networks = boot?.status.networks.length ?? 0;
  const pending = (boot?.pending.joins.length ?? 0) + (boot?.pending.blocked.length ?? 0);
  if (first === 'this') loads.push(load('this'));
  else if (first === 'welcome' || (boot !== null && networks === 0)) loads.push(load('welcome'));
  else if (first === 'n' && page !== undefined) {
    const known = PAGES.find((name) => name === page);
    if (known) loads.push(load(known));
  } else if (pending > 0) {
    // Overview opens with its Needs-you cards.
    loads.push(load('needsCard'));
  }
  return Promise.all(loads).then(
    () => undefined,
    () => undefined,
  );
}
