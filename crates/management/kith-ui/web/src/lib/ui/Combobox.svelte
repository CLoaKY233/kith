<!--
@component
Combobox (spec §5, §6.5): a filterable single choice, such as Activity's device filter.
Bits Combobox on the floating surface. Typing narrows the list by label; arrow keys move,
Enter picks, Esc closes and puts the chosen label back. The field is the height of the
controls beside it (`--control`).
Props:
  - value: string, the chosen option's value.
  - options: readonly { value: string; label: string }[]. The first is the clear option
    (such as `Any device`) and always shows.
  - label: string (from copy.ts), the accessible name.
  - empty: string, shown when nothing matches what was typed.
  - onValueChange: (value: string) => void.
-->
<script lang="ts">
  import { Combobox } from 'bits-ui';
  import Icon from '../icons/Icon.svelte';
  import { MENU_OFFSET } from './layers.svelte';
  import './overlay.css';

  type Option = { value: string; label: string };

  let {
    value,
    options,
    label,
    empty,
    onValueChange,
  }: {
    value: string;
    options: readonly Option[];
    label: string;
    empty: string;
    onValueChange: (value: string) => void;
  } = $props();

  let typed = $state('');
  let open = $state(false);
  let input = $state<HTMLInputElement | null>(null);

  const chosen = $derived(options.find((o) => o.value === value)?.label ?? '');
  const shown = $derived.by(() => {
    const q = typed.trim().toLowerCase();
    if (q === '') return options;
    return options.filter((o, i) => i === 0 || o.label.toLowerCase().includes(q));
  });

  // The field shows the chosen label whenever the person is not typing in it.
  $effect(() => {
    const text = chosen;
    if (input && !open) input.value = text;
  });

  function openChange(next: boolean): void {
    if (next) return;
    typed = '';
    if (input) input.value = chosen;
  }
</script>

<Combobox.Root
  type="single"
  {value}
  items={[...options]}
  allowDeselect={false}
  onValueChange={(next) => next && onValueChange(next)}
  bind:open
  onOpenChange={openChange}
>
  <div class="box">
    <Combobox.Input
      bind:ref={input}
      class="input"
      aria-label={label}
      defaultValue={chosen}
      oninput={(e: Event & { currentTarget: HTMLInputElement }) => (typed = e.currentTarget.value)}
      onfocus={(e: FocusEvent & { currentTarget: HTMLInputElement }) => e.currentTarget.select()}
    />
    <Combobox.Trigger class="trigger" aria-label={label} tabindex={-1}>
      <Icon name="chevron-down" />
    </Combobox.Trigger>
  </div>
  <Combobox.Portal>
    <Combobox.Content
      class="kith-float kith-menu"
      sideOffset={MENU_OFFSET}
      collisionPadding={MENU_OFFSET}
    >
      {#each shown as option (option.value)}
        <Combobox.Item class="kith-menu-item" value={option.value} label={option.label}>
          {#snippet children({ selected })}
            <span class="kith-menu-check">{#if selected}<Icon name="check" />{/if}</span>
            <span class="kith-menu-label">{option.label}</span>
          {/snippet}
        </Combobox.Item>
      {/each}
      {#if shown.length <= 1 && typed.trim() !== ''}
        <p class="none">{empty}</p>
      {/if}
    </Combobox.Content>
  </Combobox.Portal>
</Combobox.Root>

<style>
  .box {
    display: flex;
    align-items: center;
    min-width: 0;
    height: var(--control);
    border: var(--hairline) solid var(--line-strong);
    border-radius: var(--r-2);
    background: var(--sunken);
    color: var(--ink-1);
    transition: border-color var(--d-hover) var(--e-linear);
  }
  .box:hover {
    border-color: var(--ink-3);
  }
  .box:focus-within {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .box :global(.input) {
    flex: 1;
    width: 100%;
    min-width: 0;
    height: 100%;
    padding: 0 0 0 var(--s-3);
    border: 0;
    background: transparent;
    color: inherit;
    font: var(--t-input);
    outline: none;
  }
  .box :global(.trigger) {
    display: inline-grid;
    place-items: center;
    width: var(--control);
    height: 100%;
    border: 0;
    background: transparent;
    color: var(--ink-2);
    cursor: pointer;
  }
  .none {
    padding: var(--s-2);
    color: var(--ink-3);
    font: var(--t-small);
  }
</style>
