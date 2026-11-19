<script>
  /** An empty, no-results, not-found, or error state, with its next steps in the `actions` slot. */
  export let title;
  /** 1 when the notice is the whole page, 2 when it sits inside one. */
  export let level = 2;
  export let error = false;
</script>

<div class="notice" class:error>
  <svelte:element this={`h${level}`} class="title" class:app-title={level === 1}>{title}</svelte:element>
  <p><slot /></p>
  <div class="actions"><slot name="actions" /></div>
</div>

<style>
  .notice {
    display: grid;
    justify-items: center;
    gap: 8px;
    padding: 56px 24px 60px;
    text-align: center;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  h2 {
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.015em;
    overflow-wrap: anywhere;
  }
  p {
    max-width: 46ch;
    font-size: var(--fs-sm);
    color: var(--mute);
    text-wrap: pretty;
  }
  .actions {
    margin-top: 14px;
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 10px;
  }
  .error .title::before {
    content: '';
    display: inline-block;
    width: 7px;
    height: 7px;
    margin: 0 10px 2px 0;
    border-radius: 50%;
    background: var(--error);
  }
</style>
