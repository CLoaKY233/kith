<!--
@component
Inlines sprite.svg once, at the top of App, so every Icon can `<use>` it.
-->
<script lang="ts">
  import sprite from './sprite.svg?raw';
</script>

<!-- eslint-disable-next-line svelte/no-at-html-tags -- the sprite is a build-time file, not data -->
{@html sprite}
