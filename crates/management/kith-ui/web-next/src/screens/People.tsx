import { useEffect, useMemo, useState } from 'react';
import type { AuthorityChangeIn, Me, PersonInvite, PersonRole, PersonView } from '@kith/types';
import { AlertDialog, AlertDialogBackdrop, AlertDialogContent, AlertDialogDescription, AlertDialogTitle } from '@ui-sdk/react/alert-dialog';
import { Menu, MenuContent, MenuItem, MenuSeparator, MenuTrigger } from '@ui-sdk/react/menu';
import { Dialog, DialogBackdrop, DialogClose, DialogContent, DialogDescription, DialogTitle, DialogTrigger } from '@ui-sdk/react/dialog';
import { Input } from '@ui-sdk/react/input';
import { SegmentedControl, SegmentedControlIndicator, SegmentedControlItem } from '@ui-sdk/react/segmented-control';
import { MoreHorizontalIcon, UserPlus } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import { useAuthority } from '../lib/authority';
import { addableRoles, canApprove, canRemove, roleChoices } from '../lib/people';
import { useFetch, useLive } from '../lib/data';
import { useRoute } from '../lib/router';
import { Badge, Button, Copy, Empty, Page, Section, Skeleton, StatusDot, fmt, toast } from '../ui';
import { AddPasskey } from '../components/people/AddPasskey';
import { ConsoleAccess } from '../components/people/ConsoleAccess';
import { askAdmin } from '../components/people/askAdmin';

/** How often the list is read again while someone may be making their passkey. */
const INVITED_POLL_MS = 10_000;

const ROLE_HINT: Record<PersonRole, string> = {
  member: 'Uses the network with their own devices.',
  admin: 'Also approves devices and edits names and access.',
  owner: 'Full control, including other owners.',
};
const label = 'block font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3';
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;
const people = (n: number) => (n === 1 ? '1 person' : `${n} people`);
const day = (t: number) => new Date(t).toLocaleDateString();
const WAITING_FIRST = { registered: 0, active: 1, invited: 2 } as const;
const RANK = { owner: 0, admin: 1, member: 2 } as const;
/** Waiting for approval first, then owners, admins, members, then unopened invites. */
const ordered = (list: readonly PersonView[]) =>
  [...list].sort((a, b) => WAITING_FIRST[a.state] - WAITING_FIRST[b.state] || RANK[a.role] - RANK[b.role] || a.name.localeCompare(b.name));

function AddPerson({ me, onDone }: { me: Me | null; onDone: () => void }) {
  const roles = me ? addableRoles(me) : (['member', 'admin', 'owner'] as const);
  const authority = useAuthority();
  const [confirming, setConfirming] = useState(false);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [role, setRole] = useState<PersonRole>('member');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [made, setMade] = useState<PersonInvite | null>(null);
  const reset = () => (setName(''), setRole('member'), setErr(null), setMade(null), setConfirming(false));

  const submit = async () => {
    setBusy(true);
    setErr(null);
    try {
      const invite = await api.post<PersonInvite>('/api/people', { name: name.trim(), role });
      setMade(invite);
      onDone();
      // On a Console, an admin's link works once the owner's passkey signs it.
      if (me && role !== 'member') {
        authority.prepare({ kind: 'add_admin', person: invite.person.id });
        setConfirming(true);
      }
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => (setOpen(o), o || reset())}>
      <DialogTrigger render={<Button variant="primary" icon={<UserPlus />} />}>Add person</DialogTrigger>
      <DialogBackdrop />
      <DialogContent className="flex w-[min(460px,calc(100vw-32px))] flex-col gap-4 text-left">
        <div className="space-y-1.5">
          <DialogTitle className="m-0 text-left">Add a person</DialogTitle>
          <DialogDescription className="m-0 text-left">They get a one-time link to register a passkey, then join devices as themselves.</DialogDescription>
        </div>
        {made && confirming ? (
          <div className="space-y-4">
            <p className="text-[14px] text-fg-2">
              Confirm with your passkey that you are making {made.person.name} an admin: their link works once you do.
            </p>
            {authority.error && <p role="alert" className="text-[13px] text-hot">{authority.error}</p>}
            <div className="flex justify-end gap-2">
              <DialogClose render={<Button variant="ghost" />}>Not now</DialogClose>
              <Button
                variant="primary"
                busy={authority.signing}
                onClick={() => void authority.confirm().then((ok) => ok && (setConfirming(false), onDone()))}
              >
                {authority.signing ? 'Waiting for passkey…' : 'Confirm with passkey'}
              </Button>
            </div>
          </div>
        ) : made ? (
          <div className="space-y-4">
            {made.qr_svg && <div role="img" className="mx-auto w-40 bg-white p-2" aria-label="Invite QR code" dangerouslySetInnerHTML={{ __html: made.qr_svg }} />}
            <p className="font-mono text-[13px] text-fg [overflow-wrap:anywhere]">{made.link}</p>
            <p className="text-[13px] text-fg-2" title={fmt.absolute(made.expires_at_ms)}>Works once. Expires {day(made.expires_at_ms)}.</p>
            <div className="flex items-center justify-between gap-3">
              <Copy text={made.link} label="Copy link" />
              <DialogClose render={<Button />}>Done</DialogClose>
            </div>
          </div>
        ) : (
          <form
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            <label className="block space-y-2">
              <span className={label + ' text-left'}>Name</span>
              <Input className="w-full" value={name} onValueChange={setName} placeholder="Ada Lovelace" autoFocus />
            </label>
            <div className="space-y-2">
              <span className={label + ' text-left'}>Role</span>
              <SegmentedControl aria-label="Role" value={role} onValueChange={(v) => v && setRole(v as PersonRole)} className="w-full">
                <SegmentedControlIndicator />
                {roles.map((r) => (
                  <SegmentedControlItem key={r} value={r} className="flex-1 capitalize">{r}</SegmentedControlItem>
                ))}
              </SegmentedControl>
              <p className="text-left text-[13px] text-fg-2">{ROLE_HINT[role]}</p>
            </div>
            {err && <p role="alert" className="text-[13px] text-hot">{err}</p>}
            <div className="flex justify-end gap-2 pt-2">
              <DialogClose render={<Button />}>Cancel</DialogClose>
              <Button type="submit" variant="primary" busy={busy} disabled={!name.trim() || busy}>Create invite</Button>
            </div>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}

type Change = Extract<AuthorityChangeIn, { kind: 'add_admin' | 'set_role' | 'remove_person' }>;
type Pending = { person: PersonView; change: Change };

function dialogText({ person, change }: Pending, network: string) {
  const n = person.name;
  if (change.kind === 'add_admin') {
    return { title: `Approve ${n}?`, body: `${n} can then sign in to ${network} as ${person.role === 'member' ? 'a member' : `an ${person.role}`}.`, confirm: `Approve ${n}`, done: `${n} can sign in now.`, danger: false };
  }
  if (change.kind === 'remove_person') {
    const d = person.devices.length;
    return {
      title: `Remove ${n}?`,
      body: d === 0 ? `${n} can no longer sign in.` : `${n} can no longer sign in, and ${d === 1 ? 'their device loses' : `their ${d} devices lose`} access to ${network} within seconds.`,
      confirm: `Remove ${n}`,
      done: `Removed ${n}.`,
      danger: true,
    };
  }
  return change.role === 'admin'
    ? { title: `Make ${n} an admin?`, body: `${n} will be able to add and remove devices and people, and change names, exits and access in ${network}.`, confirm: 'Make admin', done: `${n} is now an admin.`, danger: false }
    : { title: `Make ${n} a member?`, body: `${n} can no longer change anything in ${network}. Their devices stay.`, confirm: 'Make member', done: `${n} is now a member.`, danger: false };
}

/** Confirms an owner change and signs it with the signed-in person's passkey. */
function ChangeDialog({ pending, network, onClose, onDone }: { pending: Pending | null; network: string; onClose: () => void; onDone: () => void }) {
  const authority = useAuthority();
  const { prepare } = authority;
  useEffect(() => {
    if (pending) prepare(pending.change);
  }, [pending, prepare]);
  const text = pending ? dialogText(pending, network) : null;
  const confirm = async () => {
    if (!text || !(await authority.confirm())) return;
    toast.success(text.done);
    onClose();
    onDone();
  };
  return (
    <AlertDialog open={pending !== null} onOpenChange={(o) => o || onClose()}>
      <AlertDialogBackdrop />
      {text && (
        <AlertDialogContent className="flex flex-col gap-4">
          <div className="space-y-1.5">
            <AlertDialogTitle className="m-0 text-left">{text.title}</AlertDialogTitle>
            <AlertDialogDescription className="m-0 text-left">{text.body}</AlertDialogDescription>
          </div>
          <p className="text-[13px] text-fg-2">{authority.signing ? 'Waiting for your passkey…' : 'Your passkey confirms this.'}</p>
          {authority.error && <p role="alert" className="text-[13px] text-hot">{authority.error}</p>}
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" onClick={onClose}>Cancel</Button>
            <Button variant={text.danger ? 'danger' : 'primary'} busy={authority.signing} onClick={() => void confirm()}>{text.confirm}</Button>
          </div>
        </AlertDialogContent>
      )}
    </AlertDialog>
  );
}

function PersonActions({ me, person, onChange, onReload }: { me: Me | null; person: PersonView; onChange: (c: Change) => void; onReload: () => void }) {
  const roles = roleChoices(me, person);
  const removable = canRemove(me, person);
  if (roles.length === 0 && !removable) return null;
  const takeBack = async () => {
    try {
      await api.post(`/api/people/${encodeURIComponent(person.id)}/remove`);
      toast.success(person.state === 'invited' ? `${person.name}'s link no longer works.` : `Removed ${person.name}.`);
      onReload();
    } catch (e) {
      toast.error(errorMessage(e));
    }
  };
  return (
    <Menu
      onAction={(v) => {
        if (v === 'admin' || v === 'member') onChange({ kind: 'set_role', person: person.id, role: v });
        else if (v === 'remove') onChange({ kind: 'remove_person', person: person.id });
        else if (v === 'cancel') void takeBack();
      }}
    >
      <MenuTrigger render={<Button size="sm" variant="ghost" icon={<MoreHorizontalIcon />} aria-label={`Actions for ${person.name}`} />} />
      <MenuContent>
        {roles.map((r) => <MenuItem key={r} value={r}>{r === 'admin' ? 'Make admin' : 'Make member'}</MenuItem>)}
        {roles.length > 0 && removable && <MenuSeparator />}
        {removable && (person.state === 'active'
          ? <MenuItem value="remove" className="text-hot">Remove…</MenuItem>
          : <MenuItem value="cancel" className="text-hot">{person.state === 'invited' ? 'Take back invite' : 'Turn down'}</MenuItem>)}
      </MenuContent>
    </Menu>
  );
}

function state(p: PersonView) {
  const passkeys = plural(p.passkeys, 'passkey');
  if (p.state === 'active') return `${plural(p.devices.length, 'device')} · ${passkeys}`;
  if (p.state === 'invited') {
    if (!p.expires_at_ms) return 'Invited';
    return p.expires_at_ms < Date.now() ? 'Invite expired' : `Invited, expires ${day(p.expires_at_ms)}`;
  }
  return `Waiting for approval · ${passkeys}`;
}

export function People() {
  const { boot } = useLive();
  const { go } = useRoute();
  const { data, error, reload } = useFetch<PersonView[]>('/api/people', 15000);
  const peers = boot?.peers;
  const devices = useMemo(() => {
    const m = new Map<string, { name: string; online: boolean }>();
    for (const list of Object.values(peers ?? {})) for (const p of list) m.set(p.node, { name: p.name, online: p.online });
    return m;
  }, [peers]);
  const sorted = useMemo(() => (data ? ordered(data) : []), [data]);
  const local = !data && boot?.status.mode === 'local';
  const failed = !data && !local && !!error;
  const me = boot?.status.me ?? null;
  const network = boot?.status.networks[0]?.name ?? 'the network';
  const [pending, setPending] = useState<Pending | null>(null);
  const canAdd = !me || addableRoles(me).length > 0;
  const node = boot?.status.node ?? null;
  const admins = sorted.filter((p) => p.state === 'active' && p.role !== 'member').map((p) => p.name);
  const invited = data?.some((p) => p.state === 'invited') ?? false;

  // Making a passkey writes no journal entry, so an open invite is checked on a timer.
  useEffect(() => {
    if (!invited) return;
    const timer = setInterval(() => document.visibilityState === 'visible' && reload(), INVITED_POLL_MS);
    return () => clearInterval(timer);
  }, [invited, reload]);

  return (
    <Page
      title="People"
      description="Who is on the network, how they sign in and which devices are theirs."
      actions={data && data.length > 0 && canAdd ? <AddPerson me={me} onDone={reload} /> : undefined}
    >
      {local && (
        <Section title="Not available here">
          <div className="max-w-2xl border border-line p-4 text-[14px] text-fg-2 md:p-6">
            People are managed on a kith Console. This page talks to the daemon on this device, which has no accounts:
            devices join with invite links from Network instead.
            <div className="mt-4">
              <Button size="sm" onClick={() => go('/settings/network')}>Go to Network</Button>
            </div>
          </div>
        </Section>
      )}
      {data && me?.role === 'owner' && node && <ConsoleAccess node={node} />}
      {data && me && !canAdd && <p className="m-0 max-w-2xl text-[14px] text-fg-2">{askAdmin(admins)}</p>}
      {failed && (
        <div className="w-full min-w-0 border border-line">
          <Empty action={<Button size="sm" onClick={reload}>Try again</Button>}>Couldn't load people. {error === 'Something went wrong. Try again.' ? '' : error}</Empty>
        </div>
      )}
      {!data && !error && !local && <Skeleton height={160} />}
      {data && data.length === 0 && (
        <div className="w-full min-w-0 border border-line"><Empty action={canAdd ? <AddPerson me={me} onDone={reload} /> : undefined}>{canAdd ? 'No one here yet. Add a person to invite them.' : 'No one here yet.'}</Empty></div>
      )}
      {data && data.length > 0 && (
        <Section title="Everyone" description={people(data.length)}>
          <ul className="w-full min-w-0 border border-line">
              {sorted.map((p) => (
                <li key={p.id} className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line px-4 py-3.5 last:border-0">
                  <div className="min-w-[12rem] flex-1">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate text-[14px] text-fg" title={p.name}>{p.name}</span>
                      {p.me && <Badge tone="strong">You</Badge>}
                    </div>
                    <div className="mt-0.5 text-[13px] text-fg-2 [overflow-wrap:anywhere]" title={p.expires_at_ms ? fmt.absolute(p.expires_at_ms) : undefined}>{state(p)}</div>
                    {p.devices.length > 0 && (
                      <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1">
                        {p.devices.map((d) => {
                          const dev = devices.get(d);
                          return (
                            <button
                              key={d}
                              type="button"
                              disabled={!dev}
                              onClick={() => dev && go(`/devices/${encodeURIComponent(dev.name)}`)}
                              title={dev ? undefined : 'Not on this network any more'}
                              className="inline-flex max-w-full items-center gap-1.5 font-mono text-[12px] text-fg-2 hover:text-fg focus-visible:text-fg disabled:hover:text-fg-2"
                            >
                              <StatusDot status={dev?.online ? 'online' : 'offline'} />
                              <span className="truncate" title={dev?.name}>{dev?.name ?? 'Unknown device'}</span>
                            </button>
                          );
                        })}
                      </div>
                    )}
                  </div>
                  <div className="ml-auto flex shrink-0 items-center gap-2">
                    {p.me && p.state === 'active' && me && <AddPasskey onDone={reload} />}
                    <Badge tone={p.role === 'owner' ? 'strong' : 'neutral'}>{p.role}</Badge>
                    {canApprove(me, p) && (
                      <Button size="sm" variant="primary" onClick={() => setPending({ person: p, change: { kind: 'add_admin', person: p.id } })}>Approve…</Button>
                    )}
                    <PersonActions me={me} person={p} onChange={(change) => setPending({ person: p, change })} onReload={reload} />
                  </div>
                </li>
              ))}
          </ul>
        </Section>
      )}
      <ChangeDialog pending={pending} network={network} onClose={() => setPending(null)} onDone={reload} />
    </Page>
  );
}
