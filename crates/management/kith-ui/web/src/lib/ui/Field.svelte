<!--
@component
Field (spec §5 Input / Field): the label above, helper in `--ink-3`, error in `--danger`
under the field, wired with aria-describedby and aria-invalid. Never a placeholder as label.
Validate on blur, then on every keystroke once the field is in error (the screen decides;
Field only shows what it is given).
Props:
  - label: string (from copy.ts).
  - helper?: string. Live helper text, such as the DNS preview.
  - error?: string | null. When set, replaces nothing: shown under the helper.
  - hint?: string | null. A short-lived note such as `Spaces become hyphens.`
  - children: Snippet<[FieldControl]>. Spread `control` on the input:
    FieldControl = { id: string; 'aria-describedby': string | undefined; 'aria-invalid': 'true' | undefined }
-->
<script lang="ts" module>
  export type FieldControl = {
    id: string;
    'aria-describedby': string | undefined;
    'aria-invalid': 'true' | undefined;
  };
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    label,
    helper,
    error = null,
    hint = null,
    children,
  }: {
    label: string;
    helper?: string;
    error?: string | null;
    hint?: string | null;
    children: Snippet<[FieldControl]>;
  } = $props();

  const id = $props.id();
  const control: FieldControl = $derived({
    id,
    'aria-describedby':
      [helper ? `${id}-help` : '', error ? `${id}-err` : '', hint ? `${id}-hint` : '']
        .filter(Boolean)
        .join(' ') || undefined,
    'aria-invalid': error ? 'true' : undefined,
  });
</script>

<div class="field">
  <label for={id}>{label}</label>
  {@render children(control)}
  {#if helper}<p class="helper" id={`${id}-help`}>{helper}</p>{/if}
  {#if hint}<p class="helper" id={`${id}-hint`}>{hint}</p>{/if}
  {#if error}<p class="error" id={`${id}-err`}>{error}</p>{/if}
</div>

<style>
  .field {
    display: grid;
    gap: var(--s-1);
  }
  label {
    font-weight: 600;
  }
  .helper {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .error {
    font: var(--t-small);
    color: var(--danger);
  }
</style>
