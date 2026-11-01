<!--
@component
PathBadge (spec §5): text plus glyph, no pill fill. Placeholder; workstream D owns it.
Props:
  - path: PathKind | null
  - relay?: string | null, the relay's name for `via vps`.
  - flapping?: boolean.
-->
<script lang="ts">
  import { copy } from '../copy';
  import Icon from '../icons/Icon.svelte';
  import type { PathKind } from '../types.gen';

  let {
    path,
    relay = null,
    flapping = false,
  }: { path: PathKind | null; relay?: string | null; flapping?: boolean } = $props();
</script>

{#if path}
  <span class="badge">
    <Icon name={path === 'direct' ? 'path-direct' : path === 'relay' ? 'path-relayed' : 'path-stealth'} />
    <span>
      {path === 'direct'
        ? copy.path.direct
        : path === 'relay'
          ? copy.path.via(relay ?? '')
          : copy.path.stealthVia(relay ?? '')}
    </span>
    {#if flapping}<span class="flap">{copy.path.flapping}</span>{/if}
  </span>
{/if}

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--ink-2);
  }
  .flap {
    color: var(--ink-3);
  }
</style>
