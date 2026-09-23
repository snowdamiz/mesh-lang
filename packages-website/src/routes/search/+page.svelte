<script>
  import { ArrowRight, Search } from 'lucide-svelte';
  import PackageList from '$lib/PackageList.svelte';

  export let data;

  $: count = data.packages.length;
</script>

<svelte:head>
  <title>{data.query ? `"${data.query}" — Search` : 'Search'} — Mesh Packages</title>
  <meta name="description" content={data.query ? `Search results for "${data.query}" on Mesh Packages.` : 'Search the Mesh package registry.'} />
  <meta name="robots" content="noindex" />
</svelte:head>

<section class="px-4 pb-12 pt-12 sm:px-6 lg:px-10 lg:pt-16">
  <p class="label enter">search</p>
  <h1 class="display enter mt-5 break-words text-[clamp(2rem,1.2rem+2.6vw,3.25rem)]" style="animation-delay: 60ms">
    {#if data.query}
      Results for <span class="text-muted-foreground">"{data.query}"</span>
    {:else}
      Search packages
    {/if}
  </h1>

  <form action="/search" method="GET" role="search" class="enter mt-8 max-w-lg" style="animation-delay: 120ms">
    <label class="relative block">
      <span class="sr-only">Search packages</span>
      <Search class="pointer-events-none absolute left-4 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
      <input
        name="q"
        value={data.query}
        placeholder="Search packages"
        autocomplete="off"
        class="h-12 w-full rounded-xl border border-line-strong bg-background pl-11 pr-28 text-[15px] text-foreground transition-colors placeholder:text-muted-foreground/80 focus:border-brand/60 focus:outline-none focus:ring-4 focus:ring-brand/15"
      />
      <button
        type="submit"
        class="absolute right-1.5 top-1/2 h-9 -translate-y-1/2 rounded-lg bg-foreground px-4 text-sm font-medium text-background transition-opacity hover:opacity-90"
      >
        Search
      </button>
    </label>
  </form>
</section>

<section class="rule">
  {#if data.error}
    <div class="px-4 py-16 text-center sm:px-6 lg:px-10">
      <p class="text-muted-foreground">{data.error}.</p>
      <a href="/" class="mt-4 inline-block text-sm text-foreground underline underline-offset-4 hover:text-brand">Browse all packages</a>
    </div>
  {:else if data.query}
    <div class="px-4 pb-5 pt-10 sm:px-6 lg:px-10">
      <h2 class="label tabular-nums">{count} result{count === 1 ? '' : 's'}</h2>
    </div>
    {#if count === 0}
      <div class="border-t border-line px-4 py-16 text-center sm:px-6 lg:px-10">
        <p class="text-foreground">Nothing matches "{data.query}".</p>
        <p class="mt-2 text-sm text-muted-foreground">Descriptions match whole words, so try a complete word, or part of a package name.</p>
        <a href="/" class="mt-6 inline-flex items-center gap-2 text-sm font-medium text-foreground underline underline-offset-4 hover:text-brand">
          Browse all packages <ArrowRight class="size-3.5" />
        </a>
      </div>
    {:else}
      <PackageList packages={data.packages} />
    {/if}
  {:else}
    <div class="px-4 py-12 sm:px-6 lg:px-10">
      <a href="/" class="inline-flex items-center gap-2 text-sm font-medium text-foreground underline underline-offset-4 hover:text-brand">
        Browse all packages <ArrowRight class="size-3.5" />
      </a>
    </div>
  {/if}
</section>
