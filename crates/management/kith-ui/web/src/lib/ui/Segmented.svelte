<!--
@component
Segmented (spec §5): a Bits ToggleGroup in single mode, exposed as a radio group. 32 px
tall (40 px under a coarse pointer), `--sunken` track, the selected segment on `--surface`
with a hairline. One option is always selected: picking the selected one again does
nothing. A disabled segment stays focusable and carries an inline reason under the control.
Keyboard: Tab lands on the selected segment; ←/→ (and ↑/↓, Home, End) move focus and
select the segment reached unless it is disabled; Enter and Space select.
Props:
  - value: string, bindable.
  - options: SegmentOption[] = { value: string; label: string; disabled?: boolean; reason?: string }[]
  - label: string (from copy.ts), the group's accessible name.
  - disabled?: boolean. The whole control as `aria-disabled` (a stale connection): focus
    still moves, nothing changes. Pair it with inline reason text.
  - onValueChange?: (value: string) => void, called only for a real change.
Labels never shift: each segment is as wide as its label in the selected weight.
-->
<script lang="ts" module>
  export type SegmentOption = {
    value: string;
    label: string;
    disabled?: boolean;
    reason?: string;
  };
</script>

<script lang="ts">
  import { mergeProps, ToggleGroup } from 'bits-ui';

  let {
    value = $bindable(''),
    options,
    label,
    disabled = false,
    onValueChange,
  }: {
    value?: string;
    options: readonly SegmentOption[];
    label: string;
    disabled?: boolean;
    onValueChange?: (value: string) => void;
  } = $props();

  const id = $props.id();
  let root: HTMLElement | null = $state(null);

  const reasons = $derived(options.filter((o) => o.disabled && o.reason));
  const tabStop = $derived(
    options.find((o) => o.value === value)?.value ?? options.find((o) => !o.disabled)?.value,
  );
  const reasonId = (index: number): string => `${id}-reason-${index}`;

  function select(next: string): void {
    if (disabled || next === '' || next === value) return;
    const option = options.find((o) => o.value === next);
    if (!option || option.disabled) return;
    value = next;
    onValueChange?.(next);
  }

  const STEP: Record<string, number> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };

  function keydown(event: KeyboardEvent): void {
    const current = options.findIndex(
      (o) => o.value === (event.currentTarget as HTMLElement).dataset['value'],
    );
    const count = options.length;
    let next: number | null = null;
    const step = STEP[event.key];
    if (step !== undefined) next = (current + step + count) % count;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = count - 1;
    const target = next === null ? undefined : options[next];
    if (!target) return;
    event.preventDefault();
    root?.querySelector<HTMLElement>(`[data-value="${CSS.escape(target.value)}"]`)?.focus();
    if (!target.disabled) select(target.value);
  }
</script>

<div class="segmented">
  <div class="scroller">
    <ToggleGroup.Root
      type="single"
      bind:value={() => value, (next: string) => select(next)}
      rovingFocus={false}
    >
      {#snippet child({ props })}
        <div
          bind:this={root}
          {...mergeProps(props, {
            class: 'track',
            role: 'radiogroup',
            'aria-label': label,
            'aria-disabled': disabled ? ('true' as const) : undefined,
          })}
        >
          {#each options as option, index (option.value)}
            <ToggleGroup.Item value={option.value}>
              {#snippet child({ props: itemProps })}
                <button
                  {...mergeProps(itemProps, {
                    class: 'segment',
                    tabindex: option.value === tabStop ? 0 : -1,
                    'aria-disabled': option.disabled ? ('true' as const) : undefined,
                    'aria-describedby': option.disabled && option.reason ? reasonId(index) : undefined,
                    onkeydown: keydown,
                  })}
                >
                  <span class="text" data-label={option.label}>{option.label}</span>
                </button>
              {/snippet}
            </ToggleGroup.Item>
          {/each}
        </div>
      {/snippet}
    </ToggleGroup.Root>
  </div>
  {#each reasons as option (option.value)}
    <p class="reason" id={reasonId(options.indexOf(option))}>{option.reason}</p>
  {/each}
</div>

<style>
  .segmented {
    display: grid;
    justify-items: start;
    gap: var(--s-1);
    min-width: 0;
  }
  /* Room for the focus ring, so a track wider than a phone scrolls without clipping it. */
  .scroller {
    max-width: calc(100% + 2 * var(--s-1));
    margin: calc(-1 * var(--s-1));
    padding: var(--s-1);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .scroller::-webkit-scrollbar {
    display: none;
  }
  .segmented :global(.track) {
    display: inline-flex;
    height: var(--control);
    border-radius: var(--r-2);
    background: var(--sunken);
  }
  .segmented :global(.segment) {
    display: inline-grid;
    place-items: center;
    height: 100%;
    padding: 0 var(--s-3);
    border: var(--hairline) solid transparent;
    border-radius: var(--r-2);
    color: var(--ink-2);
    font: var(--t-body);
    white-space: nowrap;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    transition:
      color var(--d-hover) var(--e-linear),
      background-color var(--d-hover) var(--e-linear),
      border-color var(--d-hover) var(--e-linear);
  }
  .segmented :global(.segment:focus-visible) {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .segmented :global(.segment:hover) {
    color: var(--ink-1);
  }
  .segmented :global(.segment[data-state='on']) {
    background: var(--surface);
    border-color: var(--line);
    color: var(--ink-1);
    font-weight: 600;
  }
  .segmented :global(.segment[aria-disabled='true']),
  .segmented :global(.track[aria-disabled='true'] .segment[data-state='off']) {
    color: var(--ink-3);
    cursor: default;
  }
  .segmented :global(.track[aria-disabled='true'] .segment) {
    cursor: default;
  }
  /* The selected weight's width is reserved by an invisible copy of the label. */
  .text {
    display: inline-grid;
  }
  .text::after {
    content: attr(data-label);
    height: 0;
    overflow: hidden;
    visibility: hidden;
    font-weight: 600;
  }
  .reason {
    font: var(--t-small);
    color: var(--ink-3);
    max-width: var(--measure);
  }
  @media (forced-colors: active) {
    .segmented :global(.segment[data-state='on']) {
      border-color: Highlight;
    }
  }
</style>
