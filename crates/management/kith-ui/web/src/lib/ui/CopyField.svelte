<!--
@component
CopyField (spec §5): a mono value on `--sunken` plus `Copy`, which becomes `Copied` with a
check icon for 1.5 s. The button never changes width. The result is announced through the
page's one polite announcer. If the clipboard refuses, the value is selected instead so
⌘C still works.
Props:
  - value: string, the exact text copied.
  - display?: string, the text shown when it differs from what is copied (a shortened
    key). Defaults to `value`.
  - label?: string (from copy.ts), the accessible name of the group, such as "Invite link".
  - labelledby?: string, the id of visible text that names the group, instead of `label`.
  - onCopy?: () => void, after a successful copy.
States: rest, hover, :focus-visible, copied.
-->
<script lang="ts">
  import { announce } from '../announce';
  import { copy } from '../copy';
  import Icon from '../icons/Icon.svelte';
  import { COPIED_MS } from '../live/timing';

  let {
    value,
    display,
    label,
    labelledby,
    onCopy,
  }: {
    value: string;
    display?: string;
    label?: string;
    labelledby?: string;
    onCopy?: () => void;
  } = $props();

  let copied = $state(false);
  let shown: HTMLElement | null = $state(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => () => {
    if (timer !== null) clearTimeout(timer);
  });

  function selectValue(): void {
    if (!shown) return;
    const range = document.createRange();
    range.selectNodeContents(shown);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  }

  async function write(): Promise<void> {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      selectValue();
      return;
    }
    copied = true;
    announce(copy.copyField.copied);
    onCopy?.();
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      copied = false;
    }, COPIED_MS);
  }
</script>

<div
  class="copy-field"
  role={label || labelledby ? 'group' : undefined}
  aria-label={labelledby ? undefined : label}
  aria-labelledby={labelledby}
>
  <code class="value" bind:this={shown}>{display ?? value}</code>
  <button type="button" class="action" onclick={write}>
    <span class="face" class:gone={copied}>{copy.copyField.copy}</span>
    <span class="face" class:gone={!copied}>
      <Icon name="check" />{copy.copyField.copied}
    </span>
  </button>
</div>

<style>
  .copy-field {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    min-height: var(--control-coarse);
    padding: var(--s-1) var(--s-1) var(--s-1) var(--s-3);
    /* Transparent, so forced colours draw it. */
    border: var(--hairline) solid transparent;
    border-radius: var(--r-2);
    background: var(--sunken);
  }
  .value {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    font: var(--t-mono);
    color: var(--ink-1);
    user-select: all;
  }
  .action {
    flex: none;
    display: inline-grid;
    place-items: center;
    height: var(--control);
    padding: 0 var(--s-2);
    border: var(--hairline) solid transparent;
    border-radius: var(--r-2);
    font: var(--t-label);
    color: var(--ink-1);
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .action:hover {
    background: var(--surface);
  }
  .action:active {
    background: var(--line);
  }
  .action:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .face {
    grid-area: 1 / 1;
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  /* Hidden faces leave the accessibility tree too, so the button's name is the face shown. */
  .gone {
    visibility: hidden;
  }
</style>
