<!--
@component
Map (spec §9.2): the network as a hand-built SVG ring, a view on Devices. This device sits
at the centre, peers on a ring by hash rank, relays on an inner ring, phones beside their
gateway. Nodes move only when the node set changes, sliding round the ring over 400 ms.
Above 60 devices it shows the table instead.
Props:
  - net: string, the network id (a network name also resolves).
  - focused: string | null, the device whose detail is open: drawn selected, with its
    links at full strength.
  - onSelect: (node: string) => void, opens a device's detail. Called on click, Enter,
    and the focus card's `Open device`; on touch the first tap shows the card instead.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Spark from '../charts/Spark.svelte';
  import { copy, DOT } from '../copy';
  import { ago, rate, rtt } from '../format';
  import { live } from '../live/store.svelte';
  import { BANNER_DELAY_MS, EASING, HOVERCARD_DELAY_MS, MOTION } from '../live/timing';
  import { readPx, SPACE } from '../tokens';
  import Button from '../ui/Button.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Switch from '../ui/Switch.svelte';
  import {
    ALL_LINKS_UNDER,
    between,
    curve,
    DENSE_OVER,
    layout,
    TABLE_OVER,
    type Layout,
    type Placed,
    type Point,
  } from './layout';
  import MapTable from './MapTable.svelte';
  import {
    buildModel,
    edges,
    LAST_KNOWN,
    layoutInput,
    layoutKey,
    nodeLabel,
    nodeName,
    ownLink,
    pathText,
    relayUsers,
    remember,
    report,
    resolveNetwork,
    waiting,
    type MapDevice,
    type MapEdge,
  } from './model';

  let {
    net,
    focused,
    onSelect,
  }: { net: string; focused: string | null; onSelect: (node: string) => void } = $props();

  /** Zoom steps for `+` and `−`, and the closest zoom. */
  const ZOOM_STEP = 1.5;
  const ZOOM_MAX = ZOOM_STEP ** 3;
  const ease = curve(EASING.pop);

  // Sizes from tokens.css, read once mounted.
  let size = $state({ node: 0, self: 0, relay: 0, satellite: 0, touch: 0 });
  let coarse = $state(false);
  /** Below 40rem the focus card sits under the map instead of over it. */
  let narrow = $state(false);
  let canvas = $state<HTMLDivElement | null>(null);
  let svg = $state<SVGSVGElement | null>(null);
  let card = $state<HTMLDivElement | null>(null);
  let width = $state(0);
  let height = $state(0);
  let fontsReady = $state(0);

  const netId = $derived(resolveNetwork(live.status, net));
  const model = $derived(
    netId ? buildModel(live.status, live.peers[netId], live.links[netId], netId) : null,
  );
  const mac = $derived(live.status?.host.os === 'macOS');
  const count = $derived(model?.devices.size ?? 0);
  const asTable = $derived(count - 1 > TABLE_OVER);

  // §8.5: values turn last-known 3 s after the connection goes stale.
  let stale = $state(false);
  $effect(() => {
    if (!LAST_KNOWN.has(live.conn)) {
      stale = false;
      return;
    }
    const timer = setTimeout(() => (stale = true), BANNER_DELAY_MS);
    return () => clearTimeout(timer);
  });

  // Relays are remembered per network for the life of the page (see model.remember). The
  // memory is plain data, not state: it only grows, and `relays` hands out a new map when
  // it does.
  let memory: { net: string | null; relays: ReadonlyMap<string, ReadonlySet<string>> } = {
    net: null,
    relays: new Map(),
  };
  const relays = $derived.by(() => {
    if (!model) return memory.relays;
    if (memory.net !== model.net) memory = { net: model.net, relays: new Map() };
    memory.relays = remember(memory.relays, relayUsers(model));
    return memory.relays;
  });
  const input = $derived(model ? layoutInput(model, relays) : null);
  const dense = $derived((input?.peers.length ?? 0) > DENSE_OVER);
  const key = $derived(input ? layoutKey(input, dense) : '');

  const labels = $derived(
    new Map([...(model?.devices.values() ?? [])].map((d) => [d.id, nodeLabel(d, mac)])),
  );

  /** Width of the longest label and the label line height, in px. */
  let measure: CanvasRenderingContext2D | null = null;
  const metrics = $derived.by(() => {
    void fontsReady;
    const el = svg;
    if (!el || typeof document === 'undefined') return { wide: 0, tall: 0 };
    measure ??= document.createElement('canvas').getContext('2d');
    const ctx = measure;
    if (!ctx) return { wide: 0, tall: 0 };
    const style = getComputedStyle(el);
    ctx.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
    let wide = 0;
    for (const [id, text] of labels) {
      if (id !== model?.self) wide = Math.max(wide, ctx.measureText(text).width);
    }
    const m = ctx.measureText(labels.get(model?.self ?? '') ?? '');
    return { wide, tall: m.fontBoundingBoxAscent + m.fontBoundingBoxDescent };
  });

  const hasSatellites = $derived(
    [...(input?.gateways?.values() ?? [])].some((g) => g !== input?.self),
  );

  /** The outer ring radius: as large as the well allows with labels outside it. */
  const radius = $derived.by(() => {
    if (width === 0 || height === 0) return 0;
    const reach = size.node / 2 + SPACE[2] + (hasSatellites ? size.satellite : 0);
    const across = width / 2 - SPACE[4] - reach - metrics.wide;
    const down = height / 2 - SPACE[4] - reach - metrics.tall;
    return Math.max(SPACE[8], Math.floor(Math.min(across, down)));
  });

  // Recomputed only when the key or the size changes, not on every tick.
  const placed = $derived.by((): Layout | null => {
    void key;
    const current = untrack(() => input);
    if (!current || radius === 0) return null;
    return layout(current, { radius, satellite: size.satellite, spacing: size.node + SPACE[2] });
  });

  // Where nodes are drawn right now: the layout, or part way to it while sliding.
  let shown = $state.raw<ReadonlyMap<string, Point>>(new Map());
  let shownKey = '';
  let shownNet: string | null = null;
  let frame = 0;
  $effect(() => {
    const target = placed;
    const nextKey = key;
    const from = untrack(() => shown);
    cancelAnimationFrame(frame);
    if (!target) {
      shown = new Map();
      return;
    }
    const slide =
      shownKey !== '' &&
      shownKey !== nextKey &&
      shownNet === netId &&
      MOTION.map > 0 &&
      !window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    shownKey = nextKey;
    shownNet = netId;
    if (!slide) {
      shown = target.nodes;
      return;
    }
    const start = performance.now();
    const step = (now: number): void => {
      const t = ease(Math.min(1, (now - start) / MOTION.map));
      shown = new Map(
        [...target.nodes].map(([id, to]): [string, Point] => {
          const was = from.get(id);
          return [id, was ? between(was, to, t) : to];
        }),
      );
      if (t < 1) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
  });

  // Zoom and pan (world px; the centre is 0,0).
  let zoom = $state(1);
  let pan = $state({ x: 0, y: 0 });
  function clampPan(x: number, y: number, z: number): { x: number; y: number } {
    const mx = (width / 2) * (1 - 1 / z);
    const my = (height / 2) * (1 - 1 / z);
    return { x: Math.max(-mx, Math.min(mx, x)), y: Math.max(-my, Math.min(my, y)) };
  }
  function zoomTo(z: number): void {
    zoom = Math.max(1, Math.min(ZOOM_MAX, z));
    pan = clampPan(pan.x, pan.y, zoom);
  }
  function fit(): void {
    zoom = 1;
    pan = { x: 0, y: 0 };
  }
  const viewBox = $derived(
    `${pan.x - width / (2 * zoom)} ${pan.y - height / (2 * zoom)} ${width / zoom} ${height / zoom}`,
  );

  // Focus: hover, keyboard focus or tap draws a device's links; the prop is the fallback.
  let hovered = $state<string | null>(null);
  let keyed = $state<string | null>(null);
  let tapped = $state<string | null>(null);
  let carded = $state<string | null>(null);
  let showAll = $state(false);
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;

  const known = (id: string | null): string | null => (id && model?.devices.has(id) ? id : null);
  const focus = $derived(known(hovered) ?? known(keyed) ?? known(tapped) ?? known(focused));
  const cardDevice = $derived(model?.devices.get(known(carded) ?? '') ?? null);
  const allLinks = $derived(showAll && count < ALL_LINKS_UNDER);
  const drawn = $derived(model ? edges(model, { showAll: allLinks, focus }) : []);
  const tabStop = $derived(known(keyed) ?? known(focused) ?? model?.self ?? null);

  function clearHoverTimer(): void {
    if (hoverTimer !== null) clearTimeout(hoverTimer);
    hoverTimer = null;
  }

  function inside(el: Element | null, target: EventTarget | null): boolean {
    return el !== null && target instanceof Node && el.contains(target);
  }

  function nodeEl(id: string): SVGGElement | null {
    return svg?.querySelector<SVGGElement>(`[data-node="${CSS.escape(id)}"]`) ?? null;
  }

  function enter(id: string, event: PointerEvent): void {
    if (event.pointerType === 'touch' || dragging) return;
    hovered = id;
    clearHoverTimer();
    hoverTimer = setTimeout(() => (carded = id), HOVERCARD_DELAY_MS);
  }

  function leave(event: PointerEvent): void {
    if (event.pointerType === 'touch') return;
    hovered = null;
    clearHoverTimer();
    if (!inside(card, event.relatedTarget as Element | null)) carded = keyed ?? tapped;
  }

  function leaveCardFocus(event: FocusEvent): void {
    const to = event.relatedTarget as Element | null;
    if (inside(card, to) || to?.closest?.('[data-node]')) return;
    if (hovered === null) carded = tapped;
  }

  function leaveCard(event: PointerEvent): void {
    const to = event.relatedTarget as Element | null;
    if (to?.closest?.('[data-node]')) return;
    if (!inside(card, document.activeElement)) carded = keyed ?? tapped;
  }

  function focusIn(id: string, event: FocusEvent): void {
    keyed = id;
    const target = event.currentTarget as Element;
    if (target.matches(':focus-visible')) carded = id;
  }

  function focusOut(event: FocusEvent): void {
    keyed = null;
    if (!inside(card, event.relatedTarget)) carded = hovered === null ? tapped : carded;
  }

  function open(id: string): void {
    clearHoverTimer();
    carded = null;
    tapped = null;
    onSelect(id);
  }

  function activate(id: string): void {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    if (lastPointer === 'touch' && tapped !== id) {
      tapped = id;
      carded = id;
      return;
    }
    open(id);
  }

  function move(from: string, event: KeyboardEvent): void {
    const order = placed?.order ?? [];
    const ring = order.slice(1);
    const at = ring.indexOf(from);
    let next: string | undefined;
    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        next = ring[at < 0 ? 0 : (at + 1) % ring.length];
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        next = ring[at < 0 ? ring.length - 1 : (at - 1 + ring.length) % ring.length];
        break;
      case 'Home':
        next = order[0];
        break;
      case 'End':
        next = ring[ring.length - 1];
        break;
      case 'Enter':
      case ' ':
        event.preventDefault();
        open(from);
        return;
      case 'Escape':
        if (carded !== null || tapped !== null) {
          event.stopPropagation();
          carded = null;
          tapped = null;
        }
        return;
      default:
        return;
    }
    event.preventDefault();
    if (next) nodeEl(next)?.focus();
  }

  // Drag to pan, once zoomed in. A drag never counts as a click.
  let lastPointer = 'mouse';
  let dragging = $state(false);
  let suppressClick = false;
  let press: { x: number; y: number; pan: { x: number; y: number }; id: number } | null = null;

  function down(event: PointerEvent): void {
    lastPointer = event.pointerType;
    suppressClick = false;
    if (event.button !== 0) return;
    press = { x: event.clientX, y: event.clientY, pan, id: event.pointerId };
  }

  function drag(event: PointerEvent): void {
    if (!press || press.id !== event.pointerId || zoom === 1) return;
    const dx = event.clientX - press.x;
    const dy = event.clientY - press.y;
    if (!dragging && Math.hypot(dx, dy) < SPACE[1]) return;
    if (!dragging) {
      dragging = true;
      svg?.setPointerCapture(event.pointerId);
    }
    pan = clampPan(press.pan.x - dx / zoom, press.pan.y - dy / zoom, zoom);
  }

  function up(event: PointerEvent): void {
    if (dragging) suppressClick = true;
    dragging = false;
    press = null;
    if (svg?.hasPointerCapture(event.pointerId)) svg.releasePointerCapture(event.pointerId);
  }

  function background(event: MouseEvent): void {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    if ((event.target as Element).closest('[data-node]')) return;
    tapped = null;
    carded = null;
  }

  onMount(() => {
    size = {
      node: readPx('map-node'),
      self: readPx('map-self'),
      relay: readPx('map-relay'),
      satellite: readPx('map-satellite'),
      touch: readPx('target-touch'),
    };
    const pointer = window.matchMedia('(pointer: coarse)');
    const small = window.matchMedia('(width < 40rem)');
    const sync = (): void => {
      coarse = pointer.matches;
      narrow = small.matches;
    };
    sync();
    pointer.addEventListener('change', sync);
    small.addEventListener('change', sync);
    void document.fonts?.ready.then(() => (fontsReady += 1));
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      width = Math.round(entry.contentRect.width);
      height = Math.round(entry.contentRect.height);
      pan = clampPan(pan.x, pan.y, zoom);
    });
    if (canvas) observer.observe(canvas);
    return () => {
      observer.disconnect();
      pointer.removeEventListener('change', sync);
      small.removeEventListener('change', sync);
      cancelAnimationFrame(frame);
      clearHoverTimer();
    };
  });

  function radiusOf(device: MapDevice): number {
    return (device.self ? size.self : size.node) / 2;
  }

  function pointOf(id: string): Point | null {
    return shown.get(id) ?? placed?.nodes.get(id) ?? null;
  }

  /** The SVG path of an edge through its relay, from the drawn positions. */
  function edgePath(edge: MapEdge): string {
    const a = pointOf(edge.a);
    const b = pointOf(edge.b);
    if (!a || !b) return '';
    const via = edge.via ? pointOf(edge.via) : null;
    return via
      ? `M${a.x} ${a.y}L${via.x} ${via.y}L${b.x} ${b.y}`
      : `M${a.x} ${a.y}L${b.x} ${b.y}`;
  }

  /** Where an edge's RTT sits: the middle of the leg that touches the focused device. */
  function edgeMid(edge: MapEdge): { x: number; y: number } | null {
    const stops = [edge.a, ...(edge.via ? [edge.via] : []), edge.b];
    const leg = edge.b === focus ? stops.slice(-2) : stops.slice(0, 2);
    const p = pointOf(leg[0] ?? '');
    const q = pointOf(leg[1] ?? '');
    return p && q ? { x: (p.x + q.x) / 2, y: (p.y + q.y) / 2 } : null;
  }

  /** A label beside its node: outward from the centre, or from the gateway for a phone. */
  function labelAt(
    node: Placed,
    device: MapDevice,
  ): { x: number; y: number; anchor: string; baseline: string } {
    const p = pointOf(node.id) ?? node;
    // Clear of the selected and focus rings.
    const r = radiusOf(device) + SPACE[3];
    if (device.self) return { x: 0, y: r, anchor: 'middle', baseline: 'hanging' };
    const g = node.gateway ? pointOf(node.gateway) : null;
    const out = g ? Math.atan2(p.y - g.y, p.x - g.x) : Math.atan2(p.y, p.x);
    // A gateway's phones hang outward, so its own label goes to the side instead.
    const dir = gateways.has(node.id) ? (Math.cos(out) < 0 ? Math.PI : 0) : out;
    const cos = Math.cos(dir);
    const sin = Math.sin(dir);
    // Only a node straight above or below gets a centred label; the rest read sideways,
    // so neighbours near the top and bottom of a ring do not collide.
    const side = Math.abs(cos) > 0.1;
    return {
      x: p.x + cos * r,
      y: p.y + sin * r,
      anchor: side ? (cos > 0 ? 'start' : 'end') : 'middle',
      baseline: side ? 'central' : sin < 0 ? 'auto' : 'hanging',
    };
  }

  const gateways = $derived(
    new Set([...(placed?.nodes.values() ?? [])].flatMap((n) => (n.gateway ? [n.gateway] : []))),
  );

  // The focus card's place: beside the node, inside the canvas.
  let cardW = $state(0);
  let cardH = $state(0);
  const cardAt = $derived.by(() => {
    const p = carded ? pointOf(carded) : null;
    if (!p) return { left: 0, top: 0 };
    const sx = (p.x - pan.x) * zoom + width / 2;
    const sy = (p.y - pan.y) * zoom + height / 2;
    const gap = size.node + SPACE[3];
    const left = sx < width / 2 ? sx + gap : sx - gap - cardW;
    return {
      left: Math.max(SPACE[3], Math.min(width - cardW - SPACE[3], left)),
      top: Math.max(SPACE[3], Math.min(height - cardH - SPACE[3], sy - cardH / 2)),
    };
  });

  const cardFacts = $derived.by(() => {
    const d = cardDevice;
    if (!d || !model || !netId) return null;
    const own = ownLink(model, d.id);
    const heard = report(model, d.id);
    const numbers = live.numbers[`${netId}/${d.id}`];
    const relayName = own?.relay ? (model.devices.get(own.relay)?.name ?? null) : null;
    const status = d.self
      ? copy.device.thisDevice
      : !d.online
        ? copy.status.offline
        : stale
          ? copy.status.stale
          : copy.status.online;
    const path =
      d.online && !d.self ? pathText(own?.path ?? d.peer?.path ?? null, relayName) : null;
    const lastSeen = d.peer?.last_seen_secs;
    const band = numbers?.band ?? null;
    return {
      status: path ? `${status}${DOT}${path}` : status,
      where: [d.address, copy.detail.fqdn(d.name, model.networkName)],
      band: stale ? null : band,
      down: rate(stale || !numbers ? null : numbers.rx),
      up: rate(stale || !numbers ? null : numbers.tx),
      rtt: rtt(stale ? null : (own?.rtt_ms ?? numbers?.rttMs ?? null)),
      endpoint: d.peer?.endpoint ?? null,
      reason: d.peer?.path_reason ?? null,
      seen:
        d.online || d.self
          ? null
          : copy.device.seenAgo(lastSeen == null ? copy.device.notConnectedYet : ago(lastSeen)),
      reported:
        heard && !d.self
          ? copy.path.reportedBy(model.devices.get(heard.from)?.name ?? '', ago(heard.age_secs))
          : null,
    };
  });
</script>

{#snippet focusCard()}
  {#if cardDevice && cardFacts}
    <div
      class="card"
      bind:this={card}
      bind:clientWidth={cardW}
      bind:clientHeight={cardH}
      style:--card-left="{cardAt.left}px"
      style:--card-top="{cardAt.top}px"
      role="group"
      aria-label={cardDevice.name}
      onpointerleave={leaveCard}
      onfocusout={leaveCardFocus}
    >
      <div class="line top">
        <span class="name">{cardDevice.name}</span>
        <span class="meta">{cardFacts.status}</span>
      </div>
      <p class="line mono">
        {#each cardFacts.where.filter(Boolean) as part, i (i)}
          <span class="keep">{i > 0 ? `${DOT}${part}` : part}</span>
        {/each}
      </p>
      {#if cardDevice.online && !cardDevice.self}
        <div class="line top">
          {#if cardFacts.band}
            <span>{copy.rateBand[cardFacts.band]}</span>
          {:else}
            <span class="rates">
              <span><span class="unit">{copy.units.down}</span> {cardFacts.down}</span>
              <span><span class="unit">{copy.units.up}</span> {cardFacts.up}</span>
            </span>
          {/if}
          {#if netId}<Spark peer={`${netId}/${cardDevice.id}`} />{/if}
        </div>
        <div class="line">
          <span>{cardFacts.rtt}</span>
          {#if cardFacts.endpoint}
            <span class="sep">{DOT}</span>
            <span class="mono">{cardFacts.endpoint}</span>
          {/if}
        </div>
        {#if cardFacts.reason}<p class="line reason">{cardFacts.reason}</p>{/if}
      {/if}
      {#if cardFacts.seen}<p class="line meta">{cardFacts.seen}</p>{/if}
      <div class="line top end">
        <span class="meta">{cardFacts.reported ?? ''}</span>
        <Button onclick={() => open(cardDevice.id)}>{copy.map.openDevice}</Button>
      </div>
    </div>
  {/if}
{/snippet}

{#if asTable && model && netId}
  <div class="table-view">
    <p class="note">{copy.map.tooMany(count)}</p>
    <MapTable net={netId} visible {onSelect} />
  </div>
{:else}
  <div class="well">
    <div class="tools">
      <div class="tools-start">
        {#if count < ALL_LINKS_UNDER && count > 1}
          <Switch bind:checked={showAll} label={copy.map.showAllLinks} />
        {/if}
      </div>
      <div class="tools-end">
        <!-- triggerProps: IconButton spreads the tooltip's own onclick over `onclick`. -->
        <IconButton
          icon="plus"
          label={copy.map.zoomIn}
          triggerProps={{ onclick: () => zoomTo(zoom * ZOOM_STEP) }}
        />
        <IconButton
          icon="minus"
          label={copy.map.zoomOut}
          triggerProps={{ onclick: () => zoomTo(zoom / ZOOM_STEP) }}
        />
        <Button variant="ghost" onclick={fit}>{copy.map.fit}</Button>
      </div>
    </div>
    <div class="canvas" class:panning={zoom > 1} class:dragging bind:this={canvas}>
      <!-- Pointer handlers here pan and dismiss; every node is its own keyboard button. -->
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
      <svg
        bind:this={svg}
        class="map"
        role="group"
        aria-label={copy.devices.map}
        viewBox={width > 0 ? viewBox : undefined}
        onpointerdown={down}
        onpointermove={drag}
        onpointerup={up}
        onpointercancel={up}
        onclick={background}
      >
        {#if model && placed}
          <g class="edges" aria-hidden="true">
            {#each drawn as edge (edge.key)}
              <path
                class="edge {edge.kind} {edge.tone}"
                class:dim={!edge.strong}
                d={edgePath(edge)}
                stroke-width={edge.width}
              />
            {/each}
          </g>

          {#each placed.order as id (id)}
            {@const node = placed.nodes.get(id)}
            {@const device = model.devices.get(id)}
            {#if node && device}
              {@const p = pointOf(id) ?? node}
              {@const r = radiusOf(device)}
              {@const label = labelAt(node, device)}
              {@const hit = coarse ? size.touch / 2 : SPACE[3]}
              <g
                class="node"
                class:offline={!device.self && !device.online}
                class:waiting={waiting(device)}
                class:stale={stale && (device.self || device.online)}
                class:selected={id === known(focused)}
                  class:tint={netId !== null && live.changed.has(`${netId}/${id}`)}
                data-node={id}
                role="button"
                tabindex={id === tabStop ? 0 : -1}
                aria-label={nodeName(model, device, stale)}
                aria-current={id === known(focused) ? 'true' : undefined}
                transform="translate({p.x} {p.y})"
                onpointerenter={(e) => enter(id, e)}
                onpointerleave={leave}
                onfocus={(e) => focusIn(id, e)}
                onblur={focusOut}
                onclick={() => activate(id)}
                onkeydown={(e) => move(id, e)}
              >
                <circle class="halo" r={r + SPACE[2]} />
                <circle class="hit" r={hit} />
                {#if node.ring === 'relay'}
                  <rect
                  class="dot"
                  x={-size.relay / 2}
                  y={-size.relay / 2}
                  width={size.relay}
                  height={size.relay}
                />
                {:else}
                  <circle class="dot" {r} />
                {/if}
                <circle class="ring-selected" r={r + SPACE[1]} />
                <circle class="ring-focus" r={r + SPACE[2]} />
              </g>
              <text
                class="label"
                class:offline={!device.self && !device.online}
                class:strong={id === focus || id === known(focused)}
                x={label.x}
                y={label.y}
                text-anchor={label.anchor}
                dominant-baseline={label.baseline}
                aria-hidden="true">{labels.get(id)}</text
              >
            {/if}
          {/each}

          <g class="rtts" aria-hidden="true">
            {#each drawn.filter((e) => e.labelled) as edge (edge.key)}
              {@const mid = edgeMid(edge)}
              {#if mid}
                <text class="rtt" x={mid.x} y={mid.y} text-anchor="middle" dominant-baseline="central"
                  >{rtt(stale ? null : (edge.link?.rtt_ms ?? null))}</text
                >
              {/if}
            {/each}
          </g>
        {/if}
      </svg>

      {#if !narrow}{@render focusCard()}{/if}
    </div>
    {#if narrow}{@render focusCard()}{/if}
  </div>
  {#if model && netId}<MapTable net={netId} />{/if}
{/if}

<style>
  .well {
    display: flex;
    flex-direction: column;
    width: 100%;
    aspect-ratio: 16 / 10;
    overflow: hidden;
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface);
  }
  .canvas {
    position: relative;
    flex: 1;
    min-height: 0;
    touch-action: pan-y pinch-zoom;
  }
  .canvas.panning {
    touch-action: none;
    cursor: grab;
  }
  .canvas.dragging {
    cursor: grabbing;
  }
  .map {
    display: block;
    width: 100%;
    height: 100%;
    font: var(--t-map);
    user-select: none;
  }

  .edge {
    fill: none;
    vector-effect: non-scaling-stroke;
    stroke: var(--ink-2);
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: opacity var(--d-hover) var(--e-linear);
  }
  .edge.relayed {
    stroke-dasharray: 4 3;
    stroke-linecap: butt;
  }
  .edge.gateway {
    stroke-dasharray: 1 2;
    stroke-linecap: butt;
  }
  .edge.attn {
    stroke: var(--attn);
  }
  .edge.danger {
    stroke: var(--danger);
  }
  .edge.dim {
    opacity: 0.25;
  }

  .node {
    cursor: pointer;
    outline: none;
  }
  .hit {
    fill: transparent;
  }
  .dot {
    fill: var(--ink-1);
    stroke: var(--ink-1);
    stroke-width: 0;
  }
  .node.offline .dot,
  .node.stale .dot {
    fill: var(--surface);
    stroke-width: var(--dot-ring);
  }
  .node.offline .dot {
    stroke: var(--ink-3);
  }
  .node.waiting .dot {
    stroke-width: var(--hairline);
    stroke-dasharray: 2 2;
  }
  .halo {
    fill: var(--changed);
    opacity: 0;
  }
  .node.tint .halo {
    animation: map-tint var(--d-tint) var(--e-linear) forwards;
  }
  .ring-selected,
  .ring-focus {
    fill: none;
    opacity: 0;
  }
  .ring-selected {
    stroke: var(--ink-1);
    stroke-width: var(--lead-bar);
  }
  .ring-focus {
    stroke: var(--accent);
    stroke-width: var(--focus-ring);
  }
  .node.selected .ring-selected {
    opacity: 1;
  }
  .node:focus-visible .ring-focus {
    opacity: 1;
  }

  .label {
    fill: var(--ink-2);
    stroke: var(--surface);
    stroke-width: var(--s-1);
    stroke-linejoin: round;
    paint-order: stroke;
    pointer-events: none;
  }
  .label.offline {
    fill: var(--ink-3);
  }
  .label.strong {
    fill: var(--ink-1);
  }
  .rtt {
    fill: var(--ink-2);
    stroke: var(--surface);
    stroke-width: var(--s-1);
    paint-order: stroke;
    pointer-events: none;
  }

  @keyframes map-tint {
    from {
      opacity: 1;
    }
    to {
      opacity: 0;
    }
  }
  @keyframes map-card-in {
    from {
      opacity: 0;
    }
  }

  .tools {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: var(--s-1) var(--s-2);
    padding: var(--s-1) var(--s-3);
  }
  .tools-start {
    white-space: nowrap;
  }
  .tools-end {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    margin-left: auto;
  }
  .card {
    position: absolute;
    left: var(--card-left);
    top: var(--card-top);
    z-index: var(--z-popover);
    display: grid;
    gap: var(--s-1);
    width: min(var(--recent-column), calc(100% - 2 * var(--s-3)));
    padding: var(--s-3);
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface);
    color: var(--ink-1);
    box-shadow: var(--shadow-pop);
    font: var(--t-small);
    animation: map-card-in var(--d-pop-in) var(--e-pop);
  }
  @media (width < 40rem) {
    .well {
      aspect-ratio: auto;
    }
    .canvas {
      flex: none;
      aspect-ratio: 1 / 1;
    }
    .card {
      position: static;
      width: auto;
      margin: 0 var(--s-3) var(--s-3);
      box-shadow: none;
    }
  }
  .line {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    margin: 0;
    min-width: 0;
    white-space: pre-wrap;
  }
  .line.top {
    justify-content: space-between;
    flex-wrap: nowrap;
    gap: var(--s-3);
  }
  .line.end {
    margin-top: var(--s-1);
  }
  .line.end .meta {
    white-space: normal;
    min-width: 0;
  }
  .keep {
    white-space: pre;
  }
  .name {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta,
  .sep,
  .unit {
    color: var(--ink-3);
  }
  .meta {
    white-space: nowrap;
  }
  .rates {
    display: inline-flex;
    gap: var(--s-2);
  }
  .mono {
    font: var(--t-mono);
    letter-spacing: var(--t-mono-tracking);
    overflow-wrap: anywhere;
  }
  .reason {
    color: var(--ink-2);
  }

  .table-view {
    display: grid;
    gap: var(--s-3);
  }
  .note {
    margin: 0;
    color: var(--ink-2);
  }

  @media (forced-colors: active) {
    .edge,
    .edge.attn,
    .edge.danger {
      stroke: CanvasText;
    }
    .dot {
      fill: CanvasText;
      stroke: CanvasText;
    }
    .node.offline .dot,
    .node.stale .dot {
      fill: Canvas;
      stroke: CanvasText;
    }
    .label,
    .rtt {
      fill: CanvasText;
    }
    .ring-focus {
      stroke: Highlight;
    }
  }
</style>
