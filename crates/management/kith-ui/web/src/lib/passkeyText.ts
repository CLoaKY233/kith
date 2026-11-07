import { ApiError, errorText } from './api';
import { copyConsole } from './copy.console';
import { PasskeyError } from './webauthn';

/** The sentence for a failed passkey prompt, or for the API call around it. */
export function passkeyText(cause: unknown): string {
  if (cause instanceof PasskeyError) {
    switch (cause.problem) {
      case 'unsupported':
        return copyConsole.passkey.unsupported;
      case 'cancelled':
        return copyConsole.passkey.cancelled;
      case 'exists':
        return copyConsole.passkey.exists;
      case 'failed':
        return copyConsole.passkey.failed(cause.message);
    }
  }
  return errorText(ApiError.from(cause).toFailure());
}
