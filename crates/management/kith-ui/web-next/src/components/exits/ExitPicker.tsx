import { useState } from 'react';
import type { ExitIn, ExitView, Outcome } from '@kith/types';
import { Radio, RadioGroup, RadioIndicator } from '@ui-sdk/react/radio';
import { ApiError, api, errorMessage } from '../../lib/api';
import { useLive, useNetwork } from '../../lib/data';
import { Skeleton, fmt, toast } from '../../ui';

const OWN = 'own';

/** Where this device sends its internet traffic: its own connection or one exit it may use. */
export function ExitPicker({ exits }: { exits: ExitView[] | null }) {
  const { refresh } = useLive();
  const { boot, network, peers } = useNetwork();
  const status = boot?.status;
  const self = status?.node ?? null;
  const [pick, setPick] = useState<string | null>(null);
  const [waiting, setWaiting] = useState(false);

  // `exit_node` is `network/device`, by name.
  const inUse = status?.exit_node ?? null;
  const slash = inUse?.indexOf('/') ?? -1;
  const inUseName = inUse && slash > 0 ? inUse.slice(slash + 1) : null;
  const inUseHere = inUse !== null && network !== null && [network.name, network.id].includes(inUse.slice(0, slash));
  const usable = (exits ?? []).filter((e) => e.node !== self && e.allowed).sort((a, b) => a.name.localeCompare(b.name));
  const truth = inUse === null ? OWN : inUseHere ? (usable.find((e) => e.name === inUseName)?.node ?? peers.find((p) => p.name === inUseName)?.node ?? null) : null;
  if (pick !== null && !waiting && pick === truth) setPick(null);
  const selected = pick ?? truth ?? '';
  const nameOf = (key: string) => (key === OWN ? null : (usable.find((e) => e.node === key)?.name ?? 'that exit'));

  const choose = async (key: string) => {
    if (!network || pick !== null || key === selected) return;
    const target = nameOf(key);
    setPick(key);
    setWaiting(true);
    try {
      const out = await api.post<Outcome>('/api/exit', { network: network.id, node: key === OWN ? null : key } satisfies ExitIn);
      toast.success(
        out.state === 'awaiting_os_auth'
          ? 'Confirm the change on this computer.'
          : target === null
            ? "Your internet now goes through this device's own connection."
            : `Your internet now goes through ${target}.`,
      );
      refresh();
      // The daemon reports the change on a later poll; never hold the list disabled past that.
      setTimeout(() => setPick(null), 15_000);
    } catch (e) {
      setPick(null);
      const still = inUseName ?? "this device's own connection";
      const cause = e instanceof ApiError && e.code === 'not_found' && target ? `${target} isn't reachable right now.` : errorMessage(e);
      toast.error(`Couldn't switch${target ? ` to ${target}` : ''}. ${cause} You're still using ${still}.`);
    } finally {
      setWaiting(false);
    }
  };

  const detail = (e: ExitView) => {
    if (e.node === truth && pick === null && status?.exit_state === 'offline_blocked') return 'Offline, internet is blocked until it is back';
    if (e.node === truth && pick === null && status?.exit_state === 'offline_direct') return 'Offline, internet goes directly for now';
    const peer = peers.find((p) => p.node === e.node);
    if (!peer?.online) return 'Offline';
    const link = boot?.last_tick?.links.find((l) => l.network === network?.id && l.from === self && l.to === e.node);
    return link?.rtt_ms != null ? fmt.ms(link.rtt_ms) : null;
  };

  if (!exits) return <Skeleton height={96} />;
  const options = [
    { key: OWN, name: 'No exit', detail: "This device's own connection", quiet: false },
    ...usable.map((e) => ({ key: e.node, name: e.name, detail: detail(e), quiet: peers.find((p) => p.node === e.node)?.online !== true })),
  ];

  return (
    <div className="flex flex-col gap-3">
      <RadioGroup
        aria-label="Send internet traffic through"
        value={selected}
        onValueChange={(v) => v && void choose(v)}
        disabled={pick !== null}
        className="flex flex-col border border-line"
      >
        {options.map((o) => (
          <label key={o.key} className="flex min-h-12 cursor-pointer items-center gap-3 border-b border-line px-4 py-2 last:border-0 hover:bg-ink-2">
            <Radio value={o.key}>
              <RadioIndicator />
            </Radio>
            <span className={`min-w-0 max-w-[60%] shrink-0 truncate text-[14px] ${o.quiet ? 'text-fg-3' : 'text-fg'}`} title={o.name}>{o.name}</span>
            <span className="min-w-0 flex-1 truncate text-right font-mono text-[12px] tabular-nums text-fg-3" title={pick === o.key ? undefined : (o.detail ?? undefined)}>
              {pick === o.key ? 'Switching…' : o.detail}
            </span>
          </label>
        ))}
      </RadioGroup>
      {usable.length === 0 && exits.length > 0 && <p className="text-[13px] text-fg-3">Only this device offers itself as an exit. Turn on Exit node under Network on another device to send traffic through it.</p>}
      {truth === null && inUseName && !inUseHere && <p className="text-[13px] text-fg-3">This device sends its internet traffic through {inUseName}, an exit in another network.</p>}
    </div>
  );
}
