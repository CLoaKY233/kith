import { useEffect, useState } from 'react';
import { Dialog, DialogBackdrop, DialogContent, DialogDescription, DialogTitle } from '@ui-sdk/react/dialog';
import { Field, FieldDescription, FieldError, FieldLabel } from '@ui-sdk/react/field';
import { Input } from '@ui-sdk/react/input';
import type { CreateNetworkIn, Outcome } from '@kith/types';
import { api, errorMessage } from '../../lib/api';
import { useLive } from '../../lib/data';
import { Button, toast } from '../../ui';
import { checkName, normalise } from './name';

type Props = { open: boolean; onOpenChange: (open: boolean) => void; device: string };

export function CreateNetwork({ open, onOpenChange, device }: Props) {
  const { refresh } = useLive();
  const [name, setName] = useState('home');
  const [error, setError] = useState<string | null>(null);
  const [suggestion, setSuggestion] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!open) return;
    setName('home');
    setError(null);
    setSuggestion(null);
  }, [open]);

  const submit = async () => {
    setBusy(true);
    try {
      const check = await checkName(name, `You already have a network called ${name}.`);
      if (check.problem) {
        setError(check.problem);
        setSuggestion(check.suggestion);
        return;
      }
      await api.post<Outcome>('/api/networks', { name: check.name, device_name: device } satisfies CreateNetworkIn);
      toast.success(`${check.name} is ready. You're its admin.`);
      onOpenChange(false);
      refresh();
    } catch (e) {
      toast.error('Could not create the network', { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !busy && onOpenChange(o)}>
      <DialogBackdrop />
      <DialogContent className="flex flex-col gap-4">
        <DialogTitle>Start a network</DialogTitle>
        <DialogDescription>{device ? `${device} becomes its first device, and its admin.` : 'This device becomes its first device, and its admin.'}</DialogDescription>
        <form onSubmit={(e) => (e.preventDefault(), void submit())} className="flex flex-col gap-4">
          <Field invalid={error !== null || undefined}>
            <FieldLabel>Network name</FieldLabel>
            <Input
              autoFocus
              className="w-full font-mono"
              maxLength={63}
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
              value={name}
              onValueChange={(v: string) => (setName(normalise(v)), setError(null), setSuggestion(null))}
            />
            {error === null && name !== '' && (
              <FieldDescription className="[overflow-wrap:anywhere]">Devices will be reachable as {device || 'this-device'}.{name}.kith.internal</FieldDescription>
            )}
            <FieldError>{error}</FieldError>
          </Field>
          {error !== null && suggestion !== null && (
            <div>
              <Button type="button" variant="ghost" size="sm" onClick={() => (setName(suggestion), setError(null), setSuggestion(null))}>Use {suggestion}</Button>
            </div>
          )}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>Cancel</Button>
            <Button type="submit" variant="primary" busy={busy} disabled={name === ''}>{busy ? 'Creating…' : 'Create network'}</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
