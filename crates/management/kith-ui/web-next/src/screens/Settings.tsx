import { useState } from 'react';
import type { Invite, OpenInvite, RelayView, ServerView } from '@kith/types';
import { Input } from '@ui-sdk/react/input';
import { NumberInput, NumberInputControl, NumberInputDecrement, NumberInputGroup, NumberInputIncrement } from '@ui-sdk/react/number-input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@ui-sdk/react/select';
import { Switch, SwitchThumb } from '@ui-sdk/react/switch';
import { Minus, Plus, Trash2 } from 'lucide-react';
import { api, errorMessage, net } from '../lib/api';
import { useFetch, useNetwork } from '../lib/data';
import { Button, Copy, Empty, Page, Section, Skeleton, StatusDot, fmt, toast } from '../ui';

const EXPIRY = [
  { value: '3600', label: '1 hour' },
  { value: '86400', label: '1 day' },
  { value: '604800', label: '7 days' },
  { value: '2592000', label: '30 days' },
];

function until(t: number) {
  const s = Math.max(0, Math.round((t - Date.now()) / 1000));
  if (s < 60) return 'under a minute';
  if (s < 3600) return `${Math.floor(s / 60)} min`;
  if (s < 86400) return `${Math.floor(s / 3600)} h`;
  return `${Math.floor(s / 86400)} d`;
}

const label = 'block font-mono text-[11px] leading-4 uppercase tracking-[0.08em] text-fg-3';
const field = 'flex min-w-0 flex-col gap-2';
const row = 'grid min-h-12 items-center gap-x-4 gap-y-1 border-b border-line px-4 py-3 last:border-0';
const note = 'text-[13px] text-fg-2';
const problem = 'text-[13px] text-hot [overflow-wrap:anywhere]';
const input = '!h-8 w-full !text-[13px] font-mono placeholder:text-fg-3';

function InviteForm({ id, onMade }: { id: string | null; onMade: () => void }) {
  const [uses, setUses] = useState<number | null>(1);
  const [expiry, setExpiry] = useState('86400');
  const [made, setMade] = useState<Invite | null>(null);
  const [busy, setBusy] = useState(false);

  const create = async () => {
    if (!id) return;
    setBusy(true);
    try {
      setMade(await api.post<Invite>(net(id, '/invites'), { expires_secs: Number(expiry), uses: uses && uses > 0 ? Math.floor(uses) : null }));
      onMade();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="border border-line p-4 md:p-6">
      <div className="grid grid-cols-1 items-end gap-4 sm:grid-cols-[minmax(0,1fr)_10rem] lg:grid-cols-[minmax(0,1fr)_10rem_auto]">
        <div className={field}>
          <span className={label}>Expires in</span>
          <Select value={expiry} onValueChange={(v) => v && setExpiry(v as string)} items={EXPIRY}>
            <SelectTrigger aria-label="Expires in" className="!h-8 w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {EXPIRY.map((o) => (
                <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className={field}>
          <span className={label}>Uses</span>
          <NumberInput className="w-full" value={uses} onValueChange={setUses} min={0} max={100}>
            <NumberInputGroup className="!h-8 w-full">
              <NumberInputDecrement aria-label="Fewer uses"><Minus size={13} /></NumberInputDecrement>
              <NumberInputControl aria-label="Uses" className="font-mono !text-[13px]" />
              <NumberInputIncrement aria-label="More uses"><Plus size={13} /></NumberInputIncrement>
            </NumberInputGroup>
          </NumberInput>
        </div>
        <Button variant="primary" className="!h-8 w-full sm:col-span-2 sm:w-auto sm:justify-self-start lg:col-span-1" busy={busy} disabled={!id} onClick={() => void create()}>Create invite</Button>
      </div>
      <p className={`${note} mt-3`}>Set uses to 0 for an invite that never runs out.</p>
      {made && (
        <div className="mt-6 flex flex-col gap-4 border-t border-line pt-6 sm:flex-row" aria-live="polite">
          {made.qr_svg && <div className="size-32 shrink-0 bg-white p-1.5 [&_svg]:size-full" role="img" aria-label="Invite QR code" dangerouslySetInnerHTML={{ __html: made.qr_svg }} />}
          <div className="min-w-0 flex-1 space-y-3">
            <p className="font-mono text-[13px] text-fg [overflow-wrap:anywhere]">{made.link}</p>
            <p className="text-[13px] text-fg-2">
              {made.uses ? `${made.uses} ${made.uses === 1 ? 'use' : 'uses'}` : 'Unlimited uses'} · expires in {until(made.expires_at_ms)}
            </p>
            <Copy text={made.link} label="Copy link" />
          </div>
        </div>
      )}
    </div>
  );
}

function Invites({ id, invites }: { id: string | null; invites: ReturnType<typeof useFetch<OpenInvite[]>> }) {
  const [busy, setBusy] = useState<string | null>(null);
  const revoke = async (inv: OpenInvite) => {
    if (!id) return;
    setBusy(inv.id);
    try {
      await api.post(net(id, `/invites/${encodeURIComponent(inv.id)}/revoke`));
      toast.success('Invite revoked');
      invites.reload();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  if (!id) return <p className={note}>Invites appear here once the network is reachable.</p>;
  if (invites.error && !invites.data) return <p className={problem}>Couldn't load invites. {invites.error}</p>;
  if (!invites.data) return <Skeleton height={96} />;
  const open = invites.data.filter((inv) => inv.expires_at_ms > Date.now());
  if (open.length === 0) return <div className="border border-line"><Empty>No open invites.</Empty></div>;
  return (
    <ul className="border border-line">
      {open.map((inv) => (
        <li key={inv.id} className={`${row} grid-cols-[minmax(0,1fr)_auto] sm:grid-cols-[minmax(0,1fr)_auto_auto]`}>
          <span className="min-w-0 truncate text-[14px] text-fg" title={fmt.absolute(inv.created_at_ms)}>Created {fmt.relative(inv.created_at_ms)}</span>
          <span className="order-last col-span-2 text-[13px] text-fg-2 sm:order-none sm:col-span-1 sm:whitespace-nowrap" title={`Expires ${fmt.absolute(inv.expires_at_ms)}`}>
            {inv.uses_left === null ? 'Unlimited uses' : `${inv.uses_left} ${inv.uses_left === 1 ? 'use' : 'uses'} left`} · expires in {until(inv.expires_at_ms)}
          </span>
          <Button size="sm" variant="danger" busy={busy === inv.id} aria-label={`Revoke invite created ${fmt.relative(inv.created_at_ms)}`} onClick={() => void revoke(inv)}>Revoke</Button>
        </li>
      ))}
    </ul>
  );
}

function Relays({ id, admin }: { id: string | null; admin: boolean }) {
  const { peers } = useNetwork();
  const relays = useFetch<RelayView[]>(id ? net(id, '/relays') : null);
  const [name, setName] = useState('');
  const [addr, setAddr] = useState('');
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);

  const add = async () => {
    const n = name.trim().toLowerCase();
    if (!id || !n || !addr.trim()) return;
    setBusy(true);
    try {
      await api.post(net(id, '/relays'), { name: n, address: addr.trim(), node: null });
      toast.success(`Relay ${n} added`);
      setName('');
      setAddr('');
      relays.reload();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const remove = async (r: RelayView) => {
    if (!id) return;
    setRemoving(r.name);
    try {
      await api.post(net(id, `/relays/${encodeURIComponent(r.name)}/remove`));
      toast.success(`Relay ${r.name} removed`);
      relays.reload();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setRemoving(null);
    }
  };

  return (
    <div className="space-y-4">
      {relays.error && !relays.data && <p className={problem}>Couldn't load relays. {relays.error}</p>}
      {!id && <p className={note}>Relays appear here once the network is reachable.</p>}
      {id && !relays.data && !relays.error && <Skeleton height={48} />}
      {relays.data && (
        <ul className="border border-line">
          {relays.data.length === 0 && <li><Empty>No relays. Devices use direct paths only.</Empty></li>}
          {[...relays.data].sort((a, b) => a.name.localeCompare(b.name)).map((r) => (
            <li key={r.name} className={`${row} grid-cols-[minmax(0,1fr)_auto] ${admin ? 'sm:grid-cols-[minmax(0,10rem)_minmax(0,1fr)_auto] lg:grid-cols-[minmax(0,10rem)_minmax(0,1fr)_auto_auto]' : 'sm:grid-cols-[minmax(0,10rem)_minmax(0,1fr)] lg:grid-cols-[minmax(0,10rem)_minmax(0,1fr)_auto]'}`}>
              <span className="min-w-0 truncate text-[14px] text-fg" title={r.name}>{r.name}</span>
              <span className="order-last col-span-2 min-w-0 truncate font-mono text-[13px] text-fg-2 sm:order-none sm:col-span-1" title={r.address}>{r.address}</span>
              {(() => {
                const host = r.node ? peers.find((p) => p.node === r.node)?.name : undefined;
                return <span className="hidden max-w-48 truncate whitespace-nowrap text-[13px] text-fg-3 lg:block" title={host ? `Runs on ${host}` : undefined}>{host && host !== r.name ? `on ${host}` : ''}</span>;
              })()}
              {admin && <Button size="sm" variant="ghost" icon={<Trash2 />} aria-label={`Remove ${r.name}`} busy={removing === r.name} onClick={() => void remove(r)} />}
            </li>
          ))}
        </ul>
      )}
      {!admin && <p className={note}>Ask an admin to add or remove relays.</p>}
      {admin && <form
        className="grid grid-cols-1 items-end gap-4 border border-line p-4 sm:grid-cols-2 md:p-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto]"
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        <label className={field}>
          <span className={label}>Name</span>
          <Input className={input} value={name} onValueChange={setName} placeholder="home-relay" autoComplete="off" spellCheck={false} autoCapitalize="none" />
        </label>
        <label className={field}>
          <span className={label}>Address</span>
          <Input className={input} value={addr} onValueChange={setAddr} placeholder="relay.example.com:3478" autoComplete="off" spellCheck={false} autoCapitalize="none" inputMode="url" />
        </label>
        <Button type="submit" className="!h-8 w-full sm:col-span-2 sm:w-auto sm:justify-self-start lg:col-span-1" icon={<Plus />} busy={busy} disabled={!id || !name.trim() || !addr.trim()}>Add relay</Button>
      </form>}
    </div>
  );
}

function ThisDevice() {
  const server = useFetch<ServerView>('/api/server');
  const [pick, setPick] = useState<{ next: ServerView; base: ServerView | null } | null>(null);
  const [saving, setSaving] = useState(false);
  // The choice stands until a fresh answer replaces the one it was made against.
  const shown = pick && pick.base === server.data ? pick.next : server.data;

  const set = async (next: ServerView) => {
    setSaving(true);
    setPick({ next, base: server.data });
    try {
      await api.post('/api/server', next);
      toast.success('Saved');
      server.reload();
    } catch (e) {
      setPick(null);
      toast.error(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  if (server.error && !shown) return <p className={problem}>Couldn't read this device's settings. {server.error}</p>;
  if (!shown) return <Skeleton height={120} />;
  const items: [keyof ServerView, string, string][] = [
    ['exit', 'Exit node', 'Others may send their internet traffic out through this device.'],
    ['relay', 'Relay', "Carry traffic between devices that can't reach each other."],
  ];
  return (
    <div className="border border-line">
      {items.map(([key, title, hint]) => (
        <label key={key} className={`flex items-center justify-between gap-4 border-b border-line px-4 py-3 last:border-0 ${saving ? 'cursor-progress' : 'cursor-pointer'}`}>
          <span className="min-w-0">
            <span className="block text-[14px] text-fg">{title}</span>
            <span className="block text-[13px] text-fg-2">{hint}</span>
          </span>
          <Switch className="shrink-0" aria-label={title} disabled={saving} checked={shown[key]} onCheckedChange={(c) => void set({ ...shown, [key]: c })}>
            <SwitchThumb />
          </Switch>
        </label>
      ))}
    </div>
  );
}

export function SettingsScreen() {
  const { boot, network } = useNetwork();
  const id = network?.id ?? null;
  const invites = useFetch<OpenInvite[]>(id && (network?.admin ?? true) ? net(id, '/invites') : null, 15000);
  const me = boot?.status.memberships.find((m) => m.network === id);
  const admin = network?.admin ?? true;

  return (
    <Page title="Network" description="Invite devices, run relays and choose what this device does for the others.">
      {admin ? (
        <>
          <Section title="Invite a device" description="Share the link or QR code with the device that should join.">
            <InviteForm id={id} onMade={invites.reload} />
          </Section>
          <Section title="Open invites">
            <Invites id={id} invites={invites} />
          </Section>
        </>
      ) : (
        <Section title="Invite a device">
          <div className="border border-line"><Empty>Only an admin can invite devices. Ask one to share a link with you.</Empty></div>
        </Section>
      )}
      <Section title="Relays" description="Relays carry traffic when two devices can't reach each other directly.">
        <Relays id={id} admin={admin} />
      </Section>
      <Section
        title="This device"
        description={me ? <span className="inline-flex items-center gap-2"><StatusDot status="online" /> {me.name}</span> : undefined}
      >
        <ThisDevice />
      </Section>
    </Page>
  );
}
