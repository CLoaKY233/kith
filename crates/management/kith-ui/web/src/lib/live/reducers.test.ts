import { describe, expect, it } from 'vitest';
import type { PeerInfo } from '../types.gen';
import { deviceKey, tintKey } from './keys';
import { noEffects, reduce, type ReducerState } from './reducers';
import { entry, fixture, NET, node, SELF } from './testing';

function state(): ReducerState {
  return {
    status: structuredClone(fixture.status),
    peers: Object.fromEntries(
      Object.entries(structuredClone(fixture.peers)).map(([net, list]) => [
        net,
        Object.fromEntries(list.map((p) => [p.node, p])),
      ]),
    ),
    pending: structuredClone(fixture.pending),
    seenAt: {},
  };
}

const peer = (s: ReducerState, name: string): PeerInfo | undefined => s.peers[NET]?.[node(name)];

describe('reducers', () => {
  it('replay the whole fixture journal with the network counts matching the devices', () => {
    const s = state();
    const fx = noEffects();
    for (const e of fixture.journal) reduce(s, e, fx);
    const devices = Object.values(s.peers[NET] ?? {});
    const net = s.status?.networks[0];
    expect(devices.length).toBeGreaterThan(0);
    expect(net?.peers_total).toBe(fixture.status.networks[0]?.peers_total);
    expect(devices.every((p) => p.online || p.path === null)).toBe(true);
    expect(fx.resync).toBe(false);
  });

  it('count and tint a device going offline and back only when it changes', () => {
    const s = state();
    const before = s.status?.networks[0]?.peers_online ?? 0;
    const fx = noEffects();
    const nas = node('nas');
    reduce(s, entry(1, 10, { kind: 'peer_offline', network: NET, node: nas, name: 'nas' }), fx);
    reduce(s, entry(2, 11, { kind: 'peer_offline', network: NET, node: nas, name: 'nas' }), fx);
    expect(peer(s, 'nas')?.online).toBe(false);
    expect(peer(s, 'nas')?.path).toBeNull();
    expect(s.status?.networks[0]?.peers_online).toBe(before - 1);
    expect([...fx.tint]).toEqual([tintKey.device(NET, nas)]);

    const back = noEffects();
    reduce(s, entry(3, 20, { kind: 'peer_online', network: NET, node: nas, name: 'nas', path: 'relay', gateway: null }), back);
    expect(s.status?.networks[0]?.peers_online).toBe(before);
    expect(s.seenAt[deviceKey(NET, nas)]).toBe(20);
    expect(back.paths.get(deviceKey(NET, nas))).toEqual({ path: 'relay', atMs: 20, flaps: [] });
  });

  it('leave a path change untinted for the badge to settle, and record it as a flap', () => {
    const s = state();
    const fx = noEffects();
    const nas = node('nas');
    reduce(s, entry(1, 10, { kind: 'path_changed', network: NET, node: nas, name: 'nas', from: 'direct', to: 'relay' }), fx);
    reduce(s, entry(2, 12, { kind: 'path_changed', network: NET, node: nas, name: 'nas', from: 'relay', to: 'direct' }), fx);
    expect(fx.tint.size).toBe(0);
    expect(fx.paths.get(deviceKey(NET, nas))).toEqual({ path: 'direct', atMs: 12, flaps: [10, 12] });
  });

  it('add a joined device, settle its request, and drop a removed one', () => {
    const s = state();
    const join = s.pending.joins[0];
    if (!join) throw new Error('fixture has a join request');
    const fx = noEffects();
    reduce(
      s,
      entry(1, 10, { kind: 'device_joined', network: NET, node: join.node, name: 'pixel-8', address: '100.64.0.40' }),
      fx,
    );
    expect(s.peers[NET]?.[join.node]?.name).toBe('pixel-8');
    expect(s.pending.joins).toHaveLength(0);
    expect(s.status?.networks[0]?.peers_total).toBe((fixture.status.networks[0]?.peers_total ?? 0) + 1);
    expect(fx.tint.has(tintKey.device(NET, join.node))).toBe(true);
    expect(fx.pending).toBe(true);

    const gone = noEffects();
    reduce(s, entry(2, 11, { kind: 'device_removed', network: NET, node: node('tv'), name: 'tv' }), gone);
    expect(peer(s, 'tv')).toBeUndefined();
    expect(gone.removed.has(deviceKey(NET, node('tv')))).toBe(true);
    expect(s.status?.networks[0]?.peers_online).toBe(fixture.status.networks[0]?.peers_online);
  });

  it('ask for a resync when an entry is about this device or a device it does not hold', () => {
    const s = state();
    const removed = noEffects();
    reduce(s, entry(1, 10, { kind: 'device_removed', network: NET, node: SELF, name: 'laptop' }), removed);
    expect(removed.resync).toBe(true);

    const unknown = noEffects();
    reduce(s, entry(2, 11, { kind: 'peer_online', network: NET, node: 'ff', name: 'x', path: 'direct', gateway: null }), unknown);
    expect(unknown.resync).toBe(true);
  });

  it('move the exit, tint it, and refetch what only the server knows', () => {
    const s = state();
    const fx = noEffects();
    reduce(s, entry(1, 10, { kind: 'exit_changed', exit: null }), fx);
    expect(s.status?.exit_node).toBeNull();
    expect(s.status?.exit_state).toBeNull();
    expect(fx.tint.has(tintKey.exit)).toBe(true);
    expect(fx.status).toBe(true);

    const vpsDown = noEffects();
    s.status = structuredClone(fixture.status);
    reduce(s, entry(2, 11, { kind: 'peer_offline', network: NET, node: node('vps'), name: 'vps' }), vpsDown);
    expect(vpsDown.status).toBe(true);
  });
});
