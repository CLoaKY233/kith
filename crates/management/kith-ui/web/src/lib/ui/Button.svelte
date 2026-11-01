<!--
@component
Button (spec §5). At most one `primary` per view. No icons beside worded buttons.
Props:
  - variant?: 'primary' | 'secondary' | 'ghost' | 'danger-quiet' | 'danger' (default 'secondary').
    `danger` only inside the remove dialog.
  - pending?: boolean. The label is replaced by a 12 px Spinner after 150 ms, at the same
    min-width, held at least 300 ms once shown. Clicks are ignored while pending.
  - disabled?: boolean. Rendered as `aria-disabled`, never the `disabled` attribute. Only for
    a stale connection (pair it with inline reason text) or a type-to-confirm button; never
    for form validation.
  - type?: 'button' | 'submit' (default 'button').
  - ref?: bindable HTMLButtonElement.
  - children: the label, from copy.ts.
  - Any other <button> attribute (onclick, aria-*, form, ...).
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import type { ButtonVariant } from '../types';
  import { SPINNER_DELAY_MS, SPINNER_MIN_MS } from '../live/timing';
  import Spinner from './Spinner.svelte';

  type Props = Omit<HTMLButtonAttributes, 'disabled' | 'type'> & {
    variant?: ButtonVariant;
    pending?: boolean;
    disabled?: boolean;
    type?: 'button' | 'submit';
    children: Snippet;
    ref?: HTMLButtonElement | null;
  };

  let {
    variant = 'secondary',
    pending = false,
    disabled = false,
    type = 'button',
    children,
    onclick,
    ref = $bindable(null),
    class: className = '',
    ...rest
  }: Props = $props();

  let spinning = $state(false);
  let shownAt = 0;

  $effect(() => {
    if (pending) {
      const timer = setTimeout(() => {
        spinning = true;
        shownAt = performance.now();
      }, SPINNER_DELAY_MS);
      return () => clearTimeout(timer);
    }
    if (spinning) {
      const left = SPINNER_MIN_MS - (performance.now() - shownAt);
      const timer = setTimeout(() => (spinning = false), Math.max(0, left));
      return () => clearTimeout(timer);
    }
    return undefined;
  });

  const inert = $derived(disabled || pending);

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
  aria-disabled={inert ? 'true' : undefined}
  aria-busy={pending ? 'true' : undefined}
  onclick={click}
  {...rest}
>
  <span class="label" class:hidden={spinning}>{@render children()}</span>
  {#if spinning}<span class="spin"><Spinner /></span>{/if}
</button>

<style>
  .btn {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: var(--control);
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    font: var(--t-body);
    font-weight: 600;
    border: var(--hairline) solid transparent;
    white-space: nowrap;
    cursor: pointer;
    transition:
      background-color var(--d-hover) var(--e-linear),
      color var(--d-hover) var(--e-linear);
  }
  .btn:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
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
    color: var(--accent-fg);
  }
  .secondary:hover,
  .ghost:hover,
  .danger-quiet:hover {
    background: var(--sunken);
  }
  .btn[aria-disabled='true'] {
    cursor: default;
  }
  .btn[aria-disabled='true']:not([aria-busy='true']) {
    color: var(--ink-3);
    background: var(--sunken);
    border-color: var(--line);
  }
  .hidden {
    visibility: hidden;
  }
  .spin {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
  }
</style>
