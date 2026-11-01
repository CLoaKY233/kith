import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { THIN } from '../copy';
import type { Entry, Fixture, PeerInfo } from '../types.gen';
import {
  countDevices,
  deviceItems,
  historyOf,
  isFlapping,
  matchesFilter,
  matchesQuery,
  membershipOf,
  namesPointingAt,
  resolveDevice,
  resolveNetwork,
  selfPeer,
  spokenRate,
  verdict,
  windowTotals,
} from './devices';

const fixture = JSON.parse(
  readFileSync(new URL('../../../fixtures/office.json', import.meta.url), 'utf8'),
) as Fixture;
const net = fixture.status.networks[0]?.id ?? '';
const peers = Object.fromEntries((fixture.peers[net] ?? []).map((p) => [p.node, p]));
const self = selfPeer(fixture.status, membershipOf(fixture.status, net));
const items = deviceItems(net, peers, self);
const exits = new Set((fixture.exits[net] ?? []).map((e) => e.node));

function peer(name: string): PeerInfo {
  const found = items.find((i) => i.peer.name === name)?.peer;
  if (!found) throw new Error(`no ${name} in the fixture`);
  return found;
}

describe('devices list', () => {
  it('resolves names from the URL, then short ids', () => {
    expect(resolveNetwork(fixture.status, 'home')?.id).toBe(net);
    expect(resolveNetwork(fixture.status, net.slice(0, 8))?.id).toBe(net);
    expect(resolveDevice(items, 'nas')?.peer.name).toBe('nas');
    expect(resolveDevice(items, peer('nas').node.slice(0, 8))?.peer.name).toBe('nas');
    expect(resolveDevice(items, 'nope')).toBeNull();
  });

  it('pins this device first, then sorts by name', () => {
    expect(items[0]?.self).toBe(true);
    const names = items.slice(1).map((i) => i.peer.name);
    expect(names).toEqual([...names].sort((a, b) => a.localeCompare(b)));
  });

  it('counts the chips and filters by them', () => {
    const counts = countDevices(items, exits);
    expect(counts.all).toBe(items.length);
    expect(counts.online + counts.offline).toBe(counts.all);
    for (const [filter, n] of [
      ['online', counts.online],
      ['offline', counts.offline],
      ['relayed', counts.relayed],
      ['phones', counts.phones],
      ['exits', counts.exits],
    ] as const) {
      expect(items.filter((i) => matchesFilter(i, filter, exits))).toHaveLength(n);
    }
    expect(counts.relayed).toBe(2);
    expect(counts.phones).toBe(2);
  });

  it('filters by name or address', () => {
    expect(items.filter((i) => matchesQuery(i, 'pi')).map((i) => i.peer.name)).toEqual([
      'office-pi',
      'pi-hole',
    ]);
    expect(items.filter((i) => matchesQuery(i, '100.64.0.4')).map((i) => i.peer.name)).toEqual([
      'nas',
    ]);
  });

  it('speaks rates in full words', () => {
    expect(spokenRate(1_240_000)).toBe('1.24 megabytes per second');
    expect(spokenRate(400)).toBe('idle');
  });
});

describe('verdicts (spec §12.4)', () => {
  const base = { relay: null, rttMs: null, seenAgoMs: 1000, blocked: null };

  it('says how a device is reached', () => {
    expect(verdict({ ...base, peer: peer('nas'), rttMs: 12 }).text).toBe(`Reachable directly · 12${THIN}ms`);
    const relayed = verdict({ ...base, peer: peer('dev-box'), relay: 'vps', rttMs: 84 });
    expect(relayed.text).toBe(`Reachable through vps · 84${THIN}ms.`);
    expect(relayed.reason).toBe(
      "dev-box's router changes ports for every connection, so no direct path exists.",
    );
    expect(verdict({ ...base, peer: peer('office-pi'), relay: 'vps' }).text).toBe(
      'Reachable, but large transfers may stall (path MTU 1280).',
    );
    expect(verdict({ ...base, peer: peer('nas'), seenAgoMs: 4 * 60_000 }).text).toBe('Online · quiet');
  });

  it('says how long an offline device has been gone, and offers removal after a day', () => {
    const tv = peer('tv');
    expect(verdict({ ...base, peer: tv, seenAgoMs: 4 * 60_000 }).text).toBe(
      `Not reachable right now. Seen 4${THIN}min ago. It may be asleep or switching networks.`,
    );
    const gone = verdict({ ...base, peer: tv, seenAgoMs: 3 * 86_400_000 + 5 });
    expect(gone.text).toBe('Not seen for 3 days.');
    expect(gone.action).toBe('remove');
  });

  it('names the policy that blocks a connection', () => {
    const nas = peer('nas');
    const blocked = fixture.pending.blocked[0];
    if (!blocked) throw new Error('the fixture has a blocked connection');
    const v = verdict({ ...base, peer: nas, blocked });
    expect(v.text).toBe('Reachable, but policy blocks this device → nas on port 22.');
    expect(v.action).toBe('allow');
  });
});

describe('device detail data', () => {
  it('lists names that point at the device', () => {
    const records = fixture.records[net] ?? [];
    const names = namesPointingAt(records, peer('nas'), 'home').map((r) => r.name);
    expect(names).toContain('files.home.kith.internal');
    expect(names).not.toContain('nas.home.kith.internal');
  });

  it('keeps the newest history first and spots flapping', () => {
    const node = peer('nas').node;
    const at = (seq: number, ms: number): Entry => ({
      seq,
      at_ms: ms,
      kind: 'path_changed',
      network: net,
      node,
      name: 'nas',
      from: 'direct',
      to: 'relay',
    });
    const journal = [at(1, 1_000), at(2, 20_000), at(3, 30_000), at(4, 40_000), at(5, 50_000)];
    expect(historyOf(journal, net, node, 'home/nas', 3).map((e) => e.seq)).toEqual([5, 4, 3]);
    expect(isFlapping(journal, net, node, 55_000)).toBe(true);
    expect(isFlapping(journal, net, node, 95_000)).toBe(false);
  });

  it('totals the window and drops counter resets', () => {
    const columns = {
      at: Float64Array.from([0, 2000, 4000, 6000]),
      rx: Float64Array.from([100, 300, 50, 150]),
      tx: Float64Array.from([10, 20, 30, 40]),
      length: 4,
    };
    expect(windowTotals(columns, 60_000, 6000)).toEqual({ rx: 300, tx: 30 });
    expect(windowTotals(columns, 3000, 6000)).toEqual({ rx: 100, tx: 20 });
  });
});
