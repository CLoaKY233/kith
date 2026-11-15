import { useEffect, useMemo, useRef, useState } from 'react';
import type { BlockedGroup, PeerInfo } from '@kith/types';
import { Tabs, TabsList, TabsPanel, TabsTab } from '@ui-sdk/react/tabs';
import { Textarea } from '@ui-sdk/react/textarea';
import { api, errorMessage, net } from '../lib/api';
import { useFetch, useLive, useNetwork } from '../lib/data';
import { Button, Empty, Page, Section, Skeleton, fmt, toast } from '../ui';
import { axes, covers, portText, selKey, selName, type PolicyView, type Rule, type Selector } from './policy';

function list(items: string[]) {
  if (items.length <= 1) return items.join('');
  return `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`;
}

function sentence(r: Rule, peers: PeerInfo[]) {
  const from = list(r.sources.map((s) => selName(s, peers)));
  const to = list(r.destinations.map((d) => `${selName(d.selector, peers)} on ${portText(d)}`));
  return `${from[0]?.toUpperCase() ?? ''}${from.slice(1)} can reach ${to}.`;
}

const SHORT = 24;
const short = (n: string) => (n.length > SHORT ? `${n.slice(0, SHORT - 1)}…` : n);

function Matrix({ rules, peers }: { rules: Rule[]; peers: PeerInfo[] }) {
  const axis = useMemo(() => axes(rules), [rules]);
  const [hover, setHover] = useState<[string, string] | null>(null);
  const cell = (row: Selector, col: Selector) => {
    const hits: string[] = [];
    for (const r of rules)
      if (r.sources.some((s) => covers(s, selKey(row))))
        for (const d of r.destinations) if (covers(d.selector, selKey(col))) hits.push(portText(d));
    return hits;
  };
  if (axis.length === 0) return <Empty>No rules, so nothing can reach anything.</Empty>;
  const names = axis.map((x) => selName(x, peers));
  const head = Math.min(Math.max(...names.map((n) => Math.min(n.length, SHORT))) * 7 + 16, 200);
  const readout = hover && (() => {
    const row = axis.find((x) => selKey(x) === hover[0]);
    const col = axis.find((x) => selKey(x) === hover[1]);
    if (!row || !col) return '';
    const hits = cell(row, col);
    const a = selName(row, peers);
    const b = selName(col, peers);
    return hits.length ? `${a} can reach ${b} on ${hits.join(', ')}` : `${a} can't reach ${b}`;
  })();
  return (
    <div className="p-4">
      <p className="mb-3 text-[13px] text-fg-2">Rows can reach columns. Filled cells are allowed.</p>
      <div className="max-w-full overflow-x-auto">
      <table className="border-collapse font-mono text-[12px]" aria-label="Who can reach what">
        <thead>
          <tr>
            <td className="sticky left-0 z-10 bg-ink-0" />
            {axis.map((c) => (
              <th key={selKey(c)} scope="col" title={selName(c, peers)} style={{ height: head }} className={`px-0 align-bottom font-normal ${hover?.[1] === selKey(c) ? 'text-fg' : 'text-fg-3'}`}>
                <div className="w-7 rotate-180 whitespace-nowrap [writing-mode:vertical-rl]">{short(selName(c, peers))}</div>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {axis.map((row) => (
            <tr key={selKey(row)}>
              <th scope="row" title={selName(row, peers)} className={`sticky left-0 z-10 bg-ink-0 whitespace-nowrap pr-3 text-right font-normal ${hover?.[0] === selKey(row) ? 'text-fg' : 'text-fg-3'}`}>{short(selName(row, peers))}</th>
              {axis.map((col) => {
                const hits = cell(row, col);
                return (
                  <td
                    key={selKey(col)}
                    onMouseEnter={() => setHover([selKey(row), selKey(col)])}
                    onFocus={() => setHover([selKey(row), selKey(col)])}
                    onBlur={() => setHover(null)}
                    tabIndex={0}
                    aria-label={hits.length ? `${selName(row, peers)} can reach ${selName(col, peers)} on ${hits.join(', ')}` : `${selName(row, peers)} can't reach ${selName(col, peers)}`}
                    onMouseLeave={() => setHover(null)}
                    className={`size-7 border border-line focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg-3 ${hits.length ? 'bg-fg' : ''}`}
                  />
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
      </div>
      <p className="mt-3 min-h-5 text-[13px] text-fg-2 [overflow-wrap:anywhere]" aria-live="polite">{readout}</p>
    </div>
  );
}

/** The TOML being edited. It lives above the tabs so switching tabs keeps it, and follows the server while untouched. */
function useDraft(toml: string) {
  const state = useState(toml);
  const setText = state[1];
  const base = useRef(toml);
  useEffect(() => {
    if (toml === base.current) return;
    setText((d) => (d === base.current ? toml : d));
    base.current = toml;
  }, [toml, setText]);
  return { state, saved: (t: string) => { base.current = t; } };
}

type Draft = ReturnType<typeof useDraft>;

function Raw({ id, data, reload, draft }: { id: string | null; data: PolicyView; reload: () => void; draft: Draft }) {
  const [text, setText] = draft.state;
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const dirty = text !== data.toml;

  const save = async () => {
    if (!id || !dirty || saving) return;
    setSaving(true);
    setErr(null);
    try {
      await api.post(net(id, '/policy'), { toml: text });
      draft.saved(text);
      toast.success('Policy saved');
      reload();
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-3 p-4">
      <Textarea
        aria-label="Policy TOML"
        spellCheck={false}
        value={text}
        onValueChange={setText}
        onKeyDown={(e) => {
          if ((e.metaKey || e.ctrlKey) && e.key === 's') {
            e.preventDefault();
            void save();
          }
        }}
        className="min-h-[360px] w-full resize-y font-mono text-[13px] leading-relaxed"
      />
      {err && <p role="alert" className="whitespace-pre-wrap [overflow-wrap:anywhere] border border-hot px-3 py-2 font-mono text-[13px] text-hot">{err}</p>}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span className="text-[13px] text-fg-3">Your device signs the policy when you save.</span>
        <div className="ml-auto flex gap-2">
          <Button disabled={!dirty || saving} onClick={() => { setText(data.toml); setErr(null); }}>Revert</Button>
          <Button variant="primary" busy={saving} disabled={!dirty} onClick={() => void save()}>Save</Button>
        </div>
      </div>
    </div>
  );
}

function Blocked({ id, onAllowed }: { id: string | null; onAllowed: () => void }) {
  const { boot, refresh, error } = useLive();
  const [busy, setBusy] = useState<string | null>(null);
  const blocked: BlockedGroup[] = (boot?.pending.blocked ?? []).filter((b) => !id || b.network === id);

  const allow = async (b: BlockedGroup) => {
    setBusy(b.id);
    try {
      const preview = await api.post<{ rule_sentence: string }>(`/api/blocked/${encodeURIComponent(b.id)}/allow`, { preview: true });
      await api.post(`/api/blocked/${encodeURIComponent(b.id)}/allow`, { preview: false });
      toast.success(`Allowed: ${preview.rule_sentence}`);
      onAllowed();
      refresh();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  if (!boot && error) return <div className="border border-line"><Empty>Couldn't load blocked connections. Check your connection and try again.</Empty></div>;
  if (!boot) return <Skeleton height={64} />;
  if (blocked.length === 0) return <div className="border border-line"><Empty>Nothing has been blocked lately.</Empty></div>;
  return (
    <ul className="border border-line">
      {blocked.map((b) => {
        const proto = b.protocol.toUpperCase();
        const what =
          b.protocol === 'icmp' ? 'ICMP ping' : b.protocol === 'any' ? 'any protocol' : b.port != null ? `${proto} port ${b.port}` : proto;
        return (
          <li key={b.id} className="flex min-h-14 items-center justify-between gap-4 border-b border-line px-4 py-3 last:border-0">
            <div className="min-w-0 space-y-0.5">
              <p className="text-[14px] leading-snug text-fg-2 [overflow-wrap:anywhere]">
                <span className="text-fg">{b.from_name}</span> tried to reach <span className="text-fg">{b.to_name}</span> on {what}
                {b.count > 1 ? `, ${b.count} times` : ''}.
              </p>
              <p className="text-[13px] text-fg-3" title={fmt.absolute(b.last_ms)}>{fmt.relative(b.last_ms)}</p>
            </div>
            <Button size="sm" variant="primary" className="shrink-0" busy={busy === b.id} onClick={() => void allow(b)}>Allow</Button>
          </li>
        );
      })}
    </ul>
  );
}

function PolicyTabs({ id, data, peers, reload }: { id: string | null; data: PolicyView; peers: PeerInfo[]; reload: () => void }) {
  const draft = useDraft(data.toml);
  return (
    <Tabs defaultValue="rules" className="min-w-0">
      <TabsList aria-label="Policy view" className="mb-4">
        <TabsTab value="rules">Rules</TabsTab>
        <TabsTab value="matrix">Matrix</TabsTab>
        <TabsTab value="raw">Raw TOML</TabsTab>
      </TabsList>
      <TabsPanel value="rules">
        <div className="border border-line">
          {data.rules.length === 0 ? (
            <Empty>No rules, so nothing can reach anything. Add one in Raw TOML.</Empty>
          ) : (
            <ul>
              {data.rules.map((r, i) => (
                <li key={i} className="flex min-h-14 items-center border-b border-line px-4 py-3 text-[14px] leading-snug text-fg [overflow-wrap:anywhere] last:border-0">{sentence(r, peers)}</li>
              ))}
            </ul>
          )}
        </div>
      </TabsPanel>
      <TabsPanel value="matrix">
        <div className="border border-line">
          <Matrix rules={data.rules} peers={peers} />
        </div>
      </TabsPanel>
      <TabsPanel value="raw">
        <div className="border border-line">
          <Raw id={id} data={data} reload={reload} draft={draft} />
        </div>
      </TabsPanel>
    </Tabs>
  );
}

export function Access() {
  const { boot, network, peers } = useNetwork();
  const { error: liveError } = useLive();
  const id = network?.id ?? null;
  const { data, error, reload } = useFetch<PolicyView>(id ? net(id, '/policy') : null);

  return (
    <Page title="Access" description="Nothing gets through unless a rule allows it.">
      <Section title="Who can reach what">
        {!data && (error || (!boot && liveError)) && (
          <div role="alert" className="border border-line">
            <Empty action={error ? <Button size="sm" onClick={reload}>Try again</Button> : undefined}>
              <span className="[overflow-wrap:anywhere]">Couldn't load the policy.{error ? ` ${error}` : ' Check your connection and try again.'}</span>
            </Empty>
          </div>
        )}
        {!data && !error && !(!boot && liveError) && !(boot && !network) && <Skeleton height={200} />}
        {boot && !network && <div className="border border-line"><Empty>No network yet.</Empty></div>}
        {data && <PolicyTabs id={id} data={data} peers={peers} reload={reload} />}
      </Section>
      <Section title="Blocked connections" description="Recent attempts the rules stopped. Allow one to add a rule for it.">
        <Blocked id={id} onAllowed={reload} />
      </Section>
    </Page>
  );
}
