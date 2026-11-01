// Activity (spec §6.5, §12.3): what each journal entry says, which filter it belongs to,
// and the feed the Activity screen shows: newest first, with day and gap dividers,
// grouped blocked connections and path flaps, and the cap row. Pure functions of their
// inputs, so Overview, Device detail and This device reuse them and tests run without a page.

import { copy, copyScreens } from '../copy';
import { clock, day } from '../format';
import {
  BLOCKED_GROUP_MS,
  FLAP_COUNT,
  FLAP_WINDOW_MS,
  JOURNAL_CAP,
  RANGE_MS,
} from '../live/timing';
import type { ActivityKind, Range } from '../types';
import type { Entry, PathKind } from '../types.gen';

/** The filters of the Activity toolbar, in order; `null` is All. */
export const ACTIVITY_KINDS = ['changes', 'joins', 'paths', 'blocked', 'logs'] as const;

/** §5 EventRow: the glyph column holds one of these, or nothing. */
export type Glyph = 'ban' | 'triangle-alert' | 'seal';

/** Log levels as the daemon writes them. */
export type LogLevel = keyof typeof copy.activity.levels;

/** What the page knows that an entry does not carry. */
export interface Context {
  /** This device's name in a network (or its first name when `network` is null). */
  selfName(network: string | null): string | null;
  /** A network's name. */
  networkName(network: string): string | null;
  /** The relay a device's traffic goes through now, by name. */
  relayName(network: string, node: string): string | null;
}

/** An entry in words. */
export interface Described {
  readonly sentence: string;
  readonly glyph: Glyph | null;
  /** `reported by laptop` or `signed by laptop`. */
  readonly actor: string | null;
  /** True when the actor signed it, so the seal follows the actor. */
  readonly sealed: boolean;
}

/** A device an entry is about. */
export interface DeviceRef {
  readonly network: string;
  readonly node: string;
  readonly name: string;
}

function assertNever(value: never): never {
  throw new Error(`unhandled entry kind: ${JSON.stringify(value)}`);
}

/** The toolbar filter an entry belongs to. */
export function kindOf(entry: Entry): ActivityKind {
  switch (entry.kind) {
    case 'device_removed':
    case 'exit_changed':
      return 'changes';
    case 'device_joined':
    case 'join_requested':
      return 'joins';
    case 'peer_online':
    case 'peer_offline':
    case 'path_changed':
    case 'endpoint_changed':
      return 'paths';
    case 'blocked':
    case 'dropped':
      return 'blocked';
    case 'warning':
    case 'log':
      return 'logs';
    default:
      return assertNever(entry);
  }
}

/** The level of a log line or warning; `null` for every other entry. */
export function logLevel(entry: Entry): LogLevel | null {
  if (entry.kind === 'warning') return 'warn';
  if (entry.kind !== 'log') return null;
  const level = entry.level.toLowerCase();
  return level in copy.activity.levels ? (level as LogLevel) : 'info';
}

/**
 * §6.5: All shows every event plus `warn` and `error` lines; `info`, `debug` and `trace`
 * lines show only under Logs, which shows every line.
 */
export function showsUnder(entry: Entry, kind: ActivityKind | null): boolean {
  const level = logLevel(entry);
  if (kind === null) return level === null || level === 'warn' || level === 'error';
  return kindOf(entry) === kind;
}

/** The network an entry belongs to; `null` for this device's own (exit, logs, drops). */
export function networkOf(entry: Entry): string | null {
  return 'network' in entry ? entry.network : null;
}

/** The devices an entry is about, the one it is mostly about first. */
export function devicesOf(entry: Entry): DeviceRef[] {
  switch (entry.kind) {
    case 'peer_online':
    case 'peer_offline':
    case 'path_changed':
    case 'endpoint_changed':
    case 'device_joined':
    case 'device_removed':
    case 'join_requested':
      return [{ network: entry.network, node: entry.node, name: entry.name }];
    case 'blocked':
      return [
        { network: entry.network, node: entry.from, name: entry.from_name },
        { network: entry.network, node: entry.to, name: entry.to_name },
      ];
    case 'exit_changed':
    case 'dropped':
    case 'warning':
    case 'log':
      return [];
    default:
      return assertNever(entry);
  }
}

/** Whether an entry is about `device`, given as a device id or a name. */
export function aboutDevice(entry: Entry, device: string): boolean {
  const wanted = device.toLowerCase();
  return devicesOf(entry).some((d) => d.node === device || d.name.toLowerCase() === wanted);
}

function pathWords(path: PathKind | null, relay: string | null): string {
  if (path === 'direct') return copy.path.direct;
  if (path === 'relay') return relay ? copy.path.via(relay) : copyScreens.entry.pathWas.relay;
  if (path === 'stealth') {
    return relay ? copy.path.stealthVia(relay) : copyScreens.entry.pathWas.stealth;
  }
  return copyScreens.entry.pathWas.none;
}

/** `:22`, or ` (ICMP)` when there is no port. */
function target(port: number | null, protocol: string): string {
  return port === null
    ? copyScreens.entry.protocolSuffix(protocol)
    : copyScreens.entry.portSuffix(port);
}

function dropReason(reason: string): string {
  return copyScreens.entry.dropReasons[reason] ?? reason.replaceAll('_', ' ');
}

/** The device part of `network/device`. */
function exitDevice(exit: string): string {
  const slash = exit.lastIndexOf('/');
  return slash === -1 ? exit : exit.slice(slash + 1);
}

/** §12.3: one entry as a sentence, with its glyph and actor. */
export function describe(entry: Entry, ctx: Context): Described {
  const reported = (network: string | null): string | null => {
    const self = ctx.selfName(network);
    return self ? copy.activity.reportedBy(self) : null;
  };
  const plain = (sentence: string, glyph: Glyph | null = null): Described => ({
    sentence,
    glyph,
    actor: null,
    sealed: false,
  });
  switch (entry.kind) {
    case 'peer_online': {
      const relay = ctx.relayName(entry.network, entry.node);
      const sentence = entry.path
        ? copy.entry.peerOnline(entry.name, pathWords(entry.path, relay), null)
        : copyScreens.entry.peerOnlinePlain(entry.name);
      return { sentence, glyph: null, actor: reported(entry.network), sealed: false };
    }
    case 'peer_offline':
      return {
        sentence: copy.entry.peerOffline(entry.name),
        glyph: null,
        actor: reported(entry.network),
        sealed: false,
      };
    case 'path_changed': {
      const was = copyScreens.entry.pathWas[entry.from ?? 'none'];
      const relay = ctx.relayName(entry.network, entry.node);
      let sentence: string;
      if (entry.to === 'direct') sentence = copy.entry.pathDirect(entry.name, was);
      else if (entry.to === 'relay') {
        sentence = relay
          ? copy.entry.pathRelayed(entry.name, relay, was)
          : copyScreens.entry.pathRelayedPlain(entry.name, was);
      } else if (entry.to === 'stealth') {
        sentence = relay
          ? copy.entry.pathStealth(entry.name, relay, was)
          : copyScreens.entry.pathStealthPlain(entry.name, was);
      } else sentence = copyScreens.entry.pathLost(entry.name, was);
      return { sentence, glyph: null, actor: reported(entry.network), sealed: false };
    }
    case 'endpoint_changed':
      return {
        sentence: copy.entry.endpointChanged(entry.name),
        glyph: null,
        actor: reported(entry.network),
        sealed: false,
      };
    // The journal does not name who signed a join or removal yet, so the seal stands
    // alone in the glyph column instead of following `signed by …`.
    case 'device_joined':
      return plain(copy.entry.deviceJoinedPlain(entry.name), 'seal');
    case 'device_removed':
      return plain(copy.entry.deviceRemovedPlain(entry.name), 'seal');
    case 'exit_changed':
      return plain(
        entry.exit ? copy.entry.exitChanged(exitDevice(entry.exit)) : copy.entry.exitCleared,
      );
    case 'dropped':
      return plain(copy.entry.dropped(dropReason(entry.reason), entry.count), 'ban');
    case 'blocked':
      return {
        sentence: copy.entry.blocked(
          entry.from_name,
          entry.to_name,
          target(entry.port, entry.protocol),
          entry.count,
        ),
        glyph: 'ban',
        actor: reported(entry.network),
        sealed: false,
      };
    case 'warning':
      return plain(entry.message, 'triangle-alert');
    case 'log':
      return plain(entry.message);
    case 'join_requested':
      return plain(
        copy.entry.joinRequested(entry.name, ctx.networkName(entry.network) ?? entry.network),
      );
    default:
      return assertNever(entry);
  }
}

/** §6.5 grouping: the sentence for a group, entries newest first. */
export function describeGroup(entries: readonly Entry[], ctx: Context): Described | null {
  const newest = entries[0];
  const oldest = entries[entries.length - 1];
  if (!newest || !oldest) return null;
  if (newest.kind === 'blocked') {
    const count = entries.reduce((sum, e) => sum + (e.kind === 'blocked' ? e.count : 0), 0);
    const self = ctx.selfName(newest.network);
    return {
      sentence: copy.entry.blocked(
        newest.from_name,
        newest.to_name,
        target(newest.port, newest.protocol),
        count,
      ),
      glyph: 'ban',
      actor: self ? copy.activity.reportedBy(self) : null,
      sealed: false,
    };
  }
  if (newest.kind === 'path_changed') {
    const self = ctx.selfName(newest.network);
    return {
      sentence: copy.activity.flaps(newest.name, entries.length, clock(oldest.at_ms)),
      glyph: null,
      actor: self ? copy.activity.reportedBy(self) : null,
      sealed: false,
    };
  }
  return describe(newest, ctx);
}

/** The words a search matches: the sentence, plus a log line's module and level tag. */
export function searchText(entry: Entry, ctx: Context): string {
  const level = logLevel(entry);
  const parts = [describe(entry, ctx).sentence];
  if (entry.kind === 'log') parts.push(entry.target);
  if (level) parts.push(copy.activity.levels[level]);
  return parts.join(' ').toLowerCase();
}

/** The toolbar's filters, as the URL holds them. */
export interface Filter {
  /** Network id; entries of other networks are hidden. `null` shows every network. */
  readonly network: string | null;
  readonly kind: ActivityKind | null;
  /** A device id or name. */
  readonly device: string | null;
  readonly query: string;
}

/** Whether an entry passes the toolbar's filters. */
export function matches(entry: Entry, filter: Filter, ctx: Context): boolean {
  const network = networkOf(entry);
  if (filter.network !== null && network !== null && network !== filter.network) return false;
  if (!showsUnder(entry, filter.kind)) return false;
  if (filter.device && !aboutDevice(entry, filter.device)) return false;
  const q = filter.query.trim().toLowerCase();
  return q === '' || searchText(entry, ctx).includes(q);
}

/** One row of the feed. */
export type FeedRow =
  | { readonly type: 'day'; readonly key: string; readonly label: string }
  | { readonly type: 'gap'; readonly key: string; readonly text: string }
  | { readonly type: 'entry'; readonly key: string; readonly entry: Entry }
  | {
      readonly type: 'group';
      readonly key: string;
      /** Newest first. */
      readonly entries: readonly Entry[];
    }
  | { readonly type: 'cap'; readonly key: string; readonly text: string };

/** A missing seq range, as the stream reported it (§2.6 `gap`). */
export interface Gap {
  readonly from: number;
  readonly to: number;
  readonly at_ms: number;
}

function dayKey(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

/** Local midnight of the day `ms` falls in. */
export function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** `Today`, `Yesterday` or `Mon 31 Oct`, in the viewer's time zone. */
export function dayLabel(ms: number, nowMs: number): string {
  if (dayKey(ms) === dayKey(nowMs)) return copy.time.today;
  const yesterday = new Date(nowMs);
  yesterday.setDate(yesterday.getDate() - 1);
  if (dayKey(ms) === dayKey(yesterday.getTime())) return copy.time.yesterday;
  return day(ms);
}

type Unit = { entries: Entry[]; grouped: boolean };

/**
 * Groups entries (oldest first) into rows: blocked connections with the same source,
 * destination and port within BLOCKED_GROUP_MS of the group's first, and runs of more
 * than FLAP_COUNT path changes of one device each within FLAP_WINDOW_MS of the last.
 */
function group(entries: readonly Entry[]): Unit[] {
  const units: Unit[] = [];
  const blocked = new Map<string, Unit>();
  const runs = new Map<string, Unit>();
  for (const entry of entries) {
    if (entry.kind === 'blocked') {
      const key = `${entry.network}|${entry.from}|${entry.to}|${entry.protocol}|${entry.port ?? ''}`;
      const open = blocked.get(key);
      const first = open?.entries[0];
      if (open && first && entry.at_ms - first.at_ms <= BLOCKED_GROUP_MS) {
        open.entries.push(entry);
      } else {
        const unit = { entries: [entry], grouped: true };
        units.push(unit);
        blocked.set(key, unit);
      }
    } else if (entry.kind === 'path_changed') {
      const key = `${entry.network}|${entry.node}`;
      const run = runs.get(key);
      const last = run?.entries[run.entries.length - 1];
      if (run && last && entry.at_ms - last.at_ms <= FLAP_WINDOW_MS) run.entries.push(entry);
      else {
        const unit = { entries: [entry], grouped: true };
        units.push(unit);
        runs.set(key, unit);
      }
    } else units.push({ entries: [entry], grouped: false });
  }
  const out: Unit[] = [];
  for (const unit of units) {
    const first = unit.entries[0];
    const flaps = first?.kind === 'path_changed';
    const keep = unit.grouped && (flaps ? unit.entries.length > FLAP_COUNT : unit.entries.length > 1);
    if (keep) out.push(unit);
    else for (const entry of unit.entries) out.push({ entries: [entry], grouped: false });
  }
  return out;
}

/** The newest seq of a unit. */
function anchor(unit: Unit): Entry | undefined {
  return unit.entries[unit.entries.length - 1];
}

function gapText(gap: Gap, journal: readonly Entry[]): string {
  let before: Entry | undefined;
  let after: Entry | undefined;
  for (const entry of journal) {
    if (entry.seq < gap.from) before = entry;
    else if (entry.seq > gap.to && !after) after = entry;
  }
  return copy.activity.gap(clock(before?.at_ms ?? gap.at_ms), clock(after?.at_ms ?? gap.at_ms));
}

/**
 * §6.5: the feed, newest first. `journal` is the store's, oldest first and in seq order;
 * `shown` is the part of it to show (the whole journal, or up to the pause point).
 */
export function buildFeed(
  journal: readonly Entry[],
  shown: readonly Entry[],
  filter: Filter,
  ctx: Context,
  gaps: readonly Gap[],
  nowMs: number,
): FeedRow[] {
  const units = group(shown.filter((entry) => matches(entry, filter, ctx)));
  units.sort((a, b) => (anchor(b)?.seq ?? 0) - (anchor(a)?.seq ?? 0));
  const pending = [...gaps].sort((a, b) => b.from - a.from);
  const rows: FeedRow[] = [];
  let lastDay: string | null = null;
  const emitGaps = (olderThan: number): void => {
    while (pending.length > 0 && (pending[0]?.from ?? 0) > olderThan) {
      const gap = pending.shift();
      if (gap) rows.push({ type: 'gap', key: `gap${gap.from}`, text: gapText(gap, journal) });
    }
  };
  for (const unit of units) {
    const newest = anchor(unit);
    if (!newest) continue;
    emitGaps(newest.seq);
    const key = dayKey(newest.at_ms);
    if (key !== lastDay) {
      rows.push({ type: 'day', key: `day${key}`, label: dayLabel(newest.at_ms, nowMs) });
      lastDay = key;
    }
    if (unit.grouped) {
      const first = unit.entries[0];
      rows.push({
        type: 'group',
        key: `group${first?.seq ?? newest.seq}`,
        entries: [...unit.entries].reverse(),
      });
    } else rows.push({ type: 'entry', key: `entry${newest.seq}`, entry: newest });
  }
  emitGaps(Number.NEGATIVE_INFINITY);
  if (journal.length >= JOURNAL_CAP) rows.push({ type: 'cap', key: 'cap', text: copy.activity.cap });
  return rows;
}

/** §6.5 linking: the chart range that keeps `atMs` in view, or `null` past the last hour. */
export function rangeFor(atMs: number, nowMs: number): Range | null {
  const age = Math.max(0, nowMs - atMs);
  const ranges: Range[] = ['1m', '15m', '1h'];
  return ranges.find((range) => age <= RANGE_MS[range]) ?? null;
}

const IPV4 = /\b(\d{1,3})\.(\d{1,3})\.\d{1,3}\.\d{1,3}\b/g;
const IPV6 = /(?<![\w:])([0-9a-f]{1,4}):([0-9a-f]{0,4})((?::[0-9a-f]{0,4}){1,6})(?![\w:])/gi;
const WG_KEY = /[A-Za-z0-9+/]{42}[AEIMQUYcgkosw048]=/g;
const HEX_ID = /\b[0-9a-f]{16,}\b/gi;
const KEY_HEAD = 4;

/** §6.8 `Copy redacted log`: IPs become `81.2.x.x`, keys and device ids their first characters. */
export function redact(line: string): string {
  return line
    .replace(WG_KEY, (key) => copyScreens.redact.key(key.slice(0, KEY_HEAD)))
    .replace(HEX_ID, (id) => copyScreens.redact.key(id.slice(0, KEY_HEAD)))
    .replace(IPV4, (_all, a: string, b: string) => copyScreens.redact.ipv4(a, b))
    .replace(IPV6, (all: string, a: string, b: string) => {
      // Times like 14:02:16 also have colons: an IPv6 address has `::` or 8 groups.
      const groups = all.split(':').length;
      return all.includes('::') || groups === 8 ? copyScreens.redact.ipv6(a, b) : all;
    });
}

/** One journal line of the daemon log, redacted, for the clipboard. */
export function logLine(entry: Entry): string | null {
  const level = logLevel(entry);
  if (level === null) return null;
  const message = entry.kind === 'log' || entry.kind === 'warning' ? entry.message : '';
  const source = entry.kind === 'log' ? entry.target : '';
  return redact(
    copyScreens.thisDevice.logLine(
      new Date(entry.at_ms).toISOString(),
      copy.activity.levels[level],
      source,
      message,
    ),
  );
}
