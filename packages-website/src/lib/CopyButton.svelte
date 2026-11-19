<script>
  import { onDestroy } from 'svelte';
  import { copyFeedback } from './clipboard.js';
  import Icon from './Icon.svelte';

  export let text;
  export let label;

  const ICON = { idle: 'copy', copied: 'check', failed: 'cross' };
  const ANNOUNCEMENT = { idle: '', copied: 'Copied', failed: 'Copy failed' };

  /** @type {import('./clipboard.js').CopyState} */
  let state = 'idle';
  const feedback = copyFeedback((next) => (state = next));
  onDestroy(feedback.stop);
</script>

<button type="button" class="icon-btn" data-state={state} on:click={() => feedback.copy(text)} title={label} aria-label={label}>
  <Icon name={ICON[state]} />
  <span class="sr-only" aria-live="polite">{ANNOUNCEMENT[state]}</span>
</button>
