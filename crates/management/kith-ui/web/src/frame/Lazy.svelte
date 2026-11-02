<!--
@component
Lazy: renders a screen or sheet from outside the first load (screens.svelte.ts). It starts
the load when first mounted and renders nothing until the module is there; once loaded it
renders at once, with no pending state.
Props:
  - name: ScreenKey, which module.
  - children: Snippet<[Component]>, given the module's component.
-->
<script lang="ts" generics="K extends ScreenKey">
  import type { Snippet } from 'svelte';
  import { load, loaded, type ScreenKey, type ScreenModule } from '../screens.svelte';

  let { name, children }: { name: K; children: Snippet<[ScreenModule<K>['default']]> } = $props();

  const module = $derived(loaded.all[name] as ScreenModule<K> | undefined);
  $effect(() => {
    if (!module) void load(name).catch(() => undefined);
  });
</script>

{#if module}{@render children(module.default)}{/if}
