<script>
  import '../app.css';
  import { Search, Sun, Moon } from 'lucide-svelte';
  import { onMount } from 'svelte';

  const syncDarkFromDocument = () => {
    dark = typeof document !== 'undefined' && document.documentElement.classList.contains('dark');
  };

  let dark = typeof document !== 'undefined' && document.documentElement.classList.contains('dark');
  const languageRepoUrl = 'https://github.com/hyperpush-org/mesh-lang';

  onMount(syncDarkFromDocument);

  function toggleDark() {
    dark = !dark;
    document.documentElement.classList.toggle('dark', dark);
    localStorage.setItem('theme', dark ? 'dark' : 'light');
  }

  // "/" jumps to the page's own search box, or the header's when the page has none.
  function focusSearch(event) {
    const target = event.target;
    if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return;
    if (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
    const input = [...document.querySelectorAll('input[name="q"]')].findLast((el) => el.offsetParent);
    if (!input) return;
    event.preventDefault();
    input.focus();
  }
</script>

<svelte:window on:keydown={focusSearch} />

<header class="sticky top-0 z-50 w-full border-b border-line bg-background/85 backdrop-blur-xl">
  <div class="relative mx-auto flex h-14 max-w-6xl items-center gap-2 px-4 sm:px-6 lg:px-10">
    <a href="/" class="relative z-10 flex shrink-0 items-center gap-2.5 no-underline">
      <span class="themed-logo" aria-hidden="true">
        <img src="/logo-black.svg" alt="" class="themed-logo__light h-5 w-auto" />
        <img src="/logo-white.svg" alt="" class="themed-logo__dark h-5 w-auto" />
      </span>
      <span class="sr-only">Mesh</span>
      <span class="select-none text-lg font-light text-muted-foreground/40" aria-hidden="true">/</span>
      <span class="font-mono text-[13px] text-foreground">packages</span>
    </a>

    <!-- Same links as meshlang.dev, centred on the viewport -->
    <nav class="pointer-events-none absolute inset-0 hidden items-center justify-center gap-1.5 text-sm lg:flex" aria-label="Mesh">
      <a href="https://meshlang.dev/docs/getting-started/" class="pointer-events-auto rounded-lg px-3 py-1.5 text-muted-foreground no-underline transition-colors hover:bg-muted hover:text-foreground">Docs</a>
      <a href="/" aria-current="page" class="pointer-events-auto inline-flex items-center gap-2 rounded-lg bg-muted px-3 py-1.5 font-medium text-foreground no-underline">
        <span class="size-1.5 rounded-full bg-brand" aria-hidden="true"></span>
        Packages
      </a>
      <a href={languageRepoUrl} target="_blank" rel="noopener" class="pointer-events-auto rounded-lg px-3 py-1.5 text-muted-foreground no-underline transition-colors hover:bg-muted hover:text-foreground">GitHub</a>
    </nav>

    <div class="relative z-10 ml-auto flex items-center gap-1.5">
      <form action="/search" method="GET" role="search" class="hidden sm:block">
        <label class="relative block">
          <span class="sr-only">Search packages</span>
          <Search class="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <input
            name="q"
            placeholder="Search packages"
            autocomplete="off"
            class="h-9 w-48 rounded-lg border border-line-strong bg-muted/40 pl-9 pr-8 text-sm text-foreground transition-colors placeholder:text-muted-foreground/80 focus:border-brand/60 focus:bg-background focus:outline-none focus:ring-2 focus:ring-brand/20 xl:w-60"
          />
          <kbd class="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded border border-line-strong px-1.5 font-mono text-[10px] leading-4 text-muted-foreground">/</kbd>
        </label>
      </form>
      <a
        href="/search"
        class="flex size-9 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground sm:hidden"
        aria-label="Search packages"
      >
        <Search class="size-4" />
      </a>
      <button
        type="button"
        on:click={toggleDark}
        class="flex size-9 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        aria-label="Toggle dark mode"
      >
        {#if dark}
          <Sun class="size-4" />
        {:else}
          <Moon class="size-4" />
        {/if}
      </button>
      <a
        href="/publish"
        class="hidden h-9 items-center rounded-lg bg-foreground px-3.5 text-sm font-medium text-background no-underline transition-opacity hover:opacity-90 md:inline-flex"
      >
        Publish
      </a>
    </div>
  </div>
</header>

<main>
  <div class="frame min-h-[calc(100vh-3.5rem-6rem)]">
    <slot />
  </div>
</main>

<footer class="border-t border-line">
  <div class="mx-auto flex max-w-6xl flex-col gap-5 px-4 py-9 sm:flex-row sm:items-center sm:justify-between sm:px-6 lg:px-10">
    <a href="https://meshlang.dev" class="flex items-center gap-2.5 no-underline">
      <span class="themed-logo opacity-60" aria-hidden="true">
        <img src="/logo-black.svg" alt="" class="themed-logo__light h-3.5 w-auto" />
        <img src="/logo-white.svg" alt="" class="themed-logo__dark h-3.5 w-auto" />
      </span>
      <span class="sr-only">Mesh</span>
      <span class="font-mono text-xs text-muted-foreground">the Mesh programming language</span>
    </a>
    <nav class="flex flex-wrap gap-x-6 gap-y-2 text-sm text-muted-foreground" aria-label="Footer">
      <a href="https://meshlang.dev/docs/packages/" class="no-underline transition-colors hover:text-foreground">Packages guide</a>
      <a href="https://meshlang.dev/docs/tooling/" class="no-underline transition-colors hover:text-foreground">meshpkg</a>
      <a href="/publish" class="no-underline transition-colors hover:text-foreground">Publish</a>
      <a href={languageRepoUrl} target="_blank" rel="noopener" class="no-underline transition-colors hover:text-foreground">GitHub</a>
    </nav>
  </div>
</footer>
