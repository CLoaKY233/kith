<!--
@component
Command (spec §10): the ⌘K palette primitive. Bits Command inside a modal Dialog near the top
of the screen. Groups show in the order given (Devices · Actions · Go to · Networks). Typing
`>` first limits the list to actions and `@` to devices; `nas:22` finds the device nas and
hands the port to its onSelect. Results rank by recency (picks are remembered in this
browser), then by match. ↑/↓ move, Enter picks, Esc closes. A pick closes the palette, waits
until focus is back on the element that opened it, then runs, so a sheet it opens returns
focus to the right place.
Props:
  - open: boolean, bindable.
  - groups: CommandGroup[], where
      CommandGroup = { heading: string; kind?: 'devices' | 'actions' | 'goTo' | 'networks'; items: CommandItem[] }
      CommandItem = { id: string; label: string; keywords?: string[]; hint?: string;
                      whenTyped?: boolean; onSelect: (detail: { port?: number }) => void }
    `whenTyped` items (per-device actions) stay out of the list until something is typed.
    Item ids are unique across all groups and stable between opens (they key recency).
    `kind` defaults from the heading (copy.palette.groups). Device hints (addresses) are
    mono. Members' admin actions arrive already worded "Ask an admin to rename nas…".
  - onOpenChange?: (open: boolean) => void.
-->
<script lang="ts" module>
  export type CommandItem = {
    id: string;
    label: string;
    keywords?: readonly string[];
    hint?: string;
    whenTyped?: boolean;
    onSelect: (detail: { port?: number }) => void;
  };
  export type CommandKind = 'devices' | 'actions' | 'goTo' | 'networks';
  export type CommandGroup = {
    heading: string;
    kind?: CommandKind;
    items: readonly CommandItem[];
  };
</script>

<script lang="ts">
  import { Command, Dialog } from 'bits-ui';
  import { copy } from '../copy';
  import Icon from '../icons/Icon.svelte';
  import { modalLayer } from './layers.svelte';
  import './overlay.css';
  import { parseQuery, rank, remember } from './rank';

  let {
    open = $bindable(false),
    groups,
    onOpenChange,
  }: {
    open?: boolean;
    groups: readonly CommandGroup[];
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const STORE_KEY = 'kith.palette.recent';
  const RECENT_MAX = 20;

  const layer = modalLayer(() => open);
  let search = $state('');
  let recent = $state<string[]>(load());

  function load(): string[] {
    try {
      const raw: unknown = JSON.parse(localStorage.getItem(STORE_KEY) ?? '[]');
      return Array.isArray(raw) ? raw.filter((id): id is string => typeof id === 'string') : [];
    } catch {
      return [];
    }
  }

  function save(ids: string[]): void {
    recent = ids;
    try {
      localStorage.setItem(STORE_KEY, JSON.stringify(ids));
    } catch {
      // Private windows and blocked storage: recency lasts for this page only.
    }
  }

  const byHeading: Record<string, CommandKind> = {
    [copy.palette.groups.devices]: 'devices',
    [copy.palette.groups.actions]: 'actions',
    [copy.palette.groups.goTo]: 'goTo',
    [copy.palette.groups.networks]: 'networks',
  };

  const query = $derived(parseQuery(search));

  const results = $derived(
    groups
      .map((group) => ({ group, kind: group.kind ?? byHeading[group.heading] }))
      .filter(({ kind }) => query.scope === 'all' || kind === query.scope)
      .filter(({ kind }) => query.port === undefined || kind === 'devices')
      .map(({ group, kind }) => ({
        group,
        kind,
        items: rank(
          query.text === '' ? group.items.filter((item) => !item.whenTyped) : group.items,
          query.text,
          recent,
        ),
      }))
      .filter(({ items }) => items.length > 0),
  );

  $effect(() => {
    if (!open) search = '';
  });

  function pick(item: CommandItem): void {
    const port = query.port;
    save(remember(recent, item.id, RECENT_MAX));
    layer.afterClose(() => item.onSelect(port === undefined ? {} : { port }));
    open = false;
    onOpenChange?.(false);
  }
</script>

<Dialog.Root bind:open {onOpenChange}>
  <Dialog.Portal>
    <Dialog.Overlay class="kith-scrim" />
    <Dialog.Content
      class="kith-dialog kith-palette"
      data-kith-layer={layer.id}
      onCloseAutoFocus={layer.onCloseAutoFocus}
    >
      <Dialog.Title class="sr-only">{copy.palette.label}</Dialog.Title>
      <Command.Root label={copy.palette.label} shouldFilter={false} loop>
        <div class="kith-palette-search">
          <Icon name="search" />
          <Command.Input
            class="kith-palette-input"
            placeholder={copy.palette.placeholder}
            autocapitalize="none"
            autocorrect="off"
            spellcheck={false}
            enterkeyhint="go"
            bind:value={search}
          />
        </div>
        <Command.List class="kith-palette-list">
          {#if results.length === 0}
            <p class="kith-palette-empty">{copy.palette.empty}</p>
          {/if}
          {#each results as { group, kind, items } (group.heading)}
            <Command.Group value={group.heading} class="kith-palette-group">
              <Command.GroupHeading class="kith-palette-heading">{group.heading}</Command.GroupHeading>
              <Command.GroupItems>
                {#each items as item (item.id)}
                  <Command.Item
                    value={item.id}
                    class="kith-palette-item"
                    onSelect={() => pick(item)}
                  >
                    <span class="kith-palette-label">{item.label}</span>
                    {#if item.hint}
                      <span class="kith-palette-hint" class:mono={kind === 'devices'}
                        >{kind === 'devices' && query.port !== undefined
                          ? copy.palette.withPort(item.hint, query.port)
                          : item.hint}</span
                      >
                    {/if}
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
  :global(.kith-dialog.kith-palette) {
    inset: 20dvh 0 auto;
    width: min(var(--palette), calc(100vw - 2 * var(--gutter-mobile)));
    max-height: calc(80dvh - var(--gutter-mobile));
    margin: 0 auto;
    padding: 0;
    overflow: hidden;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
  }
  :global(.kith-palette [data-command-root]) {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    min-height: 0;
  }
  :global(.kith-palette-search) {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: 0 var(--s-4);
    border-bottom: var(--hairline) solid var(--line);
    color: var(--ink-3);
  }
  :global(.kith-palette-input) {
    flex: 1;
    min-width: 0;
    height: var(--s-7);
    background: transparent;
    color: var(--ink-1);
    font: var(--t-input);
    outline: none;
  }
  :global(.kith-palette-input::placeholder) {
    color: var(--ink-3);
  }
  :global(.kith-palette-search:focus-within) {
    box-shadow: inset 0 calc(-1 * var(--focus-ring)) 0 var(--accent);
  }
  :global(.kith-palette-list) {
    max-height: var(--palette-list);
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: var(--s-2);
    scroll-padding-block: var(--s-2);
  }
  :global(.kith-palette-group + .kith-palette-group) {
    margin-top: var(--s-2);
  }
  :global(.kith-palette-heading) {
    padding: var(--s-2) var(--s-3) var(--s-1);
    color: var(--ink-3);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    font-variant-caps: all-small-caps;
  }
  :global(.kith-palette-item) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    min-height: var(--row);
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    color: var(--ink-1);
    cursor: pointer;
    user-select: none;
  }
  :global(.kith-palette-item[data-selected]) {
    background: var(--sunken);
  }
  :global(.kith-palette-label) {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  :global(.kith-palette-hint) {
    flex: none;
    color: var(--ink-3);
    font: var(--t-small);
  }
  :global(.kith-palette-hint.mono) {
    font: var(--t-mono);
  }
  :global(.kith-palette-empty) {
    padding: var(--s-4) var(--s-3);
    color: var(--ink-3);
  }
  @media (width < 40rem) {
    :global(.kith-dialog.kith-palette) {
      inset: calc(var(--gutter-mobile) + env(safe-area-inset-top)) 0 auto;
    }
  }
  @media (forced-colors: active) {
    :global(.kith-palette-item[data-selected]) {
      outline: var(--focus-ring) solid Highlight;
    }
  }
</style>
