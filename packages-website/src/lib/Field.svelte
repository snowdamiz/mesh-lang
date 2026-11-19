<script>
  export let label;
</script>

<div class="field">
  <p class="label">{label}</p>
  <slot />
</div>

<style>
  .field {
    display: grid;
    gap: 8px;
    min-width: 0;
  }
  .field > :global(*) {
    min-width: 0;
  }
  .label {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-2);
  }
</style>
