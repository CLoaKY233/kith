import { useEffect, useRef, useState } from 'react';
import { Dialog, DialogBackdrop, DialogContent, DialogDescription, DialogTitle } from '@ui-sdk/react/dialog';
import { Field, FieldDescription, FieldError, FieldLabel } from '@ui-sdk/react/field';
import { Input } from '@ui-sdk/react/input';
import type { JoinIn, JoinRequestState, Outcome, RequestJoinIn } from '@kith/types';
import { ApiError, api, errorMessage } from '../../lib/api';
import { useLive } from '../../lib/data';
import { Button, toast } from '../../ui';

type Target = { kind: 'invite'; link: string } | { kind: 'request'; code: string };

// Protocol forms: `kith://join/…`, `kith://request/…` and `network@admin`, alone or after `kith join`.
const INVITE = /^kith:\/\/join\/\S+$/i;
const REQUEST = /^kith:\/\/request\/\S+$/i;
const NETWORK_AT_ADMIN = /^([a-z0-9-]+)@([a-z0-9-]+)$/i;
const COMMAND = /^kith\s+join\s+/i;
const ASK_EVERY_MS = 3000;

function recognise(raw: string): Target | null {
  const text = raw.trim().replace(COMMAND, '');
  if (INVITE.test(text)) return { kind: 'invite', link: text };
  if (REQUEST.test(text)) return { kind: 'request', code: text };
  return NETWORK_AT_ADMIN.test(text) ? { kind: 'request', code: text.toLowerCase() } : null;
}

function joinFailure(e: unknown): string {
  const words = e instanceof ApiError ? `${e.code} ${e.detail}`.toLowerCase() : '';
  if (/\bused\b/.test(words)) return 'This invite has already been used. Ask for a new one.';
  if (words.includes('expired')) return 'This invite has expired. Ask for a new one.';
  return errorMessage(e);
}

type Props = { open: boolean; onOpenChange: (open: boolean) => void; device: string };

export function JoinNetwork({ open, onOpenChange, device }: Props) {
  const { refresh } = useLive();
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [asking, setAsking] = useState<{ code: string; answer: JoinRequestState } | null>(null);
  const run = useRef(0);
  useEffect(() => {
    run.current++;
    if (!open) return;
    setText('');
    setError(null);
    setBusy(false);
    setAsking(null);
  }, [open]);

  const entered = (network: string) => {
    toast.success(`You're in ${network}.`);
    onOpenChange(false);
    refresh();
  };

  // While an admin decides, ask again every few seconds; the daemon reuses the same request.
  useEffect(() => {
    if (!asking || asking.answer.resolution !== null) return;
    const mine = run.current;
    const timer = setTimeout(() => {
      api
        .post<JoinRequestState>('/api/join/request', { request_code: asking.code, device_name: device } satisfies RequestJoinIn)
        .then((answer) => {
          if (mine !== run.current) return;
          if (answer.resolution === 'approved') entered(answer.network_name);
          else setAsking({ code: asking.code, answer });
        })
        .catch(() => mine === run.current && setAsking({ ...asking }));
    }, ASK_EVERY_MS);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [asking, device]);

  const submit = async () => {
    const target = recognise(text);
    if (!target) {
      setError(text.trim() === '' ? 'Paste an invite link, or network@admin to ask an admin.' : 'This isn\'t an invite link. Invite links start with kith://join/, or write network@admin, such as home@studio-mac.');
      return;
    }
    const mine = run.current;
    setBusy(true);
    try {
      if (target.kind === 'invite') {
        await api.post<Outcome>('/api/join', { invite_link: target.link, device_name: device } satisfies JoinIn);
        if (mine === run.current) entered('the network');
      } else {
        const answer = await api.post<JoinRequestState>('/api/join/request', { request_code: target.code, device_name: device } satisfies RequestJoinIn);
        if (mine !== run.current) return;
        if (answer.resolution === 'approved') entered(answer.network_name);
        else setAsking({ code: target.code, answer });
      }
    } catch (e) {
      if (mine === run.current) setError(target.kind === 'invite' ? joinFailure(e) : errorMessage(e));
    } finally {
      if (mine === run.current) setBusy(false);
    }
  };

  const answer = asking?.answer ?? null;
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogBackdrop />
      <DialogContent className="flex flex-col gap-4">
        <DialogTitle>Join a network</DialogTitle>
        {answer === null ? (
          <>
            <DialogDescription>Paste an invite link, or ask an admin to let {device || 'this device'} in.</DialogDescription>
            <form onSubmit={(e) => (e.preventDefault(), void submit())} className="flex flex-col gap-4">
              <Field invalid={error !== null || undefined}>
                <FieldLabel>Invite link or network@admin</FieldLabel>
                <Input
                  autoFocus
                  className="w-full font-mono"
                  spellCheck={false}
                  autoCapitalize="none"
                  autoCorrect="off"
                  autoComplete="off"
                  value={text}
                  onValueChange={(v: string) => (setText(v), setError(null))}
                />
                {error === null && <FieldDescription>From kith invite, or the admin's Add device screen, such as home@studio-mac.</FieldDescription>}
                <FieldError className="[overflow-wrap:anywhere]">{error}</FieldError>
              </Field>
              <div className="flex justify-end gap-2">
                <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>Cancel</Button>
                <Button type="submit" variant="primary" busy={busy}>{busy ? 'Joining…' : recognise(text)?.kind === 'request' ? 'Ask to join' : 'Join'}</Button>
              </div>
            </form>
          </>
        ) : (
          <>
            <DialogDescription>
              {answer.resolution === 'denied'
                ? 'An admin denied this request.'
                : answer.resolution === 'expired'
                  ? 'No admin answered in time. Ask again.'
                  : `Waiting for an admin of ${answer.network_name}. Check that they see the same code.`}
            </DialogDescription>
            <p className="font-mono text-[32px] leading-none tracking-[0.08em] text-fg" aria-label="Check code">{answer.check_code}</p>
            <div className="flex justify-end gap-2">
              {answer.resolution !== null && <Button type="button" variant="ghost" onClick={() => (run.current++, setAsking(null), setBusy(false))}>Try again</Button>}
              <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>{answer.resolution === null ? 'Cancel' : 'Close'}</Button>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
