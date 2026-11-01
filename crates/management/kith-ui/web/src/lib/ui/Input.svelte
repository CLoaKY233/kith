<!--
@component
Input (spec §5): always 16 px text (no iOS zoom), `--sunken` fill, `--line-strong` border.
Use inside Field and spread its control props.
Props:
  - value: string, bindable.
  - kind?: 'text' | 'name' | 'mono' | 'search' (default 'text'). `name` sets
    autocapitalize=none autocorrect=off spellcheck=false enterkeyhint=done; `mono` is for
    links and addresses.
  - suffix?: string, shown after the field (the `.home` of a name).
  - ref?: bindable HTMLInputElement.
  - Any other <input> attribute (id, aria-*, onblur, oninput, onkeydown, ...).
-->
<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  type Props = Omit<HTMLInputAttributes, 'value'> & {
    value?: string;
    kind?: 'text' | 'name' | 'mono' | 'search';
    suffix?: string;
    ref?: HTMLInputElement | null;
  };

  let {
    value = $bindable(''),
    kind = 'text',
    suffix,
    ref = $bindable(null),
    class: className = '',
    ...rest
  }: Props = $props();

  const nameAttrs = $derived(
    kind === 'name'
      ? {
          autocapitalize: 'none' as const,
          autocorrect: 'off' as const,
          spellcheck: false,
          enterkeyhint: 'done' as const,
        }
      : {},
  );
</script>

<div class="wrap" class:has-suffix={suffix}>
  <input
    bind:this={ref}
    bind:value
    type={kind === 'search' ? 'search' : 'text'}
    class="input {kind} {className}"
    {...nameAttrs}
    {...rest}
  />
  {#if suffix}<span class="suffix">{suffix}</span>{/if}
</div>

<style>
  .wrap {
    display: flex;
    align-items: center;
    border-radius: var(--r-2);
    border: var(--hairline) solid var(--line-strong);
    background: var(--sunken);
  }
  .wrap:focus-within {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .input {
    flex: 1;
    min-width: 0;
    height: var(--control-coarse);
    padding: 0 var(--s-3);
    background: transparent;
    color: var(--ink-1);
    font: var(--t-input);
    outline: none;
  }
  .input.mono {
    font-family: var(--ff-mono);
  }
  .input:global([aria-invalid='true']) {
    color: var(--ink-1);
  }
  .wrap:has(:global([aria-invalid='true'])) {
    border-color: var(--danger);
  }
  .suffix {
    padding-right: var(--s-3);
    color: var(--ink-3);
    font: var(--t-input);
  }
</style>
