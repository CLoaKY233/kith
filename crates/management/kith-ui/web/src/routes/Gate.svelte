<!--
@component
Gate (spec §6.9): the page cannot show the network yet. No app frame, centred, max 520 px.
  - daemon_down: `kith isn't running on this device.`, the command, and `Checking every
    2 seconds…`. App.svelte checks again every 2 s and cross-fades into the app.
  - not_in_group: the group command for this OS, then `Then log out and back in.`
  - version_skew: `kith-ui 0.2 can't talk to kith 0.1. Update kith on this device.`
  - unauthorized, session_used, session_expired: the fresh-link page (the server renders
    the same without JS); used and expired links say which in the title.
Props:
  - reason: GateReason
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import { live } from '../lib/live/store.svelte';
  import type { GateReason } from '../lib/types';
  import CopyField from '../lib/ui/CopyField.svelte';

  let { reason }: { reason: GateReason } = $props();

  const mac = typeof navigator !== 'undefined' && /Mac/.test(navigator.platform);
  const authTitle = $derived(
    reason === 'session_used'
      ? copy.gate.authUsed
      : reason === 'session_expired'
        ? copy.gate.authExpired
        : copy.gate.authTitle,
  );
</script>

<section class="gate" aria-labelledby="gate-title">
  {#if reason === 'daemon_down'}
    <h1 id="gate-title" class="title">{copy.gate.downTitle}</h1>
    <p>{copy.gate.downBody}</p>
    <CopyField value={copy.gate.downCommand} />
    <p class="meta" role="status">{copy.gate.checking}</p>
  {:else if reason === 'not_in_group'}
    <h1 id="gate-title" class="title">{copy.gate.groupTitle}</h1>
    <CopyField value={mac ? copy.gate.groupMac : copy.gate.groupLinux} />
    <p>{copy.gate.groupThen}</p>
  {:else if reason === 'version_skew'}
    <h1 id="gate-title" class="title">
      {copy.gate.skew(live.versions?.ui ?? '', live.versions?.daemon ?? '')}
    </h1>
  {:else}
    <h1 id="gate-title" class="title">{authTitle}</h1>
    <p>{copy.gate.authBody}</p>
    <CopyField value={copy.gate.authCommand} />
    <p class="meta">{copy.gate.authOnce}</p>
  {/if}
</section>

<style>
  .gate {
    display: grid;
    align-content: center;
    gap: var(--s-4);
    min-height: 100dvh;
    max-width: var(--gate-max);
    margin: 0 auto;
    padding: var(--s-8) var(--gutter-mobile);
  }
  .title {
    font: var(--t-title);
    letter-spacing: var(--t-title-tracking);
    text-wrap: balance;
  }
  p {
    max-width: var(--measure);
    color: var(--ink-2);
  }
  .meta {
    color: var(--ink-3);
  }
</style>
