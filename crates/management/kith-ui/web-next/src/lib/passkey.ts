import { ApiError } from './api';
import { PasskeyError } from './webauthn';

export const PASSKEY_UNSUPPORTED =
  "This browser can't use passkeys on this page. Open it in a current Safari, Chrome, Edge or Firefox.";

/** The sentence for a failed passkey prompt, or for the API call around it. */
export function passkeyText(cause: unknown): string {
  if (cause instanceof PasskeyError) {
    switch (cause.problem) {
      case 'unsupported':
        return PASSKEY_UNSUPPORTED;
      case 'cancelled':
        return 'The passkey prompt closed before it finished. Nothing changed.';
      case 'exists':
        return 'This device already has a passkey for this Console. Sign in with it instead.';
      case 'failed':
        return "The passkey couldn't finish. Try again.";
    }
  }
  if (cause instanceof ApiError) return cause.message;
  return 'Something went wrong. Try again.';
}
