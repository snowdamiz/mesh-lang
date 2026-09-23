<script>
  import { ArrowLeft, ArrowUpRight } from 'lucide-svelte';
  import CopyButton from '$lib/CopyButton.svelte';
  import { ago, formatBytes, formatDate, dependencyLine } from '$lib/format.js';

  export let data;

  $: pkg = data.pkg;
  $: [owner, slug] = pkg ? pkg.name.split('/') : [];
  $: latestVersion = pkg?.latest?.version;
  $: latest = data.versions.find((ver) => ver.version === latestVersion);
</script>

<svelte:head>
  {#if data.pkg}
    <title>{data.pkg.name} — Mesh Packages</title>
    <meta name="description" content={data.pkg.description || `${data.pkg.name} — a Mesh package.`} />
    <meta property="og:title" content="{data.pkg.name} — Mesh Packages" />
    <meta property="og:description" content={data.pkg.description || `Install with meshpkg install ${data.pkg.name}`} />
    <meta property="og:url" content="https://packages.meshlang.dev/packages/{data.pkg.name}" />
    <link rel="canonical" href="https://packages.meshlang.dev/packages/{data.pkg.name}" />
  {:else if data.notFound}
    <title>Package not found — Mesh Packages</title>
    <meta name="robots" content="noindex" />
  {:else}
    <title>Package — Mesh Packages</title>
  {/if}
</svelte:head>

{#if data.notFound}
  <section class="px-4 py-24 text-center sm:px-6 lg:px-10">
    <p class="label">not found</p>
    <h1 class="display mt-5 text-[clamp(2rem,1.4rem+2vw,2.75rem)]">No package by that name.</h1>
    <p class="mt-3 text-muted-foreground">Registry names look like <code class="font-mono text-foreground">owner/package</code>.</p>
    <a href="/" class="mt-7 inline-flex items-center gap-2 text-sm font-medium text-foreground underline underline-offset-4 hover:text-brand">
      <ArrowLeft class="size-3.5" /> Browse all packages
    </a>
  </section>
{:else if data.error}
  <section class="px-4 py-24 text-center sm:px-6 lg:px-10">
    <p class="text-muted-foreground">{data.error}.</p>
    <a href="/" class="mt-4 inline-block text-sm text-foreground underline underline-offset-4 hover:text-brand">Browse all packages</a>
  </section>
{:else if pkg}
  <section class="px-4 pb-10 pt-9 sm:px-6 lg:px-10 lg:pb-12 lg:pt-12">
    <a href="/" class="enter inline-flex items-center gap-1.5 font-mono text-xs text-muted-foreground no-underline transition-colors hover:text-foreground">
      <ArrowLeft class="size-3" /> all packages
    </a>
    <h1 class="display enter mt-6 break-words text-[clamp(2rem,1.1rem+2.8vw,3.25rem)]" style="animation-delay: 50ms">
      <span class="text-muted-foreground/60">{owner}/</span>{slug}
    </h1>
    <p class="enter mt-4 max-w-2xl text-[1.0625rem] leading-relaxed text-muted-foreground" style="animation-delay: 100ms">
      {pkg.description || 'No description provided.'}
    </p>
    <dl class="enter mt-6 flex flex-wrap items-center gap-x-5 gap-y-2 font-mono text-xs text-muted-foreground tabular-nums" style="animation-delay: 150ms">
      {#if latestVersion}
        <div><dt class="sr-only">Latest version</dt><dd class="max-w-full break-all rounded-md bg-brand/12 px-2 py-1 text-brand">v{latestVersion}</dd></div>
      {/if}
      {#if latest}
        <div class="flex gap-1.5"><dt>published</dt><dd class="text-foreground"><time datetime={latest.published_at} title={formatDate(latest.published_at)}>{ago(latest.published_at)}</time></dd></div>
      {/if}
      <div class="flex gap-1.5"><dt class="sr-only">Downloads</dt><dd><span class="text-foreground">{pkg.download_count.toLocaleString('en-US')}</span> downloads</dd></div>
    </dl>
  </section>

  <!-- Phones read install, then versions, then details; desktop puts install and details in a sidebar. -->
  <section class="rule grid gap-12 px-4 py-10 sm:px-6 lg:grid-cols-[minmax(0,1fr)_20rem] lg:grid-rows-[auto_1fr] lg:gap-x-14 lg:gap-y-10 lg:px-10 lg:py-12">
    {#if latestVersion}
      <div class="lg:col-start-2 lg:row-start-1">
        <h2 class="label">install</h2>
        <div class="mt-4 overflow-hidden rounded-xl border border-line-strong bg-panel">
          <div class="flex h-10 items-center justify-between border-b border-line bg-panel-head pl-4 pr-1.5">
            <span class="font-mono text-xs text-muted-foreground">mesh.toml</span>
            <CopyButton text={dependencyLine(pkg.name, latestVersion)} label="Copy the mesh.toml line" />
          </div>
          <pre class="whitespace-pre-wrap px-4 py-3.5 font-mono text-[12.5px] leading-6 [overflow-wrap:anywhere]"><code><span class="text-muted-foreground">[dependencies]</span>
"{pkg.name}" <span class="text-muted-foreground">=</span> <span class="inline-block text-brand">"{latestVersion}"</span></code></pre>
          <div class="flex h-10 items-center justify-between border-y border-line bg-panel-head pl-4 pr-1.5">
            <span class="font-mono text-xs text-muted-foreground">terminal</span>
            <CopyButton text="meshpkg install" label="Copy the install command" />
          </div>
          <pre class="px-4 py-3.5 font-mono text-[12.5px] leading-6"><code><span class="select-none text-brand">$ </span>meshpkg install</code></pre>
        </div>
        <p class="mt-3 text-xs leading-relaxed text-muted-foreground">
          Registry versions are exact. <code class="font-mono">meshpkg install {pkg.name}</code> on its own records the
          latest release in <code class="font-mono">mesh.lock</code> without editing <code class="font-mono">mesh.toml</code>.
        </p>
      </div>
    {/if}

    <div class="min-w-0 space-y-12 lg:col-start-1 lg:row-span-2 lg:row-start-1">
      {#if data.readmeHtml}
        <section aria-labelledby="readme">
          <h2 id="readme" class="label">readme</h2>
          <div class="prose prose-neutral mt-6 max-w-none dark:prose-invert prose-headings:font-semibold prose-h1:text-2xl prose-h2:text-xl prose-a:text-brand prose-code:before:content-none prose-code:after:content-none">
            {@html data.readmeHtml}
          </div>
        </section>
      {/if}

      {#if data.versions.length > 0}
        <section id="versions" aria-labelledby="versions-heading">
          <h2 id="versions-heading" class="label">versions <span class="text-foreground tabular-nums">{data.versions.length}</span></h2>
          <table class="mt-4 w-full border-b border-line font-mono text-[13px]">
            <thead>
              <tr class="text-left text-[11px] uppercase tracking-wider text-muted-foreground">
                <th scope="col" class="pb-2.5 pr-4 font-normal">Version</th>
                <th scope="col" class="pb-2.5 pr-4 font-normal">Published</th>
                <th scope="col" class="hidden pb-2.5 pr-4 font-normal sm:table-cell">Size</th>
                <th scope="col" class="pb-2.5 text-right font-normal">Downloads</th>
                <th scope="col" class="pb-2.5"><span class="sr-only">Copy</span></th>
              </tr>
            </thead>
            <tbody>
              {#each data.versions as ver (ver.version)}
                <tr class="border-t border-line transition-colors hover:bg-muted/40">
                  <td class="py-2.5 pr-4 align-middle">
                    <span class="break-all text-foreground">v{ver.version}</span>
                    {#if ver.version === latestVersion}
                      <span class="ml-1.5 whitespace-nowrap rounded bg-brand/12 px-1.5 py-0.5 text-[10px] text-brand">latest</span>
                    {/if}
                  </td>
                  <td class="whitespace-nowrap py-2.5 pr-4 text-muted-foreground">
                    <time datetime={ver.published_at} title={formatDate(ver.published_at)}>{ago(ver.published_at)}</time>
                  </td>
                  <td class="hidden whitespace-nowrap py-2.5 pr-4 text-muted-foreground sm:table-cell">{formatBytes(ver.size_bytes)}</td>
                  <td class="py-2.5 text-right text-muted-foreground tabular-nums">{ver.download_count.toLocaleString('en-US')}</td>
                  <td class="py-1.5 pl-2 text-right">
                    <CopyButton text={dependencyLine(pkg.name, ver.version)} label="Copy the mesh.toml line for v{ver.version}" />
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </section>
      {/if}

      {#if !data.readmeHtml}
        <p class="text-sm text-muted-foreground">
          This package has no README; <code class="font-mono text-foreground">meshpkg publish</code> doesn't upload one yet.
        </p>
      {/if}
    </div>

    <div class="lg:col-start-2 lg:row-start-2">
      <h2 class="label">details</h2>
      <dl class="mt-4 divide-y divide-line border-y border-line text-sm">
        <div class="flex items-center justify-between gap-4 py-2.5">
          <dt class="text-muted-foreground">Owner</dt>
          <dd>
            <a href="https://github.com/{owner}" target="_blank" rel="noopener" class="inline-flex items-center gap-1 font-mono text-foreground no-underline transition-colors hover:text-brand">
              {owner}<ArrowUpRight class="size-3 opacity-60" />
            </a>
          </dd>
        </div>
        {#if latest}
          <div class="flex items-center justify-between gap-4 py-2.5">
            <dt class="text-muted-foreground">Published</dt>
            <dd class="font-mono text-foreground">{formatDate(latest.published_at)}</dd>
          </div>
          <div class="flex items-center justify-between gap-4 py-2.5">
            <dt class="text-muted-foreground">Size</dt>
            <dd class="font-mono text-foreground">{formatBytes(latest.size_bytes)}</dd>
          </div>
        {/if}
        {#if pkg.latest?.sha256}
          <div class="py-2.5">
            <dt class="flex items-center justify-between gap-4 text-muted-foreground">
              SHA-256
              <CopyButton text={pkg.latest.sha256} label="Copy the SHA-256 of v{latestVersion}" />
            </dt>
            <dd class="mt-1 break-all font-mono text-[11px] leading-5 text-muted-foreground">{pkg.latest.sha256}</dd>
          </div>
        {/if}
      </dl>
    </div>
  </section>
{/if}
