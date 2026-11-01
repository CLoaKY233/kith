<!--
@component
Dev-only kit (spec §16 B): each overlay in each of its states, every icon at 16 px, and a
theme switch. Content comes from copy.ts; the section names are the component names.
-->
<script lang="ts">
  import { Tooltip as TooltipProvider } from 'bits-ui';
  import { copy } from '../../copy';
  import Icon from '../../icons/Icon.svelte';
  import Sprite from '../../icons/Sprite.svelte';
  import { ICONS } from '../../icons/names';
  import { SHEET_CLOSE_AFTER_ONLINE_MS, TOOLTIP_DELAY_MS } from '../../live/timing';
  import AlertDialog from '../AlertDialog.svelte';
  import Button from '../Button.svelte';
  import Command, { type CommandGroup } from '../Command.svelte';
  import Dialog from '../Dialog.svelte';
  import DropdownMenu from '../DropdownMenu.svelte';
  import IconButton from '../IconButton.svelte';
  import Popover from '../Popover.svelte';
  import Sheet from '../Sheet.svelte';
  import Toast, { toast } from '../Toast.svelte';
  import Tooltip from '../Tooltip.svelte';

  let dialog = $state(false);
  let alert = $state(false);
  let alertPending = $state(false);
  let alertStatus = $state<string | undefined>(undefined);
  let alertError = $state<string | undefined>(undefined);
  let sheet = $state(false);
  let sheetAuto = $state(false);
  let anchored = $state(false);
  let palette = $state(false);
  let sort = $state('name');
  let card: HTMLElement | null = $state(null);
  let picked = $state('');

  const devices = ['nas', 'laptop', 'phone', 'vps', 'desk', 'tv'];

  const groups: CommandGroup[] = [
    {
      heading: copy.palette.groups.devices,
      items: devices.map((name, i) => ({
        id: `device:${name}`,
        label: name,
        hint: `100.64.0.${i + 2}`,
        keywords: [`100.64.0.${i + 2}`],
        onSelect: ({ port }) => (picked = port ? `${name}:${port}` : name),
      })),
    },
    {
      heading: copy.palette.groups.actions,
      items: [
        { id: 'add-device', label: copy.palette.addDevice, onSelect: () => (sheet = true) },
        { id: 'add-phone', label: copy.palette.addPhone, onSelect: () => (picked = copy.palette.addPhone) },
        { id: 'invite', label: copy.palette.newInvite, onSelect: () => (picked = copy.palette.newInvite) },
        { id: 'rename-nas', label: copy.palette.askRename('nas'), onSelect: () => (picked = 'rename') },
      ],
    },
    {
      heading: copy.palette.groups.goTo,
      items: [
        { id: 'go-overview', label: copy.nav.overview, onSelect: () => (picked = copy.nav.overview) },
        { id: 'go-devices', label: copy.nav.devices, onSelect: () => (picked = copy.nav.devices) },
      ],
    },
  ];

  function theme(next: 'light' | 'dark' | null): void {
    if (next) document.documentElement.dataset['theme'] = next;
    else delete document.documentElement.dataset['theme'];
  }

  function confirmRemove(): void {
    alertError = undefined;
    alertPending = true;
    alertStatus = copy.tier3.waiting;
  }
</script>

<Sprite />
<TooltipProvider.Provider delayDuration={TOOLTIP_DELAY_MS}>
  <main class="kit">
    <section>
      <h2>Theme</h2>
      <div class="row">
        <Button onclick={() => theme(null)}>{copy.settings.themes.system}</Button>
        <Button onclick={() => theme('light')}>{copy.settings.themes.light}</Button>
        <Button onclick={() => theme('dark')}>{copy.settings.themes.dark}</Button>
      </div>
    </section>

    <section>
      <h2>Dialog</h2>
      <div class="row">
        <Button onclick={() => (dialog = true)}>{copy.shortcuts.title}</Button>
      </div>
      <Dialog bind:open={dialog} title={copy.shortcuts.title} closeLabel={copy.action.close}>
        <p>{copy.shortcuts.palette}</p>
        <p>{copy.shortcuts.filter}</p>
        <p>{copy.shortcuts.close}</p>
      </Dialog>
    </section>

    <section>
      <h2>AlertDialog</h2>
      <div class="row">
        <Button variant="danger-quiet" onclick={() => (alert = true)}>{copy.detail.remove('home')}</Button>
        <Button onclick={() => (alertPending = false)}>{copy.action.cancel}</Button>
        <Button
          onclick={() => {
            alertPending = false;
            alertStatus = undefined;
            alertError = copy.error.permissionDenied;
          }}>{copy.action.later}</Button
        >
      </div>
      <AlertDialog
        bind:open={alert}
        title={copy.remove.title('phone', 'home')}
        description={copy.remove.loses('phone')}
        cancelLabel={copy.action.cancel}
        confirmLabel={copy.remove.submit('phone')}
        pending={alertPending}
        status={alertStatus}
        error={alertError}
        onConfirm={confirmRemove}
      >
        <p>{copy.remove.frees('100.64.0.7', 'phone.home')}</p>
        <p>{copy.remove.comeBack}</p>
      </AlertDialog>
    </section>

    <section>
      <h2>Sheet</h2>
      <div class="row">
        <Button onclick={() => (sheet = true)}>{copy.addDevice.title}</Button>
        <Button
          onclick={() => {
            sheetAuto = true;
            sheet = true;
          }}>{copy.join.showDevice}</Button
        >
      </div>
      <Sheet
        bind:open={sheet}
        title={copy.addDevice.title}
        closeLabel={copy.action.close}
        autoCloseMs={sheetAuto ? SHEET_CLOSE_AFTER_ONLINE_MS : null}
        onOpenChange={(next) => {
          if (!next) sheetAuto = false;
        }}
      >
        <p>{copy.join.wants('dev-box', 'home')}</p>
        <p>{copy.join.approveOnlyIf}</p>
        {#each devices as name (name)}<p>{copy.join.online(name, copy.path.direct, '14 ms')}</p>{/each}
        {#snippet footer()}
          <Button onclick={() => (sheet = false)}>{copy.join.deny}</Button>
          <Button variant="primary">{copy.join.approve}</Button>
        {/snippet}
      </Sheet>
    </section>

    <section>
      <h2>Popover</h2>
      <div class="row">
        <Popover label={copy.header.needsYouTitle} width="card">
          {#snippet trigger(props)}
            <Button {...props}>{copy.header.needsYou(2)}</Button>
          {/snippet}
          <p>{copy.join.wants('pixel-8', 'home')}</p>
        </Popover>
        <div class="card" bind:this={card}>
          <span>{copy.blocked.card('phone', 'nas', copy.blocked.port(22, 'ssh'), 4)}</span>
          <Button onclick={() => (anchored = true)}>{copy.blocked.allowThis}</Button>
        </div>
        <Popover bind:open={anchored} anchor={card} align="end" label={copy.blocked.allowThis}>
          <p>{copy.blocked.rule('phone', 'nas', 'TCP', 22)}</p>
          <p class="meta">{copy.blocked.nothingElse}</p>
        </Popover>
      </div>
    </section>

    <section>
      <h2>Tooltip and DropdownMenu</h2>
      <div class="row">
        <IconButton icon="search" label={copy.header.search} />
        <Tooltip text={copy.path.direct}>
          {#snippet trigger(props)}
            <button type="button" {...props} class="glyph"><Icon name="path-direct" /> {copy.path.direct}</button>
          {/snippet}
        </Tooltip>
        <DropdownMenu
          label={copy.device.more}
          items={[
            { label: copy.detail.rename, onSelect: () => (picked = copy.detail.rename), hint: 'F2' },
            { label: copy.detail.useAsExit, onSelect: () => (picked = copy.detail.useAsExit) },
            { label: copy.detail.allowAsExit, onSelect: () => (picked = ''), disabled: true },
            { separator: true },
            { label: copy.detail.remove('home'), onSelect: () => (alert = true), tone: 'danger' },
          ]}
        >
          {#snippet trigger(props)}
            <IconButton icon="ellipsis" label={copy.device.more} triggerProps={props} />
          {/snippet}
        </DropdownMenu>
        <DropdownMenu
          align="start"
          items={[
            {
              choice: {
                value: sort,
                options: [
                  { value: 'name', label: copy.devices.sortName },
                  { value: 'traffic', label: copy.devices.sortTraffic },
                ],
                onChange: (value) => (sort = value),
              },
            },
          ]}
        >
          {#snippet trigger(props)}
            <Button {...props} variant="ghost">{copy.devices.sortLabel}</Button>
          {/snippet}
        </DropdownMenu>
      </div>
    </section>

    <section>
      <h2>Toast</h2>
      <div class="row">
        <Button onclick={() => toast.done(copy.blocked.done('phone', 'nas', 22))}>
          {copy.blocked.addRule}
        </Button>
        <Button onclick={() => toast.done(copy.join.denied('dev-box'), { undo: () => undefined })}>
          {copy.join.deny}
        </Button>
        <Button onclick={() => toast.failed(copy.error.internal('timeout'))}>{copy.action.dismiss}</Button>
      </div>
    </section>

    <section>
      <h2>Command</h2>
      <div class="row">
        <Button onclick={() => (palette = true)}>{copy.header.search}</Button>
        <span class="meta">{picked}</span>
      </div>
      <Command bind:open={palette} {groups} />
    </section>

    <section>
      <h2>Icons</h2>
      <ul class="icons">
        {#each ICONS as name (name)}
          <li><Icon {name} /><span>{name}</span></li>
        {/each}
      </ul>
    </section>
  </main>
  <Toast />
</TooltipProvider.Provider>

<style>
  .kit {
    display: grid;
    gap: var(--s-7);
    max-width: var(--content-max);
    margin: 0 auto;
    padding: var(--s-6) var(--gutter-mobile);
  }
  h2 {
    font: var(--t-heading);
    margin-bottom: var(--s-3);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
  }
  .card {
    flex-wrap: wrap;
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-3);
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface);
  }
  .glyph {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  .meta {
    color: var(--ink-3);
  }
  .icons {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, var(--menu-min)), 1fr));
    gap: var(--s-2);
  }
  .icons li {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font: var(--t-mono);
  }
</style>
