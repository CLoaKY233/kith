import { useRef, useState } from 'react';
import type { ImportUseIn, ImportedInfo } from '@kith/types';
import { AlertDialog, AlertDialogBackdrop, AlertDialogContent, AlertDialogDescription, AlertDialogTitle } from '@ui-sdk/react/alert-dialog';
import { Dialog, DialogBackdrop, DialogClose, DialogContent, DialogDescription, DialogTitle, DialogTrigger } from '@ui-sdk/react/dialog';
import { Input } from '@ui-sdk/react/input';
import { Menu, MenuContent, MenuItem, MenuTrigger } from '@ui-sdk/react/menu';
import { Textarea } from '@ui-sdk/react/textarea';
import { FileUp, MoreHorizontalIcon, Plus } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { useLive } from '../../lib/data';
import { Badge, Button, StatusDot, fmt, toast } from '../../ui';

/** A server silent this long reads as offline, as a plain WireGuard peer does. */
const SILENT_SECS = 180;
const label = 'block font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3';
const enc = encodeURIComponent;

const nameProblem = (n: string) =>
  n === '' ? 'Give it a name.' : n.length > 63 ? 'Use 63 characters or fewer.' : /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(n) ? null : 'Use lowercase letters, digits and dashes, not at either end.';
const confProblem = (c: string) =>
  c.trim() === '' ? 'Paste a config or choose a .conf file.' : /^\s*\[Interface\]/im.test(c) && /^\s*\[Peer\]/im.test(c) ? null : "This isn't a WireGuard config. Paste the whole .conf file, from [Interface] to the end.";

function ImportDialog({ disabled }: { disabled: boolean }) {
  const { refresh } = useLive();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [conf, setConf] = useState('');
  const [file, setFile] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const picker = useRef<HTMLInputElement>(null);
  // Each opening starts empty: the last config's private key is not kept around.
  const reset = () => (setName(''), setConf(''), setFile(null), setErr(null));

  const choose = async (chosen: File | undefined) => {
    if (!chosen) return;
    try {
      setConf(await chosen.text());
      setFile(chosen.name);
      setErr(null);
      if (name === '') setName(chosen.name.replace(/\.conf$/i, '').toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, ''));
    } catch {
      setErr("Couldn't read that file. Paste its text instead.");
    }
  };

  const submit = async () => {
    const n = name.trim();
    const problem = nameProblem(n) ?? confProblem(conf);
    setErr(problem);
    if (problem) return;
    setBusy(true);
    try {
      await api.post('/api/wireguard/import', { name: n, conf_text: conf });
      setOpen(false);
      reset();
      toast.success(`Imported ${n}. Its traffic goes through it now.`);
      refresh();
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => (setOpen(o), o || reset())}>
      <DialogTrigger render={<Button icon={<Plus />} disabled={disabled} />}>Import a WireGuard config</DialogTrigger>
      <DialogBackdrop />
      <DialogContent className="flex w-[min(520px,calc(100vw-32px))] flex-col gap-4 text-left">
        <div className="space-y-1.5">
          <DialogTitle className="m-0 text-left">Import a WireGuard config</DialogTitle>
          <DialogDescription className="m-0 text-left">Its private key stays on this device.</DialogDescription>
        </div>
        <form
          className="space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <label className="block space-y-2">
            <span className={label}>Name</span>
            <Input className="w-full" value={name} onValueChange={setName} placeholder="office" autoFocus />
            <span className="block text-[13px] text-fg-3">What to call it in kith, like office or mullvad-de.</span>
          </label>
          <div className="space-y-2">
            <label className="block space-y-2">
              <span className={label}>Config</span>
              <Textarea
                className="h-40 w-full font-mono text-[12px]"
                value={conf}
                onValueChange={(v) => (setConf(v), setFile(null))}
                placeholder="[Interface]"
                spellCheck={false}
                autoComplete="off"
              />
            </label>
            <div className="flex flex-wrap items-center gap-3">
              <Button type="button" icon={<FileUp />} onClick={() => picker.current?.click()}>Choose a .conf file</Button>
              {file && <span className="min-w-0 truncate text-[13px] text-fg-3">From {file}</span>}
              <input
                ref={picker}
                type="file"
                accept=".conf,text/plain"
                className="sr-only"
                tabIndex={-1}
                aria-hidden="true"
                onChange={(e) => {
                  const f = e.currentTarget.files?.[0];
                  e.currentTarget.value = '';
                  void choose(f);
                }}
              />
            </div>
          </div>
          {err && <p role="alert" className="text-[13px] text-hot">{err}</p>}
          <div className="flex justify-end gap-2 pt-2">
            <DialogClose render={<Button />}>Cancel</DialogClose>
            <Button type="submit" variant="primary" busy={busy}>Import config</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Plain WireGuard networks imported from a wg-quick config: use, stop using, remove. */
export function WireguardImports() {
  const { boot, refresh } = useLive();
  const status = boot?.status;
  const operator = status?.operator ?? false;
  const imports = [...(status?.imported ?? [])].sort((a, b) => a.name.localeCompare(b.name));
  const [busy, setBusy] = useState<string | null>(null);
  const [removing, setRemoving] = useState<ImportedInfo | null>(null);

  const run = async (one: ImportedInfo, path: string, body: unknown, done: string) => {
    setBusy(one.network);
    try {
      await api.post(`/api/wireguard/${enc(one.network)}/${path}`, body);
      toast.success(done);
      refresh();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const use = (one: ImportedInfo, active: boolean) =>
    run(one, 'use', { active } satisfies ImportUseIn, active ? `Using ${one.name}.` : `Stopped using ${one.name}.`);

  const replaces = Boolean(status?.exit_node) && imports.some((one) => one.everything && !one.active);

  return (
    <div className="flex flex-col gap-3">
      {imports.length > 0 && (
        <ul className="border border-line" aria-label="Imported WireGuard servers">
          {imports.map((one) => {
            const heard = one.active && one.last_seen_secs !== null && one.last_seen_secs < SILENT_SECS;
            return (
              <li key={one.network} className="flex flex-wrap items-center justify-between gap-x-4 gap-y-3 border-b border-line px-4 py-3 last:border-0">
                <div className="min-w-0 flex-1 basis-56">
                  <div className="flex min-w-0 items-center gap-2">
                    <StatusDot status={heard ? 'online' : 'offline'} />
                    <span className="min-w-0 truncate text-[14px] text-fg" title={one.name}>{one.name}</span>
                    <Badge tone={one.active ? 'strong' : 'neutral'}>{one.active ? 'In use' : 'Not in use'}</Badge>
                  </div>
                  <div className="mt-1 flex flex-wrap items-baseline gap-x-3 text-[13px] text-fg-2">
                    <span className="[overflow-wrap:anywhere]">{one.everything ? 'All internet traffic' : one.routes.join(', ')}</span>
                    {one.endpoint && <span className="font-mono text-[12px] text-fg-3 [overflow-wrap:anywhere]">{one.endpoint}</span>}
                  </div>
                  <div className="mt-0.5 flex flex-wrap gap-x-3 text-[13px] text-fg-3">
                    <span>{one.last_seen_secs === null ? 'Not heard from yet' : `Heard from ${fmt.relative(Date.now() - one.last_seen_secs * 1000)}`}</span>
                    {(one.rx_bytes > 0 || one.tx_bytes > 0) && <span className="whitespace-nowrap">{fmt.bytes(one.rx_bytes)} in, {fmt.bytes(one.tx_bytes)} out</span>}
                  </div>
                </div>
                {operator && (
                  <div className="flex items-center gap-2">
                    <Button busy={busy === one.network} onClick={() => void use(one, !one.active)}>
                      {one.active ? `Stop using ${one.name}` : `Use ${one.name}`}
                    </Button>
                    <Menu onAction={(v) => v === 'remove' && setRemoving(one)}>
                      <MenuTrigger render={<Button variant="ghost" icon={<MoreHorizontalIcon />} aria-label={`More for ${one.name}`} />} />
                      <MenuContent>
                        <MenuItem value="remove" className="text-hot">Remove…</MenuItem>
                      </MenuContent>
                    </Menu>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {replaces && operator && <p className="text-[13px] text-fg-3">Using one that carries all internet traffic stops the exit in use: internet traffic goes through it instead.</p>}
      {operator ? (
        <div>
          <ImportDialog disabled={!status} />
        </div>
      ) : (
        <p className="text-[13px] text-fg-3">Only someone who may change kith on this computer can import or use a WireGuard server.</p>
      )}
      <AlertDialog open={removing !== null} onOpenChange={(o) => o || setRemoving(null)}>
        <AlertDialogBackdrop />
        {removing && (
          <AlertDialogContent className="flex flex-col gap-4">
            <div className="space-y-1.5">
              <AlertDialogTitle className="m-0 text-left">Remove {removing.name}?</AlertDialogTitle>
              <AlertDialogDescription className="m-0 text-left">
                kith forgets {removing.name}'s config and its private key. To use it again, import the .conf file again.
              </AlertDialogDescription>
            </div>
            <div className="flex justify-end gap-2 pt-2">
              <Button variant="ghost" onClick={() => setRemoving(null)}>Cancel</Button>
              <Button
                variant="danger"
                busy={busy === removing.network}
                onClick={() => {
                  const one = removing;
                  void run(one, 'remove', undefined, `Removed ${one.name}.`).then(() => setRemoving(null));
                }}
              >
                Remove {removing.name}
              </Button>
            </div>
          </AlertDialogContent>
        )}
      </AlertDialog>
    </div>
  );
}
