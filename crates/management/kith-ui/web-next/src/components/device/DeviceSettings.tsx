import { useEffect, useId, useMemo, useState } from 'react';
import type { DeviceSettingsInfo, ExitView, Outcome, PeerInfo } from '@kith/types';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@ui-sdk/react/select';
import { Switch, SwitchThumb } from '@ui-sdk/react/switch';
import { ApiError, api, errorMessage, net } from '../../lib/api';
import { useFetch } from '../../lib/data';
import { Button, Panel, toast } from '../../ui';

const OWN = '__own';

/** Where this device's internet traffic goes, whether it may change that itself, and pause. */
export function DeviceSettings({ network, peer, admin, onSaved, span }: { network: string; peer: PeerInfo; admin: boolean; onSaved: () => void; span?: 4 | 12 }) {
  const { data: exits } = useFetch<ExitView[]>(net(network, '/exits'), 15_000);
  const settings = peer.settings;
  const base = useMemo<DeviceSettingsInfo>(
    () => ({ exit: settings?.exit ?? null, exit_locked: settings?.exit_locked ?? false, paused: settings?.paused ?? false }),
    [settings?.exit, settings?.exit_locked, settings?.paused],
  );
  const [draft, setDraft] = useState(base);
  const [busy, setBusy] = useState(false);
  const exitId = useId();
  useEffect(() => setDraft(base), [base, peer.node]);
  const dirty = draft.exit !== base.exit || draft.exit_locked !== base.exit_locked || draft.paused !== base.paused;
  const options = useMemo(() => {
    const listed = (exits ?? [])
      .filter((e) => e.node !== peer.node && (e.allowed || e.node === base.exit || e.node === draft.exit))
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((e) => ({ value: e.node, label: e.name }));
    if (draft.exit && !listed.some((o) => o.value === draft.exit)) {
      listed.push({ value: draft.exit, label: exits ? 'Unavailable exit' : 'Loading exits' });
    }
    return [{ value: OWN, label: 'Its own connection' }, ...listed];
  }, [exits, peer.node, base.exit, draft.exit]);

  const save = async () => {
    setBusy(true);
    try {
      const out = await api.post<Outcome>(net(network, `/devices/${encodeURIComponent(peer.node)}/settings`), draft);
      toast(out?.state === 'awaiting_os_auth' ? 'Waiting for approval on the device' : 'Settings saved');
      onSaved();
    } catch (e) {
      const gone = e instanceof ApiError && e.code === 'not_found';
      toast.error('Could not save settings', { description: gone ? 'The network no longer lists this device. Remove it and invite it again.' : errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const toggle = (label: string, help: string, checked: boolean, set: (c: boolean) => void) => (
    <label className="flex items-center justify-between gap-4">
      <span className="min-w-0">
        <span className="block text-[14px] text-fg">{label}</span>
        <span className="block text-[13px] text-fg-3 [overflow-wrap:anywhere]">{help}</span>
      </span>
      <Switch className="shrink-0" aria-label={label} disabled={!admin || busy} checked={checked} onCheckedChange={set}>
        <SwitchThumb />
      </Switch>
    </label>
  );

  return (
    <Panel label="Internet traffic" span={span}>
      <div className="flex flex-col gap-5">
        <div className="flex flex-col gap-1.5">
          <span id={exitId} className="text-[13px] text-fg-2">Send internet traffic through</span>
          <Select
            disabled={!admin || busy}
            value={draft.exit ?? OWN}
            onValueChange={(v) => setDraft({ ...draft, exit: !v || v === OWN ? null : (v as string) })}
            items={options}
          >
            <SelectTrigger aria-labelledby={exitId} className="h-8 w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {options.map((o) => (
                <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
              ))}
            </SelectContent>
          </Select>
          {exits && options.length === 1 && <span className="text-[13px] text-fg-3">No other device offers an exit yet.</span>}
        </div>
        {toggle('Lock', 'The device cannot pick another exit itself.', draft.exit_locked, (c) => setDraft({ ...draft, exit_locked: c }))}
        {toggle('Pause', 'Stop all of its network traffic until resumed.', draft.paused, (c) => setDraft({ ...draft, paused: c }))}
        <div className="flex flex-wrap items-center justify-end gap-2 border-t border-line pt-4">
          {admin ? (
            <>
              {dirty && <Button variant="ghost" className="h-8" disabled={busy} onClick={() => setDraft(base)}>Reset</Button>}
              <Button variant="primary" className="h-8" disabled={!dirty} busy={busy} onClick={() => void save()}>Save</Button>
            </>
          ) : (
            <span className="mr-auto text-[13px] text-fg-3">You cannot change this device's settings.</span>
          )}
        </div>
      </div>
    </Panel>
  );
}
