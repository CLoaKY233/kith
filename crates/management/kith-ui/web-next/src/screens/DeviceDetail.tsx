import { useEffect, useState, type ReactNode } from 'react';
import { Input } from '@ui-sdk/react/input';
import { Menu, MenuContent, MenuItem, MenuSeparator, MenuTrigger } from '@ui-sdk/react/menu';
import { Dialog, DialogBackdrop, DialogContent, DialogDescription, DialogTitle } from '@ui-sdk/react/dialog';
import { AlertDialog, AlertDialogBackdrop, AlertDialogContent, AlertDialogDescription, AlertDialogTitle } from '@ui-sdk/react/alert-dialog';
import { Checkbox, CheckboxIndicator } from '@ui-sdk/react/checkbox';
import { CheckIcon, MonitorSmartphoneIcon, MoreHorizontalIcon, PencilIcon, Trash2Icon } from 'lucide-react';
import { api, errorMessage, net } from '../lib/api';
import { useLive, useNetwork } from '../lib/data';
import { useRoute } from '../lib/router';
import { Badge, Button, Copy, Empty, Page, Panel, PanelGrid, Skeleton, Stat, StatusDot, fmt, toast } from '../ui';
import { linkFor, pathText, seen } from '../components/device/peer';
import { PathDiagram } from '../components/device/PathDiagram';
import { DeviceTraffic } from '../components/device/DeviceTraffic';
import { DeviceSettings } from '../components/device/DeviceSettings';
import { DeviceHistory } from '../components/device/DeviceHistory';

export function DeviceDetail({ name }: { name: string }) {
  const { boot, network, peers } = useNetwork();
  const { refresh, error } = useLive();
  const { go } = useRoute();
  const [renaming, setRenaming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [renamedNode, setRenamedNode] = useState<string | null>(null);

  const byName = peers.find((p) => p.name === name);
  const peer = byName ?? peers.find((p) => p.node === name) ?? peers.find((p) => p.node === renamedNode);
  const settled = byName != null;
  useEffect(() => {
    if (settled) setRenamedNode(null);
  }, [settled]);

  if (!boot) {
    return (
      <Page title={name} description={error ? undefined : 'Loading'}>
        {error ? (
          <Empty icon={<MonitorSmartphoneIcon />} action={<Button onClick={() => refresh()}>Try again</Button>}>
            {error}
          </Empty>
        ) : (
          <>
            <div className="grid grid-cols-2 gap-x-6 gap-y-8 lg:grid-cols-4">
              {Array.from({ length: 4 }, (_, i) => <Skeleton key={i} height={64} />)}
            </div>
            <Skeleton height={160} />
            <Skeleton height={320} />
          </>
        )}
      </Page>
    );
  }
  if (!network || !peer) {
    return (
      <Page title="Device not found">
        <Empty icon={<MonitorSmartphoneIcon />} action={<Button onClick={() => go('/devices')}>All devices</Button>}>
          {network ? `No device in ${network.name} is called ${name}.` : `No device is called ${name}.`} It may have been renamed or removed.
        </Empty>
      </Page>
    );
  }

  const link = peer.online ? linkFor(boot, network.id, peer.node) : null;
  const me = peers.find((p) => p.node === boot.status.node)?.name ?? boot.status.host?.name ?? 'This device';
  const relay = link?.relay ? (peers.find((p) => p.node === link.relay)?.name ?? null) : null;
  const isSelf = peer.node === boot.status.node;
  const s = seen(peer);
  const settings = peer.settings;
  const exitName = settings?.exit ? (peers.find((p) => p.node === settings.exit)?.name ?? 'another device') : null;

  return (
    <Page
      title={peer.name}
      description={
        <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <StatusDot status={peer.online ? 'online' : 'offline'} />
          <span title={s.title}>{peer.online ? 'Online' : peer.last_seen_secs == null ? 'Offline, never seen' : `Offline, last seen ${s.text}`}</span>
          {peer.online && (
            <>
              <span className="text-fg-3">·</span>
              <span>{pathText(peer, relay)}</span>
            </>
          )}
          {peer.display_name !== peer.name && (
            <>
              <span className="text-fg-3">·</span>
              <span className="text-fg-3">{peer.display_name}</span>
            </>
          )}
        </span>
      }
      actions={
        network.admin && (
          <Menu onAction={(v) => (v === 'rename' ? setRenaming(true) : setRemoving(true))}>
            <MenuTrigger render={<Button icon={<MoreHorizontalIcon />} />}>Actions</MenuTrigger>
            <MenuContent align="end">
              <MenuItem value="rename"><PencilIcon className="size-3.5" aria-hidden="true" /> Rename</MenuItem>
              <MenuSeparator />
              <MenuItem value="remove" className="text-hot"><Trash2Icon className="size-3.5" aria-hidden="true" /> {isSelf ? 'Leave network' : 'Remove'}</MenuItem>
            </MenuContent>
          </Menu>
        )
      }
    >
      <div className="flex flex-col gap-8">
        <div className="grid grid-cols-2 gap-x-6 gap-y-8 lg:grid-cols-4">
          <Stat label="Latency" value={peer.online && link?.rtt_ms != null ? fmt.ms(link.rtt_ms) : '—'} secondary={peer.online && link?.loss ? `${fmt.percent(link.loss * 100)} loss` : undefined} />
          <Stat label={`From ${peer.name}`} value={fmt.bytes(peer.rx_bytes)} secondary={`Received by ${me}`} title={`Bytes ${me} received from ${peer.name} since kith on ${me} last started`} />
          <Stat label={`To ${peer.name}`} value={fmt.bytes(peer.tx_bytes)} secondary={`Sent by ${me}`} title={`Bytes ${me} sent to ${peer.name} since kith on ${me} last started`} />
          <Stat label="Last seen" value={s.text} title={s.title} />
        </div>
        <div className="flex flex-wrap items-center gap-x-8 gap-y-3 border-t border-line pt-4">
          <Meta label="Address"><Copy text={peer.address} /></Meta>
          <Meta label="DNS name"><Copy text={`${peer.name}.${network.name}`} /></Meta>
          {peer.endpoint && <Meta label="Endpoint"><Copy text={peer.endpoint} /></Meta>}
          {(exitName || settings?.exit_locked || settings?.paused || peer.via_this_exit) && (
            <span className="flex flex-wrap items-center gap-1.5">
              {exitName && <Badge>Exit via {exitName}</Badge>}
              {settings?.exit_locked && <Badge>Locked</Badge>}
              {settings?.paused && <Badge tone="danger">Paused</Badge>}
              {peer.via_this_exit && <Badge tone="strong">Using this exit</Badge>}
            </span>
          )}
        </div>
      </div>

      <PanelGrid>
        <Panel label="Path" span={12}>
          <PathDiagram me={me} peer={peer} link={link} relay={relay} self={isSelf} />
        </Panel>
        <DeviceTraffic span={8} network={network.id} node={peer.node} />
        <DeviceSettings span={4} network={network.id} peer={peer} admin={network.admin} onSaved={refresh} />
        <DeviceHistory span={12} node={peer.node} />
      </PanelGrid>

      {network.admin && (
        <>
          <RenameDialog
            open={renaming}
            onOpenChange={setRenaming}
            current={peer.name}
            onRename={async (next) => {
              await api.post(net(network.id, `/devices/${encodeURIComponent(peer.node)}/rename`), { name: next, based_on: peer.name });
              toast(`Renamed to ${next}`);
              setRenamedNode(peer.node);
              refresh();
              go(`/devices/${encodeURIComponent(next)}`, { replace: true });
            }}
          />
          <RemoveDialog
            open={removing}
            onOpenChange={setRemoving}
            name={peer.name}
            network={network.name}
            self={isSelf}
            onRemove={async (lost) => {
              await api.post(net(network.id, `/devices/${encodeURIComponent(peer.node)}/remove`), { lost });
              toast(isSelf ? `Left ${network.name}` : `Removed ${peer.name}`);
              refresh();
              go('/devices');
            }}
          />
        </>
      )}
    </Page>
  );
}

function Meta({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="flex min-w-0 max-w-full items-baseline gap-3">
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">{label}</span>
      {children}
    </span>
  );
}

function RenameDialog({ open, onOpenChange, current, onRename }: { open: boolean; onOpenChange: (o: boolean) => void; current: string; onRename: (n: string) => Promise<void> }) {
  const [value, setValue] = useState(current);
  const [busy, setBusy] = useState(false);
  const next = value.trim();
  const edge = next.startsWith('-') || next.endsWith('-');
  useEffect(() => {
    if (open) setValue(current);
  }, [open, current]);
  const submit = async () => {
    if (!next || next === current) return onOpenChange(false);
    if (edge) return;
    setBusy(true);
    try {
      await onRename(next);
      onOpenChange(false);
    } catch (e) {
      toast.error('Could not rename', { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={(o) => !busy && onOpenChange(o)}>
      <DialogBackdrop />
      <DialogContent className="flex flex-col gap-4">
        <DialogTitle>Rename {current}</DialogTitle>
        <DialogDescription>Lowercase letters, digits and dashes. The device's DNS name changes too.</DialogDescription>
        <form onSubmit={(e) => (e.preventDefault(), void submit())} className="flex flex-col gap-4">
          <div className="flex flex-col gap-2">
            <Input autoFocus className="!h-8 font-mono !text-[13px]" maxLength={63} value={value} onValueChange={(v: string) => setValue(v.toLowerCase().replace(/[^a-z0-9-]/g, ''))} aria-label="New name" />
            {edge && <p className="text-[13px] text-hot">Names can't start or end with a hyphen.</p>}
          </div>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>Cancel</Button>
            <Button type="submit" variant="primary" busy={busy} disabled={!next || next === current || edge}>Rename</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function RemoveDialog({ open, onOpenChange, name, network, self, onRemove }: { open: boolean; onOpenChange: (o: boolean) => void; name: string; network: string; self: boolean; onRemove: (lost: boolean) => Promise<void> }) {
  const [lost, setLost] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (open) setLost(false);
  }, [open]);
  const remove = async () => {
    setBusy(true);
    try {
      await onRemove(lost);
      onOpenChange(false);
    } catch (e) {
      toast.error(self ? 'Could not leave' : 'Could not remove', { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <AlertDialog open={open} onOpenChange={(o) => !busy && onOpenChange(o)}>
      <AlertDialogBackdrop />
      <AlertDialogContent className="flex flex-col gap-4">
        <AlertDialogTitle>{self ? `Leave ${network}?` : `Remove ${name}?`}</AlertDialogTitle>
        <AlertDialogDescription>
          {self
            ? 'This device leaves the network right away and loses its address. It needs a new invite to come back, and this computer asks you to confirm first.'
            : `It leaves ${network} right away and loses its address. Every device stops trusting it, and it needs a new invite to come back.`}
        </AlertDialogDescription>
        {!self && <label className="flex cursor-pointer items-start gap-3 border-t border-line pt-4 text-[14px] leading-5 text-fg">
          <Checkbox className="mt-[2px] shrink-0" checked={lost} onCheckedChange={(c) => setLost(c === true)}>
            <CheckboxIndicator>
              <CheckIcon className="size-3" />
            </CheckboxIndicator>
          </Checkbox>
          <span className="flex flex-col gap-0.5">
            It was lost or stolen
            <span className="text-[13px] text-fg-3">Block its key for good.</span>
          </span>
        </label>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" autoFocus onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button variant="danger" busy={busy} onClick={() => void remove()}>{self ? 'Leave' : 'Remove'}</Button>
        </div>
      </AlertDialogContent>
    </AlertDialog>
  );
}
