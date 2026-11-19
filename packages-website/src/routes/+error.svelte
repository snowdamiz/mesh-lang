<script>
  import { page } from '$app/stores';
  import Icon from '$lib/Icon.svelte';
  import Notice from '$lib/Notice.svelte';

  const COPY = {
    address: { title: 'Page not found', heading: 'Nothing lives at this address.' },
    failure: { title: 'Something went wrong', heading: 'This page failed to load.' },
  };

  $: kind = $page.status === 404 ? 'address' : 'failure';
</script>

<svelte:head>
  <title>{COPY[kind].title} · Mesh Packages</title>
  <meta name="robots" content="noindex" />
</svelte:head>

<div class="page">
  <Notice level={1} title={COPY[kind].heading} error={kind === 'failure'}>
    {#if kind === 'address'}
      Packages live at <code class="ic">/packages/owner/name</code>.
    {:else}
      The server answered {$page.status}{$page.error?.message ? `: ${$page.error.message}` : ''}. Try again in a moment.
    {/if}
    <a slot="actions" class="link-arrow" href="/">Browse all packages <Icon name="arrow" /></a>
  </Notice>
</div>
