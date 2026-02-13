<script setup lang="ts">
import VPNavBarSearch from 'vitepress/dist/client/theme-default/components/VPNavBarSearch.vue'
import { withBase, useData } from 'vitepress'
import ThemeToggle from './ThemeToggle.vue'
import { useSidebar } from '@/composables/useSidebar'
import { Menu } from 'lucide-vue-next'

const { hasSidebar, is960, toggle } = useSidebar()
const { isDark } = useData()
</script>

<template>
  <header class="sticky top-0 z-50 w-full border-b border-border/50 bg-background/80 backdrop-blur-xl">
    <div class="mx-auto flex h-14 max-w-[90rem] items-center px-4 lg:px-6">
      <!-- Logo -->
      <div class="flex shrink-0 items-center gap-3">
        <button
          v-if="hasSidebar && !is960"
          class="inline-flex items-center justify-center rounded-md p-2 text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
          aria-label="Toggle sidebar"
          @click="toggle"
        >
          <Menu class="size-5" />
        </button>
        <a href="/" class="flex items-center">
          <img :src="withBase(isDark ? '/logo-white.svg' : '/logo-black.svg')" alt="Mesh" class="h-7 w-auto" />
        </a>
      </div>

      <!-- Navigation Links (centered) -->
      <nav class="hidden flex-1 items-center justify-center gap-1 text-sm md:flex">
        <a
          href="/docs/getting-started/"
          class="rounded-md px-3 py-1.5 text-muted-foreground transition-colors hover:text-foreground hover:bg-accent"
        >
          Docs
        </a>
        <a
          href="https://github.com/user/mesh"
          class="rounded-md px-3 py-1.5 text-muted-foreground transition-colors hover:text-foreground hover:bg-accent"
        >
          GitHub
        </a>
      </nav>

      <!-- Search + Theme toggle (right) -->
      <div class="flex shrink-0 items-center gap-1">
        <VPNavBarSearch />
        <ThemeToggle />
      </div>
    </div>
  </header>
</template>
