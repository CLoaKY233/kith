import { describe, expect, it } from 'vitest';
import { ApiError } from './api';
import {
  initialSetup,
  networkNameProblem,
  setupProblem,
  setupStep,
  setupStepNumber,
  type SetupEvent,
  type SetupState,
} from './setup';
import { PasskeyError, type CreationOptionsJSON, type RequestOptionsJSON } from './webauthn';

const options: CreationOptionsJSON = {
  rp: { name: 'kith' },
  user: { id: 'AQ', name: 'ana', displayName: 'Ana' },
  challenge: 'AQID',
  pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
};
const authority: RequestOptionsJSON = { challenge: 'BAUG' };
const cancelled = { kind: 'cancelled', text: 'closed' } as const;

const run = (events: SetupEvent[], from: SetupState = initialSetup(true)): SetupState =>
  events.reduce(setupStep, from);

describe('the setup wizard', () => {
  it('walks names, passkey and words in three steps', () => {
    const seen: (number | null)[] = [];
    let state = initialSetup(true);
    const events: SetupEvent[] = [
      { type: 'start' },
      { type: 'started', options },
      { type: 'create' },
      { type: 'created', authority },
      { type: 'confirm' },
      { type: 'confirmed', words: ['apple', 'river'] },
      { type: 'saved' },
    ];
    for (const event of events) {
      seen.push(setupStepNumber(state));
      state = setupStep(state, event);
    }
    expect(seen).toEqual([1, 1, 2, 2, 2, 2, 3]);
    expect(state).toEqual({ phase: 'done' });
  });

  it('starts blocked when the browser has no passkeys', () => {
    expect(initialSetup(false)).toEqual({ phase: 'blocked', problem: { kind: 'unsupported' } });
  });

  it('keeps the step and says why when a prompt is cancelled, so it can be retried', () => {
    const atCreate = run([{ type: 'start' }, { type: 'started', options }, { type: 'create' }]);
    expect(setupStep(atCreate, { type: 'failed', problem: cancelled })).toEqual({
      phase: 'passkey',
      options,
      problem: cancelled,
    });
    const atConfirm = run([{ type: 'create' }, { type: 'created', authority }, { type: 'confirm' }], {
      phase: 'passkey',
      options,
      problem: null,
    });
    const back = setupStep(atConfirm, { type: 'failed', problem: cancelled });
    expect(back).toEqual({ phase: 'confirm', authority, problem: cancelled });
    expect(setupStep(back, { type: 'confirm' })).toEqual({ phase: 'confirming', authority });
  });

  it('ends on an expired or used link at any step', () => {
    for (const kind of ['expired', 'used', 'invalid'] as const) {
      const state = run([{ type: 'start' }, { type: 'failed', problem: { kind } }]);
      expect(state).toEqual({ phase: 'blocked', problem: { kind } });
      expect(setupStep(state, { type: 'start' })).toBe(state);
    }
  });

  it('ignores events that do not fit, such as a double click', () => {
    const starting = run([{ type: 'start' }]);
    expect(setupStep(starting, { type: 'start' })).toBe(starting);
    expect(setupStep(starting, { type: 'saved' })).toBe(starting);
  });

  it('skips the words step when the server sends none', () => {
    const state = setupStep({ phase: 'confirming', authority }, { type: 'confirmed', words: [] });
    expect(state).toEqual({ phase: 'done' });
  });

  it('names what failed', () => {
    expect(setupProblem(new ApiError('session_expired', 410, 'gone', null))).toEqual({ kind: 'expired' });
    expect(setupProblem(new ApiError('session_used', 401, 'used', null))).toEqual({ kind: 'used' });
    expect(setupProblem(new ApiError('not_found', 404, 'no', null))).toEqual({ kind: 'invalid' });
    expect(setupProblem(new PasskeyError('unsupported'))).toEqual({ kind: 'unsupported' });
    expect(setupProblem(new PasskeyError('cancelled')).kind).toBe('cancelled');
    expect(setupProblem(new ApiError('internal', 500, 'disk full', null))).toMatchObject({ kind: 'failed' });
  });

  it('checks the network name like the CLI', () => {
    expect(networkNameProblem('home')).toBeNull();
    expect(networkNameProblem('')).toBe('empty');
    expect(networkNameProblem('Home')).toBe('invalid_character');
    expect(networkNameProblem('-home')).toBe('hyphen_at_edge');
    expect(networkNameProblem('a'.repeat(64))).toBe('too_long');
  });
});
