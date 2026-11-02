<!--
@component
The ⌘K palette's content (spec §10): Devices (by name, address or full name, this device
included) · Actions (Add device, Add a phone, New invite, Use exit…, Add name, Add access rule, Add
relay, Import a WireGuard config…, Offer this device…, then Rename
nas and Copy nas address once something is typed) · Go to · Networks. Members see admin
actions as `Ask an admin to …`, which open the Ask an admin sheet (§7.10); `Ask an admin to
rename nas…` opens it on the device. `nas:22` opens nas with `?port=22`. `Use exit…` opens
the ExitChip's picker. Command ranks items picked before first.
Props:
  - net: string | null, the network id on screen.
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import AskAdminSheet from '../lib/kith/AskAdminSheet.svelte';
  import { askRenameRequest, renameRequest } from '../lib/kith/deviceState.svelte';
  import { live } from '../lib/live/store.svelte';
  import Command, { type CommandGroup, type CommandItem } from '../lib/ui/Command.svelte';
  import { toast } from '../lib/ui/Toast.svelte';
  import { go, href, netHref, openOverlay } from '../router';
  import { layers } from './layers.svelte';
  import { isAdmin, networkName, peers, selfPeer } from './net';
  import { OBSERVING, SETTINGS, SHAPING } from './nav';

  let { net }: { net: string | null } = $props();

  async function copyAddress(name: string, address: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(address);
      toast.done(copy.palette.copied(name));
    } catch {
      toast.failed(copy.palette.copyFailed(name));
    }
  }

  let askOpen = $state(false);
  let askRequest = $state('');
  function ask(request: string): void {
    askRequest = request;
    askOpen = true;
  }

  const groups = $derived.by((): CommandGroup[] => {
    const out: CommandGroup[] = [];
    if (net) {
      const id = net;
      const admin = isAdmin(id);
      const name = networkName(id);
      const others = peers(id).sort((a, b) => a.name.localeCompare(b.name));
      const self = selfPeer(id);
      const deviceHref = (dev: string): string => href({ name: 'device', net: id, dev });
      out.push({
        heading: copy.palette.groups.devices,
        items: [...(self ? [self] : []), ...others].map((p) => ({
          id: `device:${p.node}`,
          label: p.name,
          hint: p === self ? copy.palette.you(p.address) : p.address,
          keywords: [p.address, p.display_name, copy.detail.fqdn(p.name, name)],
          onSelect: ({ port }: { port?: number }) =>
            go(port ? `${deviceHref(p.name)}?port=${port}` : deviceHref(p.name)),
        })),
      });

      const actions: CommandItem[] = [];
      if (admin) {
        actions.push(
          { id: 'add:device', label: copy.palette.addDevice, onSelect: () => openOverlay('add', 'device') },
          { id: 'add:phone', label: copy.palette.addPhone, onSelect: () => openOverlay('add', 'phone') },
          { id: 'add:invite', label: copy.palette.newInvite, onSelect: () => openOverlay('add', 'invite') },
        );
      } else {
        actions.push(
          {
            id: 'ask:device',
            label: copy.palette.askAddDevice,
            onSelect: () => ask(copy.askAdmin.request.invite(name)),
          },
          {
            id: 'ask:phone',
            label: copy.palette.askAddPhone,
            onSelect: () => ask(copy.askAdmin.request.addPhone(name)),
          },
          {
            id: 'ask:invite',
            label: copy.palette.askInvite,
            onSelect: () => ask(copy.askAdmin.request.invite(name)),
          },
        );
      }
      actions.push({ id: 'exit', label: copy.palette.useExit, onSelect: () => (layers.exit = true) });
      actions.push(
        admin
          ? {
              id: 'add:name',
              label: copy.palette.addName,
              onSelect: () => go(netHref(id, 'names', { new: '1' })),
            }
          : {
              id: 'ask:name',
              label: copy.palette.askAddName,
              onSelect: () => ask(copy.askAdmin.request.addName(name)),
            },
      );
      actions.push(
        admin
          ? { id: 'add:rule', label: copy.palette.addRule, onSelect: () => go(netHref(id, 'access', { new: '1' })) }
          : { id: 'ask:access', label: copy.palette.askAccess, onSelect: () => ask(copy.askAdmin.request.policy(name)) },
        admin
          ? { id: 'add:relay', label: copy.palette.addRelay, onSelect: () => go(netHref(id, 'settings', { relay: 'new' })) }
          : { id: 'ask:relay', label: copy.palette.askRelay, onSelect: () => ask(copy.askAdmin.request.addRelay(name)) },
        {
          id: 'import:wireguard',
          label: copy.palette.importWg,
          onSelect: () => go(netHref(id, 'exits', { sheet: 'wireguard' })),
        },
        { id: 'offer:this', label: copy.palette.offerThis, onSelect: () => go(href({ name: 'this' })) },
      );
      for (const p of others) {
        const device = deviceHref(p.name);
        const key = `${id}/${p.node}`;
        actions.push(
          admin
            ? {
                id: `rename:${p.node}`,
                label: copy.palette.rename(p.name),
                whenTyped: true,
                onSelect: () => {
                  renameRequest.key = key;
                  go(device);
                },
              }
            : {
                id: `ask-rename:${p.node}`,
                label: copy.palette.askRename(p.name),
                whenTyped: true,
                onSelect: () => {
                  askRenameRequest.key = key;
                  go(device);
                },
              },
          {
            id: `copy:${p.node}`,
            label: copy.palette.copyAddress(p.name),
            whenTyped: true,
            onSelect: () => void copyAddress(p.name, p.address),
          },
        );
      }
      out.push({ heading: copy.palette.groups.actions, items: actions });

      out.push({
        heading: copy.palette.groups.goTo,
        items: [
          ...[...OBSERVING, ...SHAPING, SETTINGS].map((item) => ({
            id: `go:${item.page}`,
            label: item.label,
            onSelect: () => go(netHref(id, item.page)),
          })),
          { id: 'go:this', label: copy.nav.thisDevice, onSelect: () => go(href({ name: 'this' })) },
        ],
      });
    }

    const networks = live.status?.networks ?? [];
    if (networks.length > 0) {
      out.push({
        heading: copy.palette.groups.networks,
        items: networks.map((n) => ({
          id: `network:${n.id}`,
          label: n.name,
          onSelect: () => go(href({ name: 'overview', net: n.id })),
        })),
      });
    }
    return out;
  });
</script>

<Command bind:open={layers.palette} {groups} />

{#if net && askOpen}
  <AskAdminSheet bind:open={askOpen} {net} request={askRequest} />
{/if}
