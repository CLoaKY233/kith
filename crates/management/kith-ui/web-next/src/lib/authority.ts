import { useCallback, useRef, useState } from 'react';
import type { AuthorityChangeIn } from '@kith/types';
import { accounts } from './accounts';
import { passkeyText } from './passkey';
import { signWithPasskey, type RequestOptionsJSON } from './webauthn';

/**
 * One authority change (a role, removing someone, adding someone) signed by the person's
 * passkey. The challenge is fetched as soon as the change is known, so the prompt opens
 * straight from the click: Safari only shows it inside a user gesture.
 */
export function useAuthority() {
  const [signing, setSigning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const change = useRef<AuthorityChangeIn | null>(null);
  const options = useRef<Promise<RequestOptionsJSON> | null>(null);
  const busy = useRef(false);

  const refresh = useCallback(() => {
    if (!change.current) return;
    const next = accounts.authorityBegin(change.current).then((answer) => answer.public_key);
    next.catch(() => undefined);
    options.current = next;
  }, []);

  const prepare = useCallback(
    (next: AuthorityChangeIn) => {
      change.current = next;
      setError(null);
      refresh();
    },
    [refresh],
  );

  /** Opens the passkey prompt and submits the signature; true once the server took it. */
  const confirm = useCallback(async () => {
    if (!options.current || busy.current) return false;
    busy.current = true;
    setSigning(true);
    setError(null);
    try {
      await accounts.authorityFinish(await signWithPasskey(await options.current));
      options.current = null;
      return true;
    } catch (cause) {
      setError(passkeyText(cause));
      // A challenge signs once; the next try needs a fresh one.
      refresh();
      return false;
    } finally {
      busy.current = false;
      setSigning(false);
    }
  }, [refresh]);

  return { signing, error, prepare, confirm };
}
