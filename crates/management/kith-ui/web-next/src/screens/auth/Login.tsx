import { useEffect, useRef, useState } from 'react';
import { accounts } from '../../lib/accounts';
import { ApiError } from '../../lib/api';
import { PASSKEY_UNSUPPORTED, passkeyText } from '../../lib/passkey';
import { passkeysSupported, signWithPasskey, type RequestOptionsJSON } from '../../lib/webauthn';
import { Button } from '../../ui';
import { AuthLink, AuthPage, Problem } from './AuthPage';

/** Prompts are good for 5 minutes on the server; ask again before they go stale. */
const PROMPT_FRESH_MS = 4 * 60 * 1000;

export default function Login() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(passkeysSupported() ? null : PASSKEY_UNSUPPORTED);
  const options = useRef<{ made: number; promise: Promise<RequestOptionsJSON> } | null>(null);

  // Fetched on arrival so the prompt opens straight from the click.
  const fetchOptions = () => {
    const next = accounts.loginBegin().then((answer) => answer.public_key);
    next.catch(() => undefined);
    options.current = { made: Date.now(), promise: next };
  };
  useEffect(() => {
    if (passkeysSupported()) fetchOptions();
  }, []);

  const signIn = async () => {
    if (!passkeysSupported()) return setError(PASSKEY_UNSUPPORTED);
    setBusy(true);
    setError(null);
    try {
      if (!options.current || Date.now() - options.current.made > PROMPT_FRESH_MS) fetchOptions();
      await accounts.loginFinish(await signWithPasskey(await options.current!.promise));
      location.assign('/');
    } catch (cause) {
      const unknown = cause instanceof ApiError && (cause.code === 'unauthorized' || cause.code === 'not_found');
      setError(
        unknown
          ? "kith doesn't know this passkey. Use the one you made for this Console, or ask an admin for a new link."
          : passkeyText(cause),
      );
      setBusy(false);
      fetchOptions();
    }
  };

  return (
    <AuthPage title="Sign in to kith">
      <Button variant="primary" className="w-full" busy={busy} disabled={!passkeysSupported()} onClick={() => void signIn()}>
        {busy ? 'Signing in…' : 'Sign in with passkey'}
      </Button>
      {error && <Problem>{error}</Problem>}
      <AuthLink href="/recover">Lost your passkey? Recover with your words</AuthLink>
    </AuthPage>
  );
}
