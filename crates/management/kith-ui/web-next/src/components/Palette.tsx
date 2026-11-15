import type { Invite } from '@kith/types';
import { Command, CommandEmpty, CommandGroup, CommandGroupLabel, CommandInput, CommandItem, CommandItemIcon, CommandList, CommandSeparator } from '@ui-sdk/react/command';
import { Dialog, DialogBackdrop, DialogContent, DialogTitle } from '@ui-sdk/react/dialog';
import { Link2, Map as MapIcon, Monitor, Pause, Play, Search, SlidersHorizontal, type LucideIcon } from 'lucide-react';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { NAV, SETTINGS } from '../nav';
import { Button, StatusDot, toast } from '../ui';
import { api, net } from '../lib/api';
import { useLive, useNetwork } from '../lib/data';
import { useRoute } from '../lib/router';

const item = 'flex items-center gap-3';

const failure = (e: unknown, what: string) => {
  console.error(e);
  const m = e instanceof Error ? e.message.trim() : '';
  return m && m.length < 120 && !/failed to fetch|networkerror|load failed|json/i.test(m) ? m : `${what}. Check your connection and try again.`;
};

const Kbd = ({ children }: { children: string }) => (
  <kbd className="border border-line-2 px-1.5 py-0.5 font-mono text-[10px] text-fg-3">{children}</kbd>
);

type Kind = 'device' | 'page' | 'setting' | 'action';
type Entry = { value: string; kind: Kind; label: string; keywords: string[]; render: ReactNode };

const KIND_RANK: Record<Kind, number> = { device: 0, page: 1, setting: 2, action: 3 };

/** 0 exact, 1 prefix, 2 word start, 3 substring, null no match. */
function score(text: string, q: string): number | null {
  const t = text.toLowerCase();
  if (t === q) return 0;
  if (t.startsWith(q)) return 1;
  const i = t.indexOf(q);
  if (i < 0) return null;
  for (let j = i; j >= 0; j = t.indexOf(q, j + 1)) if (/[^a-z0-9]/.test(t[j - 1])) return 2;
  return 3;
}

function best(e: Entry, q: string): number | null {
  let s: number | null = null;
  for (const t of [e.label, ...e.keywords]) {
    const r = score(t, q);
    if (r !== null && (s === null || r < s)) s = r;
  }
  return s;
}

function Row({ icon: Icon, label, sub, mono, right }: { icon: LucideIcon; label: string; sub?: string; mono?: boolean; right?: ReactNode }) {
  return (
    <>
      <CommandItemIcon><Icon size={16} /></CommandItemIcon>
      <span className="min-w-0 shrink-0 max-w-full truncate">{label}</span>
      {sub && <span className={`min-w-0 truncate text-fg-3 ${mono ? 'font-mono text-[11px]' : 'text-xs'}`}>{sub}</span>}
      {right && <span className="ml-auto flex items-center gap-1.5 font-mono text-[11px] text-fg-3">{right}</span>}
    </>
  );
}

export function Palette({ className }: { className?: string }) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const { go } = useRoute();
  const { refresh } = useLive();
  const { boot, network, peers } = useNetwork();
  const mac = typeof navigator !== 'undefined' && /Mac|iP/.test(navigator.platform);
  const paused = network?.my_settings?.paused ?? false;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.repeat && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    addEventListener('keydown', onKey);
    return () => removeEventListener('keydown', onKey);
  }, []);

  useEffect(() => {
    if (!open) setQuery('');
  }, [open]);

  const hasNetwork = Boolean(network);
  const canPause = hasNetwork && Boolean(boot?.status.node);

  const { places, devices, recent, actions } = useMemo(() => {
    const places: Entry[] = [
      ...NAV.flatMap(({ path, label, icon, question }): Entry[] => [
        {
          value: `go:${path}`, kind: 'page', label, keywords: [],
          render: <Row icon={icon} label={label} sub={question} />,
        },
        ...(path === '/devices' ? [{
          value: 'go:/devices?view=map', kind: 'page' as const, label: 'Map', keywords: ['devices map'],
          render: <Row icon={MapIcon} label="Map" sub="Where each device is and how it is reached" />,
        }] : []),
      ]),
    ];
    const settings: Entry[] = SETTINGS.map(({ slug, label, description }) => ({
      value: `go:/settings/${slug}`, kind: 'setting', label, keywords: [`settings ${label}`],
      render: <Row icon={SlidersHorizontal} label={`Settings · ${label}`} sub={description} />,
    }));
    const devEntry = (p: (typeof peers)[number]): Entry => ({
      value: `dev:${p.name}`, kind: 'device', label: p.display_name, keywords: [p.name, p.address],
      render: <Row icon={Monitor} label={p.display_name} sub={p.address} mono right={<StatusDot status={p.online ? 'online' : 'offline'} />} />,
    });
    const devices = peers.map(devEntry);
    const recent = [...peers]
      .sort((a, b) => Number(b.online) - Number(a.online) || (a.last_seen_secs ?? Infinity) - (b.last_seen_secs ?? Infinity))
      .slice(0, 5)
      .map(devEntry);
    const pauseLabel = paused ? 'Resume traffic' : 'Pause traffic';
    const actions: Entry[] = [
      ...(hasNetwork ? [{
        value: 'act:invite', kind: 'action' as const, label: 'Copy invite link', keywords: ['invite', 'share', 'add device'],
        render: <Row icon={Link2} label="Copy invite link" right="24h · 1 use" />,
      } satisfies Entry] : []),
      ...(canPause ? [{
        value: 'act:pause', kind: 'action' as const, label: pauseLabel, keywords: ['stop', 'vpn off'],
        render: <Row icon={paused ? Play : Pause} label={pauseLabel} right="this device" />,
      } satisfies Entry] : []),
    ];
    return { places: [...places, ...settings], devices, recent, actions };
  }, [peers, paused, hasNetwork, canPause]);

  const q = query.trim().toLowerCase();
  const matches = useMemo(() => {
    if (!q) return [];
    return [...devices, ...places, ...actions]
      .map((e) => ({ e, s: best(e, q) }))
      .filter((m): m is { e: Entry; s: number } => m.s !== null)
      .sort((a, b) => a.s - b.s || KIND_RANK[a.e.kind] - KIND_RANK[b.e.kind])
      .map((m) => m.e);
  }, [q, devices, places, actions]);

  const run = async (value: string) => {
    setOpen(false);
    if (value.startsWith('go:')) go(value.slice(3));
    else if (value.startsWith('dev:')) go(`/devices/${encodeURIComponent(value.slice(4))}`);
    else if (value === 'act:invite' && network) {
      try {
        const inv = await api.post<Invite>(net(network.id, '/invites'), { expires_secs: 86400, uses: 1 });
        const copied = await navigator.clipboard.writeText(inv.link).then(() => true, () => false);
        if (copied) toast('Invite link copied · valid 24h, one use');
        else toast.error('Invite created, but it could not be copied. Make a new one from Settings, Network.');
      } catch (e) {
        toast.error(failure(e, 'Could not create the invite'));
      }
    } else if (value === 'act:pause' && network && boot?.status.node) {
      const s = network.my_settings ?? { exit: null, exit_locked: false, paused: false };
      try {
        await api.post(net(network.id, `/devices/${encodeURIComponent(boot.status.node)}/settings`), { ...s, paused: !paused });
        toast(paused ? 'Traffic resumed' : 'Traffic paused');
        refresh();
      } catch (e) {
        toast.error(failure(e, paused ? 'Could not resume traffic' : 'Could not pause traffic'));
      }
    }
  };

  const items = (list: Entry[]) =>
    list.map((e) => (
      <CommandItem key={e.value} value={e.value} className={item}>{e.render}</CommandItem>
    ));

  const group = (label: string, list: Entry[]) => (
    <CommandGroup>
      <CommandGroupLabel>{label}</CommandGroupLabel>
      {items(list)}
    </CommandGroup>
  );

  return (
    <>
      <Button size="sm" variant="ghost" icon={<Search size={14} />} onClick={() => setOpen(true)} aria-label="Search" className={className}>
        <span className="hidden sm:inline">Search</span>
        <span className="hidden font-mono text-[11px] text-fg-3 sm:inline">{mac ? '⌘K' : 'Ctrl K'}</span>
      </Button>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogBackdrop />
        <DialogContent className="top-[8vh] sm:top-[15vh] bottom-auto mt-0 !w-[min(640px,calc(100vw-32px))] !max-w-none max-h-[80vh] overflow-hidden !p-0">
          <DialogTitle className="sr-only">Command palette</DialogTitle>
          <Command className="!w-full !max-w-none !border-0" onAction={run} query={query} onQueryChange={setQuery} filter={false} autoHighlight loop>
            <div className="flex items-center gap-3 border-b border-line px-[13px]">
              <Search size={16} className="shrink-0 text-fg-3" />
              <CommandInput aria-label="Search commands" placeholder="Search screens, devices and actions" className="h-12 min-w-0 flex-1 !border-0 !bg-transparent !px-0 !outline-none !ring-0" />
              <span className="hidden sm:inline"><Kbd>esc</Kbd></span>
            </div>
            {q && matches.length === 0 && <CommandEmpty>No matches</CommandEmpty>}
            <CommandList className="max-h-[50vh] overflow-y-auto">
              {q ? (
                items(matches)
              ) : (
                <>
                  {group('Go to', places.filter((e) => e.kind === 'page'))}
                  {recent.length > 0 && (
                    <>
                      <CommandSeparator />
                      {group('Recent devices', recent)}
                    </>
                  )}
                  {actions.length > 0 && (
                    <>
                      <CommandSeparator />
                      {group('Actions', actions)}
                    </>
                  )}
                </>
              )}
            </CommandList>
            <div className="hidden items-center gap-4 border-t border-line px-[13px] py-2.5 sm:flex font-mono text-[11px] text-fg-3">
              <span className="flex items-center gap-1.5"><Kbd>↑</Kbd><Kbd>↓</Kbd> move</span>
              <span className="flex items-center gap-1.5"><Kbd>↵</Kbd> select</span>
            </div>
          </Command>
        </DialogContent>
      </Dialog>
    </>
  );
}
