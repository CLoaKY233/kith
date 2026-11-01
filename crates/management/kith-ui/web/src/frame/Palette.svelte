<!--
@component
The ⌘K palette's content (spec §10): Devices (by name or address) · Actions (Add device,
Add a phone, New invite, Use exit…, Add name, Rename nas, Copy nas address) · Go to ·
Networks. Members see admin actions as `Ask an admin to rename nas…`, which opens the
device, where the Ask an admin sheet lives. Command ranks items picked before first.
Props:
  - net: string | null, the network id on screen.
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import { live } from '../lib/live/store.svelte';
  import Command, { type CommandGroup, type CommandItem } from '../lib/ui/Command.svelte';
  import { toast } from '../lib/ui/Toast.svelte';
  import { go, href, netHref, openOverlay } from '../router';
  import { layers } from './layers.svelte';
  import { isAdmin, peers } from './net';
  import { OBSERVING, SETTINGS, SHAPING } from './nav';

  let { net }: { net: string | null } = $props();

  async function copyAddress(name: string, address: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(address);
      toast.done(copy.palette.copied(name));
    } catch {
      // Clipboard refused (no permission or not focused); nothing was copied.
    }
  }

  const groups = $derived.by((): CommandGroup[] => {
    const out: CommandGroup[] = [];
    if (net) {
      const id = net;
      const admin = isAdmin(id);
      const list = peers(id).sort((a, b) => a.name.localeCompare(b.name));
      out.push({
        heading: copy.palette.groups.devices,
        items: list.map((p) => ({
            id: `device:${p.node}`,
            label: p.name,
            hint: p.address,
            keywords: [p.address, p.display_name],
          onSelect: () => go(href({ name: 'device', net: id, dev: p.node })),
        })),
      });

      const actions: CommandItem[] = [];
      if (admin) {
        actions.push(
          { id: 'add:device', label: copy.palette.addDevice, onSelect: () => openOverlay('add', 'device') },
          { id: 'add:phone', label: copy.palette.addPhone, onSelect: () => openOverlay('add', 'phone') },
          { id: 'add:invite', label: copy.palette.newInvite, onSelect: () => openOverlay('add', 'invite') },
        );
      }
      actions.push({ id: 'exit', label: copy.palette.useExit, onSelect: () => go(netHref(id, 'exits')) });
      if (admin) {
        actions.push({
          id: 'add:name',
          label: copy.palette.addName,
          onSelect: () => go(netHref(id, 'names', { new: '1' })),
        });
      }
      for (const p of list) {
        const device = href({ name: 'device', net: id, dev: p.node });
        actions.push(
          admin
            ? {
                id: `rename:${p.node}`,
                label: copy.palette.rename(p.name),
                onSelect: () => go(`${device}?rename=1`),
              }
            : {
                id: `ask-rename:${p.node}`,
                label: copy.palette.askRename(p.name),
                onSelect: () => go(`${device}?ask=rename`),
              },
          {
            id: `copy:${p.node}`,
            label: copy.palette.copyAddress(p.name),
            hint: p.address,
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
