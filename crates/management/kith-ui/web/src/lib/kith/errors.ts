// Error mapping (spec §12.5): cause, then the next step, in the spec's words. Screens pass
// the failure and the thing it was about; nothing shows a raw server message alone.

import { copy } from '../copy';
import type { ApiFailure } from '../types';
import type { NameProblem } from '../types.gen';

/** The sentence for a failed action. `subject` names the device or thing it was about. */
export function explain(failure: ApiFailure, subject: string): string {
  switch (failure.code) {
    case 'permission_denied':
    case 'forbidden':
      return copy.error.permissionDenied;
    case 'not_found':
      return copy.error.notFound(subject);
    case 'conflict':
      return copy.error.conflict(failure.conflict?.resolved_by ?? subject);
    case 'invalid_argument':
      return failure.message === '' ? copy.error.nameChars : failure.message;
    case 'daemon_down':
    case 'not_in_group':
    case 'version_skew':
    case 'unauthorized':
    case 'session_used':
    case 'session_expired':
      return copy.conn.staleActions;
    case 'os_auth_denied':
      return copy.error.osAuthDenied;
    case 'not_implemented':
      return copy.error.notImplemented;
    case 'internal':
      return copy.error.internal(failure.message);
  }
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
