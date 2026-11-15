import { useEffect, useMemo, useState } from 'react';
import type { JournalPage } from '@kith/types';
import {
  Timeline, TimelineItem, TimelineMarker, TimelineTime, TimelineTitle,
} from '@ui-sdk/react/timeline';
import { Toggle, ToggleGroup } from '@ui-sdk/react/toggle';
import {
  Combobox, ComboboxClear, ComboboxContent, ComboboxEmpty, ComboboxInput, ComboboxItem, ComboboxList, ComboboxTrigger,
} from '@ui-sdk/react/combobox';
import { ChevronDown, History, X } from 'lucide-react';
import { api } from '../lib/api';
import { useLive } from '../lib/data';
import { useRoute } from '../lib/router';
import { Button, Empty, Page, Panel, Section, Skeleton, fmt } from '../ui';
import { CATEGORIES, devicesOf, look, sentence, type Category } from '../components/activity/kinds';
import { Density } from '../components/activity/Density';
import { Sessions, sessionsOf } from '../components/activity/Sessions';

type Row = JournalPage['entries'][number];

function dayLabel(t: number, now: number) {
  const d = new Date(t).toDateString();
  if (d === new Date(now).toDateString()) return 'Today';
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (d === yesterday.toDateString()) return 'Yesterday';
  return new Date(t).toLocaleDateString([], { weekday: 'long', month: 'short', day: 'numeric' });
}

export function ActivityScreen() {
  const { boot } = useLive();
  const { go } = useRoute();
  const now = boot?.server_time_ms ?? Date.now();
  const [rows, setRows] = useState<Row[] | null>(null);
  const [before, setBefore] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [olderError, setOlderError] = useState<string | null>(null);
  const [cats, setCats] = useState<Category[]>(CATEGORIES.filter((c) => c.id !== 'logs' && c.id !== 'dropped').map((c) => c.id));
  const [device, setDevice] = useState<string | null>(null);

  const load = (from: number | null) => {
    if (from !== null) {
      setBusy(true);
      setOlderError(null);
    }
    api
      .get<JournalPage>(`/api/history/journal${from === null ? '' : `?before=${from}`}`)
      .then((page) => {
        setRows((old) => {
          const seen = new Set<number>();
          return [...page.entries, ...(old ?? [])]
            .filter((r) => !seen.has(r.id) && !!seen.add(r.id))
            .sort((a, b) => b.entry.at_ms - a.entry.at_ms || b.id - a.id);
        });
        setBefore((b) => (from !== null || b === null ? page.next_before : b));
        if (from === null) setError(null);
      })
      .catch((e: unknown) => {
        console.warn('activity history failed', e);
        if (from === null) setError("Couldn't refresh the history.");
        else setOlderError("Couldn't load older events.");
      })
      .finally(() => {
        if (from !== null) setBusy(false);
      });
  };

  useEffect(() => {
    load(null);
    const t = setInterval(() => {
      if (!document.hidden) load(null);
    }, 10_000);
    return () => clearInterval(t);
  }, []);

  const entries = useMemo(() => (rows ?? []).map((r) => r.entry), [rows]);
  const deviceNames = useMemo(() => [...new Set(entries.flatMap(devicesOf))].sort((a, b) => a.localeCompare(b)), [entries]);
  const counts = useMemo(() => {
    const m = new Map<Category, number>();
    for (const e of entries) m.set(look(e).category, (m.get(look(e).category) ?? 0) + 1);
    return m;
  }, [entries]);
  const shown = useMemo(
    () => {
      const out: Row[] = [];
      for (const r of rows ?? []) {
        if (!cats.includes(look(r.entry).category) || (device && !devicesOf(r.entry).includes(device))) continue;
        const prev = out[out.length - 1]?.entry;
        // A run of drops for one reason in the same hour reads as one line.
        if (r.entry.kind === 'dropped' && prev?.kind === 'dropped' && prev.reason === r.entry.reason && prev.at_ms - r.entry.at_ms < 3_600_000) {
          out[out.length - 1] = { ...out[out.length - 1], entry: { ...prev, count: prev.count + r.entry.count } };
          continue;
        }
        out.push(r);
      }
      return out;
    },
    [rows, cats, device],
  );
  const shownEntries = useMemo(() => shown.map((r) => r.entry), [shown]);
  const lastDay = now - 86_400_000;
  const inLastDay = shownEntries.filter((e) => e.at_ms >= lastDay && e.at_ms <= now + 60_000).length;
  const peers = boot ? Object.values(boot.peers).flat() : null;
  const sessions = sessionsOf(
    entries.filter((e) => !device || devicesOf(e).includes(device)),
    now,
    peers,
  );
  const days: { label: string; items: Row[] }[] = [];
  for (const r of shown) {
    const label = dayLabel(r.entry.at_ms, now);
    if (days[days.length - 1]?.label !== label) days.push({ label, items: [] });
    days[days.length - 1].items.push(r);
  }
  const ready = rows !== null;

  const latest = entries.find((e) => e.kind !== 'log' && e.kind !== 'dropped');
  const description = ready ? (
    latest ? (
      <span title={fmt.absolute(latest.at_ms)} className="[text-wrap:balance]">Latest: {sentence(latest)}, {fmt.relative(latest.at_ms, now)}</span>
    ) : (
      'Nothing has happened yet.'
    )
  ) : (
    <Skeleton height={20} width={280} />
  );

  if (rows === null && error) {
    return (
      <Page title="Activity">
        <Empty icon={<History />} action={<Button onClick={() => load(null)}>Try again</Button>}>
          Couldn't load the activity history.
        </Empty>
      </Page>
    );
  }

  return (
    <Page title="Activity" description={description}>
      <div className="flex flex-col gap-3 xl:flex-row xl:items-center">
        <div className="-mx-4 min-w-0 overflow-x-auto px-4 md:-mx-8 md:px-8 xl:mx-0 xl:px-0 max-xl:[mask-image:linear-gradient(to_right,#000_calc(100%-32px),transparent)]">
          <ToggleGroup
            multiple
            value={cats}
            onValueChange={(v) => setCats(v as Category[])}
            aria-label="Kinds of events"
            className="w-max flex-nowrap [&>*]:shrink-0 [&>*]:whitespace-nowrap"
          >
            {CATEGORIES.map((c) => (
              <Toggle key={c.id} value={c.id}>
                {c.label}
                {ready && <span className="ml-2 font-mono tabular-nums text-fg-3">{fmt.count(counts.get(c.id) ?? 0)}</span>}
              </Toggle>
            ))}
          </ToggleGroup>
        </div>
        <Combobox value={device} onValueChange={(v) => setDevice((v as string | null) ?? null)}>
          <div className="flex h-8 items-center xl:ml-auto">
            <ComboboxInput placeholder="All devices" aria-label="Device" className="h-8 min-w-0 flex-1 sm:w-56 sm:flex-none" />
            {device && (
              <ComboboxClear aria-label="Show all devices" className="grid h-8 w-8 shrink-0 place-items-center">
                <X size={14} />
              </ComboboxClear>
            )}
            <ComboboxTrigger aria-label="Show devices" className="grid h-8 w-8 shrink-0 place-items-center">
              <ChevronDown size={14} />
            </ComboboxTrigger>
          </div>
          <ComboboxContent>
            <ComboboxEmpty>No device by that name.</ComboboxEmpty>
            <ComboboxList>
              {deviceNames.map((n) => (
                <ComboboxItem key={n} value={n} label={n}>{n}</ComboboxItem>
              ))}
            </ComboboxList>
          </ComboboxContent>
        </Combobox>
      </div>

      {(error ?? olderError) && <p role="alert" className="text-[13px] text-hot">{error ?? olderError}</p>}

      <Panel label="Last 24 hours" right={rows ? `${fmt.count(inLastDay)} ${inLastDay === 1 ? 'event' : 'events'}` : undefined}>
        {rows ? <Density entries={shownEntries} now={now} /> : <Skeleton height={52} />}
      </Panel>

      {(rows === null || sessions.devices.length > 0) && (
        <Section title="Who was online, when" description="Expand a device to see each session.">
          {rows ? <Sessions {...sessions} now={now} /> : <Skeleton height={160} />}
        </Section>
      )}

      <Section title="Timeline">
        {rows === null ? (
          <div className="flex flex-col gap-2">
            {Array.from({ length: 6 }, (_, i) => <Skeleton key={i} height={48} />)}
          </div>
        ) : days.length === 0 ? (
          <Empty icon={<History />}>
            {rows.length === 0
              ? 'Nothing has been recorded yet.'
              : device
                ? `Nothing for ${device} with these filters. Turn on more kinds of events or choose all devices.`
                : 'Nothing matches these filters. Turn on more kinds of events above to see them.'}
          </Empty>
        ) : (
          <div className="flex flex-col gap-8">
            {days.map((day) => (
              <div key={day.label}>
                <div className="sticky top-[57px] z-10 bg-ink-0 py-2 font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">
                  {day.label} · {day.items.length}
                </div>
                <Timeline aria-label={`Activity, ${day.label}`} className="divide-y divide-line border border-line">
                  {day.items.map(({ id, entry }) => {
                    const l = look(entry);
                    const Icon = l.icon;
                    const who = devicesOf(entry)[0];
                    const known = who && peers?.some((p) => p.name === who);
                    const text = sentence(entry);
                    const at = !who
                      ? -1
                      : entry.kind === 'device_renamed'
                        ? text.lastIndexOf(who)
                        : entry.kind === 'exit_changed'
                          ? text.length - ' as its exit'.length - who.length
                          : text.indexOf(who);
                    return (
                      <TimelineItem key={id} className="grid grid-cols-[48px_20px_1fr] items-start gap-x-3 px-3 py-2.5 md:grid-cols-[56px_20px_1fr] md:px-4">
                        <TimelineTime
                          dateTime={new Date(entry.at_ms).toISOString()}
                          title={fmt.absolute(entry.at_ms)}
                          className="text-right font-mono text-[11px] leading-5 tabular-nums text-fg-3"
                        >
                          {fmt.clock(entry.at_ms)}
                        </TimelineTime>
                        <TimelineMarker className={`grid h-5 w-5 place-items-center ${l.problem ? 'text-hot' : 'text-fg-3'}`}>
                          <Icon size={14} />
                        </TimelineMarker>
                        <TimelineTitle className="min-w-0 text-[14px] [overflow-wrap:anywhere] leading-5 text-fg">
                          {known && at >= 0 ? (
                            <>
                              {text.slice(0, at)}
                              <button
                                type="button"
                                className="cursor-pointer text-fg hover:underline focus-visible:underline focus-visible:outline focus-visible:outline-1 focus-visible:outline-offset-2 focus-visible:outline-fg-3"
                                onClick={() => go(`/devices/${encodeURIComponent(who)}`)}
                              >
                                {who}
                              </button>
                              {text.slice(at + who.length)}
                            </>
                          ) : (
                            text
                          )}
                          {entry.kind === 'log' && <span className="ml-2 font-mono text-[11px] text-fg-3">{entry.level} · {entry.target}</span>}
                        </TimelineTitle>
                      </TimelineItem>
                    );
                  })}
                </Timeline>
              </div>
            ))}
          </div>
        )}
        {before !== null && rows !== null && (
          <div className="flex justify-center pt-6">
            <Button variant="secondary" busy={busy} onClick={() => load(before)}>Load older events</Button>
          </div>
        )}
      </Section>
    </Page>
  );
}
