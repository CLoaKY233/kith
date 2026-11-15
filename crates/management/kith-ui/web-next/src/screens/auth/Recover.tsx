import { Field, FieldLabel } from '@ui-sdk/react/field';
import { Input } from '@ui-sdk/react/input';
import { Textarea } from '@ui-sdk/react/textarea';
import { useState } from 'react';
import { accounts } from '../../lib/accounts';
import { ApiError } from '../../lib/api';
import { passkeyText } from '../../lib/passkey';
import { createPasskey } from '../../lib/webauthn';
import { Button } from '../../ui';
import { AuthLink, AuthPage, Problem } from './AuthPage';

const WORD_COUNT = 18;
const WRONG_WORDS = "Those words didn't work. Check all 18 and try again.";

export default function Recover() {
  const [name, setName] = useState('');
  const [words, setWords] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const recover = async () => {
    if (busy) return;
    const phrase = words.trim().split(/\s+/);
    if (phrase.length !== WORD_COUNT) return setError(`Enter all ${WORD_COUNT} words, separated by spaces. You entered ${phrase.length}.`);
    setBusy(true);
    setError(null);
    try {
      const options = await accounts.recoverBegin(name.trim());
      const made = await createPasskey(options.public_key);
      await accounts.recoverFinish(made, phrase.join(' '));
      location.assign('/');
    } catch (cause) {
      const wrong = cause instanceof ApiError && ['unauthorized', 'permission_denied', 'invalid_argument'].includes(cause.code);
      setError(wrong ? WRONG_WORDS : passkeyText(cause));
      setBusy(false);
    }
  };

  return (
    <AuthPage title="Recover with your words" description="Makes a new passkey for an owner who lost theirs.">
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          void recover();
        }}
      >
        <Field>
          <FieldLabel>Your name</FieldLabel>
          <Input className="w-full" value={name} onValueChange={setName} autoComplete="name" disabled={busy} required />
        </Field>
        <Field>
          <FieldLabel>The 18 recovery words</FieldLabel>
          <Textarea className="w-full font-mono" rows={4} value={words} onValueChange={setWords} autoComplete="off" spellCheck={false} disabled={busy} required />
        </Field>
        <Button type="submit" variant="primary" className="w-full" busy={busy} disabled={!name.trim() || !words.trim()}>
          {busy ? 'Making a passkey…' : 'Make a new passkey'}
        </Button>
        {error && <Problem>{error}</Problem>}
      </form>
      <AuthLink href="/login">Back to sign in</AuthLink>
    </AuthPage>
  );
}
