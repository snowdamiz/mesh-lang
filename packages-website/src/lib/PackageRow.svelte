<script>
  import { formatDate, formatShortDate } from './format.js';
  import { splitName } from './packages.js';

  /** @type {import('./packages.js').PackageSummary} */
  export let pkg;

  $: ({ owner, slug } = splitName(pkg));
</script>

<li class="pkg-row">
  <h2 class="pkg-name"><a href="/packages/{pkg.name}"><span class="owner">{owner}/</span>{slug}</a></h2>
  <p class="pkg-desc">{pkg.description || 'No description provided.'}</p>
  <span class="pkg-ver" title={pkg.version}>{pkg.version}</span>
  <time class="pkg-date" datetime={pkg.updated_at} title={formatDate(pkg.updated_at)}>{formatShortDate(pkg.updated_at)}</time>
</li>

<style>
  .pkg-row {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 72px;
    gap: 4px 24px;
    align-items: center;
    padding: 18px 0;
    border-bottom: 1px solid var(--line);
  }
  .pkg-name {
    grid-column: 1;
    grid-row: 1;
    font-family: var(--mono);
    font-size: 14.5px;
    font-weight: 500;
    letter-spacing: -0.01em;
    overflow-wrap: anywhere;
    transition: color 0.15s;
  }
  .pkg-row:hover .pkg-name {
    color: var(--accent-ink);
  }
  .pkg-name a::after {
    content: '';
    position: absolute;
    inset: 0;
  }
  .pkg-name a:focus-visible {
    outline: none;
  }
  .pkg-row:has(a:focus-visible) {
    outline: 2px solid var(--accent-ink);
    outline-offset: 2px;
    border-radius: 4px;
  }
  .owner {
    color: var(--mute);
    font-weight: 400;
  }
  .pkg-desc {
    grid-column: 1;
    grid-row: 2;
    font-size: var(--fs-sm);
    color: var(--mute);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pkg-ver,
  .pkg-date {
    grid-row: 1 / span 2;
    text-align: right;
    white-space: nowrap;
  }
  .pkg-ver {
    grid-column: 2;
    min-width: 64px;
    max-width: 16ch;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--mono);
    font-size: 13px;
    color: var(--ink-2);
  }
  .pkg-date {
    grid-column: 3;
    font-size: 13px;
    color: var(--mute);
  }
  @media (max-width: 640px) {
    .pkg-row {
      grid-template-columns: minmax(0, 1fr) auto;
    }
    .pkg-date {
      display: none;
    }
    .pkg-desc {
      white-space: normal;
      display: -webkit-box;
      -webkit-line-clamp: 2;
      -webkit-box-orient: vertical;
    }
  }
</style>
