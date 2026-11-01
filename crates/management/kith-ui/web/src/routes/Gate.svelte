<!--
@component
Gate (spec §6.9): daemon down, not in the group, version skew, 401. No app frame, centred,
max 520 px. Placeholder with the exact copy; workstream D owns it.
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
</script>

<section class="gate">
  {#if reason === 'daemon_down'}
    <h1 class="t-title">{copy.gate.downTitle}</h1>
    <p>{copy.gate.downBody}</p>
    <CopyField value={copy.gate.downCommand} />
    <p class="meta">{copy.gate.checking}</p>
  {:else if reason === 'not_in_group'}
    <h1 class="t-title">{copy.gate.groupTitle}</h1>
    <CopyField value={mac ? copy.gate.groupMac : copy.gate.groupLinux} />
    <p>{copy.gate.groupThen}</p>
  {:else if reason === 'version_skew'}
    <h1 class="t-title">
      {copy.gate.skew(live.versions?.ui ?? '', live.versions?.daemon ?? '')}
    </h1>
  {:else}
    <h1 class="t-title">{copy.gate.authTitle}</h1>
    {#if reason === 'session_used'}<p>{copy.gate.authUsed}</p>{/if}
    {#if reason === 'session_expired'}<p>{copy.gate.authExpired}</p>{/if}
    <p>{copy.gate.authBody}</p>
    <CopyField value={copy.gate.authCommand} />
    <p class="meta">{copy.gate.authOnce}</p>
  {/if}
</section>

<style>
  .gate {
    display: grid;
    gap: var(--s-4);
    max-width: var(--gate-max);
    margin: 0 auto;
    padding: var(--s-8) var(--gutter-mobile);
  }
  .meta {
    color: var(--ink-3);
  }
</style>
