// Routes of spec §4.1 (the server allowlist), with pushState and no library. Overlays are
// query parameters (`?add=`, `?need=`, `?remove=`) so Back closes them.

export type Route =
  | { name: 'root' }
  | { name: 'welcome' }
  | { name: 'this' }
  | { name: 'overview'; net: string }
  | { name: 'devices'; net: string }
  | { name: 'device'; net: string; dev: string }
  | { name: 'activity'; net: string }
  | { name: 'names'; net: string }
  | { name: 'exits'; net: string }
  | { name: 'settings'; net: string }
  | { name: 'not_found' };

const NET_PAGES = ['devices', 'activity', 'names', 'exits', 'settings'] as const;

export function parse(pathname: string): Route {
  const parts = pathname.split('/').filter(Boolean).map(decodeURIComponent);
  const [first, net, page, dev, ...rest] = parts;
  if (rest.length > 0) return { name: 'not_found' };
  if (first === undefined) return { name: 'root' };
  if (first === 'welcome' && net === undefined) return { name: 'welcome' };
  if (first === 'this' && net === undefined) return { name: 'this' };
  if (first !== 'n' || net === undefined) return { name: 'not_found' };
  if (page === undefined) return { name: 'overview', net };
  if (page === 'devices' && dev !== undefined) return { name: 'device', net, dev };
  if (dev !== undefined) return { name: 'not_found' };
  const known = NET_PAGES.find((p) => p === page);
  return known ? { name: known, net } : { name: 'not_found' };
}

export function href(route: Route): string {
  const n = (net: string): string => `/n/${encodeURIComponent(net)}`;
  switch (route.name) {
    case 'root':
    case 'not_found':
      return '/';
    case 'welcome':
      return '/welcome';
    case 'this':
      return '/this';
    case 'overview':
      return n(route.net);
    case 'device':
      return `${n(route.net)}/devices/${encodeURIComponent(route.dev)}`;
    default:
      return `${n(route.net)}/${route.name}`;
  }
}

type Listener = (route: Route, query: URLSearchParams) => void;
const listeners = new Set<Listener>();

function emit(): void {
  const route = parse(location.pathname);
  const query = new URLSearchParams(location.search);
  for (const listener of listeners) listener(route, query);
}

if (typeof window !== 'undefined') window.addEventListener('popstate', emit);

/** Calls `listener` now and on every navigation. Returns the unsubscribe function. */
export function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  listener(parse(location.pathname), new URLSearchParams(location.search));
  return () => listeners.delete(listener);
}

export function go(to: string, options: { replace?: boolean } = {}): void {
  if (options.replace) history.replaceState(null, '', to);
  else history.pushState(null, '', to);
  emit();
}

/** Sets or clears query parameters on the current path; Back undoes it. */
export function setQuery(
  changes: Record<string, string | null>,
  options: { replace?: boolean } = {},
): void {
  const next = new URLSearchParams(location.search);
  for (const [key, value] of Object.entries(changes)) {
    if (value === null) next.delete(key);
    else next.set(key, value);
  }
  const search = next.toString();
  go(`${location.pathname}${search ? `?${search}` : ''}`, options);
}
