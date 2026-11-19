<script>
  import CopyButton from './CopyButton.svelte';

  export let text;
  export let label;
</script>

<div class="cmd">
  <code>{text}</code>
  <CopyButton {text} {label} />
</div>
