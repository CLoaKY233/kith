import { useEffect, useState } from 'react';
import { consoleMode } from '../../lib/account';
import { Copy } from '../../ui';
import { AuthLink, AuthPage, Note } from './AuthPage';

export type SignedOutReason = 'session_used' | 'session_expired' | null;

const WHY: Record<Exclude<SignedOutReason, null>, string> = {
  session_used: 'This link was already used. Links work once.',
  session_expired: 'This link expired. Links last 2 minutes.',
};

/** No session: a public Console goes to passkey sign-in; the local one asks for a fresh link. */
export function SignedOut({ reason }: { reason: SignedOutReason }) {
  const [local, setLocal] = useState(false);
  useEffect(() => {
    let live = true;
    void consoleMode().then((mode) => {
      if (!live) return;
      if (mode === 'public') location.assign('/login');
      else setLocal(true);
    });
    return () => {
      live = false;
    };
  }, []);
  if (!local) return null;
  return (
    <AuthPage title={<>Run <span className="font-mono text-[0.85em] tracking-normal">kith ui</span> to sign in again</>} description={reason ? WHY[reason] : 'For safety, kith only opens for the person at this computer.'}>
      <div className="border border-line px-4 py-3">
        <Copy text="kith ui" />
      </div>
      <Note>The link it opens works once.</Note>
      {location.hostname === 'localhost' && (
        <AuthLink href="/login">Have a passkey for this Console? Sign in with it</AuthLink>
      )}
    </AuthPage>
  );
}
