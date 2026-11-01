<!--
@component
StatusDot (spec §5): 8 px, always paired with a visible word.
Props:
  - state: 'online' | 'offline' | 'stale' | 'problem' | 'needs_you' | 'waiting'
    online: filled ink-1 · offline: 1.5 px ring ink-3 · stale (last known): ring ink-1 ·
    problem: filled danger · needs_you: filled attn · waiting (phone): dashed ring.
  - label?: string, the accessible name (for example "online"). Set it only when no
    visible word sits next to the dot.
-->
<script lang="ts">
  import type { DotState } from '../types';

  let { state, label }: { state: DotState; label?: string } = $props();
</script>

<span
  class="dot {state}"
  role={label ? 'img' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : 'true'}
></span>

<style>
  .dot {
    display: inline-block;
    flex: none;
    width: var(--dot);
    height: var(--dot);
    border-radius: var(--r-full);
    border: var(--dot-ring) solid transparent;
  }
  .online {
    background: var(--ink-1);
    border-color: var(--ink-1);
  }
  .offline {
    border-color: var(--ink-3);
  }
  .stale {
    border-color: var(--ink-1);
  }
  .problem {
    background: var(--danger);
    border-color: var(--danger);
  }
  .needs_you {
    background: var(--attn);
    border-color: var(--attn);
  }
  .waiting {
    border: var(--hairline) dashed var(--ink-3);
  }
  @media (forced-colors: active) {
    .online,
    .problem,
    .needs_you {
      background: CanvasText;
      border-color: CanvasText;
    }
    .offline,
    .stale,
    .waiting {
      border-color: CanvasText;
    }
  }
</style>
