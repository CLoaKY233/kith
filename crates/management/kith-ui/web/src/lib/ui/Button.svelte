<!--
@component
Button (spec §5). At most one `primary` per view. No icons beside worded buttons.
Props:
  - variant?: 'primary' | 'secondary' | 'ghost' | 'danger-quiet' | 'danger' (default 'secondary').
    primary: `--accent` fill · secondary: `--surface`, 1 px `--line-strong` border ·
    ghost: transparent · danger-quiet: secondary with `--danger` text ·
    danger: filled `--danger`, only inside the remove dialog.
  - pending?: boolean. After 150 ms a 12 px Spinner replaces the label in place, held at
    least 300 ms once shown; the button keeps its width and position. Clicks are ignored
    and `aria-busy` is set while pending.
  - pendingLabel?: string (from copy.ts), such as `Creating…`. Shown beside the spinner and
    becomes the accessible name while the spinner shows. The button is sized for the wider
    of the two labels, so it never changes width.
  - disabled?: boolean. Rendered as `aria-disabled`, never the `disabled` attribute, so the
    button stays focusable and its reason can be read. Only for a stale connection (point
    `aria-describedby` at the inline reason text) or a type-to-confirm button; never for
    form validation.
  - block?: boolean, full width (the pinned primary action of a mobile flow sheet).
  - type?: 'button' | 'submit' (default 'button').
  - ref?: bindable HTMLButtonElement.
  - children: the label, from copy.ts.
  - Any other <button> attribute (onclick, aria-*, form, ...).
States: rest, hover (fill changes, nothing moves), active, :focus-visible (2 px accent
outline, 2 px offset), aria-disabled, pending. Height 32 px, 40 px under a coarse pointer.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import type { ButtonVariant } from '../types';
  import { pendingGate } from './pending';
  import Spinner from './Spinner.svelte';

  type Props = Omit<HTMLButtonAttributes, 'disabled' | 'type'> & {
    variant?: ButtonVariant;
    pending?: boolean;
    pendingLabel?: string;
    disabled?: boolean;
    block?: boolean;
    type?: 'button' | 'submit';
    children: Snippet;
    ref?: HTMLButtonElement | null;
  };

  let {
    variant = 'secondary',
    pending = false,
    pendingLabel,
    disabled = false,
    block = false,
    type = 'button',
    children,
    onclick,
    ref = $bindable(null),
    class: className = '',
    ...rest
  }: Props = $props();

  let spinning = $state(false);
  const gate = pendingGate((shown) => (spinning = shown));

  $effect(() => gate.set(pending));
  $effect(() => () => gate.dispose());

  const inert = $derived(disabled || pending);
  const speakPending = $derived(spinning && pendingLabel !== undefined);

  function click(event: MouseEvent & { currentTarget: EventTarget & HTMLButtonElement }): void {
    if (inert) {
      event.preventDefault();
      return;
    }
    onclick?.(event);
  }
</script>

<button
  bind:this={ref}
  {type}
  class="btn {variant} {className}"
  class:block
  aria-disabled={inert ? 'true' : undefined}
  aria-busy={pending ? 'true' : undefined}
  data-pending={pending ? '' : undefined}
  onclick={click}
  {...rest}
>
  <span class="face" class:gone={spinning} aria-hidden={speakPending ? 'true' : undefined}>
    {@render children()}
  </span>
  <span class="face busy" class:gone={!spinning} aria-hidden={speakPending ? undefined : 'true'}>
    <Spinner />
    {#if pendingLabel !== undefined}<span>{pendingLabel}</span>{/if}
  </span>
</button>

<style>
  .btn {
    display: inline-grid;
    place-items: center;
    height: var(--control);
    padding: 0 var(--s-3);
    border: var(--hairline) solid transparent;
    border-radius: var(--r-2);
    font: var(--t-label);
    white-space: nowrap;
    cursor: pointer;
    user-select: none;
    -webkit-tap-highlight-color: transparent;
    transition:
      background-color var(--d-hover) var(--e-linear),
      border-color var(--d-hover) var(--e-linear),
      color var(--d-hover) var(--e-linear);
  }
  .btn:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .block {
    display: grid;
    width: 100%;
  }

  /* Both faces share one grid cell, so the button is as wide as the wider face. */
  .face {
    grid-area: 1 / 1;
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
  }
  .gone {
    opacity: 0;
  }

  .primary {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .secondary,
  .danger-quiet {
    background: var(--surface);
    border-color: var(--line-strong);
    color: var(--ink-1);
  }
  .danger-quiet {
    color: var(--danger);
  }
  .ghost {
    background: transparent;
    color: var(--ink-1);
  }
  .danger {
    background: var(--danger);
    color: var(--danger-fg);
  }

  .btn:not([aria-disabled='true']):hover {
    background: var(--sunken);
  }
  .primary:not([aria-disabled='true']):hover {
    background: var(--accent-hover);
  }
  .danger:not([aria-disabled='true']):hover {
    background: var(--danger-hover);
  }
  .secondary:not([aria-disabled='true']):active,
  .ghost:not([aria-disabled='true']):active {
    background: var(--line);
  }
  .danger-quiet:not([aria-disabled='true']):active {
    background: var(--danger-wash);
  }
  .primary:not([aria-disabled='true']):active {
    background: var(--accent-press);
  }
  .danger:not([aria-disabled='true']):active {
    background: var(--danger-press);
  }

  /* Pending keeps its variant's colours; only the label is swapped. */
  .btn[data-pending] {
    cursor: progress;
  }
  .btn[aria-disabled='true']:not([data-pending]) {
    cursor: default;
    color: var(--ink-3);
    background: var(--surface);
    border-color: var(--line);
  }
  .ghost[aria-disabled='true']:not([data-pending]) {
    background: transparent;
    border-color: transparent;
  }

  @media (forced-colors: active) {
    .btn {
      border-color: ButtonText;
    }
    .btn[aria-disabled='true']:not([data-pending]) {
      color: GrayText;
      border-color: GrayText;
    }
  }
</style>
