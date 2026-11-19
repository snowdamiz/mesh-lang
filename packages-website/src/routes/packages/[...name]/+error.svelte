<script>
  import { page } from '$app/stores';
  import Icon from '$lib/Icon.svelte';
  import Notice from '$lib/Notice.svelte';
  import SiteError from '../../+error.svelte';

  $: missing = $page.status === 404;
</script>

<svelte:head>
  {#if missing}
    <title>Package not found · Mesh Packages</title>
    <meta name="robots" content="noindex" />
  {/if}
</svelte:head>

{#if missing}
  <div class="page">
    <Notice level={1} title="No package by that name.">
      Registry names look like <code class="ic">owner/package</code>, where the owner is the publisher's GitHub login.
      <a slot="actions" class="link-arrow" href="/">Browse all packages <Icon name="arrow" /></a>
    </Notice>
  </div>
{:else}
  <SiteError />
{/if}
