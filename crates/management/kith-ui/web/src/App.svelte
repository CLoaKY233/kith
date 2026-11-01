<!--
@component
The shell (spec §4, §6.9, §10). Picks a Gate, Welcome, or the frame around a screen for
the current route; opens the overlays the URL names (`?add=`, `?need=`, `?remove=`,
`?flow=`); owns the palette, the global shortcuts, the theme on <html>, the browser
title and the live regions. Screens receive the network id as `net`.
-->
<script lang="ts">
  import { Tooltip } from 'bits-ui';
  import { onMount } from 'svelte';
  import { fade } from 'svelte/transition';
  import { connection } from './frame/conn.svelte';
  import Frame from './frame/Frame.svelte';
  import { layers } from './frame/layers.svelte';
  import { deviceName, membership, networkName } from './frame/net';
  import { section, title } from './frame/nav';
  import NeedSheet from './frame/NeedSheet.svelte';
  import { needs } from './frame/needs.svelte';
  import { prefs } from './frame/prefs.svelte';
  import UnauthOverlay from './frame/UnauthOverlay.svelte';
  import { attachAnnouncer } from './lib/announce';
  import { copy } from './lib/copy';
  import Sprite from './lib/icons/Sprite.svelte';
  import { live, start } from './lib/live/store.svelte';
  import { DAEMON_POLL_MS, MOTION, TOOLTIP_DELAY_MS } from './lib/live/timing';
  import Toast from './lib/ui/Toast.svelte';
  import {
    closeOverlay,
    go,
    href,
    netHref,
    parse,
    setNetworks,
    subscribe,
    type Route,
  } from './router';
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
  import AddDevice from './sheets/AddDevice.svelte';
  import AddPhone from './sheets/AddPhone.svelte';
  import Invite from './sheets/Invite.svelte';
  import RemoveDevice from './sheets/RemoveDevice.svelte';

  let route = $state<Route>(parse(location.pathname));
  let query = $state(new URLSearchParams(location.search));
  let polite = $state('');
  let assertive = $state('');

  connection.track();
  needs.track();

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

  $effect(() => {
    setNetworks(live.status?.networks ?? []);
  });

  const networks = $derived(live.status?.networks ?? []);
  /** The network the frame points into: the one on screen, else the last one used. */
  const net = $derived(
    'net' in route
      ? route.net
      : (networks.find((n) => n.id === prefs.lastNetwork)?.id ?? networks[0]?.id ?? null),
  );

  // `/` → the last network, or Welcome; an unknown path or network → `/`.
  $effect(() => {
    if (!live.ready) return;
    if (route.name === 'not_found') {
      go('/', { replace: true });
    } else if (route.name === 'root') {
      go(net ? href({ name: 'overview', net }) : href({ name: 'welcome' }), { replace: true });
    }
  });

  $effect(() => {
    if ('net' in route) prefs.lastNetwork = route.net;
  });

  // Theme and density from Settings, on <html> (§3.2, §3.4).
  $effect(() => {
    const root = document.documentElement;
    if (prefs.theme === 'system') delete root.dataset['theme'];
    else root.dataset['theme'] = prefs.theme;
    if (prefs.density === 'compact') root.dataset['density'] = 'compact';
    else delete root.dataset['density'];
  });

  // §4.1: `(n) laptop · home — kith`.
  $effect(() => {
    const me = net ? membership(net) : null;
    const device = me?.name ?? live.status?.host.name ?? copy.brand;
    document.title = copy.documentTitle(device, net ? networkName(net) : null, live.unseen);
  });

  // §6.9: the daemon-down and group gates check again every 2 s.
  $effect(() => {
    if (live.gate !== 'daemon_down' && live.gate !== 'not_in_group') return;
    const timer = setInterval(() => void start(), DAEMON_POLL_MS);
    return () => clearInterval(timer);
  });

  // The gate cross-fades into the app (§6.9); the first paint never fades (§8.1).
  const reduced =
    typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  let gated = $state(false);
  $effect(() => {
    if (live.gate) gated = true;
  });
  const crossfade = $derived({ duration: reduced || !gated ? 0 : MOTION.dialog });

  // Overlays named by the URL (§4.1).
  const add = $derived(query.get('add'));
  const need = $derived(query.get('need'));
  const remove = $derived(query.get('remove'));
  const closeAdd = (open: boolean): void => {
    if (!open) closeOverlay('add');
  };

  // The palette and the shortcut sheet load on first use, off the first paint (§8.1).
  let paletteUsed = $state(false);
  let shortcutsUsed = $state(false);
  $effect(() => {
    if (layers.palette) paletteUsed = true;
    if (layers.shortcuts) shortcutsUsed = true;
  });

  // §10 global shortcuts. ⌘K always works; single keys respect Settings and stay quiet
  // while typing or while a layer is open.
  let chord = false;
  function keys(event: KeyboardEvent): void {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      layers.palette = !layers.palette;
      return;
    }
    if (event.metaKey || event.ctrlKey || event.altKey || event.defaultPrevented) return;
    if (!prefs.shortcuts || !live.ready || !net) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest('input, textarea, select, [contenteditable], [role="dialog"], [role="alertdialog"]')) return;
    if (layers.any || document.querySelector('[role="dialog"], [role="alertdialog"]')) return;
    const key = event.key;
    if (chord) {
      chord = false;
      const page = key === 'o' ? 'overview' : key === 'd' ? 'devices' : key === 'a' ? 'activity' : null;
      if (page) {
        event.preventDefault();
        go(netHref(net, page));
      }
      return;
    }
    if (key === 'g') {
      chord = true;
    } else if (key === '/') {
      const filter = document.querySelector<HTMLElement>('[data-filter]');
      if (filter) {
        event.preventDefault();
        filter.focus();
      }
    } else if (key === '?') {
      event.preventDefault();
      layers.shortcuts = true;
    }
  }
</script>

<svelte:window onkeydown={keys} />

<Sprite />
<Tooltip.Provider delayDuration={TOOLTIP_DELAY_MS}>
  <a class="skip" href="#content">{copy.skipToContent}</a>
  <div class="stack">
    {#if live.gate}
      <div class="layer" transition:fade={crossfade}>
        <main id="content"><Gate reason={live.gate} /></main>
      </div>
    {:else if live.ready}
      <div class="layer" transition:fade={crossfade}>
        {#if route.name === 'welcome' || net === null}
          <main id="content"><Welcome flow={query.get('flow')} /></main>
        {:else if route.name !== 'root' && route.name !== 'not_found'}
          <Frame title={title(route)} quietTitle={route.name === 'overview'} {net} page={section(route)}>
            {#if route.name === 'this'}
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
          </Frame>

          <AddDevice bind:open={() => add === 'device', closeAdd} {net} />
          <AddPhone bind:open={() => add === 'phone', closeAdd} {net} node={query.get('node') ?? undefined} />
          <Invite bind:open={() => add === 'invite', closeAdd} {net} />
          <NeedSheet {net} id={need} />
          <NeedSheet {net} id={null} list />
          {#if remove}
            <RemoveDevice
              bind:open={() => remove !== null, (open) => !open && closeOverlay('remove')}
              {net}
              node={remove}
              name={deviceName(net, remove)}
              network={networkName(net)}
            />
          {/if}
          {#if paletteUsed}
            {#await import('./frame/Palette.svelte') then { default: Palette }}
              <Palette {net} />
            {/await}
          {/if}
          {#if shortcutsUsed}
            {#await import('./frame/Shortcuts.svelte') then { default: Shortcuts }}
              <Shortcuts />
            {/await}
          {/if}
        {/if}
        {#if connection.shown === 'unauth'}<UnauthOverlay />{/if}
      </div>
    {/if}
  </div>
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
  /* The gate and the app share one cell, so they cross-fade in place. */
  .stack {
    display: grid;
    min-height: 100dvh;
  }
  .layer {
    grid-area: 1 / 1;
    min-width: 0;
  }
</style>
