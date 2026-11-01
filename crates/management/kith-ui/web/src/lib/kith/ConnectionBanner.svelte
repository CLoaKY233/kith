<!--
@component
ConnectionBanner (spec §5, §8.5): a full-width bar under the header.
  - daemon_down, on `--attn-wash`: `kith isn't running on this device. Showing what was
    true at 14:02.` with a `How to start it` disclosure holding `sudo kith up`.
  - ui_gone, neutral: `This page lost its connection to kith-ui. Run kith ui to reopen it.`
On recovery it collapses over 200 ms (grid rows, so nothing below jumps). The assertive
announcement comes from frame/conn, so the bar itself is not a live region.
Props:
  - kind: 'daemon_down' | 'ui_gone' | null. Null hides it.
  - asOfMs: number, server time of the last known values, for `as of 14:02`.
-->
<script lang="ts">
  import { copy } from '../copy';
  import { clock } from '../format';
  import Icon from '../icons/Icon.svelte';
  import CopyField from '../ui/CopyField.svelte';

  let { kind, asOfMs }: { kind: 'daemon_down' | 'ui_gone' | null; asOfMs: number } = $props();

  // Keep the last message while the bar collapses.
  let shown = $state<'daemon_down' | 'ui_gone'>('daemon_down');
  $effect(() => {
    if (kind !== null) shown = kind;
  });
  let howOpen = $state(false);
</script>

<div class="wrap" class:open={kind !== null} inert={kind === null}>
  <div class="clip">
    <div class="banner {shown}">
      {#if shown === 'daemon_down'}
        <p>{copy.conn.daemonDown(clock(asOfMs))}</p>
        <details bind:open={howOpen}>
          <summary>
            <Icon name={howOpen ? 'chevron-down' : 'chevron-right'} />
            <span>{copy.conn.howToStart}</span>
          </summary>
          <div class="how">
            <p>{copy.gate.downBody}</p>
            <CopyField value={copy.gate.downCommand} />
          </div>
        </details>
      {:else}
        <p>{copy.conn.uiGone}</p>
      {/if}
    </div>
  </div>
</div>

<style>
  .wrap {
    display: grid;
    grid-template-rows: 0fr;
    transition: grid-template-rows var(--d-banner) var(--e-sheet);
  }
  .wrap.open {
    grid-template-rows: 1fr;
  }
  .clip {
    overflow: hidden;
    min-height: 0;
  }
  .banner {
    display: grid;
    gap: var(--s-2);
    padding: var(--s-3) var(--content-pad);
    border-bottom: var(--hairline) solid var(--line);
    background: var(--surface);
    color: var(--ink-1);
  }
  .banner.daemon_down {
    background: var(--attn-wash);
  }
  summary {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--ink-2);
    cursor: pointer;
    list-style: none;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  .how {
    display: grid;
    gap: var(--s-2);
    padding-top: var(--s-2);
    max-width: var(--gate-max);
  }
  @media (width < 40rem) {
    .banner {
      padding-inline: var(--gutter-mobile);
    }
  }
</style>
