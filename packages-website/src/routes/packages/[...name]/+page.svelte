<script>
  import { onMount, tick } from 'svelte';
  import { page } from '$app/stores';
  import { copyCodeBlocks } from '$lib/clipboard.js';
  import CopyButton from '$lib/CopyButton.svelte';
  import CopyCommand from '$lib/CopyCommand.svelte';
  import Field from '$lib/Field.svelte';
  import Notice from '$lib/Notice.svelte';
  import { dependencyLine, formatDate, installCommand } from '$lib/format.js';
  import { splitName } from '$lib/packages.js';

  export let data;

  const KEY_TO_TAB = {
    ArrowRight: (i, n) => (i + 1) % n,
    ArrowDown: (i, n) => (i + 1) % n,
    ArrowLeft: (i, n) => (i + n - 1) % n,
    ArrowUp: (i, n) => (i + n - 1) % n,
    Home: () => 0,
    End: (i, n) => n - 1,
  };

  let tab = 'readme';
  // Without JavaScript the tabs cannot switch, so both panels stay visible until the page hydrates.
  let hydrated = false;
  onMount(() => (hydrated = true));

  /** @type {import('$lib/packages.js').PackageDetail | null} */
  $: pkg = data.pkg;
  /** @type {import('$lib/packages.js').PackageVersion[]} */
  $: versions = data.versions;
  $: tabs = [
    { id: 'readme', label: 'Readme' },
    { id: 'versions', label: 'Versions', count: versions.length },
  ];
  $: names = pkg && splitName(pkg);
  $: latestVersion = pkg?.latest?.version;
  $: latest = versions.find((ver) => ver.version === latestVersion);
  $: repository = /^https?:\/\//i.test(pkg?.repository ?? '') ? pkg.repository : null;

  async function moveTab(event) {
    const move = KEY_TO_TAB[event.key];
    if (!move) return;
    event.preventDefault();
    tab = tabs[move(tabs.findIndex((t) => t.id === tab), tabs.length)].id;
    await tick();
    document.getElementById(`tab-${tab}`)?.focus();
  }
</script>

<svelte:head>
  {#if data.pkg}
    <title>{data.pkg.name} · Mesh Packages</title>
    <meta name="description" content={data.pkg.description || `${data.pkg.name}, a Mesh package.`} />
    <meta property="og:title" content="{data.pkg.name} · Mesh Packages" />
    <meta property="og:description" content={data.pkg.description || `Install with meshpkg install ${data.pkg.name}`} />
    <meta property="og:url" content="https://packages.meshlang.dev/packages/{data.pkg.name}" />
    <link rel="canonical" href="https://packages.meshlang.dev/packages/{data.pkg.name}" />
  {:else}
    <title>Package · Mesh Packages</title>
  {/if}
</svelte:head>

{#if data.error}
  <div class="page">
    <Notice level={1} title="This package didn't load." error>
      {data.error}. The registry may be busy; try again in a moment.
      <svelte:fragment slot="actions">
        <a class="btn btn-quiet" href={$page.url.pathname} data-sveltekit-reload>Try again</a>
        <a class="btn btn-quiet" href="/">Browse all packages</a>
      </svelte:fragment>
    </Notice>
  </div>
{:else if pkg}
  <div class="pkg">
    <header class="pkg-header">
      <div class="pkg-title">
        <h1><span class="owner">{names.owner}/</span>{names.slug}</h1>
        {#if latestVersion}<span class="ver-pill">{latestVersion}</span>{/if}
      </div>
      <p class="lead">{pkg.description || 'No description provided.'}</p>
    </header>

    <div class="tabs" role="tablist" aria-label="Package">
      {#each tabs as t (t.id)}
        <button
          type="button"
          role="tab"
          id="tab-{t.id}"
          aria-controls="panel-{t.id}"
          aria-selected={tab === t.id}
          tabindex={tab === t.id ? 0 : -1}
          on:click={() => (tab = t.id)}
          on:keydown={moveTab}
        >
          {t.label}
          {#if t.count != null}<span class="count">{t.count}</span>{/if}
        </button>
      {/each}
    </div>

    <div class="pkg-layout">
      <div class="pkg-main">
        <div role="tabpanel" id="panel-readme" aria-labelledby="tab-readme" tabindex="0" hidden={hydrated && tab !== 'readme'}>
          {#if data.readmeHtml}
            <article class="readme prose" use:copyCodeBlocks={data.readmeHtml}>{@html data.readmeHtml}</article>
          {:else}
            <div class="no-readme">
              <h2>No README yet.</h2>
              <p><code class="ic">meshpkg publish</code> uploads <code class="ic">mesh.toml</code> and the package's <code class="ic">.mpl</code> sources, not a README.</p>
            </div>
          {/if}
        </div>

        <div role="tabpanel" id="panel-versions" aria-labelledby="tab-versions" tabindex="0" hidden={hydrated && tab !== 'versions'}>
          {#if versions.length}
            <ol class="versions">
              {#each versions as ver (ver.version)}
                <li>
                  <code>{ver.version}</code>
                  {#if ver.version === latestVersion}<span class="badge">Latest</span>{:else}<span></span>{/if}
                  <time datetime={ver.published_at}>{formatDate(ver.published_at)}</time>
                  <CopyButton text={dependencyLine(pkg.name, ver.version)} label="Copy the mesh.toml line for {ver.version}" />
                </li>
              {/each}
            </ol>
          {:else}
            <div class="no-readme">
              <h2>The version list didn't load.</h2>
              <p>The latest version is {latestVersion}. Reload the page to fetch the rest.</p>
            </div>
          {/if}
        </div>
      </div>

      <aside class="meta-col" aria-label="Package details">
        <Field label="Install">
          <CopyCommand text={installCommand(pkg.name)} label="Copy install command" />
        </Field>
        {#if latestVersion}
          <Field label="mesh.toml">
            <CopyCommand text={dependencyLine(pkg.name, latestVersion)} label="Copy mesh.toml line" />
            <p class="field-hint">Goes under <code class="ic">[dependencies]</code>. Versions are exact.</p>
          </Field>
        {/if}
        <dl class="kv">
          {#if latestVersion}<div><dt>Latest version</dt><dd>{latestVersion}</dd></div>{/if}
          {#if pkg.license}<div><dt>License</dt><dd>{pkg.license}</dd></div>{/if}
          {#if pkg.repository}
            <div>
              <dt>Repository</dt>
              <dd>
                {#if repository}<a href={repository} rel="noopener">{repository.replace(/^https?:\/\/(www\.)?(github\.com\/)?/i, '')}</a>{:else}{pkg.repository}{/if}
              </dd>
            </div>
          {/if}
          {#if latest}<div><dt>Published</dt><dd><time datetime={latest.published_at}>{formatDate(latest.published_at)}</time></dd></div>{/if}
          <div><dt>Downloads</dt><dd>{pkg.download_count.toLocaleString('en-US')}</dd></div>
        </dl>
      </aside>
    </div>
  </div>
{/if}

<style>
  .pkg {
    width: min(1120px, calc(100% - 2 * var(--gutter)));
    margin-inline: auto;
    padding-top: 48px;
  }
  .pkg-header {
    display: grid;
    gap: 12px;
    margin-bottom: 28px;
  }
  .pkg-title {
    display: flex;
    align-items: center;
    gap: 10px 14px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .pkg-header h1 {
    font-size: clamp(30px, 3.4vw, 42px);
    line-height: 1.1;
    letter-spacing: -0.045em;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .pkg-header h1 .owner {
    font-weight: 500;
    color: var(--mute);
  }
  .ver-pill {
    display: inline-flex;
    align-items: center;
    min-height: 24px;
    max-width: 100%;
    padding: 2px 10px;
    border-radius: var(--pill);
    background: var(--tint);
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 1.4;
    color: var(--ink-2);
    overflow-wrap: anywhere;
  }
  .pkg-header .lead {
    max-width: 62ch;
    font-size: 17.5px;
  }
  .tabs {
    display: flex;
    gap: 28px;
    border-bottom: 1px solid var(--line);
    margin-bottom: 40px;
  }
  .tabs [role='tab'] {
    position: relative;
    height: 44px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 14.5px;
    color: var(--mute);
    transition: color 0.15s;
  }
  .tabs [role='tab']:hover {
    color: var(--ink);
  }
  .tabs [role='tab']:focus-visible {
    outline-offset: -2px;
  }
  .tabs [role='tab'][aria-selected='true'] {
    color: var(--ink);
    font-weight: 500;
  }
  .tabs [role='tab'][aria-selected='true']::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    bottom: -1px;
    height: 2px;
    border-radius: 2px;
    background: var(--ink);
  }
  .tabs .count {
    font-size: 12.5px;
    color: var(--mute);
    font-weight: 400;
  }
  .pkg-layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 336px;
    gap: clamp(40px, 6vw, 88px);
    align-items: start;
  }
  .pkg-main {
    min-width: 0;
  }
  .pkg-main > :not([hidden]) + :not([hidden]) {
    margin-top: 48px;
  }
  .readme {
    max-width: 720px;
  }
  .no-readme {
    max-width: 720px;
    padding: 28px 32px;
    border-radius: var(--r-xl);
    background: var(--tint);
    font-size: var(--fs-sm);
    color: var(--mute);
  }
  .no-readme h2 {
    margin-bottom: 4px;
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.015em;
    color: var(--ink);
  }
  .no-readme .ic {
    background: var(--tint-2);
  }
  .versions {
    list-style: none;
    margin: 0;
    padding: 0;
    max-width: 720px;
    display: grid;
    grid-template-columns: minmax(96px, max-content) 1fr auto auto;
    column-gap: 16px;
    border-top: 1px solid var(--line);
  }
  .versions li {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: subgrid;
    align-items: center;
    padding: 7px 0;
    min-height: 50px;
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-sm);
  }
  .versions code {
    font-size: 13.5px;
    color: var(--ink);
    overflow-wrap: anywhere;
  }
  .versions time {
    color: var(--mute);
    font-size: 13.5px;
    white-space: nowrap;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .badge {
    justify-self: start;
    font-size: 12px;
    font-weight: 500;
    color: var(--accent-ink);
    background: var(--accent-wash);
    padding: 2px 8px;
    border-radius: var(--pill);
  }
  .meta-col {
    position: sticky;
    top: calc(var(--nav-h) + 24px);
    display: grid;
    gap: 22px;
    min-width: 0;
  }
  .field-hint {
    font-size: var(--fs-xs);
    color: var(--mute);
  }
  .kv {
    margin: 8px 0 0;
    display: grid;
  }
  .kv div {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
    padding: 11px 0;
    border-top: 1px solid var(--line);
    font-size: var(--fs-sm);
  }
  .kv dt {
    color: var(--mute);
    flex: none;
  }
  .kv dd {
    margin: 0;
    color: var(--ink);
    text-align: right;
    min-width: 0;
    overflow-wrap: anywhere;
    font-variant-numeric: tabular-nums;
  }
  .kv a {
    text-decoration: underline;
    text-decoration-color: var(--line-2);
    text-underline-offset: 3px;
  }
  .kv a:hover {
    text-decoration-color: var(--accent);
  }

  /* The README is server-rendered HTML, so its rules reach inside with :global. */
  .prose {
    font-size: 16.5px;
    line-height: 1.75;
    color: var(--ink-2);
    overflow-wrap: break-word;
  }
  .prose > :global(:first-child) {
    margin-top: 0;
  }
  .prose > :global(:last-child) {
    margin-bottom: 0;
  }
  .prose :global(h1),
  .prose :global(h2),
  .prose :global(h3),
  .prose :global(h4),
  .prose :global(h5),
  .prose :global(h6) {
    color: var(--ink);
    font-weight: 600;
    text-wrap: balance;
  }
  .prose :global(h1) {
    font-size: 26px;
    line-height: 1.15;
    letter-spacing: -0.035em;
    margin: 48px 0 16px;
  }
  .prose :global(h2) {
    font-size: 22px;
    line-height: 1.2;
    letter-spacing: -0.03em;
    margin: 48px 0 12px;
  }
  .prose :global(h3) {
    font-size: 18px;
    line-height: 1.3;
    letter-spacing: -0.02em;
    margin: 36px 0 8px;
  }
  .prose :global(h4),
  .prose :global(h5),
  .prose :global(h6) {
    font-size: 16px;
    letter-spacing: -0.01em;
    margin: 28px 0 6px;
  }
  .prose :global(p) {
    margin: 0 0 18px;
  }
  .prose :global(ul),
  .prose :global(ol) {
    margin: 0 0 22px;
    display: grid;
    gap: 8px;
  }
  .prose :global(ul) {
    padding-left: 0;
    list-style: none;
  }
  .prose :global(ul > li) {
    position: relative;
    padding-left: 20px;
  }
  .prose :global(ul > li::before) {
    content: '';
    position: absolute;
    left: 4px;
    top: 0.78em;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--faint);
  }
  .prose :global(ul > li:has(> input[type='checkbox'])::before) {
    display: none;
  }
  .prose :global(ol) {
    padding-left: 1.5em;
  }
  .prose :global(ol > li) {
    padding-left: 4px;
  }
  .prose :global(ol > li::marker) {
    color: var(--mute);
    font-variant-numeric: tabular-nums;
  }
  .prose :global(li > ul),
  .prose :global(li > ol) {
    margin: 8px 0 0;
  }
  .prose :global(li > p) {
    margin-bottom: 8px;
  }
  .prose :global(input[type='checkbox']) {
    margin: 0 8px 0 0;
    accent-color: var(--accent);
  }
  .prose :global(strong) {
    color: var(--ink);
    font-weight: 600;
  }
  .prose :global(a) {
    color: var(--ink);
    text-decoration: underline;
    text-decoration-color: var(--accent);
    text-underline-offset: 3px;
    text-decoration-thickness: 1.5px;
  }
  .prose :global(a:hover) {
    color: var(--accent-ink);
  }
  .prose :global(:not(pre) > code) {
    font-size: 0.84em;
    background: var(--tint);
    border-radius: 5px;
    padding: 0.12em 0.38em;
    color: var(--ink);
  }
  .prose :global(blockquote) {
    margin: 0 0 22px;
    padding: 2px 0 2px 18px;
    border-left: 2px solid var(--line-2);
    color: var(--mute);
  }
  .prose :global(blockquote > :last-child) {
    margin-bottom: 0;
  }
  .prose :global(hr) {
    margin: 40px 0;
    height: 1px;
    border: 0;
    background: var(--line);
  }
  .prose :global(img) {
    display: inline-block;
    max-width: 100%;
    height: auto;
    border-radius: var(--r-sm);
    vertical-align: middle;
  }
  .prose :global(p > img:only-child) {
    display: block;
    border-radius: var(--r);
  }
  .prose :global(.table-wrap) {
    overflow-x: auto;
    margin: 0 0 28px;
  }
  .prose :global(table) {
    width: 100%;
    border-collapse: collapse;
    font-size: 14.5px;
  }
  .prose :global(th) {
    text-align: left;
    font-weight: 500;
    font-size: 13px;
    color: var(--mute);
    padding: 0 20px 10px 0;
    border-bottom: 1px solid var(--line-2);
    white-space: nowrap;
  }
  .prose :global(td) {
    padding: 12px 20px 12px 0;
    border-bottom: 1px solid var(--line);
    vertical-align: top;
    line-height: 1.55;
  }
  .prose :global(th:last-child),
  .prose :global(td:last-child) {
    padding-right: 0;
  }
  .prose :global(.codeblock-copy) {
    position: absolute;
    top: 8px;
    right: 8px;
    color: var(--night-mute);
    background: var(--night);
    opacity: 0;
    transition: opacity 0.15s, color 0.15s, background 0.15s;
  }
  .prose :global(.codeblock-copy:hover) {
    color: var(--night-text);
    background: var(--night-line);
  }
  .prose :global(.codeblock:hover .codeblock-copy),
  .prose :global(.codeblock-copy:focus-visible),
  .prose :global(.codeblock-copy:not([data-state='idle'])) {
    opacity: 1;
  }
  .prose :global(.codeblock-copy[data-state='copied']) {
    color: var(--accent-hi);
  }
  .prose :global(.codeblock-copy[data-state='failed']) {
    color: var(--night-error);
  }
  .prose :global(.codeblock-copy svg) {
    display: none;
  }
  .prose :global(.codeblock-copy[data-state='idle'] .when-idle),
  .prose :global(.codeblock-copy[data-state='copied'] .when-copied),
  .prose :global(.codeblock-copy[data-state='failed'] .when-failed) {
    display: block;
  }

  @media (max-width: 980px) {
    .pkg-layout {
      grid-template-columns: minmax(0, 1fr);
    }
    .meta-col {
      position: static;
      grid-row: 1;
    }
  }
  @media (max-width: 640px) {
    .pkg {
      padding-top: 32px;
    }
    .versions {
      grid-template-columns: minmax(0, 1fr) auto auto auto;
      column-gap: 10px;
    }
    .no-readme {
      padding: 22px;
    }
  }
</style>
