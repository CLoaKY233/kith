import { useState } from 'react';
import { accounts } from '../../lib/accounts';
import { passkeyText } from '../../lib/passkey';
import { createPasskey, signWithPasskey } from '../../lib/webauthn';
import { Button, toast } from '../../ui';

/** Makes another passkey, then signs its addition with one the person already has. */
export function AddPasskey({ onDone }: { onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const add = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const made = await createPasskey((await accounts.passkeyBegin()).public_key);
      const prompt = await accounts.passkeyFinish(made);
      await accounts.authorityFinish(await signWithPasskey(prompt.public_key));
      toast.success('Your new passkey works now.');
      onDone();
    } catch (cause) {
      toast.error(passkeyText(cause));
    } finally {
      setBusy(false);
    }
  };
  return <Button size="sm" busy={busy} onClick={() => void add()}>Add another passkey</Button>;
}
