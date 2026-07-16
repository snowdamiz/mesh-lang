<script setup lang="ts">
import { ref, computed } from 'vue'
import VPNavBarSearch from 'vitepress/dist/client/theme-default/components/VPNavBarSearch.vue'
import { withBase, useData } from 'vitepress'
import ThemeToggle from './ThemeToggle.vue'
import { useSidebar } from '@/composables/useSidebar'
import { Menu, X } from 'lucide-vue-next'

const { hasSidebar, is960, toggle } = useSidebar()
const { isDark, page } = useData()

// Mobile menu for non-docs pages (landing, packages, etc.)
const mobileMenuOpen = ref(false)

const navLinks = [
  { text: 'Docs', href: '/docs/getting-started/', target: '_self', match: /^docs\// },
  { text: 'Packages', href: 'https://packages.meshlang.dev', target: '_blank', match: /^packages\// },
  { text: 'GitHub', href: 'https://github.com/hyperpush-org/mesh-lang', target: '_blank', match: null },
]

const activeSection = computed(() => {
  const path = page.value.relativePath
  for (const link of navLinks) {
    if (link.match?.test(path)) return link.text
  }
  return null
})
</script>

<template>
  <header class="sticky top-0 z-50 w-full border-b border-border/70 bg-background/85 backdrop-blur-xl">
    <div class="relative mx-auto flex h-14 max-w-[90rem] items-center px-4 lg:px-6">
      <!-- Logo + mobile hamburger -->
      <div class="flex shrink-0 items-center gap-3">
        <!-- Docs sidebar toggle (mobile, inside docs) -->
        <button
          v-if="hasSidebar && !is960"
          class="inline-flex items-center justify-center rounded-lg p-2 text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
          aria-label="Toggle sidebar"
          @click="toggle"
        >
          <Menu class="size-5" />
        </button>
        <!-- Mobile menu toggle (outside docs) -->
        <button
          v-if="!hasSidebar || is960"
          class="md:hidden inline-flex items-center justify-center rounded-lg p-2 text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
          :aria-label="mobileMenuOpen ? 'Close menu' : 'Open menu'"
          :aria-expanded="mobileMenuOpen"
          @click="mobileMenuOpen = !mobileMenuOpen"
        >
          <X v-if="mobileMenuOpen" class="size-5" />
          <Menu v-else class="size-5" />
        </button>
        <a href="/" class="flex items-center">
          <img :src="withBase(isDark ? '/logo-white.svg' : '/logo-black.svg')" alt="Mesh" class="h-7 w-auto" />
        </a>
      </div>

      <!-- Navigation Links (viewport-centered, desktop) -->
      <nav class="hidden items-center justify-center gap-1.5 text-sm md:flex absolute inset-0 pointer-events-none">
        <a
          v-for="link in navLinks"
          :key="link.text"
          :href="link.href"
          :target="link.target"
          class="pointer-events-auto inline-flex items-center gap-2 rounded-lg px-3 py-1.5 transition-colors"
          :class="
            activeSection === link.text
              ? 'bg-muted font-medium text-foreground'
              : 'text-muted-foreground hover:text-foreground hover:bg-muted'
          "
        >
          <span v-if="activeSection === link.text" class="size-1.5 rounded-full bg-brand" aria-hidden="true" />
          {{ link.text }}
        </a>
      </nav>

      <!-- Search + Theme toggle (right) -->
      <div class="flex shrink-0 items-center gap-1 ml-auto">
        <VPNavBarSearch />
        <ThemeToggle />
      </div>
    </div>

    <!-- Mobile dropdown menu -->
    <div
      v-if="mobileMenuOpen"
      class="md:hidden border-t border-border/70 bg-background/95 backdrop-blur-xl"
    >
      <nav class="mx-auto max-w-[90rem] flex flex-col px-4 py-3 gap-0.5">
        <a
          v-for="link in navLinks"
          :key="link.href"
          :href="link.href"
          :target="link.target"
          class="flex items-center gap-2 rounded-xl px-3.5 py-2.5 text-sm transition-colors"
          :class="
            activeSection === link.text
              ? 'bg-muted font-medium text-foreground'
              : 'text-muted-foreground hover:text-foreground hover:bg-muted'
          "
          @click="mobileMenuOpen = false"
        >
          <span v-if="activeSection === link.text" class="size-1.5 rounded-full bg-brand" aria-hidden="true" />
          {{ link.text }}
        </a>
      </nav>
    </div>
  </header>
</template>
