import { useState } from 'react';
import { Field, FieldDescription, FieldError, FieldLabel } from '@ui-sdk/react/field';
import { Input } from '@ui-sdk/react/input';
import { errorMessage } from '../../lib/api';
import { Button, Page, Panel, PanelGrid } from '../../ui';
import { CreateNetwork } from './CreateNetwork';
import { JoinNetwork } from './JoinNetwork';
import { checkName, normalise, useDeviceName } from './name';

/** This device is in no network yet: rename it, then start a network or join one. */
export function Welcome() {
  const host = useDeviceName();
  const [flow, setFlow] = useState<'start' | 'join' | null>(null);
  const close = (open: boolean) => !open && setFlow(null);
  return (
    <Page title="Start or join a network" description="A private network between your devices. No server. No account.">
      <PanelGrid>
        <Choice title="Start a network" body="Make this device the first one in it, and its admin." action="Start" primary onClick={() => setFlow('start')} />
        <Choice title="Join a network" body="Use an invite link, or ask an admin to let this device in." action="Join" onClick={() => setFlow('join')} />
      </PanelGrid>
      <Rename name={host.name} os={host.os} onSave={host.set} />
      <CreateNetwork open={flow === 'start'} onOpenChange={close} device={host.name} />
      <JoinNetwork open={flow === 'join'} onOpenChange={close} device={host.name} />
    </Page>
  );
}

function Choice({ title, body, action, primary, onClick }: { title: string; body: string; action: string; primary?: boolean; onClick: () => void }) {
  return (
    <Panel span={6} label={title}>
      <div className="flex h-full flex-col items-start gap-4">
        <p className="text-[14px] leading-5 text-fg-2">{body}</p>
        <Button variant={primary ? 'primary' : 'secondary'} onClick={onClick}>{action}</Button>
      </div>
    </Panel>
  );
}

function Rename({ name, os, onSave }: { name: string; os: string; onSave: (name: string) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const stop = () => (setDraft(null), setError(null));
  const save = async (value: string) => {
    setBusy(true);
    try {
      const check = await checkName(value, `${value} is already taken.`);
      if (check.problem) return setError(check.problem);
      onSave(check.name);
      stop();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  if (draft === null)
    return (
      <div className="flex min-h-12 items-center justify-between gap-3 border border-line px-4 py-2 text-[13px] leading-5 text-fg-2">
        <span className="min-w-0 truncate" title={`${name || 'Unnamed'}${os ? ` · ${os}` : ''}`}>
          This device <span className="ml-1 font-mono text-fg">{name || 'unnamed'}</span>{os && <span className="text-fg-3"> · {os}</span>}
        </span>
        <Button className="shrink-0" variant="ghost" size="sm" onClick={() => (setDraft(name), setError(null))}>Rename</Button>
      </div>
    );
  return (
    <form className="flex max-w-[480px] flex-col gap-3" onSubmit={(e) => (e.preventDefault(), void save(draft))}>
      <Field invalid={error !== null || undefined}>
        <FieldLabel>This device's name</FieldLabel>
        <Input
          autoFocus
          className="w-full font-mono"
          maxLength={63}
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
          value={draft}
          onValueChange={(v: string) => (setDraft(normalise(v)), setError(null))}
          onKeyDown={(e) => e.key === 'Escape' && (e.preventDefault(), stop())}
        />
        {error === null && draft !== '' && <FieldDescription className="[overflow-wrap:anywhere]">Other devices will see it as {draft}.</FieldDescription>}
        <FieldError>{error}</FieldError>
      </Field>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={stop}>Cancel</Button>
        <Button type="submit" variant="primary" busy={busy} disabled={draft === ''}>Save name</Button>
      </div>
    </form>
  );
}
