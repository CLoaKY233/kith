<!--
@component
Field (spec §5 Input / Field): the label above the control, helper text in `--ink-3`, the
error in `--danger` under them, wired with `aria-describedby` and `aria-invalid`. A
placeholder is never the label. Field only shows what it is given; the screen validates on
blur, then on every keystroke once the field is in error.
Props:
  - label: string (from copy.ts).
  - hideLabel?: boolean. Keeps the label for screen readers only (the inline rename field,
    whose label is the name it replaces).
  - helper?: string. Live helper text, such as the DNS preview.
  - hint?: string | null. A short-lived note such as `Spaces become hyphens.`
  - error?: string | null. Replaces nothing; read with the control through aria-describedby.
  - children: Snippet<[FieldControl]>. Spread `control` on the Input:
    FieldControl = { id: string; 'aria-describedby': string | undefined; 'aria-invalid': 'true' | undefined }
Order under the control: helper, hint, error, so nothing above the error moves when it
appears.
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
    hideLabel = false,
    helper,
    error = null,
    hint = null,
    children,
  }: {
    label: string;
    hideLabel?: boolean;
    helper?: string;
    error?: string | null;
    hint?: string | null;
    children: Snippet<[FieldControl]>;
  } = $props();

  const id = $props.id();
  const control: FieldControl = $derived({
    id,
    'aria-describedby':
      [helper ? `${id}-help` : '', hint ? `${id}-hint` : '', error ? `${id}-err` : '']
        .filter(Boolean)
        .join(' ') || undefined,
    'aria-invalid': error ? 'true' : undefined,
  });
</script>

<div class="field">
  <label for={id} class:hidden-label={hideLabel}>{label}</label>
  {@render children(control)}
  {#if helper}<p class="helper" id={`${id}-help`}>{helper}</p>{/if}
  {#if hint}<p class="helper" id={`${id}-hint`}>{hint}</p>{/if}
  {#if error}<p class="error" id={`${id}-err`}>{error}</p>{/if}
</div>

<style>
  .field {
    display: grid;
    align-content: start;
    gap: var(--s-1);
    min-width: 0;
  }
  label {
    font: var(--t-label);
    color: var(--ink-1);
  }
  label:not(.hidden-label) + :global(*) {
    margin-top: var(--s-1);
  }
  .helper,
  .error {
    font: var(--t-small);
    max-width: var(--measure);
    overflow-wrap: anywhere;
    /* Names like phone-2.home: tabular figures would widen the hyphen. */
    font-variant-numeric: slashed-zero;
  }
  .helper {
    color: var(--ink-3);
  }
  .error {
    color: var(--danger);
  }
  .hidden-label {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
