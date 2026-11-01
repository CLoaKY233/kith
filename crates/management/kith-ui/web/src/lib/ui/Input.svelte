<!--
@component
Input (spec §5 Input / Field): text always 16 px so iOS never zooms on focus, `--sunken`
fill, 1 px `--line-strong` border (3:1 against the page), the height of a Button beside it.
Use inside Field and spread its control props on it; on its own it needs an aria-label.
A click anywhere in the box, on the icon or the suffix too, focuses the field.
Props:
  - value: string, bindable.
  - kind?: 'text' | 'name' | 'mono' | 'search' (default 'text').
    name: autocapitalize=none autocorrect=off spellcheck=false enterkeyhint=done, no
    autocomplete · mono: links, addresses, keys · search: a filter field (`/` focuses it).
  - icon?: IconName, a leading 16 px icon (the filter's `search`).
  - suffix?: string, shown after the text inside the border (the `.home` of a name).
  - ref?: bindable HTMLInputElement.
  - class?: applied to the outer box, to size it.
  - Any other <input> attribute (id, type, placeholder, aria-*, onblur, oninput, ...).
States: rest, hover (border `--ink-3`), focus (2 px accent ring, 2 px offset), invalid
(`aria-invalid`: border `--danger`), disabled and readonly. A placeholder is never the label.
-->
<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';
  import Icon from '../icons/Icon.svelte';
  import type { IconName } from '../icons/names';

  type Props = Omit<HTMLInputAttributes, 'value'> & {
    value?: string;
    kind?: 'text' | 'name' | 'mono' | 'search';
    icon?: IconName;
    suffix?: string;
    ref?: HTMLInputElement | null;
  };

  let {
    value = $bindable(''),
    kind = 'text',
    icon,
    suffix,
    ref = $bindable(null),
    class: className = '',
    ...rest
  }: Props = $props();

  const kindAttrs = $derived(
    kind === 'name'
      ? {
          autocapitalize: 'none' as const,
          autocorrect: 'off' as const,
          autocomplete: 'off' as const,
          spellcheck: false,
          enterkeyhint: 'done' as const,
        }
      : kind === 'mono'
        ? {
            autocapitalize: 'none' as const,
            autocorrect: 'off' as const,
            spellcheck: false,
          }
        : kind === 'search'
          ? { autocomplete: 'off' as const, spellcheck: false }
          : {},
  );
</script>

<label class="box {className}">
  {#if icon}<span class="lead"><Icon name={icon} /></span>{/if}
  <input
    bind:this={ref}
    bind:value
    type={kind === 'search' ? 'search' : 'text'}
    class="input"
    class:mono={kind === 'mono'}
    {...kindAttrs}
    {...rest}
  />
  {#if suffix}<span class="suffix" aria-hidden="true">{suffix}</span>{/if}
</label>

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
    cursor: text;
    transition: border-color var(--d-hover) var(--e-linear);
  }
  .box:hover {
    border-color: var(--ink-3);
  }
  .box:focus-within {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .box:has(:global([aria-invalid='true'])) {
    border-color: var(--danger);
  }
  .box:has(:global(:disabled)),
  .box:has(:global([readonly])) {
    border-color: var(--line);
    background: var(--surface);
  }
  .box:has(:global(:disabled)) {
    cursor: default;
  }

  .input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 var(--s-3);
    border: 0;
    background: transparent;
    color: inherit;
    font: var(--t-input);
    outline: none;
  }
  .input:focus-visible {
    outline: none;
  }
  .input.mono {
    font-family: var(--ff-mono);
  }
  .input::placeholder {
    color: var(--ink-2);
    opacity: 1;
  }
  .input:disabled {
    color: var(--ink-3);
  }
  .input[type='search']::-webkit-search-decoration,
  .input[type='search']::-webkit-search-cancel-button {
    appearance: none;
  }

  .lead {
    display: inline-flex;
    padding-left: var(--s-3);
    color: var(--ink-3);
  }
  .lead + .input {
    padding-left: var(--s-2);
  }
  .suffix {
    padding-right: var(--s-3);
    color: var(--ink-2);
    font: var(--t-input);
    white-space: nowrap;
  }
  .input:has(+ .suffix) {
    padding-right: 0;
  }

  @media (forced-colors: active) {
    .box {
      border-color: FieldText;
    }
  }
</style>
