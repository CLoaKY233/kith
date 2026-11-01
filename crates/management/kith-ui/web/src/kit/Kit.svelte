<!--
@component
The `/__kit` page (spec §16 B): every lib/ui control in every state, in the current theme,
for looking at and keyboard-testing them. Dev only: main.ts mounts it under `npm run dev`
and production builds drop it. It reads no store and calls no API. Section names are
component names for engineers, not product copy; everything a control shows comes from
copy.ts.
-->
<script lang="ts">
  import { Tooltip } from 'bits-ui';
  import { copy } from '../lib/copy';
  import Sprite from '../lib/icons/Sprite.svelte';
  import { TOOLTIP_DELAY_MS } from '../lib/live/timing';
  import type { DotState, Theme } from '../lib/types';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import CopyField from '../lib/ui/CopyField.svelte';
  import Field from '../lib/ui/Field.svelte';
  import IconButton from '../lib/ui/IconButton.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Kbd from '../lib/ui/Kbd.svelte';
  import RoleChip from '../lib/ui/RoleChip.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import StatusDot from '../lib/ui/StatusDot.svelte';
  import Switch from '../lib/ui/Switch.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';

  const NET = 'home';
  const ME = 'studio-mac';

  let theme = $state<Theme>('system');
  $effect(() => {
    if (theme === 'system') delete document.documentElement.dataset['theme'];
    else document.documentElement.dataset['theme'] = theme;
  });

  let compact = $state(false);
  $effect(() => {
    if (compact) document.documentElement.dataset['density'] = 'compact';
    else delete document.documentElement.dataset['density'];
  });

  let pending = $state(false);
  let stale = $state(false);
  const variants = ['primary', 'secondary', 'ghost', 'danger-quiet', 'danger'] as const;

  let network = $state<string>(copy.createNetwork.initial);
  const networkError = $derived(
    network === '' ? copy.error.nameEmpty : /^[a-z0-9-]+$/.test(network) ? null : copy.error.nameChars,
  );
  let blurred = $state(false);
  let filter = $state('');
  let link = $state('');

  let range = $state('15m');
  let validity = $state('hour');
  let scope = $state('mine');

  let notifications = $state(false);
  let share = $state(true);

  let addTab = $state('live');

  const dots: { state: DotState; text: string }[] = [
    { state: 'online', text: copy.status.online },
    { state: 'offline', text: copy.status.offline },
    { state: 'stale', text: copy.status.stale },
    { state: 'problem', text: copy.status.problem },
    { state: 'needs_you', text: copy.status.needsYou },
    { state: 'waiting', text: copy.device.waitingFirst },
  ];
</script>

<Sprite />
<Tooltip.Provider delayDuration={TOOLTIP_DELAY_MS}>
  <div class="kit">
    <header class="top">
      <h1>/__kit</h1>
      <Segmented
        label={copy.settings.theme}
        bind:value={theme}
        options={(['system', 'light', 'dark'] as const).map((t) => ({
          value: t,
          label: copy.settings.themes[t],
        }))}
      />
      <Switch label={copy.settings.compact} bind:checked={compact} />
    </header>

    <section>
      <h2>Button</h2>
      <div class="controls">
        <Switch label="pending" bind:checked={pending} />
        <Switch label="stale (aria-disabled)" bind:checked={stale} />
      </div>
      {#each variants as variant (variant)}
        <div class="row">
          <code class="tag">{variant}</code>
          <Button {variant} {pending} disabled={stale} aria-describedby={stale ? 'kit-stale' : undefined}>
            {variant === 'danger' || variant === 'danger-quiet'
              ? copy.addPhone.remove
              : copy.createNetwork.submit(NET)}
          </Button>
          <Button {variant} {pending} pendingLabel={copy.createNetwork.pending} disabled={stale}>
            {copy.createNetwork.submit(NET)}
          </Button>
        </div>
      {/each}
      {#if stale}<p class="reason" id="kit-stale">{copy.conn.staleActions}</p>{/if}
      <div class="row footer">
        <Button>{copy.join.deny}</Button>
        <Button variant="primary">{copy.join.approve}</Button>
      </div>
      <div class="narrow">
        <Button variant="primary" block {pending}>{copy.addPhone.create}</Button>
      </div>
    </section>

    <section>
      <h2>IconButton</h2>
      <div class="row">
        <IconButton icon="ellipsis" label={copy.device.more} />
        <IconButton icon="copy" label={copy.copyField.copy} />
        <IconButton icon="search" label={copy.header.search} pressed />
        <IconButton icon="x" label={copy.action.close} disabled />
      </div>
    </section>

    <section>
      <h2>Input · Field</h2>
      <div class="grid">
        <Field
          label={copy.createNetwork.label}
          helper={copy.createNetwork.helper(ME, network || NET)}
          error={blurred ? networkError : null}
        >
          {#snippet children(control)}
            <Input
              {...control}
              kind="name"
              bind:value={network}
              onblur={() => (blurred = true)}
            />
          {/snippet}
        </Field>
        <Field label={copy.addPhone.name} helper={copy.addPhone.helper('phone', NET)} hint={copy.createNetwork.spaces}>
          {#snippet children(control)}
            <Input {...control} kind="name" value="phone" suffix={`.${NET}`} />
          {/snippet}
        </Field>
        <Field label={copy.joinNetwork.label} error={copy.joinNetwork.used}>
          {#snippet children(control)}
            <Input {...control} kind="mono" bind:value={link} />
          {/snippet}
        </Field>
        <Field label={copy.devices.filterLabel} hideLabel>
          {#snippet children(control)}
            <Input {...control} kind="search" icon="search" bind:value={filter} placeholder={copy.devices.filter} />
          {/snippet}
        </Field>
        <Field label={copy.thisDevice.name}>
          {#snippet children(control)}
            <Input {...control} value={ME} disabled />
          {/snippet}
        </Field>
        <Field label={copy.thisDevice.fingerprint}>
          {#snippet children(control)}
            <Input {...control} kind="mono" value="k7f3·9a21" readonly />
          {/snippet}
        </Field>
      </div>
    </section>

    <section>
      <h2>Segmented</h2>
      <div class="stack">
        <Segmented
          label={copy.chart.rangeLabel}
          bind:value={range}
          options={(['1m', '15m', '1h'] as const).map((r) => ({ value: r, label: copy.chart.ranges[r] }))}
        />
        <Segmented
          label={copy.invite.validFor}
          bind:value={validity}
          options={(['hour', 'day', 'week'] as const).map((v) => ({
            value: v,
            label: copy.invite.validity[v],
          }))}
          disabled={stale}
        />
        <Segmented
          label={copy.addPhone.stepScan}
          bind:value={scope}
          options={[
            { value: 'mine', label: copy.addPhone.justMine },
            { value: 'all', label: copy.addPhone.everything, disabled: true, reason: copy.addPhone.noExit },
          ]}
        />
      </div>
    </section>

    <section>
      <h2>Switch</h2>
      <div class="narrow">
        <Switch label={copy.settings.notificationsSwitch} bind:checked={notifications} />
        <Switch label={copy.settings.shareLinks(NET)} helper={copy.settings.shareLinksHelp} bind:checked={share} />
        <Switch label={copy.settings.shortcuts} checked disabled />
        <Switch label={copy.settings.compact} disabled />
      </div>
    </section>

    <section>
      <h2>CopyField</h2>
      <div class="narrow stack">
        <CopyField value={copy.gate.downCommand} />
        <CopyField value={copy.addDevice.joinCommand(NET, ME)} label={copy.invite.link} />
        <CopyField value={copy.addDevice.installCommand} />
      </div>
    </section>

    <section>
      <h2>Badge · Kbd · Spinner</h2>
      <div class="row">
        <Badge>{copy.device.you}</Badge>
        <Badge tone="quiet">{copy.path.flapping}</Badge>
        <Badge tone="attn">{copy.status.needsYou}</Badge>
        <Badge tone="danger">{copy.role.removed}</Badge>
        <Kbd>{copy.header.searchShortcut}</Kbd>
        <Kbd>{copy.header.searchShortcutOther}</Kbd>
        <Spinner />
        <Spinner label={copy.action.signing} />
      </div>
    </section>

    <section>
      <h2>StatusDot</h2>
      <div class="row">
        {#each dots as dot (dot.state)}
          <StatusDot state={dot.state} text={dot.text} />
        {/each}
      </div>
      <div class="row">
        {#each dots as dot (dot.state)}
          <StatusDot state={dot.state} label={dot.text} />
        {/each}
      </div>
    </section>

    <section>
      <h2>RoleChip</h2>
      <div class="row">
        <RoleChip role="admin" network={NET} keyFpr="k7f3·9a21" madeAdminBy="desk" madeAdminAtMs={Date.UTC(2026, 9, 12)} />
        <RoleChip role="member" network={NET} admins={['desk', 'nas']} />
        <RoleChip role="view_only" network={NET} />
        <RoleChip role="removed" network={NET} />
      </div>
    </section>

    <section>
      <h2>Tabs</h2>
      <div class="narrow">
        <Tabs
          label={copy.addDevice.title}
          bind:value={addTab}
          tabs={[
            { value: 'live', label: copy.addDevice.approveLive },
            { value: 'invite', label: copy.addDevice.sendInvite },
          ]}
        >
          {#snippet panel(tab)}
            {#if tab === 'live'}
              <div class="stack">
                <p>{copy.addDevice.runOnNew}</p>
                <CopyField value={copy.addDevice.joinCommand(NET, ME)} />
                <p class="reason">{copy.addDevice.waiting}</p>
              </div>
            {:else}
              <Button variant="primary">{copy.invite.create}</Button>
            {/if}
          {/snippet}
        </Tabs>
      </div>
    </section>

    <section class="surface">
      <h2>On --surface</h2>
      <div class="row">
        <Button variant="primary">{copy.join.approve}</Button>
        <Button>{copy.join.deny}</Button>
        <Badge>{copy.device.you}</Badge>
        <RoleChip role="member" network={NET} admins={['desk']} />
        <StatusDot state="offline" text={copy.status.offline} />
      </div>
    </section>
  </div>
</Tooltip.Provider>

<style>
  .kit {
    display: grid;
    gap: var(--s-7);
    max-width: var(--content-max);
    margin: 0 auto;
    padding: var(--s-6) var(--content-pad) var(--s-8);
  }
  @media (width < 40rem) {
    .kit {
      padding-inline: var(--gutter-mobile);
    }
  }
  .top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-5);
  }
  h1 {
    font: var(--t-title);
    letter-spacing: var(--t-title-tracking);
    font-family: var(--ff-mono);
  }
  h2 {
    margin-bottom: var(--s-4);
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
    margin-bottom: var(--s-3);
  }
  .footer {
    justify-content: flex-end;
    max-width: var(--sheet);
  }
  .tag {
    width: calc(var(--s-8) + var(--s-6));
    color: var(--ink-3);
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-5);
    margin-bottom: var(--s-3);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, var(--recent-column)), 1fr));
    gap: var(--s-5);
  }
  .stack {
    display: grid;
    gap: var(--s-4);
  }
  .narrow {
    max-width: var(--sheet);
  }
  .reason {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .surface {
    padding: var(--s-5);
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface);
  }
</style>
