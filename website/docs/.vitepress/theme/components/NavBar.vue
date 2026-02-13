<script setup lang="ts">
import ThemeToggle from './ThemeToggle.vue'
</script>

<template>
  <header class="sticky top-0 z-50 w-full border-b border-border bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
    <div class="mx-auto flex h-14 max-w-7xl items-center justify-between px-4">
      <!-- Logo / Wordmark -->
      <a href="/" class="flex items-center gap-2 font-semibold text-foreground">
        Mesh
      </a>

      <!-- Navigation Links -->
      <nav class="hidden items-center gap-6 text-sm md:flex">
        <a href="/docs/getting-started/" class="text-muted-foreground transition-colors hover:text-foreground">
          Docs
        </a>
        <a href="https://github.com/user/mesh" class="text-muted-foreground transition-colors hover:text-foreground">
          GitHub
        </a>
      </nav>

      <!-- Right side: theme toggle -->
      <div class="flex items-center gap-2">
        <ThemeToggle />
      </div>
    </div>
  </header>
</template>
