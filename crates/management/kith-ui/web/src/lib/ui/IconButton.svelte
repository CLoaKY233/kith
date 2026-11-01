<!--
@component
IconButton (spec §5): 32×32, 44×44 on touch. Always has an accessible name and a Tooltip
with the same text.
Props:
  - icon: IconName
  - label: string (from copy.ts): the aria-label and the tooltip text.
  - pressed?: boolean, for toggles (aria-pressed).
  - triggerProps?: props from a parent primitive (for example a DropdownMenu trigger),
    merged onto the button.
  - Any other <button> attribute (onclick, aria-expanded, ...).
-->
<script lang="ts">
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import Icon from '../icons/Icon.svelte';
  import type { IconName } from '../icons/names';
  import Tooltip from './Tooltip.svelte';

  type Props = Omit<HTMLButtonAttributes, 'aria-label'> & {
    icon: IconName;
    label: string;
    pressed?: boolean;
    triggerProps?: Record<string, unknown>;
  };

  let {
    icon,
    label,
    pressed,
    triggerProps = {},
    class: className = '',
    ...rest
  }: Props = $props();
</script>

<Tooltip text={label}>
  {#snippet trigger(props)}
    <button
      type="button"
      class="icon-btn {className}"
      aria-label={label}
      aria-pressed={pressed}
      {...rest}
      {...props}
      {...triggerProps}
    >
      <Icon name={icon} />
    </button>
  {/snippet}
</Tooltip>

<style>
  .icon-btn {
    display: inline-grid;
    place-items: center;
    width: var(--control);
    height: var(--control);
    border-radius: var(--r-2);
    color: var(--ink-2);
    background: transparent;
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .icon-btn:hover,
  .icon-btn[aria-pressed='true'],
  .icon-btn[aria-expanded='true'] {
    background: var(--sunken);
    color: var(--ink-1);
  }
  @media (pointer: coarse) {
    .icon-btn {
      width: var(--target-touch);
      height: var(--target-touch);
    }
  }
</style>
