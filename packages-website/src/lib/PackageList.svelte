<script>
  import Icon from './Icon.svelte';
  import PackageRow from './PackageRow.svelte';
  import { SORTS, matches, sortPackages } from './packages.js';

  /** @type {import('./packages.js').PackageSummary[]} */
  export let packages;
  /** @type {(keyof typeof SORTS)[]} */
  export let sorts;
  export let filter = '';

  let sort = sorts[0];

  $: shown = sortPackages(packages.filter((pkg) => matches(pkg, filter)), sort);
</script>

<div class="list-bar">
  <span aria-live="polite">{shown.length} {shown.length === 1 ? 'package' : 'packages'}</span>
  {#if packages.length > 1}
    <label class="select-wrap">
      <span class="sr-only">Sort by</span>
      <select class="select" bind:value={sort}>
        {#each sorts as key}
          <option value={key}>{SORTS[key].label}</option>
        {/each}
      </select>
      <Icon name="chevron" />
    </label>
  {/if}
</div>

{#if shown.length}
  <ul class="pkg-list">
    {#each shown as pkg (pkg.name)}
      <PackageRow {pkg} />
    {/each}
  </ul>
{:else}
  <slot name="empty" />
{/if}

<style>
  .list-bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    min-height: 44px;
    padding: 0 0 12px;
    font-size: var(--fs-sm);
    color: var(--mute);
  }
  .select-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
  }
  .select-wrap :global(svg) {
    position: absolute;
    right: 9px;
    width: 14px;
    height: 14px;
    pointer-events: none;
    color: var(--mute);
  }
  .select {
    appearance: none;
    -webkit-appearance: none;
    height: 32px;
    padding: 0 30px 0 12px;
    border-radius: var(--r-sm);
    border: 0;
    background: var(--tint);
    font-size: 13.5px;
    font-weight: 500;
    color: var(--ink);
    cursor: pointer;
  }
  .select:hover {
    background: var(--tint-2);
  }
  .pkg-list {
    list-style: none;
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--line);
  }
</style>
