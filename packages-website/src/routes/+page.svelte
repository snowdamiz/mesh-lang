<script>
  import Icon from '$lib/Icon.svelte';
  import Notice from '$lib/Notice.svelte';
  import PackageList from '$lib/PackageList.svelte';
  import SearchField from '$lib/SearchField.svelte';

  export let data;

  let query = '';
</script>

<svelte:head>
  <title>Packages · Mesh</title>
  <meta name="description" content="Find, install, and publish packages for the Mesh programming language." />
  <meta property="og:title" content="Mesh Packages" />
  <meta property="og:description" content="Find, install, and publish packages for the Mesh programming language." />
  <meta property="og:url" content="https://packages.meshlang.dev" />
  <link rel="canonical" href="https://packages.meshlang.dev" />
</svelte:head>

<div class="reg">
  <div class="reg-head">
    <h1 class="app-title">Packages</h1>
    <SearchField bind:value={query} />
  </div>

  {#if data.error}
    <Notice title="The registry didn't answer." error>
      {data.error}. Try again in a moment.
      <a slot="actions" class="btn btn-quiet" href="/" data-sveltekit-reload>Try again</a>
    </Notice>
  {:else if data.packages.length === 0}
    <Notice title="No packages yet.">
      The first package published to the registry shows up here.
      <a slot="actions" class="link-arrow" href="/publish">Publish a package <Icon name="arrow" /></a>
    </Notice>
  {:else}
    <PackageList packages={data.packages} sorts={['updated', 'downloads', 'name']} filter={query}>
      <svelte:fragment slot="empty">
        <Notice title="No packages match “{query.trim()}”.">
          This list filters names and descriptions as you type.
          <a slot="actions" class="link-arrow" href="/search?q={encodeURIComponent(query.trim())}">Search the registry <Icon name="arrow" /></a>
        </Notice>
      </svelte:fragment>
    </PackageList>
  {/if}

  <section class="publish" aria-labelledby="publish-heading">
    <div>
      <h2 id="publish-heading">Publish a package</h2>
      <p>Your GitHub login must match the owner scope. Published versions are immutable.</p>
      <a class="link-arrow" href="/publish">How publishing works <Icon name="arrow" /></a>
    </div>
    <pre><span class="pr">$ </span>meshpkg login
<span class="pr">$ </span>meshpkg publish</pre>
  </section>
</div>

<style>
  .publish {
    margin-top: 80px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 20px 40px;
    align-items: center;
    padding: 28px 32px;
    border-radius: var(--r-xl);
    background: var(--tint);
  }
  .publish h2 {
    margin: 0 0 4px;
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }
  .publish p {
    font-size: var(--fs-sm);
    color: var(--mute);
  }
  .publish .link-arrow {
    margin-top: 14px;
    font-size: var(--fs-sm);
  }
  .publish pre {
    margin: 0;
    font-size: 13px;
    line-height: 1.8;
    color: var(--ink);
  }
  @media (max-width: 640px) {
    .publish {
      grid-template-columns: 1fr;
      padding: 22px;
    }
  }
</style>
