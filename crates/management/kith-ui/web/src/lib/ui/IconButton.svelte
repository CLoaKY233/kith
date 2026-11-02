<!--
@component
IconButton (spec §5): 32×32, 44×44 under a coarse pointer. Always has an accessible name
and a Tooltip with the same text (the tooltip is skipped on touch). The icon is never the
only carrier of state: a pressed toggle also sets `aria-pressed`.
Props:
  - icon: IconName.
  - label: string (from copy.ts): the aria-label and the tooltip text.
  - pressed?: boolean, for toggles (aria-pressed).
  - disabled?: boolean. Rendered as `aria-disabled`; clicks are ignored. Same rules as Button.
  - triggerProps?: props from a parent primitive (for example a DropdownMenu or Popover
    trigger), merged onto the button; event handlers are chained, not replaced.
  - ref?: bindable HTMLButtonElement.
  - Any other <button> attribute (onclick, aria-expanded, ...).
States: rest (`--ink-2`), hover (`--sunken`, `--ink-1`), active, pressed or expanded,
:focus-visible, aria-disabled.
-->
<script lang="ts">
  import { mergeProps } from 'bits-ui';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import Icon from '../icons/Icon.svelte';
  import type { IconName } from '../icons/names';
  import Tooltip from './Tooltip.svelte';

  type Props = Omit<HTMLButtonAttributes, 'aria-label' | 'disabled' | 'type'> & {
    icon: IconName;
    label: string;
    pressed?: boolean;
    disabled?: boolean;
    triggerProps?: Record<string, unknown>;
    ref?: HTMLButtonElement | null;
  };

  let {
    icon,
    label,
    pressed,
    disabled = false,
    triggerProps = {},
    onclick,
    ref = $bindable(null),
    class: className = '',
    ...rest
  }: Props = $props();

  function click(event: MouseEvent & { currentTarget: EventTarget & HTMLButtonElement }): void {
    if (disabled) {
      event.preventDefault();
      return;
    }
    onclick?.(event);
  }

  const ACTIVATE = new Set(['Enter', ' ', 'ArrowDown', 'ArrowUp']);

  /**
   * Run first, so a parent primitive's trigger (a menu that opens on pointerdown or Enter)
   * stays shut while disabled: a prevented event stops the handlers merged after it.
   */
  const guards = {
    onpointerdown: (event: PointerEvent): void => {
      if (disabled) event.preventDefault();
    },
    onkeydown: (event: KeyboardEvent): void => {
      if (disabled && ACTIVATE.has(event.key)) event.preventDefault();
    },
  };

  const own = $derived({
    type: 'button' as const,
    class: `icon-btn ${className}`,
    'aria-label': label,
    'aria-pressed': pressed,
    'aria-disabled': disabled ? ('true' as const) : undefined,
    onclick: click,
  });
  /** A caller's tabindex (a row's roving focus) wins over the primitives' trigger props. */
  const tabStop: Record<string, unknown> = $derived(
    rest.tabindex === undefined ? {} : { tabindex: rest.tabindex },
  );
</script>

<Tooltip text={label}>
  {#snippet trigger(props)}
    <button bind:this={ref} {...mergeProps(guards, rest, own, props, triggerProps, tabStop)}>
      <Icon name={icon} />
    </button>
  {/snippet}
</Tooltip>

<style>
  .icon-btn {
    display: inline-grid;
    flex: none;
    place-items: center;
    width: var(--control);
    height: var(--control);
    border: var(--hairline) solid transparent;
    border-radius: var(--r-2);
    color: var(--ink-2);
    background: transparent;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    transition:
      background-color var(--d-hover) var(--e-linear),
      color var(--d-hover) var(--e-linear);
  }
  .icon-btn:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .icon-btn:not([aria-disabled='true']):hover,
  .icon-btn[aria-pressed='true'],
  .icon-btn[aria-expanded='true'] {
    background: var(--sunken);
    color: var(--ink-1);
  }
  .icon-btn:not([aria-disabled='true']):active {
    background: var(--line);
    color: var(--ink-1);
  }
  .icon-btn[aria-disabled='true'] {
    cursor: default;
    color: var(--ink-3);
  }
  @media (pointer: coarse) {
    .icon-btn {
      width: var(--target-touch);
      height: var(--target-touch);
    }
  }
  @media (forced-colors: active) {
    .icon-btn[aria-pressed='true'],
    .icon-btn[aria-expanded='true'] {
      border-color: Highlight;
    }
  }
</style>
