<!--
@component
An icon from the sprite (spec §5 Icons): 16 px, 1.5 stroke, round caps, in currentColor.
The Lucide subset plus the custom `path-direct`, `path-relayed`, `path-stealth`, `seal` and
the kith `mark`. Never beside a worded button, never the only carrier of state, never a
brand or OS logo.
Props:
  - name: IconName (required).
  - label?: string. Without it the icon is decorative (aria-hidden); with it, it is an image
    with that name. Even then a word must say the same thing nearby.
-->
<script lang="ts">
  import type { IconName } from './names';

  let { name, label }: { name: IconName; label?: string } = $props();
</script>

<svg
  class="icon"
  viewBox="0 0 24 24"
  role={label ? 'img' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : 'true'}
  focusable="false"
>
  <use href={`#i-${name}`} />
</svg>

<style>
  /* A <use> inherits from this element, not from the sprite's root, so the stroke style
     the sprite declares has to be set here too. */
  .icon {
    flex: none;
    width: var(--icon);
    height: var(--icon);
    fill: none;
    stroke: currentColor;
    stroke-width: var(--icon-stroke);
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
