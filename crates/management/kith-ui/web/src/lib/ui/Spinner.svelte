<!--
@component
Spinner (spec §5 Button pending): a 12 px ring. Only inside a pending Button; never a
loading state for a screen (no skeletons or loaders on localhost). Still under reduced motion.
Props:
  - label?: string, for screen readers when the spinner stands alone.
-->
<script lang="ts">
  let { label }: { label?: string } = $props();
</script>

<span
  class="spinner"
  role={label ? 'status' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : 'true'}
></span>

<style>
  .spinner {
    display: inline-block;
    width: var(--spinner);
    height: var(--spinner);
    border-radius: var(--r-full);
    border: var(--dot-ring) solid currentColor;
    border-right-color: transparent;
    animation: spin var(--d-spin) linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(1turn);
    }
  }
</style>
