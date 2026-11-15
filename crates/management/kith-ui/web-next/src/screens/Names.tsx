import { useMemo, useState } from 'react';
import type { RecordType, RecordView } from '@kith/types';
import { Input } from '@ui-sdk/react/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@ui-sdk/react/select';
import { Plus, Trash2 } from 'lucide-react';
import { api, errorMessage, net } from '../lib/api';
import { useFetch, useLive, useNetwork } from '../lib/data';
import { useRoute } from '../lib/router';
import { Badge, Button, Empty, Page, Section, Skeleton, StatusDot, toast } from '../ui';

const TYPES: RecordType[] = ['A', 'AAAA', 'CNAME', 'TXT'];
const HINT: Record<RecordType, string> = { A: '100.64.0.10', AAAA: 'fd7a::10', CNAME: 'nas.office.kith.internal', TXT: 'Any text' };
const TYPE_ITEMS = TYPES.map((t) => ({ value: t, label: t }));
const label = 'block font-mono text-[11px] leading-4 uppercase tracking-[0.08em] text-fg-3';
const input = '!h-8 w-full !text-[13px] font-mono placeholder:text-fg-3';
const field = 'flex min-w-0 flex-col gap-2';

const IPV4 = /^(25[0-5]|2[0-4]\d|1?\d?\d)(\.(25[0-5]|2[0-4]\d|1?\d?\d)){3}$/;
const IPV6 = /^[0-9a-f:.]+$/i;

function problem(type: RecordType, value: string) {
  if (type === 'A' && !IPV4.test(value)) return 'An A record needs an IPv4 address, like 100.64.0.10.';
  if (type === 'AAAA' && !(value.includes(':') && IPV6.test(value))) return 'An AAAA record needs an IPv6 address, like fd7a::10.';
  return null;
}

export function Names() {
  const { boot, network, peers } = useNetwork();
  const { go } = useRoute();
  const { error: liveError } = useLive();
  const id = network?.id ?? null;
  const { data, error, reload } = useFetch<RecordView[]>(id ? net(id, '/records') : null, 10000);
  const [name, setName] = useState('');
  const [type, setType] = useState<RecordType>('A');
  const [value, setValue] = useState('');
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);

  const zone = network ? `${network.name}.kith.internal` : null;
  const failed = !boot && liveError ? liveError : error && !data ? error : null;
  const records = (data ?? []).filter((r) => !r.device);
  const auto = useMemo(() => [...peers].sort((a, b) => a.name.localeCompare(b.name)), [peers]);

  const add = async () => {
    if (!id || busy || !name.trim() || !value.trim()) return;
    const bad = problem(type, value.trim());
    if (bad) {
      toast.error(bad);
      return;
    }
    setBusy(true);
    try {
      await api.post(net(id, '/records'), { name: name.trim(), type, value: value.trim(), based_on: null });
      toast.success(`${name.trim()} saved`);
      setName('');
      setValue('');
      reload();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const remove = async (r: RecordView) => {
    if (!id || removing) return;
    setRemoving(r.name);
    try {
      await api.post(net(id, `/records/${encodeURIComponent(r.name)}/delete`));
      toast.success(`${r.name} removed`);
      reload();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setRemoving(null);
    }
  };

  return (
    <Page title="Names" description={zone ? `Devices answer at their own names under ${zone}. Add more here.` : 'Devices answer at their own names on your network. Add more here.'}>
      <Section title="Records" description="Signed and served to every device.">
        {failed && <p role="alert" className="text-[13px] leading-5 text-hot">Couldn't load the records. {failed}</p>}
        {!data && !failed && !(boot && !network) && <Skeleton height={120} />}
        {boot && !network && <div className="border border-line"><Empty>No network yet.</Empty></div>}
        {data && records.length === 0 && (
          <div className="border border-line"><Empty>No records yet. Add one below.</Empty></div>
        )}
        {records.length > 0 && (
          <ul className="border border-line">
            {records.map((r) => (
              <li key={`${r.name}/${r.type}/${r.value}`} className="grid min-h-12 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1.5 border-b border-line px-4 py-3 last:border-0 lg:grid-cols-[minmax(0,5fr)_64px_minmax(0,6fr)_auto] lg:gap-y-0">
                <span className="col-span-1 min-w-0 truncate font-mono text-[13px] leading-5 text-fg lg:col-auto" title={r.name}>{r.name}</span>
                <Button className="col-start-2 row-span-2 row-start-1 -mr-2 lg:col-start-4 lg:row-span-1 lg:row-start-1 lg:-my-1.5" size="sm" variant="ghost" icon={<Trash2 />} aria-label={`Remove ${r.name}`} title="Remove" busy={removing === r.name} onClick={() => void remove(r)} />
                <span className="col-start-1 flex min-w-0 items-center gap-3 lg:contents">
                  <span className="justify-self-start"><Badge>{r.type}</Badge></span>
                  <span className="min-w-0 flex-1 truncate font-mono text-[13px] leading-5 text-fg-2" title={r.value}>{r.value}</span>
                </span>
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section title="Add a record">
        <form
          className="grid grid-cols-1 items-end gap-4 border border-line p-4 sm:grid-cols-[minmax(0,1fr)_136px] md:p-6 lg:grid-cols-[minmax(0,1fr)_96px_minmax(0,1fr)_auto]"
          onSubmit={(e) => {
            e.preventDefault();
            void add();
          }}
        >
          <label className={field}>
            <span className={label}>Name</span>
            <Input className={input} value={name} onValueChange={setName} placeholder="printer" autoComplete="off" autoCapitalize="none" spellCheck={false} />
          </label>
          <div className={field}>
            <span className={label}>Type</span>
            <Select value={type} onValueChange={(v) => v && setType(v as RecordType)} items={TYPE_ITEMS}>
              <SelectTrigger aria-label="Record type" className="!h-8 w-full min-w-0">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {TYPE_ITEMS.map((t) => (
                  <SelectItem key={t.value} value={t.value}>{t.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <label className={`${field} lg:col-span-1`}>
            <span className={label}>Value</span>
            <Input className={input} value={value} onValueChange={setValue} placeholder={HINT[type]} autoComplete="off" autoCapitalize="none" spellCheck={false} />
          </label>
          <Button type="submit" variant="primary" className="!h-8 w-full lg:w-auto" icon={<Plus />} busy={busy} disabled={!id || !name.trim() || !value.trim()}>Add record</Button>
        </form>
      </Section>

      <Section title="Automatic device names" description="Kept up to date as devices join, leave or are renamed.">
        {!boot ? (
          failed ? null : <Skeleton height={96} />
        ) : auto.length === 0 ? (
          <div className="border border-line"><Empty>No devices yet.</Empty></div>
        ) : (
          <ul className="border border-line">
            {auto.map((p) => (
              <li key={p.node} className="border-b border-line last:border-0">
                <button
                  type="button"
                  title={`${p.name}.${zone} · ${p.address}`}
                  onClick={() => go(`/devices/${encodeURIComponent(p.name)}`)}
                  className="grid min-h-12 w-full grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-0.5 px-4 py-3 text-left sm:grid-cols-[auto_minmax(0,1fr)_auto] hover:bg-ink-2 focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg-3"
                >
                  <StatusDot status={p.online ? 'online' : 'offline'} />
                  <span className="min-w-0 font-mono text-[13px] leading-5 [overflow-wrap:anywhere]">
                    <span className="text-fg">{p.name}</span>
                    <span className="text-fg-3">.{zone}</span>
                  </span>
                  <span className="col-start-2 font-mono text-[13px] leading-5 tabular-nums text-fg-2 sm:col-start-3">{p.address}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </Section>
    </Page>
  );
}
