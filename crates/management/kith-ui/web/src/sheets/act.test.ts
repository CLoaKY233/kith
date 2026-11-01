import { describe, expect, it } from 'vitest';
import { copy } from '../lib/copy';
import type { ActionHandle, ActionState, ApiFailure, Spread } from '../lib/types';
import type { RecordView, StatusView } from '../lib/types.gen';
import {
  failureText,
  freeName,
  nameProblemText,
  namesPointingAt,
  networkContext,
  normalizeName,
  progressText,
} from './act';

function handle(
  state: ActionState,
  seen: Spread | null = null,
  slow = false,
): ActionHandle<unknown> {
  return {
    state,
    seen,
    slow,
    error: null,
    conflict: null,
    result: null,
    ledger: true,
    done: new Promise(() => undefined),
  };
}

function failure(code: ApiFailure['code'], extra: Partial<ApiFailure> = {}): ApiFailure {
  return { code, status: 400, message: 'disk full', conflict: null, ...extra };
}

describe('names (spec §7.6, §7.11)', () => {
  it('types names the way the CLI stores them', () => {
    expect(normalizeName('My Phone')).toEqual({ value: 'my-phone', spaced: true });
    expect(normalizeName('NAS')).toEqual({ value: 'nas', spaced: false });
  });

  it('suggests the first free phone name', () => {
    expect(freeName('phone', ['nas'])).toBe('phone');
    expect(freeName('phone', ['phone', 'phone-2'])).toBe('phone-3');
  });

  it('maps every validator problem to its sentence', () => {
    const check = (problem: Parameters<typeof nameProblemText>[0]['problem']) =>
      nameProblemText({ name: 'laptop', problem, suggestion: 'laptop-2' }, 'home');
    expect(check(null)).toBeNull();
    expect(check('invalid_character')).toBe('Names can use a–z, 0–9 and hyphens.');
    expect(check('taken')).toBe('laptop is taken in home. Use laptop-2?');
  });

  it('counts the names that follow a device when it is renamed', () => {
    const record = (name: string, type: RecordView['type'], value: string, device = false) => ({
      name,
      type,
      value,
      device,
      set_by: device ? null : 'desk',
    });
    const records = [
      record('dev-box.home.kith.internal', 'A', '100.64.0.9', true),
      record('build.home.kith.internal', 'CNAME', 'dev-box.home.kith.internal'),
      record('ci.home.kith.internal', 'A', '100.64.0.9'),
      record('note.home.kith.internal', 'TXT', 'dev-box'),
      record('nas2.home.kith.internal', 'CNAME', 'nas.home.kith.internal'),
    ];
    expect(namesPointingAt(records, { name: 'dev-box', address: '100.64.0.9' }, 'home')).toBe(2);
  });
});

describe('progress and failures (spec §8.6, §12.5)', () => {
  it('never shows a count without acks', () => {
    expect(progressText(handle('pending'))).toBe('Signing…');
    expect(progressText(handle('accepted'))).toBe('Saved on this device');
    expect(progressText(handle('accepted', null, true))).toBe(copy.action.savedWaiting);
    expect(progressText(handle('accepted', { by: 2, of: 3 }))).toBe(
      'Signed. Spreading · 2 of 3 devices',
    );
    expect(progressText(handle('accepted', { by: 3, of: 5 }), copy.blocked.spreading)).toBe(
      'Allowed by you · spreading 3 of 5',
    );
    expect(progressText(handle('awaiting_os_auth'))).toBe(
      'Waiting for confirmation on this computer…',
    );
  });

  it('states the cause and the next step', () => {
    expect(failureText(failure('permission_denied'), 'tv')).toBe(copy.error.permissionDenied);
    expect(failureText(failure('not_found'), 'tv')).toBe('tv is no longer in this network.');
    expect(failureText(failure('internal'), 'tv')).toBe(
      "kith couldn't do this: disk full. Details are in Logs.",
    );
    const conflict = { resolved_by: 'laptop', resolution: null, at_ms: 0, op: null };
    expect(failureText(failure('conflict', { conflict }), 'tv')).toBe(
      'laptop changed this a moment ago. Showing the latest.',
    );
  });
});

describe('asking an admin (spec §7.10)', () => {
  it('writes requests an admin can paste into the CLI', () => {
    expect(copy.askAdmin.request.allow('home', 'phone', 'nas', 'tcp', 22)).toBe(
      'kith allow home phone nas tcp 22',
    );
    expect(copy.askAdmin.request.allow('home', 'phone', 'nas', 'icmp', null)).toBe(
      'kith allow home phone nas icmp',
    );
    expect(copy.askAdmin.request.remove('home', 'tv')).toBe('kith remove home tv');
  });

  it('finds a network by id or by name', () => {
    const status = {
      networks: [{ id: 'ab12', name: 'home', admin: false }],
      memberships: [{ network: 'ab12', name: 'laptop', admins: ['desk'] }],
    } as unknown as StatusView;
    expect(networkContext(status, 'home')?.id).toBe('ab12');
    expect(networkContext(status, 'ab12')?.membership?.admins).toEqual(['desk']);
    expect(networkContext(status, 'work')).toBeNull();
  });
});

describe('remove (spec §7.9)', () => {
  it('lists exit users the way the dialog reads', () => {
    expect(copy.remove.exitUsers(copy.remove.andList(['laptop', 'tv']), 'phone')).toBe(
      'laptop and tv use phone as their exit. Their internet will go directly.',
    );
    expect(copy.remove.andList(['a', 'b', 'c'])).toBe('a, b and c');
  });
});
