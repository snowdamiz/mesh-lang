<script>
  import { Download } from 'lucide-svelte';
  import CopyButton from './CopyButton.svelte';
  import { ago, dependencyLine } from './format.js';

  export let packages;
</script>

<ol class="divide-y divide-line border-t border-line">
  {#each packages as pkg, i (pkg.name)}
    {@const [owner, slug] = pkg.name.split('/')}
    <li
      class="enter group relative px-4 py-5 transition-colors hover:bg-muted/40 has-[a:focus-visible]:bg-muted/60 has-[a:focus-visible]:ring-2 has-[a:focus-visible]:ring-inset has-[a:focus-visible]:ring-brand/60 sm:flex sm:items-start sm:gap-10 sm:px-6 lg:px-10"
      style="animation-delay: {Math.min(i, 10) * 35}ms"
    >
      <div class="min-w-0 flex-1">
        <h3 class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <!-- The link stretches over the whole row -->
          <a
            href="/packages/{pkg.name}"
            class="min-w-0 break-words font-mono text-[15px] font-semibold text-foreground no-underline transition-colors after:absolute after:inset-0 group-hover:text-brand focus-visible:outline-none"
          >
            <span class="font-normal text-muted-foreground">{owner}/</span>{slug}
          </a>
          <span class="max-w-full truncate font-mono text-xs text-muted-foreground">v{pkg.version}</span>
        </h3>
        <p class="mt-1.5 line-clamp-2 max-w-3xl text-sm leading-relaxed text-muted-foreground">
          {pkg.description || 'No description provided.'}
        </p>
      </div>

      <div class="mt-3 flex items-center gap-4 font-mono text-xs text-muted-foreground tabular-nums sm:mt-0 sm:shrink-0">
        {#if pkg.download_count != null}
          <span class="inline-flex items-center gap-1.5 sm:w-16 sm:justify-end">
            <Download class="size-3" aria-hidden="true" />
            <span class="sr-only">Downloads:</span>
            {pkg.download_count.toLocaleString('en-US')}
          </span>
        {/if}
        {#if pkg.updated_at}
          <time datetime={pkg.updated_at} class="whitespace-nowrap sm:w-28 sm:text-right">{ago(pkg.updated_at)}</time>
        {/if}
        <span class="ml-auto sm:ml-0">
          <CopyButton text={dependencyLine(pkg.name, pkg.version)} label="Copy the mesh.toml line for {pkg.name}">mesh.toml</CopyButton>
        </span>
      </div>
    </li>
  {/each}
</ol>
