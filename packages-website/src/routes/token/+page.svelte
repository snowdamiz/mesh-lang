<script>
  import { onMount } from 'svelte';
  import CopyCommand from '$lib/CopyCommand.svelte';
  import Field from '$lib/Field.svelte';
  import Icon from '$lib/Icon.svelte';
  import Notice from '$lib/Notice.svelte';

  // null until the URL has been read in the browser, so the empty state never flashes before the token.
  let token = null;
  let login = '';

  onMount(() => {
    // The registry puts the token in the fragment, which the browser never
    // sends to a server. The query form still works for an older redirect.
    const fragment = new URLSearchParams(window.location.hash.slice(1));
    const query = new URLSearchParams(window.location.search);
    token = fragment.get('value') || query.get('value') || '';
    login = fragment.get('login') || query.get('login') || '';
    // Remove token from URL so it doesn't linger in browser history
    if (token) {
      history.replaceState({}, '', window.location.pathname);
    }
  });
</script>

<svelte:head>
  <title>Your publish token · Mesh Packages</title>
  <meta name="robots" content="noindex, nofollow" />
</svelte:head>

<div class="page">
  {#if token}
    <header class="page-head">
      <h1 class="app-title">Your publish token</h1>
      <p class="lead">
        {#if login}Signed in as <strong>{login}</strong>.{/if} Copy it now. The registry shows it only once.
      </p>
    </header>

    <div class="token-fields">
      <Field label="Token">
        <CopyCommand text={token} label="Copy token" />
      </Field>
      <Field label="Save it for meshpkg">
        <CopyCommand text="meshpkg login --token {token}" label="Copy login command" />
      </Field>
    </div>

    <p class="note">
      Then run <code class="ic">meshpkg publish</code> in your package directory. If you lose the token, sign in again to
      create a new one.
    </p>
    <div class="page-links">
      <a class="link-arrow" href="/publish">Publishing guide <Icon name="arrow" /></a>
      <a class="link-arrow" href="/">Browse packages <Icon name="arrow" /></a>
    </div>
  {:else if token === ''}
    <Notice level={1} title="No token on this page.">
      Tokens appear here once, right after you sign in from the publishing guide.
      <a slot="actions" class="btn btn-ink" href="/publish">Publishing guide</a>
    </Notice>
  {/if}
</div>

<style>
  .token-fields {
    display: grid;
    gap: 22px;
    max-width: 640px;
  }
  .note {
    max-width: 640px;
    margin-top: 28px;
    padding: 16px 20px;
    border-radius: var(--r);
    background: var(--tint);
    font-size: 14.5px;
    line-height: 1.6;
    color: var(--ink-2);
  }
  .note .ic {
    background: var(--tint-2);
  }
  .page-links {
    margin-top: 36px;
    display: flex;
    flex-wrap: wrap;
    gap: 12px 28px;
  }
</style>
