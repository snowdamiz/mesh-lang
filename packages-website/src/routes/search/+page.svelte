<script>
  import Icon from '$lib/Icon.svelte';
  import Notice from '$lib/Notice.svelte';
  import PackageList from '$lib/PackageList.svelte';
  import SearchField from '$lib/SearchField.svelte';

  export let data;
</script>

<svelte:head>
  <title>{data.query ? `“${data.query}” · Search` : 'Search'} · Mesh Packages</title>
  <meta name="description" content={data.query ? `Search results for "${data.query}" on Mesh Packages.` : 'Search the Mesh package registry.'} />
  <meta name="robots" content="noindex" />
</svelte:head>

<div class="reg">
  <div class="reg-head">
    <h1 class="app-title">
      {#if data.query}Results for "{data.query}"{:else}Search packages{/if}
    </h1>
    <SearchField value={data.query} />
  </div>

  {#if data.error}
    <Notice title="The search didn't finish." error>
      {data.error}. Try again in a moment.
      <svelte:fragment slot="actions">
        <a class="btn btn-quiet" href="/search?q={encodeURIComponent(data.query)}" data-sveltekit-reload>Try again</a>
        <a class="btn btn-quiet" href="/">Browse all packages</a>
      </svelte:fragment>
    </Notice>
  {:else if !data.query.trim()}
    <Notice title="Search by name or description.">
      Part of a package name, like <code class="ic">mesh-</code>, or a whole word from its description.
      <a slot="actions" class="link-arrow" href="/">Browse all packages <Icon name="arrow" /></a>
    </Notice>
  {:else}
    <PackageList packages={data.packages} sorts={['relevance', 'updated', 'downloads', 'name']}>
      <svelte:fragment slot="empty">
        <Notice title="Nothing matches “{data.query}”.">
          Names match any part, like <code class="ic">mesh-</code>. Descriptions match whole words, so try a complete word.
          <a slot="actions" class="link-arrow" href="/">Browse all packages <Icon name="arrow" /></a>
        </Notice>
      </svelte:fragment>
    </PackageList>
  {/if}
</div>
