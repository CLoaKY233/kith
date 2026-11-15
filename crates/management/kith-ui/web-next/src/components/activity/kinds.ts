import type { Entry, PathKind } from '@kith/types';
import {
  AlertTriangle, DoorOpen, Globe, MapPin, PackageX, PenLine, Route, ShieldX, Terminal,
  UserMinus, UserPlus, Wifi, WifiOff, type LucideIcon,
} from 'lucide-react';

export type Category = 'presence' | 'members' | 'paths' | 'exits' | 'warnings' | 'dropped' | 'logs';

export const CATEGORIES: { id: Category; label: string }[] = [
  { id: 'presence', label: 'Online / offline' },
  { id: 'members', label: 'Joining / leaving' },
  { id: 'paths', label: 'Path changes' },
  { id: 'exits', label: 'Exits' },
  { id: 'warnings', label: 'Warnings' },
  { id: 'dropped', label: 'Dropped packets' },
  { id: 'logs', label: 'Logs' },
];

type Look = { icon: LucideIcon; problem?: boolean; category: Category };

const LOOKS: Record<Entry['kind'], Look> = {
  peer_online: { icon: Wifi, category: 'presence' },
  peer_offline: { icon: WifiOff, category: 'presence' },
  device_joined: { icon: UserPlus, category: 'members' },
  device_removed: { icon: UserMinus, category: 'members' },
  join_requested: { icon: DoorOpen, category: 'members' },
  device_renamed: { icon: PenLine, category: 'members' },
  path_changed: { icon: Route, category: 'paths' },
  endpoint_changed: { icon: MapPin, category: 'paths' },
  exit_changed: { icon: Globe, category: 'exits' },
  warning: { icon: AlertTriangle, category: 'warnings' },
  dropped: { icon: PackageX, category: 'dropped' },
  blocked: { icon: ShieldX, problem: true, category: 'warnings' },
  log: { icon: Terminal, category: 'logs' },
};

export const look = (e: Entry): Look => LOOKS[e.kind];

const WAS: Record<PathKind | 'none', string> = { direct: 'direct', relay: 'relayed', stealth: 'stealth', none: 'offline' };
const REASONS: Record<string, string> = {
  policy: 'the access rules don\'t allow them',
  no_route: 'no device has the address they were sent to',
  source_not_allowed: 'they came from an address the sending device doesn\'t own',
};
const plural = (n: number, one: string, many: string) => (n === 1 ? one : many);

/** The device part of `network/device`. */
const exitDevice = (exit: string) => exit.slice(exit.lastIndexOf('/') + 1);

/** A plain sentence for one journal entry. */
export function sentence(e: Entry): string {
  switch (e.kind) {
    case 'peer_online':
      return `${e.name} came online${e.path ? `, ${WAS[e.path]}` : ''}${e.gateway ? ` through ${e.gateway}` : ''}`;
    case 'peer_offline':
      return `${e.name} went offline`;
    case 'path_changed':
      return e.to
        ? `${e.name} is now ${WAS[e.to]} (was ${WAS[e.from ?? 'none']})`
        : e.from
          ? `${e.name} lost its ${WAS[e.from]} path`
          : `${e.name} lost its path`;
    case 'endpoint_changed':
      return `${e.name} changed network`;
    case 'device_joined':
      return `${e.name} joined`;
    case 'device_removed':
      return `${e.name} was removed`;
    case 'join_requested':
      return `${e.name} asked to join`;
    case 'device_renamed':
      return `${e.old_name} is now called ${e.name}`;
    case 'exit_changed':
      return e.exit ? `This device now uses ${exitDevice(e.exit)} as its exit` : 'This device now uses its own connection';
    case 'dropped':
      return `${e.count.toLocaleString()} ${plural(e.count, 'packet', 'packets')} dropped: ${REASONS[e.reason] ?? e.reason.replaceAll('_', ' ')}`;
    case 'warning':
      return e.message;
    case 'blocked': {
      const proto = e.protocol.toUpperCase();
      const where = e.port != null ? `${proto} port ${e.port}` : proto;
      return `${e.from_name} was blocked from reaching ${e.to_name} (${where})${e.count > 1 ? `, ${e.count.toLocaleString()} times` : ''}`;
    }
    case 'log':
      return e.message;
  }
}

/** Device names an entry is about, for the device filter. */
export function devicesOf(e: Entry): string[] {
  if (e.kind === 'blocked') return [e.from_name, e.to_name];
  if (e.kind === 'device_renamed') return [e.name, e.old_name];
  if ('name' in e) return [e.name];
  if (e.kind === 'exit_changed' && e.exit) return [exitDevice(e.exit)];
  return [];
}
