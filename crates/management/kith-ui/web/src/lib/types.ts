// Types the page uses that the server does not send. Server types are in types.gen.ts,
// generated from Rust; never edit those by hand.

import type {
  ApiErrorCode,
  Conflict,
  Entry,
  EntryKind,
  LinkTick,
  OutcomeState,
  PathKind,
  RateBand,
  Role,
} from './types.gen';

/**
 * What an invite link says before joining with it (§7.2 step 2), from
 * `POST /api/join/preview`. Not generated yet: the server side is still to come.
 */
export type InvitePreview = {
  readonly network_name: string;
  readonly invited_by: string;
  readonly expires_at_ms: number;
};

/** §8.2: the page's connection to kith-ui and the daemon. */
export type ConnState = 'live' | 'catching_up' | 'stale' | 'daemon_down' | 'ui_gone' | 'unauth';

/** Why the page shows a Gate screen instead of the app (§6.9). */
export type GateReason =
  | 'daemon_down'
  | 'not_in_group'
  | 'version_skew'
  | 'unauthorized'
  | 'session_used'
  | 'session_expired';

/** §8.6: where a ledger op or other action is. */
export type ActionState =
  | 'idle'
  | 'pending'
  | 'awaiting_os_auth'
  | 'accepted'
  | 'seen'
  | 'conflict'
  | 'failed';

/** How many devices have heard about an op (`Spreading · n of m`). */
export type Spread = { readonly by: number; readonly of: number };

/** An API failure, normalised by api.ts. */
export type ApiFailure = {
  readonly code: ApiErrorCode;
  readonly status: number;
  readonly message: string;
  readonly conflict: Conflict | null;
};

/**
 * One running action, returned by every `actions.*` call. Reactive: components read its
 * fields in markup and re-render as it moves through pending, accepted and seen.
 */
export interface ActionHandle<T = void> {
  readonly state: ActionState;
  /** Set when `state` is `failed`. */
  readonly error: ApiFailure | null;
  /** Set when `state` is `conflict`: who won the race, and how (§7.3 step 7). */
  readonly conflict: Conflict | null;
  /** Propagation, once the journal or acks report it; `null` without acks ([fallback]). */
  readonly seen: Spread | null;
  /** True once SEEN_TIMEOUT_MS passed with no seen: show `savedWaiting`. */
  readonly slow: boolean;
  /** It signs a ledger op, so the page shows `Signing…` and `Saved on this device`. */
  readonly ledger: boolean;
  /** The 2xx body, once accepted. */
  readonly result: T | null;
  /**
   * Resolves when the request settles (accepted, awaiting_os_auth, conflict, failed, or
   * idle when a newer call replaced it); `seen` can follow later.
   */
  readonly done: Promise<ActionHandle<T>>;
}

export type Range = '1m' | '15m' | '1h';
export type Theme = 'system' | 'light' | 'dark';
export type Density = 'comfortable' | 'compact';
export type DevicesView = 'list' | 'map';
export type DevicesFilter = 'online' | 'offline' | 'relayed' | 'phones' | 'exits';
export type ActivityKind = 'changes' | 'joins' | 'paths' | 'blocked' | 'logs';
export type AddKind = 'device' | 'phone' | 'invite';

/** §5 StatusDot. */
export type DotState = 'online' | 'offline' | 'stale' | 'problem' | 'needs_you' | 'waiting';

/** §5 Button. */
export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger-quiet' | 'danger';

/** §5 NeedsYouCard kinds. */
export type NeedKind = 'join' | 'blocked' | 'health';

/** A Needs-you item, already filtered to what this person can act on. */
export type Need = { readonly id: string; readonly kind: NeedKind; readonly network: string };

/** The kind tag of a journal entry. */
export type EntryKindTag = EntryKind['kind'];

/** A journal entry the page holds, in seq order. */
export type JournalEntry = Entry;

/** The latest numbers for one device, committed together on each tick (§8.3). */
export type PeerLive = {
  readonly rx: number;
  readonly tx: number;
  readonly rttMs: number | null;
  readonly loss: number | null;
  readonly band: RateBand | null;
  readonly path: PathKind | null;
  readonly relay: string | null;
};

/** Columnar traffic for one device: up to TRAFFIC_SAMPLES samples (§8.2). */
export type TrafficColumns = {
  readonly at: Float64Array;
  readonly rx: Float64Array;
  readonly tx: Float64Array;
  /** Number of valid samples, oldest first. */
  readonly length: number;
};

export type { LinkTick, Role };

/** Per-viewer preferences kept in localStorage. */
export type Prefs = {
  readonly theme: Theme;
  readonly density: Density;
  readonly shortcuts: boolean;
  readonly notifications: boolean;
  readonly lastNetwork: string | null;
};

/** Outcome states the server returns, re-exported for screens. */
export type { OutcomeState };
