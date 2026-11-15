// The setup wizard as a pure state machine: names, then the passkey (create it, then
// confirm the owner with it), then the recovery words. Each passkey prompt opens from its
// own click, since browsers want a user gesture, so the server's options are fetched a step ahead.

import { ApiError } from './api';
import { PASSKEY_UNSUPPORTED, passkeyText } from './passkey';
import { PasskeyError, type CreationOptionsJSON, type RequestOptionsJSON } from './webauthn';

/** Why a step failed. The last four end the wizard: no retry can help. */
export type SetupProblem =
  | { readonly kind: 'cancelled' | 'exists' | 'failed'; readonly text: string; readonly restart?: true }
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

export function initialSetup(supported: boolean): SetupState {
  return supported ? { phase: 'names', problem: null } : { phase: 'blocked', problem: { kind: 'unsupported' } };
}

const fatal = (problem: SetupProblem) =>
  problem.kind === 'unsupported' || problem.kind === 'expired' || problem.kind === 'used' || problem.kind === 'invalid';

/** The next state; an event that does not fit the current phase changes nothing. */
export function setupStep(state: SetupState, event: SetupEvent): SetupState {
  if (event.type === 'failed') {
    if (fatal(event.problem)) return { phase: 'blocked', problem: event.problem };
    switch (state.phase) {
      case 'starting':
        return { phase: 'names', problem: event.problem };
      case 'creating':
        // The server closes a passkey prompt once it has seen an answer, so after its refusal start over.
        return event.problem.kind === 'failed' && event.problem.restart
          ? { phase: 'names', problem: event.problem }
          : { phase: 'passkey', options: state.options, problem: event.problem };
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

/** Which of the three steps a phase belongs to; null off the steps. */
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

/** What is wrong with a network name, checked here because nobody is signed in to ask the server. */
export function networkNameProblem(name: string): string | null {
  if (name === '') return 'Give it a name.';
  if (name.length > 63) return 'Names can be up to 63 characters.';
  if (!/^[a-z0-9-]+$/.test(name)) return 'Names can use a–z, 0–9 and hyphens.';
  if (name.startsWith('-') || name.endsWith('-')) return "Names can't start or end with a hyphen.";
  return null;
}

const RETRY_TEXT = "Setup couldn't finish. Try again.";
const OFFLINE_TEXT = "Couldn't reach the Console. Check your connection and try again.";
const NAME_MAX_BYTES = 64;

/** The owner's name, checked to match the server's limit. */
export function personNameProblem(name: string): string | null {
  if (name.trim() === '') return 'Give your name.';
  if (new TextEncoder().encode(name.trim()).length > NAME_MAX_BYTES) return 'Names can be up to 64 characters.';
  return null;
}

const sentence = (text: string) => {
  const said = text.trim();
  const capital = said.charAt(0).toUpperCase() + said.slice(1);
  return /[.!?]$/.test(capital) ? capital : `${capital}.`;
};

/**
 * Names what a step threw. `link` steps talk to the setup link, so a refused link ends the wizard;
 * `form` is the step that checks the typed names, whose 400s read as sentences; `account` steps run
 * after the owner exists, where a refusal says nothing about the link. Anything else can be retried.
 */
export function setupProblem(cause: unknown, stage: 'link' | 'form' | 'account' = 'link'): SetupProblem {
  if (cause instanceof PasskeyError) {
    if (cause.problem === 'unsupported') return { kind: 'unsupported' };
    return { kind: cause.problem, text: passkeyText(cause) };
  }
  if (cause instanceof ApiError) {
    if (cause.code === 'offline') return { kind: 'failed', text: OFFLINE_TEXT };
    if (stage === 'account') {
      return { kind: 'failed', text: cause.code === 'unauthorized' ? cause.message : RETRY_TEXT };
    }
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
    if (stage === 'form' && cause.code === 'invalid_argument' && cause.message.trim() !== '') {
      return { kind: 'failed', text: sentence(cause.message) };
    }
    return { kind: 'failed', text: RETRY_TEXT, restart: true };
  }
  return { kind: 'failed', text: RETRY_TEXT };
}

export function setupProblemText(problem: SetupProblem): string {
  switch (problem.kind) {
    case 'unsupported':
      return PASSKEY_UNSUPPORTED;
    case 'expired':
      return 'This setup link expired. Setup links last 30 minutes: make a new one on the server that runs the Console.';
    case 'used':
      return 'This setup link was already used. Sign in with the passkey made with it.';
    case 'invalid':
      return "This setup link isn't valid. Check it was copied whole.";
    default:
      return problem.text;
  }
}
