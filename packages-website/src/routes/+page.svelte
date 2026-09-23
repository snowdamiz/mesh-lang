<script>
  import { ArrowRight, Search } from 'lucide-svelte';
  import PackageList from '$lib/PackageList.svelte';
  import CopyButton from '$lib/CopyButton.svelte';
  import { ago, dependencyLine } from '$lib/format.js';

  export let data;

  // The registry lists the most downloaded package first.
  $: featured = data.packages[0];
  $: example = featured ?? { name: 'your-login/your-package', version: '1.0.0' };
  $: count = data.packages.length;
  $: downloads = data.packages.reduce((sum, pkg) => sum + (pkg.download_count ?? 0), 0);
  $: lastPublish = data.packages.map((pkg) => pkg.updated_at).filter(Boolean).sort().at(-1);
</script>

<svelte:head>
  <title>Mesh Packages — Community package registry for the Mesh programming language</title>
  <meta name="description" content="Browse, search, and install community packages for the Mesh programming language. Publish your own with meshpkg." />
  <meta property="og:title" content="Mesh Packages" />
  <meta property="og:description" content="Community package registry for the Mesh programming language." />
  <meta property="og:url" content="https://packages.meshlang.dev" />
  <link rel="canonical" href="https://packages.meshlang.dev" />
</svelte:head>

<section class="relative">
  <div class="dot-grid pointer-events-none absolute inset-0" aria-hidden="true"></div>

  <div class="relative grid gap-12 px-4 pb-16 pt-14 sm:px-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,27rem)] lg:gap-14 lg:px-10 lg:pb-20 lg:pt-20">
    <div>
      <p class="label enter">mesh package registry</p>
      <h1 class="display enter mt-6 text-[clamp(2.6rem,1.2rem+4.4vw,4.4rem)]" style="animation-delay: 60ms">
        <span class="text-muted-foreground/60">Find a package.</span><br />Pin it exactly.
      </h1>
      <p class="enter mt-6 max-w-lg text-[1.0625rem] leading-relaxed text-muted-foreground" style="animation-delay: 120ms">
        Source packages for Mesh. You depend on an exact version, and every install is checked against its SHA-256
        and recorded in <code class="font-mono text-[0.9em] text-foreground">mesh.lock</code>.
      </p>

      <form action="/search" method="GET" role="search" class="enter mt-8 max-w-lg" style="animation-delay: 180ms">
        <label class="relative block">
          <span class="sr-only">Search packages</span>
          <Search class="pointer-events-none absolute left-4 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <input
            name="q"
            placeholder="Search packages"
            autocomplete="off"
            class="h-12 w-full rounded-xl border border-line-strong bg-background pl-11 pr-28 text-[15px] text-foreground shadow-[0_1px_2px_rgba(0,0,0,0.04)] transition-colors placeholder:text-muted-foreground/80 focus:border-brand/60 focus:outline-none focus:ring-4 focus:ring-brand/15"
          />
          <button
            type="submit"
            class="absolute right-1.5 top-1/2 h-9 -translate-y-1/2 rounded-lg bg-foreground px-4 text-sm font-medium text-background transition-opacity hover:opacity-90"
          >
            Search
          </button>
        </label>
      </form>

      {#if !data.error}
        <dl class="enter mt-5 flex flex-wrap gap-x-5 gap-y-1 font-mono text-xs text-muted-foreground tabular-nums" style="animation-delay: 240ms">
          <div class="flex gap-1.5"><dt class="sr-only">Packages</dt><dd><span class="text-foreground">{count}</span> package{count === 1 ? '' : 's'}</dd></div>
          {#if downloads}
            <div class="flex gap-1.5"><dt class="sr-only">Downloads</dt><dd><span class="text-foreground">{downloads.toLocaleString('en-US')}</span> downloads</dd></div>
          {/if}
          {#if lastPublish}
            <div class="flex gap-1.5"><dt>last publish</dt><dd class="text-foreground"><time datetime={lastPublish}>{ago(lastPublish)}</time></dd></div>
          {/if}
        </dl>
      {/if}
    </div>

    <!-- What depending on a package looks like, using the most downloaded one -->
    <div class="enter self-center" style="animation-delay: 200ms">
      <figure class="overflow-hidden rounded-xl border border-line-strong bg-panel shadow-[0_28px_60px_-34px_rgba(0,0,0,0.45)]">
        <div class="flex h-10 items-center justify-between border-b border-line bg-panel-head pl-4 pr-1.5">
          <span class="font-mono text-xs text-muted-foreground">mesh.toml</span>
          <CopyButton text={dependencyLine(example.name, example.version)} label="Copy the mesh.toml line" />
        </div>
        <pre class="whitespace-pre-wrap px-4 py-4 font-mono text-[13px] leading-6 [overflow-wrap:anywhere]"><code><span class="text-muted-foreground">[dependencies]</span>
<span class="text-foreground">"{example.name}"</span> <span class="text-muted-foreground">=</span> <span class="inline-block text-brand">"{example.version}"</span></code></pre>
        <div class="flex h-10 items-center justify-between border-y border-line bg-panel-head pl-4 pr-1.5">
          <span class="font-mono text-xs text-muted-foreground">terminal</span>
          <CopyButton text="meshpkg install" label="Copy the install command" />
        </div>
        <pre class="whitespace-pre-wrap px-4 py-4 font-mono text-[13px] leading-6"><code><span class="select-none text-brand">$ </span>meshpkg install
<span class="text-muted-foreground"># fetches it, checks the SHA-256, updates mesh.lock</span></code></pre>
      </figure>
      <p class="mt-3 font-mono text-xs text-muted-foreground">
        {#if featured}
          Most downloaded: <a href="/packages/{featured.name}" class="text-foreground underline decoration-line-strong underline-offset-4 transition-colors hover:text-brand hover:decoration-brand">{featured.name}</a>
        {:else}
          Registry versions are exact; ranges like ^1.2 aren't accepted.
        {/if}
      </p>
    </div>
  </div>
</section>

<section class="rule">
  <div class="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-2 px-4 pb-5 pt-12 sm:px-6 lg:px-10">
    <h2 class="label">all packages</h2>
    {#if count > 1}
      <p class="font-mono text-xs text-muted-foreground">most downloaded first</p>
    {/if}
  </div>

  {#if data.error}
    <div class="border-t border-line px-4 py-16 text-center sm:px-6 lg:px-10">
      <p class="text-muted-foreground">{data.error}.</p>
      <a href="/" class="mt-4 inline-block text-sm text-foreground underline underline-offset-4 hover:text-brand">Try again</a>
    </div>
  {:else if count === 0}
    <div class="border-t border-line px-4 py-16 text-center sm:px-6 lg:px-10">
      <p class="display text-2xl">No packages yet.</p>
      <p class="mt-3 text-muted-foreground">The first one published shows up here.</p>
      <a href="/publish" class="mt-6 inline-flex items-center gap-2 text-sm font-medium text-foreground underline underline-offset-4 hover:text-brand">
        Publish a package <ArrowRight class="size-3.5" />
      </a>
    </div>
  {:else}
    <PackageList packages={data.packages} />
  {/if}
</section>

<section class="rule grid items-center gap-10 px-4 py-16 sm:px-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,27rem)] lg:gap-14 lg:px-10">
  <div>
    <p class="label">publish</p>
    <h2 class="display mt-5 text-[clamp(1.9rem,1.2rem+2vw,2.6rem)]">Share a library.</h2>
    <p class="mt-4 max-w-md leading-relaxed text-muted-foreground">
      Sign in with GitHub to get a publish token. Package names are scoped to your GitHub login, and a published
      version can't be changed.
    </p>
    <a
      href="/publish"
      class="mt-7 inline-flex h-10 items-center gap-2 rounded-lg bg-foreground px-4 text-sm font-medium text-background no-underline transition-opacity hover:opacity-90"
    >
      Get a publish token
      <ArrowRight class="size-3.5" />
    </a>
  </div>
  <pre class="overflow-x-auto rounded-xl border border-line-strong bg-panel px-4 py-4 font-mono text-[13px] leading-7"><code><span class="select-none text-brand">$ </span>meshpkg login --token <span class="text-muted-foreground">&lt;your-token&gt;</span>
<span class="select-none text-brand">$ </span>meshpkg publish</code></pre>
</section>
