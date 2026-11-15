import { Field, FieldDescription, FieldError, FieldLabel } from '@ui-sdk/react/field';
import { Input } from '@ui-sdk/react/input';
import { useEffect, useRef, useState } from 'react';
import type { SetupInfo } from '@kith/types';
import { accounts } from '../../lib/accounts';
import { ApiError } from '../../lib/api';
import {
  initialSetup,
  networkNameProblem,
  personNameProblem,
  setupProblem,
  setupProblemText,
  setupStep,
  setupStepNumber,
  type SetupEvent,
  type SetupState,
} from '../../lib/setup';
import { createPasskey, passkeysSupported, signWithPasskey } from '../../lib/webauthn';
import { Button, Copy, Skeleton } from '../../ui';
import { AuthLink, AuthPage, Note, Problem } from './AuthPage';

const STEPS = ['Names', 'Passkey', 'Recovery'];

function Steps({ at }: { at: number }) {
  return (
    <ol aria-label="Setup progress" className="grid grid-cols-3 gap-2 font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">
      {STEPS.map((label, i) => (
        <li
          key={label}
          aria-current={i + 1 === at ? 'step' : undefined}
          className={`border-t pt-2 ${i + 1 === at ? 'border-fg text-fg' : i + 1 < at ? 'border-fg-3' : 'border-line'}`}
        >
          <span className="block truncate">{label}</span>
        </li>
      ))}
    </ol>
  );
}

/** The recovery words, shown once; none when the network already had a recovery code. */
async function recoveryWords(): Promise<readonly string[]> {
  try {
    return (await accounts.recovery()).words;
  } catch (cause) {
    if (cause instanceof ApiError && cause.code === 'conflict') return [];
    throw cause;
  }
}

export default function Setup({ token }: { token: string }) {
  const [wizard, setWizard] = useState<SetupState>(() => initialSetup(passkeysSupported()));
  const [info, setInfo] = useState<SetupInfo | null>(null);
  const [network, setNetwork] = useState('home');
  const [person, setPerson] = useState('');
  const [networkError, setNetworkError] = useState<string | null>(null);
  const [personError, setPersonError] = useState<string | null>(null);
  const confirmed = useRef(false);
  const send = (event: SetupEvent) => setWizard((w) => setupStep(w, event));
  const fail = (cause: unknown, stage?: 'link' | 'form' | 'account') =>
    send({ type: 'failed', problem: setupProblem(cause, stage) });
  const body = useRef<HTMLDivElement>(null);
  const named = info?.network != null;
  const step = setupStepNumber(wizard);

  useEffect(() => {
    if (wizard.phase === 'blocked') return;
    let current = true;
    accounts.setupInfo(token).then(
      (answer) => {
        if (!current) return;
        setInfo(answer);
        if (answer.network) setNetwork(answer.network.name);
      },
      (cause: unknown) => {
        if (!current) return;
        fail(cause);
        // Not knowing is no reason to stop: the server ignores a network name it has.
        setInfo({ network: null, expires_at_ms: 0 });
      },
    );
    return () => {
      current = false;
    };
  }, [token]);

  // A step's button goes away with the step, so focus moves to the new step for keyboards and readers.
  const lastStep = useRef(step);
  useEffect(() => {
    if (lastStep.current !== step && step !== null) body.current?.focus({ preventScroll: true });
    lastStep.current = step;
  }, [step]);

  useEffect(() => {
    if (wizard.phase === 'done') location.assign('/');
  }, [wizard.phase]);

  const start = async () => {
    if (wizard.phase !== 'names') return;
    const netProblem = named ? null : networkNameProblem(network);
    const personProblem = personNameProblem(person);
    setNetworkError(netProblem);
    setPersonError(personProblem);
    if (netProblem || personProblem) return;
    send({ type: 'start' });
    try {
      const answer = await accounts.setupBegin(token, person.trim(), named ? null : network);
      send({ type: 'started', options: answer.public_key });
    } catch (cause) {
      fail(cause, 'form');
    }
  };

  const create = async () => {
    if (wizard.phase !== 'passkey') return;
    const options = wizard.options;
    send({ type: 'create' });
    try {
      const { authority } = await accounts.setupFinish(token, await createPasskey(options));
      send({ type: 'created', authority: authority.public_key });
    } catch (cause) {
      fail(cause);
    }
  };

  const confirm = async () => {
    if (wizard.phase !== 'confirm') return;
    const authority = wizard.authority;
    send({ type: 'confirm' });
    try {
      if (!confirmed.current) {
        await accounts.authorityFinish(await signWithPasskey(authority));
        confirmed.current = true;
      }
      send({ type: 'confirmed', words: await recoveryWords() });
    } catch (cause) {
      fail(cause, 'account');
    }
  };

  const problem =
    wizard.phase === 'names' || wizard.phase === 'passkey' || wizard.phase === 'confirm' ? wizard.problem : null;
  const shownNetwork = info?.network?.name ?? (network || 'home');

  return (
    <AuthPage title="Set up kith">
      <div ref={body} tabIndex={-1} className="flex flex-col gap-6 outline-none">
      {step !== null && <Steps at={step} />}
      {wizard.phase === 'blocked' && (
        <>
          <Problem>{setupProblemText(wizard.problem)}</Problem>
          {wizard.problem.kind === 'used' && (
            <AuthLink href="/login">Go to sign in</AuthLink>
          )}
        </>
      )}
      {step === 1 && info === null && <Skeleton height={160} />}
      {step === 1 && info !== null && (
        <form
          className="flex flex-col gap-5"
          onSubmit={(e) => {
            e.preventDefault();
            void start();
          }}
        >
          <Note>{named ? <>You become the owner of <span className="[overflow-wrap:anywhere]">{shownNetwork}</span>.</> : 'Name your network and yourself. Both can change later.'}</Note>
          {!named && (
            <Field invalid={networkError !== null || undefined}>
              <FieldLabel>Network name</FieldLabel>
              <Input
                className="w-full font-mono"
                maxLength={63}
                spellCheck={false}
                autoCapitalize="none"
                autoCorrect="off"
                autoComplete="off"
                value={network}
                onValueChange={(v) => (setNetwork(v.toLowerCase().replace(/\s+/g, '-')), setNetworkError(null))}
              />
              <FieldDescription className="[overflow-wrap:anywhere]">Devices will be reachable as laptop.{network || 'home'}.kith.internal</FieldDescription>
              <FieldError>{networkError}</FieldError>
            </Field>
          )}
          <Field invalid={personError !== null || undefined}>
            <FieldLabel>Your name</FieldLabel>
            <Input className="w-full" value={person} onValueChange={(v) => (setPerson(v), setPersonError(null))} autoComplete="name" />
            <FieldDescription>Shown to the people you add.</FieldDescription>
            <FieldError>{personError}</FieldError>
          </Field>
          {problem && <Problem>{setupProblemText(problem)}</Problem>}
          <Button type="submit" variant="primary" className="w-full" busy={wizard.phase === 'starting'}>Continue</Button>
        </form>
      )}
      {(wizard.phase === 'passkey' || wizard.phase === 'creating') && (
        <>
          <Note>
            Create your passkey. You sign in with it, and it approves every change to who is in the network. It stays on
            this device, behind Touch ID, Face ID or your device password.
          </Note>
          {problem && <Problem>{setupProblemText(problem)}</Problem>}
          <div className="flex flex-col gap-2">
            <Button variant="primary" className="w-full" busy={wizard.phase === 'creating'} onClick={() => void create()}>
              {wizard.phase === 'creating' ? 'Creating…' : 'Create passkey'}
            </Button>
            <Button variant="ghost" className="w-full" disabled={wizard.phase !== 'passkey'} onClick={() => send({ type: 'back' })}>Back</Button>
          </div>
        </>
      )}
      {(wizard.phase === 'confirm' || wizard.phase === 'confirming') && (
        <>
          <Note>Your passkey is made. Confirm once more so it signs you into <span className="[overflow-wrap:anywhere]">{shownNetwork}</span> as its owner.</Note>
          {problem && <Problem>{setupProblemText(problem)}</Problem>}
          <Button variant="primary" className="w-full" busy={wizard.phase === 'confirming'} onClick={() => void confirm()}>
            {wizard.phase === 'confirming' ? 'Confirming…' : 'Confirm with your passkey'}
          </Button>
        </>
      )}
      {wizard.phase === 'words' && (
        <>
          <Note>
            Save your recovery words. If every owner and admin loses their passkey, these words are the only way back in.
            They are shown once.
          </Note>
          <div className="border border-line">
            <ol aria-label="Recovery words" className="grid list-none grid-cols-2 gap-x-4 gap-y-2 p-4 font-mono text-[13px] text-fg sm:grid-cols-3">
              {wizard.words.map((word, i) => (
                <li key={i} className="flex min-w-0 gap-2">
                  <span aria-hidden="true" className="w-[2ch] shrink-0 text-right tabular-nums text-fg-3">{i + 1}</span>
                  <span className="truncate" title={word}>{word}</span>
                </li>
              ))}
            </ol>
            <div className="flex justify-end border-t border-line px-4 py-2">
              <Copy text={wizard.words.join(' ')} label="Copy words" />
            </div>
          </div>
          <Button variant="primary" className="w-full" onClick={() => send({ type: 'saved' })}>I saved them</Button>
        </>
      )}
      {wizard.phase === 'done' && <Note>Opening the Console…</Note>}
      </div>
    </AuthPage>
  );
}
