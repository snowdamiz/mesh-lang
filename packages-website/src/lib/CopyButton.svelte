<script>
  import { Copy, Check } from 'lucide-svelte';

  export let text;
  export let label;

  let copied = false;
  let timer;

  async function copy() {
    await navigator.clipboard.writeText(text);
    copied = true;
    clearTimeout(timer);
    timer = setTimeout(() => (copied = false), 1600);
  }
</script>

<!-- z-10 keeps it clickable above a row's stretched link -->
<button
  type="button"
  on:click={copy}
  title={label}
  aria-label={label}
  class="relative z-10 inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-1.5 font-mono text-[11px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand/50"
>
  {#if copied}
    <Check class="size-3.5 text-brand" />
  {:else}
    <Copy class="size-3.5" />
  {/if}
  <slot />
  <span class="sr-only" aria-live="polite">{copied ? 'Copied' : ''}</span>
</button>
