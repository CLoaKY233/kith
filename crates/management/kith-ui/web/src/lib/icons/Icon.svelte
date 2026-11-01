<!--
@component
An icon from the sprite (spec §5 Icons): 16 px, 1.5 stroke, currentColor.
Props:
  - name: IconName (required)
  - label?: string. Without it the icon is decorative (aria-hidden). Never the only carrier of state.
-->
<script lang="ts">
  import type { IconName } from './names';

  let { name, label }: { name: IconName; label?: string } = $props();
</script>

<svg
  class="icon"
  role={label ? 'img' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : 'true'}
  focusable="false"
>
  <use href={`#i-${name}`} />
</svg>

<style>
  .icon {
    flex: none;
    width: var(--icon);
    height: var(--icon);
    stroke-width: var(--icon-stroke);
  }
</style>
