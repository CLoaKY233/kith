<!--
@component
DropdownMenu (spec §5, §6.3, §10): the ⋯ menu on rows and the `Sort: Name ▾` menu. Bits
DropdownMenu on the floating surface with `--shadow-pop`, fading in 140 ms and out 100 ms.
Arrow keys move, typeahead jumps by label, Enter or Space picks, Esc closes; focus then
returns to the trigger. Rows are 32 px (44 px on touch). Members never see disabled items:
screens pass the "Ask an admin…" item instead. `disabled` is only for a stale connection.
Inside a modal it stacks above it.
Props:
  - trigger: Snippet<[Record<string, unknown>]>. Spread the props on the trigger (an
    IconButton's `triggerProps`, or a Button).
  - items: MenuItem[]:
      { label: string; onSelect: () => void; tone?: 'danger'; hint?: string; disabled?: boolean }
      an action. `hint` is a short `--ink-3` text on the right, such as a shortcut.
      { separator: true }
      { choice: { value: string; options: { value: string; label: string }[]; onChange: (value: string) => void } }
      a single-choice group; the chosen option carries a check and `aria-checked`.
  - open?: boolean, bindable. Set it to open the menu from code (the `.` key on a row).
  - label?: string, the menu's accessible name when the trigger's text does not say it.
  - onCloseAutoFocus?: (event: Event) => void. Prevent it to send focus somewhere other
    than the trigger, such as back to a grid row.
  - align?: 'start' | 'center' | 'end' (default 'end'); side?: 'top' | 'bottom' (default 'bottom').
-->
<script lang="ts" module>
  export type MenuAction = {
    label: string;
    onSelect: () => void;
    tone?: 'danger';
    hint?: string;
    disabled?: boolean;
  };
  export type MenuChoice = {
    choice: {
      value: string;
      options: readonly { value: string; label: string }[];
      onChange: (value: string) => void;
    };
  };
  export type MenuItem = MenuAction | { separator: true } | MenuChoice;
</script>

<script lang="ts">
  import { DropdownMenu } from 'bits-ui';
  import type { Snippet } from 'svelte';
  import Icon from '../icons/Icon.svelte';
  import { insideModal, MENU_OFFSET } from './layers.svelte';
  import './overlay.css';

  let {
    trigger,
    items,
    open = $bindable(false),
    label,
    align = 'end',
    side = 'bottom',
    onCloseAutoFocus,
  }: {
    trigger: Snippet<[Record<string, unknown>]>;
    items: readonly MenuItem[];
    open?: boolean;
    label?: string;
    align?: 'start' | 'center' | 'end';
    side?: 'top' | 'bottom';
    onCloseAutoFocus?: (event: Event) => void;
  } = $props();

  const modal = insideModal();
</script>

<DropdownMenu.Root bind:open>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      {@render trigger(props)}
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal>
    <DropdownMenu.Content
      class="kith-float kith-menu {modal ? 'in-modal' : ''}"
      {align}
      {side}
      sideOffset={MENU_OFFSET}
      collisionPadding={MENU_OFFSET}
      aria-label={label}
      {onCloseAutoFocus}
    >
      {#each items as item, index (index)}
        {#if 'separator' in item}
          <DropdownMenu.Separator class="kith-menu-sep" />
        {:else if 'choice' in item}
          <DropdownMenu.RadioGroup
            value={item.choice.value}
            onValueChange={(value) => item.choice.onChange(value)}
          >
            {#each item.choice.options as option (option.value)}
              <DropdownMenu.RadioItem value={option.value} class="kith-menu-item">
                {#snippet children({ checked })}
                  <span class="kith-menu-check">
                    {#if checked}<Icon name="check" />{/if}
                  </span>
                  <span class="kith-menu-label">{option.label}</span>
                {/snippet}
              </DropdownMenu.RadioItem>
            {/each}
          </DropdownMenu.RadioGroup>
        {:else}
          <DropdownMenu.Item
            class="kith-menu-item {item.tone ?? ''}"
            disabled={item.disabled}
            textValue={item.label}
            onSelect={item.onSelect}
          >
            <span class="kith-menu-label">{item.label}</span>
            {#if item.hint}<span class="kith-menu-hint">{item.hint}</span>{/if}
          </DropdownMenu.Item>
        {/if}
      {/each}
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>

<style>
  :global(.kith-menu) {
    min-width: var(--menu-min);
    max-width: calc(100vw - 2 * var(--gutter-mobile));
    max-height: var(--bits-floating-available-height);
    overflow-y: auto;
    padding: var(--s-1);
  }
  :global(.kith-menu-item) {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--control);
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    color: var(--ink-1);
    cursor: pointer;
    outline: none;
    user-select: none;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  :global(.kith-menu-item[data-highlighted]) {
    background: var(--sunken);
  }
  :global(.kith-menu-item.danger) {
    color: var(--danger);
  }
  :global(.kith-menu-item[data-disabled]) {
    color: var(--ink-3);
    cursor: default;
  }
  :global(.kith-menu-label) {
    flex: 1;
    white-space: nowrap;
  }
  :global(.kith-menu-hint) {
    color: var(--ink-3);
    font: var(--t-small);
  }
  :global(.kith-menu-check) {
    display: inline-grid;
    place-items: center;
    width: var(--icon);
    height: var(--icon);
  }
  :global(.kith-menu-sep) {
    height: var(--hairline);
    margin: var(--s-1) calc(-1 * var(--s-1));
    background: var(--line);
  }
  @media (pointer: coarse) {
    :global(.kith-menu-item) {
      min-height: var(--target-touch);
    }
  }
  @media (forced-colors: active) {
    :global(.kith-menu-item[data-highlighted]) {
      outline: var(--focus-ring) solid Highlight;
    }
  }
</style>
