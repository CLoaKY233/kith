// Error mapping (spec §12.5): cause, then the next step, in the spec's words. Screens pass
// the failure and the thing it was about; nothing shows a raw server message alone.

import { errorText } from '../api';
import { copy } from '../copy';
import type { ApiFailure } from '../types';
import type { NameProblem } from '../types.gen';

/** The sentence for a failed action, from the one §12.5 mapper (api.ts errorText).
 * `subject` names the device or thing it was about. */
export function explain(failure: ApiFailure, subject: string, invalid?: string): string {
  return errorText(failure, invalid === undefined ? { subject } : { subject, invalid });
}

/** The field error for a name the validator refused; `taken` has its own sentence. */
export function nameProblem(problem: NameProblem): string {
  switch (problem) {
    case 'empty':
      return copy.error.nameEmpty;
    case 'too_long':
      return copy.error.nameTooLong;
    case 'invalid_character':
      return copy.error.nameChars;
    case 'hyphen_at_edge':
      return copy.error.nameHyphenEdge;
    case 'taken':
      return copy.error.nameTaken;
  }
}
