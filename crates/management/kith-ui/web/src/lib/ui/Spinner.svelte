<!--
@component
Spinner (spec §5 Button pending): a 12 px ring in the current text colour. Only inside a
pending Button; never a loading state for a screen (no skeletons or loaders on localhost,
§14). Under reduced motion it stays still: a ring with a gap.
Props:
  - label?: string (from copy.ts), for screen readers when the spinner stands alone.
    Without it the spinner is decorative.
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
    flex: none;
    width: var(--spinner);
    height: var(--spinner);
    border-radius: var(--r-full);
    border: var(--dot-ring) solid currentColor;
    border-right-color: transparent;
    animation: kith-spin var(--d-spin) var(--e-linear) infinite;
  }
  @keyframes kith-spin {
    to {
      transform: rotate(1turn);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
