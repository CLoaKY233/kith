import { describe, expect, it } from 'vitest';
import type { DeviceSettingsInfo, ExitView, PersonView } from '../types.gen';
import { appliedOf, choosable, isDefault, owns } from './deviceSettings';

const exit = (node: string, name: string, allowed: boolean, in_use_by: string[] = []): ExitView => ({
  network: 'n1',
  node,
  name,
  allowed,
  access: [],
  in_use_by,
});
const via = (node: string | null, paused = false): DeviceSettingsInfo => ({ exit: node, exit_locked: false, paused });
const exits = [exit('b', 'vps', true, ['tv']), exit('c', 'nas', false)];

describe('appliedOf', () => {
  it('says using only when the exit lists the device', () => {
    expect(appliedOf({ settings: via('b'), name: 'tv', online: true, exits })).toEqual({ kind: 'using', exit: 'vps' });
    expect(appliedOf({ settings: via('b'), name: 'desk', online: true, exits })).toEqual({ kind: 'waiting' });
  });

  it('keeps waiting and offline apart, and reads the Console device from the status', () => {
    expect(appliedOf({ settings: via('b'), name: 'desk', online: false, exits })).toEqual({ kind: 'offline' });
    const own = { settings: via('b'), name: 'vps2', online: true, exits, network: 'home' };
    expect(appliedOf({ ...own, usedNow: 'home/vps' })).toEqual({ kind: 'using', exit: 'vps' });
    expect(appliedOf({ ...own, usedNow: null })).toEqual({ kind: 'waiting' });
  });

  it('reports pause and its own connection', () => {
    expect(appliedOf({ settings: via(null, true), name: 'tv', online: true, exits })).toEqual({ kind: 'paused' });
    expect(appliedOf({ settings: via(null), name: 'tv', online: true, exits })).toEqual({ kind: 'own' });
    expect(appliedOf({ settings: via('b', true), name: 'tv', online: true, exits })).toEqual({ kind: 'paused' });
  });
});

describe('who may pick what', () => {
  it('offers allowed exits other than the device, and the one already chosen', () => {
    expect(choosable(exits, 'x', null).map((e) => e.name)).toEqual(['vps']);
    expect(choosable(exits, 'x', 'c').map((e) => e.name)).toEqual(['nas', 'vps']);
    expect(choosable(exits, 'b', null)).toEqual([]);
  });

  it('knows a person owns a device from the people list', () => {
    const person = (me: boolean, devices: string[]) => ({ me, devices }) as PersonView;
    expect(owns([person(false, ['a']), person(true, ['b'])], 'b')).toBe(true);
    expect(owns([person(false, ['a'])], 'a')).toBe(false);
    expect(owns(null, 'a')).toBe(false);
  });

  it('treats no record and an empty one alike', () => {
    expect(isDefault(null)).toBe(true);
    expect(isDefault(via(null))).toBe(true);
    expect(isDefault({ exit: null, exit_locked: true, paused: false })).toBe(false);
  });
});
