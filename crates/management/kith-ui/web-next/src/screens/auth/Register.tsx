import { useEffect, useRef, useState } from 'react';
import type { RegisterInfo } from '@kith/types';
import { accounts } from '../../lib/accounts';
import { ApiError } from '../../lib/api';
import { PASSKEY_UNSUPPORTED, passkeyText } from '../../lib/passkey';
import { createPasskey, passkeysSupported, type CreationOptionsJSON } from '../../lib/webauthn';
import { Button, Skeleton } from '../../ui';
import { AuthLink, AuthPage, Note, Problem } from './AuthPage';

type Phase = 'loading' | 'ready' | 'creating' | 'registered' | 'blocked';
/** Prompts are good for 5 minutes on the server; ask again before they go stale. */
const PROMPT_FRESH_MS = 4 * 60 * 1000;
const article = (role: string) => (role === 'member' ? 'a member' : `an ${role}`);

/** Why the link or the prompt failed, and whether trying again can help. */
function explain(cause: unknown, inviter: string): { text: string; final: boolean } {
  if (cause instanceof ApiError) {
    switch (cause.code) {
      case 'session_expired':
        return { text: 'This link expired. Links last 7 days: ask for a new one.', final: true };
      case 'session_used':
        return { text: 'This link was already used. Sign in with the passkey made with it.', final: true };
      case 'not_found':
        return { text: "This link isn't valid. Check it was copied whole.", final: true };
      case 'permission_denied':
        return {
          text: `This link works once ${inviter || 'whoever invited you'} confirms it with their passkey. Ask them to finish adding you, then try again.`,
          final: false,
        };
    }
  }
  return { text: passkeyText(cause), final: false };
}

export default function Register({ token }: { token: string }) {
  const [info, setInfo] = useState<RegisterInfo | null>(null);
  const [phase, setPhase] = useState<Phase>('loading');
  const [problem, setProblem] = useState<string | null>(null);
  const [retry, setRetry] = useState(false);
  const options = useRef<{ made: number; promise: Promise<CreationOptionsJSON> } | null>(null);

  const fetchOptions = () => {
    const next = accounts.registerBegin(token).then((answer) => answer.public_key);
    next.catch(() => undefined);
    options.current = { made: Date.now(), promise: next };
  };

  useEffect(() => {
    let live = true;
    accounts.registerInfo(token).then(
      (answer) => {
        if (!live) return;
        setInfo(answer);
        if (answer.registered) setPhase('registered');
        else if (!passkeysSupported()) (setProblem(PASSKEY_UNSUPPORTED), setPhase('blocked'));
        else (setPhase('ready'), fetchOptions());
      },
      (cause: unknown) => {
        if (!live) return;
        const why = explain(cause, '');
        setProblem(why.text);
        setRetry(!why.final);
        setPhase('blocked');
      },
    );
    return () => {
      live = false;
    };
  }, [token]);

  const create = async () => {
    setPhase('creating');
    setProblem(null);
    try {
      if (!options.current || Date.now() - options.current.made > PROMPT_FRESH_MS) fetchOptions();
      await accounts.registerFinish(token, await createPasskey(await options.current!.promise));
      setPhase('registered');
    } catch (cause) {
      const why = explain(cause, info?.invited_by ?? '');
      setProblem(why.text);
      setPhase(why.final ? 'blocked' : 'ready');
      if (!why.final) fetchOptions();
    }
  };

  return (
    <AuthPage title={info ? `Join ${info.network.name}` : 'Join kith'}>
      {phase === 'loading' && <Skeleton height={88} />}
      {phase === 'blocked' && problem && (
        <>
          <Problem>{problem}</Problem>
          {retry && (
            <Button variant="primary" className="w-full" onClick={() => location.reload()}>
              Try again
            </Button>
          )}
          <AuthLink href="/login">Go to sign in</AuthLink>
        </>
      )}
      {phase === 'registered' && info && (
        <>
          <Note>Your passkey is made. {info.invited_by || 'Whoever invited you'} approves you next, with their passkey. Then sign in here.</Note>
          <AuthLink href="/login">Go to sign in</AuthLink>
        </>
      )}
      {(phase === 'ready' || phase === 'creating') && info && (
        <>
          <div className="flex flex-col gap-2">
            <Note>Hi {info.name}. {info.invited_by || 'Someone'} invited you as {article(info.role)}.</Note>
            <Note>Create a passkey to sign in. It stays on this device, behind Touch ID, Face ID or your device password.</Note>
          </div>
          <Button variant="primary" className="w-full" busy={phase === 'creating'} onClick={() => void create()}>
            {phase === 'creating' ? 'Creating…' : 'Create passkey'}
          </Button>
          {problem && <Problem>{problem}</Problem>}
        </>
      )}
    </AuthPage>
  );
}
