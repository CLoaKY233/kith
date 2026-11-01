// The single store (spec §8.2). Public surface, frozen for the other workstreams:
//
//   live.*     readonly reactive state: read it in markup or $derived, never assign it.
//   actions.*  every change the page can make; each returns an ActionHandle.
//   start()    hydrates from the boot snapshot and opens the stream; main.ts calls it.
//   onEntry()  calls back with each new journal entry, after the frame that applied it.
//
// The state and its live wiring are in core.svelte.ts; this file makes the page's one
// instance with the browser's fetch, EventSource and frames.

import { api, ApiError, networkPath } from '../api';
import type {
  AllowPreview,
  Boot,
  Conflict,
  Entry,
  ExitView,
  Invite,
  JoinRequestState,
  NameCheck,
  Outcome,
  PhoneConfig,
  PhoneCreated,
  RecordType,
  RecordView,
} from '../types.gen';
import type { ActionHandle, ActionState, ApiFailure, Spread } from '../types';
import { readInlineBoot } from './boot';
import { Store, viewOf, type Deps, type Live } from './core.svelte';
import { connect } from './stream';
import { SEEN_TIMEOUT_MS } from './timing';

const browser: Deps = {
  get: (path) => api.get(path),
  connect: (resume, on) => connect(resume, on),
  frame: (flush) => requestAnimationFrame(() => flush()),
  visible: () => typeof document === 'undefined' || document.visibilityState !== 'hidden',
  watchVisibility: (changed) => {
    if (typeof document === 'undefined') return;
    document.addEventListener('visibilitychange', () =>
      changed(document.visibilityState !== 'hidden'),
    );
  },
};

const store = new Store(browser);

export type { Live };
export type { JournalGap } from './core.svelte';
export type { ShownPath } from './stability.svelte';

/** The store, read-only. */
export const live: Live = viewOf(store);

class Handle<T> implements ActionHandle<T> {
  state = $state<ActionState>('pending');
  error = $state<ApiFailure | null>(null);
  conflict = $state<Conflict | null>(null);
  seen = $state<Spread | null>(null);
  slow = $state(false);
  result = $state.raw<T | null>(null);
  readonly done: Promise<ActionHandle<T>>;

  constructor(run: () => Promise<T>) {
    const slow = setTimeout(() => {
      if (this.state === 'accepted') this.slow = true;
    }, SEEN_TIMEOUT_MS);
    this.done = run().then(
      (value) => {
        this.result = value;
        const outcome = value as Partial<Outcome> | null;
        this.state = outcome?.state === 'awaiting_os_auth' ? 'awaiting_os_auth' : 'accepted';
        return this;
      },
      (cause: unknown) => {
        clearTimeout(slow);
        const failure =
          cause instanceof ApiError
            ? cause
            : new ApiError('internal', 0, cause instanceof Error ? cause.message : String(cause), null);
        if (failure.code === 'conflict' && failure.conflict) {
          this.conflict = failure.conflict;
          this.state = 'conflict';
        } else {
          this.error = {
            code: failure.code,
            status: failure.status,
            message: failure.message,
            conflict: failure.conflict,
          };
          this.state = 'failed';
        }
        return this;
      },
    );
  }
}

function act<T>(run: () => Promise<T>): ActionHandle<T> {
  if (store.conn === 'stale' || store.conn === 'daemon_down') {
    // §8.5: nothing is queued while stale.
    return new Handle<T>(() =>
      Promise.reject(new ApiError('daemon_down', 0, 'stale', null)),
    );
  }
  return new Handle(run);
}

function load<T>(path: string, into: (value: T) => void): ActionHandle<T> {
  return new Handle(async () => {
    const value = await api.get<T>(path);
    into(value);
    return value;
  });
}

const enc = encodeURIComponent;

/** Every change and on-demand read. Each returns an ActionHandle (§16 C). */
export const actions = {
  // Reads that are not in the boot snapshot.
  loadRecords: (net: string) =>
    load<RecordView[]>(networkPath(net, '/records'), (v) => (store.records[net] = v)),
  loadExits: (net: string) =>
    load<ExitView[]>(networkPath(net, '/exits'), (v) => (store.exits[net] = v)),
  loadInvites: (net: string) =>
    load<Invite[]>(networkPath(net, '/invites'), (v) => (store.invites[net] = v)),
  validateName: (net: string | null, name: string) =>
    new Handle(() =>
      api.get<NameCheck>(`/api/validate/name?net=${enc(net ?? '')}&name=${enc(name)}`),
    ) as ActionHandle<NameCheck>,

  // Networks (§7.1, §7.2).
  createNetwork: (name: string, deviceName: string) =>
    act(() => api.post<Outcome>('/api/networks', { name, device_name: deviceName })),
  join: (inviteLink: string, deviceName: string) =>
    act(() => api.post<Outcome>('/api/join', { invite_link: inviteLink, device_name: deviceName })),
  requestJoin: (requestCode: string, deviceName: string) =>
    act(() =>
      api.post<JoinRequestState>('/api/join/request', {
        request_code: requestCode,
        device_name: deviceName,
      }),
    ),
  leave: (net: string) => act(() => api.post<Outcome>(networkPath(net, '/leave'))),

  // Join requests (§7.3).
  approveJoin: (net: string, node: string, name: string) =>
    act(() => api.post<Outcome>(networkPath(net, `/joins/${enc(node)}/approve`), { name })),
  denyJoin: (net: string, node: string) =>
    act(() => api.post<Outcome>(networkPath(net, `/joins/${enc(node)}/deny`))),

  // Invites (§7.4).
  createInvite: (net: string, expiresSecs: number, uses: number | null) =>
    act(() => api.post<Invite>(networkPath(net, '/invites'), { expires_secs: expiresSecs, uses })),
  revokeInvite: (net: string, id: string) =>
    act(() => api.post<Outcome>(networkPath(net, `/invites/${enc(id)}/revoke`))),

  // Blocked connections (§7.5).
  previewAllow: (id: string) =>
    act(() => api.post<AllowPreview>(`/api/blocked/${enc(id)}/allow`, { preview: true })),
  allowBlocked: (id: string) =>
    act(() => api.post<Outcome>(`/api/blocked/${enc(id)}/allow`, { preview: false })),

  // Phones (§7.6).
  createPhone: (net: string, name: string, tags: string[]) =>
    act(() => api.post<PhoneCreated>(networkPath(net, '/phones'), { name, tags })),
  phoneConfig: (net: string, node: string, everything: boolean) =>
    act(() => api.post<PhoneConfig>(networkPath(net, `/phones/${enc(node)}/config`), { everything })),

  // Names (§6.6).
  setRecord: (net: string, name: string, type: RecordType, value: string, basedOn: string | null = null) =>
    act(() => api.post<Outcome>(networkPath(net, '/records'), { name, type, value, based_on: basedOn })),
  deleteRecord: (net: string, name: string) =>
    act(() => api.post<Outcome>(networkPath(net, `/records/${enc(name)}/delete`))),

  // Exits (§7.8). Optimistic in the screen; the store keeps the server's truth.
  useExit: (net: string | null, node: string | null) =>
    act(() => api.post<Outcome>('/api/exit', { network: net, node })),
  grantExit: (net: string, node: string, allowed: boolean) =>
    act(() => api.post<Outcome>(networkPath(net, `/exits/${enc(node)}`), { allowed })),

  // Devices (§7.9, §7.11).
  renameDevice: (net: string, node: string, name: string, basedOn: string | null = null) =>
    act(() =>
      api.post<Outcome>(networkPath(net, `/devices/${enc(node)}/rename`), { name, based_on: basedOn }),
    ),
  removeDevice: (net: string, node: string, lost: boolean) =>
    act(() => api.post<Outcome>(networkPath(net, `/devices/${enc(node)}/remove`), { lost })),
} as const;

export type Actions = typeof actions;

/** Hydrates from `#boot` (or /api/boot) and opens the stream. Safe to call more than once.
 * With an inline snapshot the state is complete before this returns, so the first frame
 * has it (§8.1). */
export function start(): Promise<void> {
  return store.start(readInlineBoot(typeof document === 'undefined' ? null : document));
}

/** Calls `listener` with each new journal entry; returns a function that stops it. */
export function onEntry(listener: (entry: Entry) => void): () => void {
  return store.onEntry(listener);
}

/** Test hook: loads a snapshot without a server. */
export function hydrateForTest(snapshot: Boot): void {
  store.hydrate(snapshot);
}
