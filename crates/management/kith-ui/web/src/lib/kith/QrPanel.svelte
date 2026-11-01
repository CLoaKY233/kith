<!--
@component
QrPanel (spec §5, §7.4, §7.6): a 240 px QR code on a white plate with a 24 px quiet zone,
in both themes. Hidden behind a click (`Show QR code`) so it never appears during screen
sharing; never press-and-hold (§14). Hides itself after 5 min (QR_AUTO_HIDE_MS) or when
`collapse` turns true (the first handshake).
Props:
  - svg: string | null. The server's SVG (`qr_svg`). With null and no `onReveal`, nothing
    renders: the text was too long for a QR code.
  - revealLabel?: string. The reveal button's text (default `Show QR code`); AddPhone passes
    `Show QR code (contains this phone's private key)`.
  - onReveal?: () => void. Called instead of showing at once, for a reveal that must fetch
    first (tier 3 in AddPhone). The panel shows the code once `svg` arrives.
  - pending?: boolean. The reveal is in flight (the button shows its spinner).
  - collapse?: boolean. When it turns true the code hides, for good.
  - shown?: boolean, bindable. Whether the code is showing.
-->
<script lang="ts">
  import { copy } from '../copy';
  import { QR_AUTO_HIDE_MS } from '../live/timing';
  import Button from '../ui/Button.svelte';

  let {
    svg,
    revealLabel = copy.qr.show,
    onReveal,
    pending = false,
    collapse = false,
    shown = $bindable(false),
  }: {
    svg: string | null;
    revealLabel?: string;
    onReveal?: () => void;
    pending?: boolean;
    collapse?: boolean;
    shown?: boolean;
  } = $props();

  const id = $props.id();
  /** Once collapsed, the reveal button goes too: the phone or device is connected. */
  let collapsed = $state(false);
  let asked = $state(false);

  const src = $derived(svg === null ? null : `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`);
  const visible = $derived(shown && src !== null);

  $effect(() => {
    if (collapse) {
      shown = false;
      collapsed = true;
    }
  });

  // A reveal that fetched shows the code as soon as it arrives.
  $effect(() => {
    if (asked && src !== null) {
      asked = false;
      shown = true;
    }
  });

  $effect(() => {
    if (!visible) return;
    const timer = setTimeout(() => (shown = false), QR_AUTO_HIDE_MS);
    return () => clearTimeout(timer);
  });

  function toggle(): void {
    if (shown) {
      shown = false;
      return;
    }
    if (src === null && onReveal) {
      asked = true;
      onReveal();
      return;
    }
    shown = true;
  }
</script>

{#if !collapsed && (src !== null || onReveal)}
  <div class="qr-panel">
    <Button
      variant="secondary"
      pending={pending && !shown}
      aria-expanded={visible}
      aria-controls={id}
      onclick={toggle}
    >
      {visible ? copy.qr.hide : revealLabel}
    </Button>
    <div {id} class="plate" hidden={!visible}>
      {#if visible && src !== null}
        <img {src} alt={copy.qr.label} />
      {/if}
    </div>
  </div>
{/if}

<style>
  .qr-panel {
    display: grid;
    justify-items: start;
    gap: var(--s-3);
  }
  .plate {
    padding: var(--qr-quiet);
    border-radius: var(--r-3);
    border: var(--hairline) solid var(--line);
    background: var(--qr-plate);
  }
  .plate[hidden] {
    display: none;
  }
  img {
    display: block;
    width: var(--qr);
    height: var(--qr);
  }
</style>
