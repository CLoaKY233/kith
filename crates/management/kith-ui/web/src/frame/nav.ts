// The navigation of spec §4.2: Observing (Overview, Devices, Activity), then Shaping
// (Names, Exits), then Add device, with Settings pinned to the bottom. The sidebar, the
// rail, the mobile Network sheet and the palette's Go to group all read this list.

import { copy } from '../lib/copy';
import type { IconName } from '../lib/icons/names';
import type { NetPage, Route } from '../router';

export type NavPage = Exclude<NetPage, 'device'>;
export type NavItem = { readonly page: NavPage; readonly label: string; readonly icon: IconName };

export const OBSERVING: readonly NavItem[] = [
  { page: 'overview', label: copy.nav.overview, icon: 'activity' },
  { page: 'devices', label: copy.nav.devices, icon: 'monitor-smartphone' },
  { page: 'activity', label: copy.nav.activity, icon: 'list' },
];

export const SHAPING: readonly NavItem[] = [
  { page: 'names', label: copy.nav.names, icon: 'at-sign' },
  { page: 'exits', label: copy.nav.exits, icon: 'globe' },
  { page: 'access', label: copy.nav.access, icon: 'key-round' },
];

export const SETTINGS: NavItem = { page: 'settings', label: copy.nav.settings, icon: 'settings-2' };

/** The nav page a route belongs to: a device's detail lives under Devices. */
export function section(route: Route): NavPage | null {
  switch (route.name) {
    case 'device':
      return 'devices';
    case 'overview':
    case 'devices':
    case 'activity':
    case 'names':
    case 'exits':
    case 'access':
    case 'settings':
      return route.name;
    default:
      return null;
  }
}

/** The screen title in the header (§4.2). */
export function title(route: Route): string {
  const page = section(route);
  if (page) return [...OBSERVING, ...SHAPING, SETTINGS].find((i) => i.page === page)?.label ?? '';
  return route.name === 'this' ? copy.nav.thisDevice : '';
}
