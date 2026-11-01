<!--
@component
The shell: picks a Gate, Welcome or a screen for the current route. Workstream D owns the
frame (sidebar, header, tab bar, palette wiring); this version only routes.
-->
<script lang="ts">
  import { Tooltip } from 'bits-ui';
  import { onMount } from 'svelte';
  import { attachAnnouncer } from './lib/announce';
  import { copy } from './lib/copy';
  import Sprite from './lib/icons/Sprite.svelte';
  import { live } from './lib/live/store.svelte';
  import { TOOLTIP_DELAY_MS } from './lib/live/timing';
  import Toast from './lib/ui/Toast.svelte';
  import { go, href, parse, subscribe, type Route } from './router';
  import Activity from './routes/Activity.svelte';
  import Device from './routes/Device.svelte';
  import Devices from './routes/Devices.svelte';
  import Exits from './routes/Exits.svelte';
  import Gate from './routes/Gate.svelte';
  import Names from './routes/Names.svelte';
  import Overview from './routes/Overview.svelte';
  import Settings from './routes/Settings.svelte';
  import ThisDevice from './routes/ThisDevice.svelte';
  import Welcome from './routes/Welcome.svelte';

  let route = $state<Route>(parse(location.pathname));
  let query = $state(new URLSearchParams(location.search));
  let polite = $state('');
  let assertive = $state('');

  onMount(() => {
    attachAnnouncer((politeness, message) => {
      if (politeness === 'assertive') assertive = message;
      else polite = message;
    });
    return subscribe((next, params) => {
      route = next;
      query = params;
    });
  });

  const firstNetwork = $derived(live.status?.networks[0]?.name ?? null);

  $effect(() => {
    if (!live.ready || route.name !== 'root') return;
    go(firstNetwork ? href({ name: 'overview', net: firstNetwork }) : '/welcome', {
      replace: true,
    });
  });

  $effect(() => {
    const net = 'net' in route ? route.net : firstNetwork;
    document.title = copy.documentTitle(live.status?.memberships[0]?.name ?? copy.brand, net, live.unseen);
  });
</script>

<Sprite />
<Tooltip.Provider delayDuration={TOOLTIP_DELAY_MS}>
  <a class="skip" href="#content">{copy.skipToContent}</a>
  <main id="content">
    {#if live.gate}
      <Gate reason={live.gate} />
    {:else if !live.ready}
      <!-- No skeletons on localhost (§14): nothing until the snapshot is in. -->
    {:else if route.name === 'welcome' || route.name === 'root'}
      <Welcome />
    {:else if route.name === 'this'}
      <ThisDevice />
    {:else if route.name === 'overview'}
      <Overview net={route.net} />
    {:else if route.name === 'devices'}
      <Devices net={route.net} {query} />
    {:else if route.name === 'device'}
      <Device net={route.net} dev={route.dev} {query} />
    {:else if route.name === 'activity'}
      <Activity net={route.net} {query} />
    {:else if route.name === 'names'}
      <Names net={route.net} />
    {:else if route.name === 'exits'}
      <Exits net={route.net} />
    {:else if route.name === 'settings'}
      <Settings net={route.net} />
    {/if}
  </main>
  <Toast />
</Tooltip.Provider>
<div class="sr-only" aria-live="polite">{polite}</div>
<div class="sr-only" aria-live="assertive">{assertive}</div>

<style>
  .skip {
    position: absolute;
    left: var(--s-2);
    top: calc(-1 * var(--s-8));
    z-index: var(--z-toast);
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-2);
    background: var(--surface);
    color: var(--ink-1);
  }
  .skip:focus {
    top: var(--s-2);
  }
</style>
