import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Boot, Fixture } from '../types.gen';
import { reduce, type ReducerState } from './reducers';
import { hydrateForTest, live } from './store.svelte';

const fixture = JSON.parse(
  readFileSync(new URL('../../../fixtures/office.json', import.meta.url), 'utf8'),
) as Fixture;

const boot: Boot = {
  boot: fixture.boot,
  seq: 0,
  server_time_ms: fixture.recorded_at_ms,
  status: fixture.status,
  peers: fixture.peers,
  traffic_tail: fixture.traffic,
  pending: fixture.pending,
  versions: fixture.status.versions,
  last_tick: null,
};

describe('store', () => {
  it('hydrates the office fixture into peers[net][node] and traffic columns', () => {
    hydrateForTest(boot);
    const net = fixture.status.networks[0]?.id ?? '';
    const peers = Object.values(live.peers[net] ?? {});
    expect(peers.map((p) => p.name)).toContain('nas');
    expect(live.traffic.size).toBe(fixture.traffic.length);
    expect(live.pending.joins).toHaveLength(1);
    expect(live.ready).toBe(true);
  });

  it('applies every journal entry of the fixture without losing devices', () => {
    const state: ReducerState = {
      status: structuredClone(fixture.status),
      peers: Object.fromEntries(
        Object.entries(structuredClone(fixture.peers)).map(([net, list]) => [
          net,
          Object.fromEntries(list.map((p) => [p.node, p])),
        ]),
      ),
      pending: structuredClone(fixture.pending),
      changed: new Set(),
    };
    for (const entry of fixture.journal) reduce(state, entry);
    const net = fixture.status.networks[0]?.id ?? '';
    expect(Object.keys(state.peers[net] ?? {}).length).toBeGreaterThan(0);
  });
});
