// Routes of spec §4.1 (the server allowlist), with pushState and no library.
//
// Inside the page a route's `net` is always the network's id: the store and every action
// are keyed by id. The URL addresses a network by name, or by a short id when two names
// collide; `parse` and `href` translate with the list App.svelte passes to `setNetworks`.
//
// Overlays are query parameters so Back closes them: `?add=device|phone|invite`,
// `?need=<id>`, `?remove=<dev>`, `?sheet=network` (the phone Network sheet), and on
// Welcome `?flow=start|join`.

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
  | { name: 'access'; net: string }
  | { name: 'settings'; net: string }
  | { name: 'not_found' };

/** A page that belongs to one network. */
export type NetPage = Extract<Route, { net: string }>['name'];

/** The query keys that open overlays; closing one goes Back when the page opened it. */
export type OverlayKey = 'add' | 'need' | 'remove' | 'flow' | 'sheet';

const NET_PAGES = ['devices', 'activity', 'names', 'exits', 'access', 'settings'] as const;
/** Characters of the id used when two networks share a name. */
const SHORT_ID = 8;

type Network = { readonly id: string; readonly name: string };
let networks: readonly Network[] = [];

/** The URL segment for a network: its name, or a short id when the name is shared. */
export function slug(id: string): string {
  const net = networks.find((n) => n.id === id);
  if (!net) return id;
  const shared = networks.filter((n) => n.name === net.name).length > 1;
  return shared ? id.slice(0, SHORT_ID) : net.name;
}

/** The network id for a URL segment, or null when no network matches. */
export function resolve(segment: string): string | null {
  const byName = networks.filter((n) => n.name === segment);
  if (byName.length === 1) return byName[0]?.id ?? null;
  const byId = networks.filter((n) => n.id === segment || n.id.startsWith(segment));
  return segment.length >= SHORT_ID && byId.length === 1 ? (byId[0]?.id ?? null) : null;
}

export function parse(pathname: string): Route {
  let parts: string[];
  try {
    parts = pathname.split('/').filter(Boolean).map(decodeURIComponent);
  } catch {
    return { name: 'not_found' };
  }
  const [first, segment, page, dev, ...rest] = parts;
  if (rest.length > 0) return { name: 'not_found' };
  if (first === undefined) return { name: 'root' };
  if (first === 'welcome' && segment === undefined) return { name: 'welcome' };
  if (first === 'this' && segment === undefined) return { name: 'this' };
  if (first !== 'n' || segment === undefined) return { name: 'not_found' };
  const net = resolve(segment);
  if (net === null) return { name: 'not_found' };
  if (page === undefined) return { name: 'overview', net };
  if (page === 'devices' && dev !== undefined) return { name: 'device', net, dev };
  if (dev !== undefined) return { name: 'not_found' };
  const known = NET_PAGES.find((p) => p === page);
  return known ? { name: known, net } : { name: 'not_found' };
}

export function href(route: Route): string {
  const n = (net: string): string => `/n/${encodeURIComponent(slug(net))}`;
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

/** `href` for a page of a network, with an optional query. */
export function netHref(
  net: string,
  page: Exclude<NetPage, 'device'>,
  query: Record<string, string> = {},
): string {
  const path = href({ name: page, net });
  const search = new URLSearchParams(query).toString();
  return search ? `${path}?${search}` : path;
}

type Listener = (route: Route, query: URLSearchParams) => void;
const listeners = new Set<Listener>();

function emit(): void {
  const route = parse(location.pathname);
  const query = new URLSearchParams(location.search);
  for (const listener of listeners) listener(route, query);
}

if (typeof window !== 'undefined') window.addEventListener('popstate', emit);

/** Tells the router which networks exist, and re-parses the current URL. */
export function setNetworks(next: readonly Network[]): void {
  const same =
    next.length === networks.length &&
    next.every((n, i) => n.id === networks[i]?.id && n.name === networks[i]?.name);
  if (same) return;
  networks = next.map((n) => ({ id: n.id, name: n.name }));
  if (typeof window !== 'undefined') emit();
}

/** Calls `listener` now and on every navigation. Returns the unsubscribe function. */
export function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  listener(parse(location.pathname), new URLSearchParams(location.search));
  return () => listeners.delete(listener);
}

type HistoryMark = { readonly overlay?: boolean };

export function go(to: string, options: { replace?: boolean; overlay?: boolean } = {}): void {
  const state: HistoryMark | null = options.overlay ? { overlay: true } : null;
  if (options.replace) history.replaceState(state, '', to);
  else history.pushState(state, '', to);
  emit();
}

/** Same-origin link clicks go through the router; modified clicks keep the browser's way. */
export function follow(event: MouseEvent, to: string): void {
  if (event.defaultPrevented || event.button !== 0) return;
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  event.preventDefault();
  go(to);
}

function withQuery(changes: Record<string, string | null>): string {
  const next = new URLSearchParams(location.search);
  for (const [key, value] of Object.entries(changes)) {
    if (value === null) next.delete(key);
    else next.set(key, value);
  }
  const search = next.toString();
  return `${location.pathname}${search ? `?${search}` : ''}`;
}

/** Sets or clears query parameters on the current path; Back undoes it. */
export function setQuery(
  changes: Record<string, string | null>,
  options: { replace?: boolean } = {},
): void {
  go(withQuery(changes), options);
}

/** Opens an overlay (`?add=device`, `?need=<id>`, ...) as a history entry Back closes. */
export function openOverlay(key: OverlayKey, value: string): void {
  go(withQuery({ [key]: value }), { overlay: true });
}

/**
 * Closes an overlay. When this page opened it, that is Back, so Forward reopens it;
 * otherwise (a pasted link) the parameter is dropped in place.
 */
export function closeOverlay(key: OverlayKey): void {
  if (!new URLSearchParams(location.search).has(key)) return;
  const mark = history.state as HistoryMark | null;
  if (mark?.overlay) history.back();
  else go(withQuery({ [key]: null }), { replace: true });
}
