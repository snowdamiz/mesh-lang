<script>
  import { tick } from 'svelte';
  import Icon from './Icon.svelte';

  export let value = '';
  /** The header's field: the docs header's size, and a plain link to /search on phones. */
  export let compact = false;

  let input;
  let link;
  // Phones show only the link; `reveal` drops the field below the header until focus leaves it.
  let revealed = false;

  export async function reveal() {
    revealed = true;
    await tick();
    // The header can drop this field before the tick settles, as when the route changes.
    input?.focus();
  }

  function onKeydown(event) {
    if (event.key === 'Escape' && revealed) link.focus();
  }
</script>

<form action="/search" method="GET" role="search" class:compact class:revealed>
  <label class="search">
    <Icon name="search" />
    <span class="sr-only">Search packages</span>
    <input
      name="q"
      type="search"
      placeholder="Search packages"
      autocomplete="off"
      spellcheck="false"
      bind:this={input}
      bind:value
      on:blur={() => (revealed = false)}
      on:keydown={onKeydown}
    />
    <kbd aria-hidden="true">/</kbd>
  </label>
</form>
{#if compact}
  <a class="search-link" href="/search" aria-label="Search packages" bind:this={link}><Icon name="search" /></a>
{/if}

<style>
  .search {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 56px;
    padding: 0 16px 0 18px;
    border-radius: var(--r-lg);
    background: var(--paper-hi);
    box-shadow: inset 0 0 0 1px var(--line-2);
    transition: box-shadow 0.15s;
    cursor: text;
  }
  .search:focus-within {
    box-shadow: inset 0 0 0 1.5px var(--ink), 0 0 0 4px var(--accent-wash);
  }
  .search :global(svg) {
    width: 18px;
    height: 18px;
    color: var(--mute);
    flex: none;
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    outline: 0;
    background: transparent;
    font-size: 17px;
    color: var(--ink);
    letter-spacing: -0.01em;
  }
  .search input::placeholder {
    color: var(--mute);
  }
  .search input::-webkit-search-cancel-button {
    display: none;
  }
  .search:focus-within kbd {
    visibility: hidden;
  }

  .compact .search {
    width: 260px;
    height: 36px;
    gap: 10px;
    padding: 0 8px 0 12px;
    border-radius: 10px;
    background: var(--tint);
    box-shadow: none;
  }
  .compact .search:focus-within {
    box-shadow: 0 0 0 2px var(--accent-ink);
  }
  .compact .search :global(svg) {
    width: 15px;
    height: 15px;
  }
  .compact .search input {
    font-size: 14px;
    letter-spacing: 0;
  }
  .compact kbd {
    line-height: 20px;
    background: var(--paper-hi);
  }
  .search-link {
    display: none;
    width: 38px;
    height: 38px;
    border-radius: 50%;
    place-items: center;
    background: var(--tint);
    color: var(--ink);
  }
  .search-link :global(svg) {
    width: 15px;
    height: 15px;
  }

  @media (max-width: 1120px) {
    .compact .search {
      width: 180px;
    }
  }
  @media (max-width: 640px) {
    .search {
      height: 52px;
    }
    .search kbd {
      display: none;
    }
    .compact {
      display: none;
    }
    .compact.revealed {
      display: block;
      position: absolute;
      top: var(--nav-h);
      left: 0;
      right: 0;
      padding: 10px var(--gutter) 12px;
      background: var(--paper);
      border-bottom: 1px solid var(--line);
    }
    .compact.revealed .search {
      width: 100%;
      height: 44px;
    }
    /* 16px keeps iOS from zooming in when the field takes focus. */
    .compact.revealed .search input {
      font-size: 16px;
    }
    .search-link {
      display: inline-grid;
    }
  }
</style>
