<script setup lang="ts">
import { VPNavBarSearch } from 'vitepress/theme'
import ThemeToggle from './ThemeToggle.vue'
import { useSidebar } from '@/composables/useSidebar'
import { Menu } from 'lucide-vue-next'

const { hasSidebar, is960, toggle } = useSidebar()
</script>

<template>
  <header class="sticky top-0 z-50 w-full border-b border-border bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
    <div class="mx-auto flex h-14 max-w-7xl items-center justify-between px-4">
      <!-- Mobile sidebar toggle + Logo -->
      <div class="flex items-center gap-2">
        <button
          v-if="hasSidebar && !is960"
          class="inline-flex items-center justify-center rounded-md p-2 text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
          aria-label="Toggle sidebar"
          @click="toggle"
        >
          <Menu class="size-5" />
        </button>
        <a href="/" class="flex items-center gap-2 font-semibold text-foreground">
          Mesh
        </a>
      </div>

      <!-- Navigation Links -->
      <nav class="hidden items-center gap-6 text-sm md:flex">
        <a href="/docs/getting-started/" class="text-muted-foreground transition-colors hover:text-foreground">
          Docs
        </a>
        <a href="https://github.com/user/mesh" class="text-muted-foreground transition-colors hover:text-foreground">
          GitHub
        </a>
      </nav>

      <!-- Right side: search + theme toggle -->
      <div class="flex items-center gap-2">
        <VPNavBarSearch />
        <ThemeToggle />
      </div>
    </div>
  </header>
</template>
