<!--
@component
Command (spec §10): the ⌘K palette. Bits Command inside a Dialog. Groups: Devices ·
Actions · Go to · Networks. `>` limits to actions, `@` to devices. Ranking (recency, then
fuzzy) is the caller's: items arrive in the order to show.
Props:
  - open: boolean, bindable.
  - groups: CommandGroup[], where CommandGroup = { heading: string; items: CommandItem[] }
    and CommandItem = { id: string; label: string; keywords?: string[]; hint?: string; onSelect: () => void }.
-->
<script lang="ts" module>
  export type CommandItem = {
    id: string;
    label: string;
    keywords?: string[];
    hint?: string;
    onSelect: () => void;
  };
  export type CommandGroup = { heading: string; items: readonly CommandItem[] };
</script>

<script lang="ts">
  import { Command, Dialog } from 'bits-ui';
  import { copy } from '../copy';

  let { open = $bindable(false), groups }: { open?: boolean; groups: readonly CommandGroup[] } =
    $props();

  let search = $state('');

  const shown = $derived.by(() => {
    if (search.startsWith('>')) return groups.filter((g) => g.heading === copy.palette.groups.actions);
    if (search.startsWith('@')) return groups.filter((g) => g.heading === copy.palette.groups.devices);
    return groups;
  });

  function pick(item: CommandItem): void {
    open = false;
    search = '';
    item.onSelect();
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Portal>
    <Dialog.Overlay class="kith-scrim" />
    <Dialog.Content class="kith-dialog kith-palette" aria-label={copy.palette.label}>
      <Command.Root label={copy.palette.label} shouldFilter>
        <Command.Input
          class="kith-palette-input"
          placeholder={copy.palette.placeholder}
          bind:value={search}
        />
        <Command.List class="kith-palette-list">
          <Command.Empty class="kith-palette-empty">{copy.palette.empty}</Command.Empty>
          {#each shown as group (group.heading)}
            <Command.Group>
              <Command.GroupHeading class="t-micro kith-palette-heading">{group.heading}</Command.GroupHeading>
              <Command.GroupItems>
                {#each group.items as item (item.id)}
                  <Command.Item
                    value={item.id}
                    keywords={[item.label, ...(item.keywords ?? [])]}
                    class="kith-palette-item"
                    onSelect={() => pick(item)}
                  >
                    <span>{item.label}</span>
                    {#if item.hint}<span class="kith-palette-hint">{item.hint}</span>{/if}
                  </Command.Item>
                {/each}
              </Command.GroupItems>
            </Command.Group>
          {/each}
        </Command.List>
      </Command.Root>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  :global(.kith-palette) {
    top: 20%;
    transform: translateX(-50%);
    width: min(calc(var(--dialog) + var(--s-8) * 2), calc(100vw - 2 * var(--gutter-mobile)));
    padding: var(--s-2);
  }
  :global(.kith-palette-input) {
    width: 100%;
    height: var(--control-coarse);
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    background: var(--sunken);
    font: var(--t-input);
  }
  :global(.kith-palette-list) {
    max-height: 50vh;
    overflow-y: auto;
  }
  :global(.kith-palette-heading) {
    padding: var(--s-2) var(--s-3) var(--s-1);
    color: var(--ink-3);
  }
  :global(.kith-palette-item) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    height: var(--control-coarse);
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    cursor: pointer;
  }
  :global(.kith-palette-item[data-selected]) {
    background: var(--sunken);
  }
  :global(.kith-palette-hint) {
    font: var(--t-mono);
    color: var(--ink-3);
  }
  :global(.kith-palette-empty) {
    padding: var(--s-4) var(--s-3);
    color: var(--ink-3);
  }
</style>
