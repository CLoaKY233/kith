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
  readonly lastNetwork: string | null;
};

/** Outcome states the server returns, re-exported for screens. */
export type { OutcomeState };

// Endpoints whose types are not generated yet; the shapes are the server's (§2).

/** An open invite, from `GET /api/networks/:net/invites`. */
export type OpenInvite = {
  readonly id: string;
  readonly expires_at_ms: number;
  /** Uses still open; `null` is unlimited. */
  readonly uses_left: number | null;
  readonly created_at_ms: number;
};

/** Which devices a policy selector names. */
export type PolicySelector =
  | { readonly kind: 'any' }
  | { readonly kind: 'device'; readonly node: string }
  | { readonly kind: 'tag'; readonly tag: string };

export type PolicyProtocol = 'any' | 'tcp' | 'udp' | 'icmp';

/** An inclusive port range. */
export type PortRange = { readonly start: number; readonly end: number };

/** One destination of a rule; `ports` null is every port. */
export type PolicyTarget = {
  readonly selector: PolicySelector;
  readonly protocol: PolicyProtocol;
  readonly ports: readonly PortRange[] | null;
};

/** One allow rule: these sources may reach these destinations. */
export type PolicyRule = {
  readonly sources: readonly PolicySelector[];
  readonly destinations: readonly PolicyTarget[];
};

/** `GET /api/networks/:net/policy`; lib/kith/policy.ts reads `rules` leniently. */
export type PolicyView = { readonly toml: string; readonly rules: readonly PolicyRule[] };

/** `POST /api/networks/:net/policy`. */
export type PolicyIn = { readonly toml: string };

/** A relay the network advertises; `node` is the device that runs it, if one does. */
export type Relay = { readonly name: string; readonly address: string; readonly node: string | null };

/** `GET` and `POST /api/server`: what this device offers its networks. */
export type ServerMode = { readonly exit: boolean; readonly relay: boolean };

/** `POST /api/wireguard/import`. */
export type WireguardImportIn = { readonly name: string; readonly conf_text: string };
export type WireguardImported = { readonly network: string };

/** `POST /api/networks/:net/devices/:node/rename`. */
export type RenameDeviceIn = { readonly name: string };

// The Console: people with passkeys (docs/plans/console.md). Not generated yet.

/** The three roles a person can hold. */
export type PersonRole = 'owner' | 'admin' | 'member';

/** Where kith-ui runs: on this computer, or reachable from anywhere with passkeys. */
export type ConsoleMode = 'local' | 'public';

/** Who is signed in, from `/api/status` and `/api/login/finish`. */
export type Me = { readonly id: string; readonly name: string; readonly role: PersonRole };

/** What `/api/status` adds for the Console; both are absent from an older server. */
export type ConsoleStatus = { readonly me?: Me | null; readonly mode?: ConsoleMode };

export type PersonDevice = { readonly node: string; readonly name: string; readonly online: boolean };

/** `GET /api/people`. `pending` is a person who has not opened their link yet. */
export type Person = {
  readonly id: string;
  readonly name: string;
  readonly role: PersonRole;
  readonly pending: boolean;
  readonly devices: readonly PersonDevice[];
};

/** `POST /api/people`. */
export type AddPersonIn = { readonly name: string; readonly role: PersonRole };
export type PersonAdded = {
  readonly id: string;
  readonly link: string;
  /** The link as a QR code, when the server sends one (the page has no QR library). */
  readonly qr_svg?: string | null;
};

/**
 * What `POST /api/authority/begin` asks the person's passkey to sign: a role change,
 * removing someone, or making a just-added person an admin.
 */
export type AuthorityChange =
  | { readonly kind: 'set_role'; readonly person: string; readonly role: PersonRole }
  | { readonly kind: 'remove_person'; readonly person: string }
  | { readonly kind: 'add_admin'; readonly person: string };

/** `POST /api/device-codes`: a code that joins one computer for `person`. */
export type DeviceCodeIn = { readonly person: string };
export type DeviceCode = {
  readonly code: string;
  readonly expires_at_ms: number;
  readonly qr_svg: string;
  /** `kith join https://<domain>/j/<code>`. */
  readonly command: string;
};
