import { useState } from 'react';
import type { ExitView, GrantIn, PeerInfo, TrafficSeries } from '@kith/types';
import { AlertDialog, AlertDialogBackdrop, AlertDialogContent, AlertDialogDescription, AlertDialogTitle } from '@ui-sdk/react/alert-dialog';
import { Switch, SwitchThumb } from '@ui-sdk/react/switch';
import { api, errorMessage, net } from '../lib/api';
import { useFetch, useLive, useNetwork } from '../lib/data';
import { Button, Empty, Page, Section, Skeleton, StatusDot, fmt, toast } from '../ui';
import { ExitPicker } from '../components/exits/ExitPicker';
import { WireguardImports } from '../components/exits/WireguardImports';

/** The latest rate, only when the newest sample is fresh: the tail keeps old samples of idle exits. */
function bpsOf(series: TrafficSeries | undefined, now: number): number | null {
  const s = (series?.samples ?? []).filter((x) => x.at_ms >= now - 30_000);
  if (s.length < 2) return null;
  const a = s[0];
  const b = s[s.length - 1];
  const dt = (b.at_ms - a.at_ms) / 1000;
  const bytes = b.rx_bytes + b.tx_bytes - a.rx_bytes - a.tx_bytes;
  return dt > 0 && bytes >= 0 ? bytes / dt : null;
}

function Allowed({ exit, locked, onAsk }: { exit: ExitView; locked: boolean; onAsk: (allowed: boolean) => void }) {
  return (
    <Switch aria-label={`Allow ${exit.name} as an exit`} checked={exit.allowed} disabled={locked} onCheckedChange={onAsk}>
      <SwitchThumb />
    </Switch>
  );
}

const and = (names: readonly string[]) =>
  names.length < 2 ? (names[0] ?? '') : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;

const th = 'px-4 py-3 text-left font-mono text-[11px] font-normal uppercase tracking-[0.08em] text-fg-3';

export function Exits() {
  const { boot, network, peers } = useNetwork();
  const { error: liveError } = useLive();
  const unreachable = !boot && liveError !== null;
  const self = boot?.status.node ?? null;
  const selfName = boot?.status.memberships.find((m) => m.network === network?.id)?.name ?? null;
  const { data: listed, error, reload } = useFetch<ExitView[]>(network ? net(network.id, '/exits') : null, 5000);
  // The list holds only allowed exits: one switched off here stays as a row, off, so it can be allowed again.
  const [off, setOff] = useState<Record<string, ExitView>>({});
  const [asking, setAsking] = useState<{ exit: ExitView; allowed: boolean } | null>(null);
  const [saving, setSaving] = useState(false);
  const exits = listed
    ? [...listed, ...Object.values(off).filter((o) => o.network === network?.id && !listed.some((e) => e.node === o.node))].sort((a, b) => a.name.localeCompare(b.name))
    : null;
  const usersOf = (names: readonly string[]) => names.map((n) => (n === selfName ? 'This device' : n));
  const tail = boot?.traffic_tail.filter((t) => t.network === network?.id) ?? [];
  const peer = (node: string): PeerInfo | undefined => peers.find((p) => p.node === node);

  const setAllowed = async () => {
    if (!network || !asking) return;
    const { exit: e, allowed } = asking;
    setSaving(true);
    try {
      await api.post(net(network.id, `/exits/${encodeURIComponent(e.node)}`), { allowed } satisfies GrantIn);
      setOff((o) => {
        const { [e.node]: _, ...rest } = o;
        return allowed ? rest : { ...rest, [e.node]: { ...e, allowed: false, in_use_by: [] } };
      });
      toast.success(allowed ? `${e.name} can now be used as an exit.` : `${e.name} can no longer be used as an exit.`);
      setAsking(null);
      reload();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSaving(false);
    }
  };
  const ask = (exit: ExitView) => (allowed: boolean) => setAsking({ exit, allowed });
  const publicConsole = boot?.status.mode === 'public';

  const derive = (e: ExitView) => {
    const p = peer(e.node);
    const isSelf = e.node === self;
    const online = isSelf || (p?.online ?? false);
    const using = e.in_use_by.length > 0;
    const bps = using ? bpsOf(tail.find((t) => t.node === e.node), boot?.server_time_ms ?? 0) : null;
    return { p, isSelf, online, using, bps };
  };

  return (
    <Page title="Exits" description="Devices that let others reach the internet through them.">
      {unreachable && <p className="text-[13px] text-hot [overflow-wrap:anywhere]">Couldn't reach kith on this device, so its exits can't be shown. {liveError}</p>}
      {!publicConsole && !unreachable && (
        <Section title="Send internet traffic through" description="Where this device's internet traffic goes. Each other device picks its own exit on its page.">
          <ExitPicker exits={listed} />
        </Section>
      )}
      {!unreachable && (
      <Section title="All exits">
        {error && !exits && <p className="text-[13px] text-hot [overflow-wrap:anywhere]">Couldn't load exits. {error}</p>}
        {!exits && !error && <Skeleton height={160} />}
        {exits && exits.length === 0 && (
          <div className="border border-line">
            <Empty>No device offers itself as an exit yet. Turn on Exit node under Network on that device.</Empty>
          </div>
        )}
        {exits && exits.length > 0 && (
          <>
            <ul className="border border-line lg:hidden" aria-label="Devices that offer themselves as an exit">
              {exits.map((e) => {
                const { p, isSelf, online, using, bps } = derive(e);
                return (
                  <li key={e.node} className="flex flex-col gap-3 border-b border-line px-4 py-3 last:border-0">
                    <div className="flex items-center justify-between gap-4">
                      <div className="min-w-0">
                        <div className="truncate text-[14px] text-fg" title={e.name}>{e.name}</div>
                        <div className="mt-0.5 truncate font-mono text-[12px] text-fg-3">{isSelf || e.name === selfName ? 'This device' : (p?.address ?? '')}</div>
                      </div>
                      <Allowed exit={e} locked={network?.admin === false} onAsk={ask(e)} />
                    </div>
                    <dl className="grid grid-cols-[6.5rem_1fr] items-baseline gap-x-4 gap-y-2 text-[13px]">
                      <dt className="font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">Status</dt>
                      <dd className="flex items-center gap-2 text-fg-2">
                        <StatusDot status={online ? 'online' : 'offline'} />
                        {online ? 'Online' : 'Offline'}
                      </dd>
                      <dt className="font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">Using it now</dt>
                      <dd className="text-fg-2 [overflow-wrap:anywhere]">{!e.allowed ? <span className="text-fg-3">Not allowed</span> : using ? and(usersOf(e.in_use_by)) : <span className="text-fg-3">Nobody</span>}</dd>
                      <dt className="font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">Rate</dt>
                      <dd className="font-mono tabular-nums text-fg-2">{bps === null ? <span className="text-fg-3">–</span> : fmt.rate(bps)}</dd>
                    </dl>
                  </li>
                );
              })}
            </ul>
            <div className="hidden border border-line lg:block">
              <table className="w-full table-fixed text-[14px]">
                <caption className="sr-only">Devices that offer themselves as an exit</caption>
                <thead className="border-b border-line">
                  <tr>
                    <th className={`${th} w-[32%]`}>Name</th>
                    <th className={`${th} w-[16%]`}>Status</th>
                    <th className={th}>Using it now</th>
                    <th className={`${th} w-32 !text-right`}>Rate</th>
                    <th className={`${th} w-28 !text-right`}>Allowed</th>
                  </tr>
                </thead>
                <tbody>
                  {exits.map((e) => {
                    const { p, isSelf, online, using, bps } = derive(e);
                    return (
                      <tr key={e.node} className="border-b border-line last:border-0">
                        <td className="px-4 py-3 align-middle">
                          <div className="truncate text-fg" title={e.name}>{e.name}</div>
                          <div className="mt-0.5 truncate font-mono text-[12px] text-fg-3">{isSelf || e.name === selfName ? 'This device' : (p?.address ?? '')}</div>
                        </td>
                        <td className="px-4 py-3 align-middle">
                          <span className="inline-flex items-center gap-2 text-fg-2">
                            <StatusDot status={online ? 'online' : 'offline'} />
                            {online ? 'Online' : 'Offline'}
                          </span>
                        </td>
                        <td className="px-4 py-3 align-middle text-fg-2 [overflow-wrap:anywhere]" title={!e.allowed ? undefined : e.access.length ? `May use it: ${and(usersOf(e.access))}` : 'Every device may use it'}>
                          {!e.allowed ? <span className="text-fg-3">Not allowed</span> : using ? and(usersOf(e.in_use_by)) : <span className="text-fg-3">Nobody</span>}
                        </td>
                        <td className="whitespace-nowrap px-4 py-3 text-right align-middle font-mono tabular-nums text-fg-2">{bps === null ? <span className="text-fg-3">–</span> : fmt.rate(bps)}</td>
                        <td className="px-4 py-3 align-middle">
                          <div className="flex justify-end"><Allowed exit={e} locked={network?.admin === false} onAsk={ask(e)} /></div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </>
        )}
      </Section>
      )}
      {!publicConsole && !unreachable && (
        <Section title="Use a WireGuard server" description="A config from a VPN provider or your own WireGuard server: one that carries all internet traffic works as this device's exit.">
          <WireguardImports />
        </Section>
      )}
      <AlertDialog open={asking !== null} onOpenChange={(o) => o || saving || setAsking(null)}>
        <AlertDialogBackdrop />
        {asking && network && (
          <AlertDialogContent className="flex flex-col gap-4">
            <div className="space-y-1.5">
              <AlertDialogTitle className="m-0 text-left">
                {asking.allowed ? `Allow ${asking.exit.name} as an exit in ${network.name}?` : `Stop offering ${asking.exit.name} as an exit?`}
              </AlertDialogTitle>
              <AlertDialogDescription className="m-0 text-left">
                {asking.allowed
                  ? `Devices in ${network.name} can then send their internet traffic through ${asking.exit.name}. It can see which addresses that traffic goes to.`
                  : asking.exit.in_use_by.length > 0
                    ? `${and(usersOf(asking.exit.in_use_by))} ${asking.exit.in_use_by.length === 1 ? 'uses it as its exit. Its' : 'use it as their exit. Their'} internet will go directly. Devices can use it again once an admin allows it.`
                    : `No device uses ${asking.exit.name} right now. Devices can use it again once an admin allows it.`}
              </AlertDialogDescription>
            </div>
            <div className="flex justify-end gap-2 pt-2">
              <Button variant="ghost" onClick={() => setAsking(null)}>Cancel</Button>
              <Button variant="primary" busy={saving} onClick={() => void setAllowed()}>
                {asking.allowed ? `Allow ${asking.exit.name}` : `Stop offering ${asking.exit.name}`}
              </Button>
            </div>
          </AlertDialogContent>
        )}
      </AlertDialog>
    </Page>
  );
}
