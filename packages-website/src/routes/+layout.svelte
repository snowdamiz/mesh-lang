<script>
  import '@fontsource-variable/geist';
  import '@fontsource-variable/geist-mono';
  import '../app.css';
  import { afterNavigate } from '$app/navigation';
  import { page } from '$app/stores';
  import { onMount, tick } from 'svelte';
  import BrandSymbol from '$lib/BrandSymbol.svelte';
  import Icon from '$lib/Icon.svelte';
  import SearchField from '$lib/SearchField.svelte';
  import { FOOTER_COLUMNS, MAIN_SITE, NAV_LINKS, STATUS, STATUS_URL } from '$lib/site.js';

  const THEME_COLOR = { light: '#f7f2ea', dark: '#141312' };
  // Routes whose page leads with its own search field; every other page gets the header's.
  const ROUTES_WITH_SEARCH = new Set(['/', '/search']);

  let dark = false;
  let menuOpen = false;
  let menuButton;
  let navLinks;
  let main;
  let headerSearchField;

  onMount(() => {
    dark = document.documentElement.classList.contains('dark');
    // The menu exists only below the nav breakpoint in app.css.
    const narrow = matchMedia('(max-width: 980px)');
    const onChange = () => {
      if (!narrow.matches) closeMenu({ restoreFocus: false });
    };
    narrow.addEventListener('change', onChange);
    return () => narrow.removeEventListener('change', onChange);
  });
  afterNavigate(() => {
    if (!menuOpen) return;
    menuOpen = false;
    // The link that was followed is hidden with the menu, and SvelteKit parks focus on the body.
    main.focus({ preventScroll: true });
  });

  $: headerSearch = Boolean($page.error) || !ROUTES_WITH_SEARCH.has($page.route.id);

  function toggleTheme() {
    dark = !dark;
    const theme = dark ? 'dark' : 'light';
    document.documentElement.classList.toggle('dark', dark);
    document.querySelector('meta[name="theme-color"]')?.setAttribute('content', THEME_COLOR[theme]);
    localStorage.setItem('theme', theme);
  }

  async function openMenu() {
    menuOpen = true;
    await tick();
    navLinks.querySelector('a')?.focus();
  }

  function closeMenu({ restoreFocus = true } = {}) {
    if (!menuOpen) return;
    menuOpen = false;
    if (restoreFocus) menuButton.focus();
  }

  function onKeydown(event) {
    if (event.key === 'Escape' && menuOpen) closeMenu();
    else focusSearch(event);
  }

  // "/" jumps to the one search field on the page, the page's own or the header's.
  // Phones fold the header's field into a link, so it drops down first.
  function focusSearch(event) {
    const target = event.target;
    if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return;
    if (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
    const input = [...document.querySelectorAll('input[name="q"]')].find((field) => field.getClientRects().length);
    if (input) {
      event.preventDefault();
      input.focus();
    } else if (headerSearchField) {
      event.preventDefault();
      closeMenu({ restoreFocus: false });
      headerSearchField.reveal();
    }
  }
</script>

<svelte:window on:keydown={onKeydown} />

<BrandSymbol />
<a class="skip" href="#main">Skip to content</a>

<header class="nav" class:open={menuOpen}>
  <div class="nav-in">
    <a class="brand" href={MAIN_SITE} aria-label="Mesh home">
      <svg viewBox="0 0 317.64 80.81" width="98" height="25" aria-hidden="true"><use href="#mesh-lockup" /></svg>
    </a>
    <nav class="nav-links" id="nav-links" aria-label="Primary" bind:this={navLinks}>
      {#each NAV_LINKS as link}
        <a href={link.href} aria-current={link.current ? 'page' : undefined}>{link.label}</a>
      {/each}
      <a class="nav-status" href={STATUS_URL}><i></i>{STATUS}</a>
    </nav>
    <div class="nav-end">
      {#if headerSearch}<SearchField compact bind:this={headerSearchField} />{/if}
      <a class="status" href={STATUS_URL} title="The language, runtime, package format, and cluster protocol can still change before a stable release."><i></i>{STATUS}</a>
      <button type="button" class="icon-btn theme-btn" on:click={toggleTheme} aria-pressed={dark} aria-label="Dark theme">
        <Icon name="moon" class="when-light" />
        <Icon name="sun" class="when-dark" />
      </button>
      <a class="btn btn-ink" href="/publish">Publish</a>
      <button
        type="button"
        class="menu-btn"
        aria-label="Menu"
        aria-controls="nav-links"
        aria-expanded={menuOpen}
        bind:this={menuButton}
        on:click={() => (menuOpen ? closeMenu() : openMenu())}
      >
        <Icon name="menu" />
      </button>
    </div>
  </div>
</header>

<main id="main" tabindex="-1" bind:this={main}>
  <slot />
</main>

<footer class="footer">
  <div class="app-wrap footer-in">
    <div class="footer-brand">
      <a class="brand" href={MAIN_SITE} aria-label="Mesh home">
        <svg viewBox="0 0 317.64 80.81" width="98" height="25" aria-hidden="true"><use href="#mesh-lockup" /></svg>
      </a>
    </div>
    {#each FOOTER_COLUMNS as column}
      <div>
        <h2>{column.title}</h2>
        <ul>
          {#each column.links as link}
            <li><a href={link.href}>{link.label}</a></li>
          {/each}
        </ul>
      </div>
    {/each}
  </div>
  <div class="app-wrap footer-base"><span>MIT licensed</span><span>Mesh is in active development.</span></div>
</footer>
