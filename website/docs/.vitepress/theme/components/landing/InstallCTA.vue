<script setup lang="ts">
import InstallCommand from './InstallCommand.vue'
</script>

<template>
  <section class="l-section">
    <div class="l-pad grid gap-10 lg:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)] lg:items-center lg:gap-16">
      <div>
        <h2 class="l-h2">Start building.</h2>
        <p class="l-lede mt-5 max-w-[26rem]">
          One script installs the <code class="font-mono text-[0.9em] text-foreground">meshc</code> compiler and the
          <code class="font-mono text-[0.9em] text-foreground">meshpkg</code> package manager.
        </p>
      </div>

      <div class="min-w-0">
        <dl class="grid gap-4">
          <div>
            <dt class="mb-2 font-mono text-xs text-muted-foreground">macOS and Linux</dt>
            <dd><InstallCommand command="curl -sSf https://meshlang.dev/install.sh | sh" class="w-full" /></dd>
          </div>
          <div>
            <dt class="mb-2 font-mono text-xs text-muted-foreground">Windows (PowerShell)</dt>
            <dd><InstallCommand command="irm https://meshlang.dev/install.ps1 | iex" prompt=">" class="w-full" /></dd>
          </div>
        </dl>
        <div class="mt-7 flex flex-col gap-3 sm:flex-row">
          <a href="/docs/getting-started/" class="l-btn l-btn-primary">
            Read the guide <span class="l-arrow" aria-hidden="true">→</span>
          </a>
          <a href="https://packages.meshlang.dev" target="_blank" rel="noopener" class="l-btn l-btn-quiet">Browse packages</a>
        </div>
      </div>
    </div>
  </section>
</template>
