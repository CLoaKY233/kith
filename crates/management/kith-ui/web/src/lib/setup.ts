// The setup wizard as a pure state machine (docs/plans/console.md, flow 1). Three steps:
// names, then the passkey (create it, then confirm the owner with it), then the recovery
// words. Each passkey prompt opens from its own click, since browsers want a user gesture
// for one, so the server's options are fetched a step ahead.

import { ApiError } from './api';
import { passkeyText } from './passkeyText';
import type { NameProblem } from './types.gen';
import { PasskeyError, type CreationOptionsJSON, type RequestOptionsJSON } from './webauthn';

/** Why a step failed. The last three end the wizard: no retry can help. */
export type SetupProblem =
  | { readonly kind: 'cancelled' | 'exists' | 'failed'; readonly text: string }
  | { readonly kind: 'unsupported' | 'expired' | 'used' | 'invalid' };

export type SetupState =
  | { readonly phase: 'names'; readonly problem: SetupProblem | null }
  | { readonly phase: 'starting' }
  | { readonly phase: 'passkey'; readonly options: CreationOptionsJSON; readonly problem: SetupProblem | null }
  | { readonly phase: 'creating'; readonly options: CreationOptionsJSON }
  | { readonly phase: 'confirm'; readonly authority: RequestOptionsJSON; readonly problem: SetupProblem | null }
  | { readonly phase: 'confirming'; readonly authority: RequestOptionsJSON }
  | { readonly phase: 'words'; readonly words: readonly string[] }
  | { readonly phase: 'done' }
  | { readonly phase: 'blocked'; readonly problem: SetupProblem };

export type SetupEvent =
  | { readonly type: 'start' }
  | { readonly type: 'started'; readonly options: CreationOptionsJSON }
  | { readonly type: 'create' }
  | { readonly type: 'created'; readonly authority: RequestOptionsJSON }
  | { readonly type: 'confirm' }
  | { readonly type: 'confirmed'; readonly words: readonly string[] }
  | { readonly type: 'saved' }
  | { readonly type: 'back' }
  | { readonly type: 'failed'; readonly problem: SetupProblem };

export const SETUP_STEPS = 3;

export function initialSetup(supported: boolean): SetupState {
  return supported ? { phase: 'names', problem: null } : { phase: 'blocked', problem: { kind: 'unsupported' } };
}

function fatal(problem: SetupProblem): boolean {
  return problem.kind === 'unsupported' || problem.kind === 'expired' || problem.kind === 'used' || problem.kind === 'invalid';
}

/** The next state; an event that does not fit the current phase changes nothing. */
export function setupStep(state: SetupState, event: SetupEvent): SetupState {
  if (event.type === 'failed') {
    if (fatal(event.problem)) return { phase: 'blocked', problem: event.problem };
    switch (state.phase) {
      case 'starting':
        return { phase: 'names', problem: event.problem };
      case 'creating':
        return { phase: 'passkey', options: state.options, problem: event.problem };
      case 'confirming':
        return { phase: 'confirm', authority: state.authority, problem: event.problem };
      default:
        return state;
    }
  }
  switch (state.phase) {
    case 'names':
      return event.type === 'start' ? { phase: 'starting' } : state;
    case 'starting':
      return event.type === 'started' ? { phase: 'passkey', options: event.options, problem: null } : state;
    case 'passkey':
      if (event.type === 'create') return { phase: 'creating', options: state.options };
      if (event.type === 'back') return { phase: 'names', problem: null };
      return state;
    case 'creating':
      return event.type === 'created' ? { phase: 'confirm', authority: event.authority, problem: null } : state;
    case 'confirm':
      return event.type === 'confirm' ? { phase: 'confirming', authority: state.authority } : state;
    case 'confirming':
      if (event.type !== 'confirmed') return state;
      return event.words.length > 0 ? { phase: 'words', words: event.words } : { phase: 'done' };
    case 'words':
      return event.type === 'saved' ? { phase: 'done' } : state;
    case 'done':
    case 'blocked':
      return state;
  }
}

/** Which of the three steps a phase belongs to, for the progress line; null off the steps. */
export function setupStepNumber(state: SetupState): 1 | 2 | 3 | null {
  switch (state.phase) {
    case 'names':
    case 'starting':
      return 1;
    case 'passkey':
    case 'creating':
    case 'confirm':
    case 'confirming':
      return 2;
    case 'words':
      return 3;
    default:
      return null;
  }
}

const NAME_MAX = 63;

/** The network name's shape, checked here because nobody is signed in to ask the server. */
export function networkNameProblem(name: string): NameProblem | null {
  if (name === '') return 'empty';
  if (name.length > NAME_MAX) return 'too_long';
  if (!/^[a-z0-9-]+$/.test(name)) return 'invalid_character';
  if (name.startsWith('-') || name.endsWith('-')) return 'hyphen_at_edge';
  return null;
}

/** Names what a step threw: a refused link ends the wizard, anything else can be retried. */
export function setupProblem(cause: unknown): SetupProblem {
  if (cause instanceof PasskeyError) {
    if (cause.problem === 'unsupported') return { kind: 'unsupported' };
    return { kind: cause.problem, text: passkeyText(cause) };
  }
  if (cause instanceof ApiError) {
    switch (cause.code) {
      case 'session_expired':
        return { kind: 'expired' };
      case 'session_used':
        return { kind: 'used' };
      case 'not_found':
      case 'unauthorized':
      case 'forbidden':
        return { kind: 'invalid' };
    }
  }
  return { kind: 'failed', text: passkeyText(cause) };
}

/** True while a request or a passkey prompt is out. */
export function setupBusy(state: SetupState): boolean {
  return state.phase === 'starting' || state.phase === 'creating' || state.phase === 'confirming';
}
