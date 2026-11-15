import { Menu, MenuContent, MenuItem, MenuSeparator, MenuTrigger } from '@ui-sdk/react/menu';
import { useCallback, useRef, useState } from 'react';
import { signOut } from '../lib/account';
import { accounts } from '../lib/accounts';
import { useLive } from '../lib/data';
import { passkeyText } from '../lib/passkey';
import { createPasskey, signWithPasskey } from '../lib/webauthn';
import { Button, toast } from '../ui';

const ROLE = { owner: 'Owner', admin: 'Admin', member: 'Member' } as const;

/** Makes another passkey, then signs its addition with one the person already has. */
async function addPasskey(): Promise<void> {
  try {
    const made = await createPasskey((await accounts.passkeyBegin()).public_key);
    const prompt = await accounts.passkeyFinish(made);
    await accounts.authorityFinish(await signWithPasskey(prompt.public_key));
    toast.success('Your new passkey works now.');
  } catch (cause) {
    toast.error(passkeyText(cause));
  }
}

function useMe() {
  return useLive().boot?.status.me ?? null;
}

/** Runs `addPasskey` one at a time, so a second click can't open a second prompt. */
function useAddPasskey() {
  const running = useRef(false);
  const [busy, setBusy] = useState(false);
  const run = useCallback(async () => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    try {
      await addPasskey();
    } finally {
      running.current = false;
      setBusy(false);
    }
  }, []);
  return { busy, run };
}

function Who({ name, role }: { name: string; role: keyof typeof ROLE }) {
  return (
    <div className="min-w-0 [overflow-wrap:anywhere]">
      <div className="text-[14px] text-fg">{name}</div>
      <div className="font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">{ROLE[role] ?? ''}</div>
    </div>
  );
}

/** Who is signed in to the Console, with sign out; nothing for the local link's session. */
export function AccountMenu() {
  const me = useMe();
  const { run } = useAddPasskey();
  if (!me) return null;
  return (
    <Menu onAction={(v) => void (v === 'passkey' ? run() : v === 'out' && signOut())}>
      <MenuTrigger
        title={me.name}
        aria-label={`Account, ${me.name}`}
        className="hidden !size-8 shrink-0 place-items-center !border !border-line-2 !bg-ink-2 !p-0 font-mono text-[11px] font-medium tracking-[0.04em] text-fg transition-colors duration-150 hover:!border-fg lg:grid"
      >
        {initials(me.name)}
      </MenuTrigger>
      <MenuContent>
        <div className="max-w-64 px-2 py-1.5">
          <Who name={me.name} role={me.role} />
        </div>
        <MenuSeparator />
        <MenuItem value="passkey">Add a passkey</MenuItem>
        <MenuItem value="out">Sign out</MenuItem>
      </MenuContent>
    </Menu>
  );
}

/** The same, laid out for the mobile drawer. */
export function AccountPanel() {
  const me = useMe();
  const { busy, run } = useAddPasskey();
  const [leaving, setLeaving] = useState(false);
  if (!me) return null;
  return (
    <div className="mt-8 flex flex-col items-start gap-3 border-t border-line pt-6">
      <Who name={me.name} role={me.role} />
      <Button size="sm" busy={busy} onClick={() => void run()}>
        Add a passkey
      </Button>
      <Button
        size="sm"
        variant="ghost"
        busy={leaving}
        onClick={() => {
          setLeaving(true);
          void signOut();
        }}
      >
        Sign out
      </Button>
    </div>
  );
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const first = (word: string) => Array.from(word)[0] ?? '';
  const letters =
    parts.length > 1 ? first(parts[0]) + first(parts[parts.length - 1]) : Array.from(parts[0] ?? '?').slice(0, 2).join('');
  return letters.toUpperCase();
}
