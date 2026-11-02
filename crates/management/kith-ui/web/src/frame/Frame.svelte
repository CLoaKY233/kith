<!--
@component
The app frame (spec §4.2–4.4) around every network screen and This device.
  - ≥1024 px (64rem): the 232 px sidebar and the 56 px header.
  - 640–1023 px (40–64rem): the 64 px rail and the same header.
  - Below 640 px: the 52 px top bar, the screen title at the top of the content, and the
    bottom tab bar.
The ConnectionBanner is the first thing in <main>, so it belongs to a landmark. Breakpoints
are in rem, so at 400 % zoom the phone layout takes over. Content is `--bg`; each screen
sets its own width and padding, left-aligned at the same x on every screen (§4.2).
Props:
  - title: string, the screen title.
  - quietTitle?: boolean. On phones the title is only for screen readers (Overview, whose
    headline and active tab already say where you are).
  - net: string, the network id the nav points into.
  - page: NavPage | null, the active nav page.
  - self?: boolean, true on This device (the identity block and the Network tab are active).
  - children: the screen.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { MediaQuery } from 'svelte/reactivity';
  import ConnectionBanner from '../lib/kith/ConnectionBanner.svelte';
  import { connection } from './conn.svelte';
  import Header from './Header.svelte';
  import MobileBars from './MobileBars.svelte';
  import type { NavPage } from './nav';
  import Sidebar from './Sidebar.svelte';

  let {
    title,
    quietTitle = false,
    net,
    page,
    self = false,
    children,
  }: {
    title: string;
    quietTitle?: boolean;
    net: string;
    page: NavPage | null;
    self?: boolean;
    children: Snippet;
  } = $props();

  const sidebar = new MediaQuery('min-width: 64rem');
  const tablet = new MediaQuery('min-width: 40rem');

  const banner = $derived(
    connection.shown === 'daemon_down' || connection.shown === 'ui_gone' ? connection.shown : null,
  );
</script>

{#if tablet.current}
  <div class="frame">
    <Sidebar {net} {page} {self} rail={!sidebar.current} />
    <div class="main">
      <Header {title} {net} />
      <main id="content" class="content" tabindex="-1" aria-labelledby="page-title">
        <ConnectionBanner kind={banner} asOfMs={connection.asOf} />
        {@render children()}
      </main>
    </div>
  </div>
{:else}
  <div class="phone">
    <MobileBars {net} {page} {self} part="top" />
    <main id="content" class="content" tabindex="-1" aria-labelledby="page-title">
      <ConnectionBanner kind={banner} asOfMs={connection.asOf} />
      <h1 id="page-title" class="title" class:sr-only={quietTitle}>{title}</h1>
      {@render children()}
    </main>
    <MobileBars {net} {page} {self} part="tabs" />
  </div>
{/if}

<style>
  .frame {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    min-height: 100dvh;
  }
  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .content {
    flex: 1;
    min-width: 0;
    outline: none;
  }
  .phone {
    min-height: 100dvh;
    padding-bottom: calc(var(--tabbar) + env(safe-area-inset-bottom));
  }
  .title {
    padding: var(--s-4) var(--gutter-mobile) 0;
    font: var(--t-title);
    letter-spacing: var(--t-title-tracking);
  }
</style>
