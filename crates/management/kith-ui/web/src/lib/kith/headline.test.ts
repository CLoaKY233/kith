import { describe, expect, it } from 'vitest';
import { THIN } from '../copy';
import type { PeerInfo } from '../types.gen';
import { headline, problemTarget, type HeadlineInput } from './headline';

function peer(name: string, online: boolean, path: PeerInfo['path'] = 'direct'): PeerInfo {
  return {
    node: `id-${name}`,
    invite: null,
    settings: null,
    via_this_exit: false,
    name,
    address: '100.64.0.9',
    path: online ? path : null,
    path_reason: null,
    last_seen_secs: online ? 1 : 3 * 3600,
    endpoint: null,
    rx_bytes: 0,
    tx_bytes: 0,
    online,
    display_name: name,
    gateway: null,
    path_mtu: null,
  };
}

const quiet: HeadlineInput = {
  network: 'home',
  netId: 'n1',
  admin: true,
  pausedUntilMs: null,
  problems: [],
  exit: null,
  joins: [],
  peers: [peer('nas', true), peer('dev-box', true, 'relay'), peer('tv', false)],
  expected: [],
  relayOf: { 'id-dev-box': 'vps' },
  down: 3_400_000,
  up: 220_000,
};

describe('Overview headline (spec §6.2)', () => {
  it('says everything works, with the sub-line, when nothing else applies', () => {
    const view = headline(quiet);
    expect(view.text).toBe('Everything is working. 3 of 4 devices online.');
    expect(view.sub).toBe(`1 direct · 1 via vps · ↓ 3.40${THIN}MB/s ↑ 220${THIN}kB/s`);
    expect(view.action).toBeNull();
  });

  it('takes the first matching condition in priority order', () => {
    const all: HeadlineInput = {
      ...quiet,
      pausedUntilMs: Date.UTC(2026, 10, 2, 15, 10),
      problems: [{ what: 'phone lost its address.', fix: 'Remove phone and add it again.' }],
      exit: { name: 'vps', state: 'offline_blocked' },
      joins: ['pixel-8'],
      expected: [peer('nas', false)],
    };
    expect(headline(all).text).toMatch(/^kith is paused until /);

    const problem = headline({ ...all, pausedUntilMs: null });
    expect(problem).toMatchObject({ text: 'phone lost its address.', tone: 'danger' });
    expect(problem.action).toMatchObject({ kind: 'fix', label: 'Remove phone and add it again.' });

    const exit = headline({ ...all, pausedUntilMs: null, problems: [] });
    expect(exit.text).toBe('Your exit, vps, is offline. Traffic is blocked.');

    const join = headline({ ...all, pausedUntilMs: null, problems: [], exit: null });
    expect(join.text).toBe('pixel-8 is asking to join home.');
    expect(headline({ ...all, pausedUntilMs: null, problems: [], exit: null, joins: ['a', 'b'] }).text).toBe(
      '2 devices are asking to join home.',
    );

    const gone = headline({ ...all, pausedUntilMs: null, problems: [], exit: null, joins: [] });
    expect(gone.text).toBe(`nas can't be reached. Seen 3${THIN}h ago.`);
    expect(gone.action).toMatchObject({ kind: 'why', node: 'id-nas' });
  });

  it('shows join requests only to admins, and Add a device only to admins', () => {
    expect(headline({ ...quiet, admin: false, joins: ['pixel-8'] }).text).toMatch(/^Everything/);
    expect(headline({ ...quiet, peers: [] }).action).toMatchObject({ kind: 'add_device' });
    expect(headline({ ...quiet, peers: [], admin: false }).action).toBeNull();
  });

  it("points a problem's fix at the device it names, else its command, else This device", () => {
    const peers = [peer('nas', true), peer('tv', false)];
    expect(problemTarget({ what: 'nas and tv share a key.', fix: 'Remove the older one.' }, peers)).toEqual({
      kind: 'device',
      node: 'id-nas',
    });
    expect(problemTarget({ what: 'The ledger is stale.', fix: 'Run `kith sync` here.' }, peers)).toEqual({
      kind: 'command',
      command: 'kith sync',
    });
    expect(problemTarget({ what: 'Clock is off.', fix: 'Set the clock.' }, peers)).toEqual({ kind: 'this' });
  });
});
