<!--
@component
StatusDot (spec §5): 8 px, always paired with a word; colour is never the only carrier.
  online: filled `--ink-1` · offline: 1.5 px ring in `--ink-3` · stale (last known): ring
  in `--ink-1` · problem: filled `--danger` · needs_you: filled `--attn` · waiting (a phone
  waiting for its first connection): a dashed 1 px `--ink-3` outline.
Never pulses or animates.
Props:
  - state: DotState ('online' | 'offline' | 'stale' | 'problem' | 'needs_you' | 'waiting').
  - text?: string (from copy.ts, for example `online`). Rendered as the visible word beside
    the dot, in the dot's ink (`--ink-3` for offline); the dot is then decorative.
  - label?: string, the accessible name, for example "online". Set it only when neither
    `text` nor any other visible word sits next to the dot.
-->
<script lang="ts">
  import type { DotState } from '../types';

  let { state, text, label }: { state: DotState; text?: string; label?: string } = $props();
  const named = $derived(label !== undefined && text === undefined);
</script>

{#if text !== undefined}
  <span class="status {state}">
    <span class="dot {state}" aria-hidden="true"></span>
    <span>{text}</span>
  </span>
{:else}
  <span
    class="dot {state}"
    role={named ? 'img' : undefined}
    aria-label={named ? label : undefined}
    aria-hidden={named ? undefined : 'true'}
  ></span>
{/if}

<style>
  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--ink-1);
  }
  .status.offline,
  .status.waiting {
    color: var(--ink-3);
  }
  .status.problem {
    color: var(--danger);
  }
  .status.needs_you {
    color: var(--attn-text);
  }
  .dot {
    display: inline-block;
    flex: none;
    box-sizing: border-box;
    width: var(--dot);
    height: var(--dot);
    border: var(--dot-ring) solid transparent;
    border-radius: var(--r-full);
  }
  .dot.online {
    background: var(--ink-1);
    border-color: var(--ink-1);
  }
  .dot.offline {
    border-color: var(--ink-3);
  }
  .dot.stale {
    border-color: var(--ink-1);
  }
  .dot.problem {
    background: var(--danger);
    border-color: var(--danger);
  }
  .dot.needs_you {
    background: var(--attn);
    border-color: var(--attn);
  }
  .dot.waiting {
    border: var(--dot-dash) dashed var(--ink-3);
  }
  @media (forced-colors: active) {
    .dot {
      forced-color-adjust: none;
    }
    .dot.online,
    .dot.problem,
    .dot.needs_you {
      background: CanvasText;
      border-color: CanvasText;
    }
    .dot.offline,
    .dot.stale,
    .dot.waiting {
      border-color: CanvasText;
    }
  }
</style>
