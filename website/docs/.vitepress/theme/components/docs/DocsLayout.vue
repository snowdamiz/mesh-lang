<script setup lang="ts">
import { Content } from 'vitepress'
import { useSidebar } from '@/composables/useSidebar'
import { useMediaQuery } from '@vueuse/core'
import DocsSidebar from './DocsSidebar.vue'
import DocsTableOfContents from './DocsTableOfContents.vue'
import DocsPrevNext from './DocsPrevNext.vue'
import MobileSidebar from './MobileSidebar.vue'

const { sidebar, hasSidebar } = useSidebar()
const isDesktop = useMediaQuery('(min-width: 960px)')
const isWide = useMediaQuery('(min-width: 1280px)')
</script>

<template>
  <div class="relative mx-auto flex max-w-[90rem]">
    <!-- Desktop sidebar -->
    <aside
      v-if="hasSidebar && isDesktop"
      class="sticky top-14 h-[calc(100vh-3.5rem)] w-64 shrink-0 border-r border-border"
    >
      <DocsSidebar :items="sidebar" />
    </aside>

    <!-- Mobile sidebar -->
    <MobileSidebar v-if="hasSidebar && !isDesktop" :items="sidebar" />

    <!-- Main content -->
    <main class="min-w-0 flex-1 px-6 py-8 lg:px-8">
      <div class="docs-content prose dark:prose-invert max-w-none">
        <Content />
      </div>
      <DocsPrevNext class="mt-12" />
    </main>

    <!-- Right aside: Table of Contents -->
    <aside
      v-if="isWide"
      class="sticky top-14 h-[calc(100vh-3.5rem)] w-56 shrink-0 pl-4"
    >
      <DocsTableOfContents />
    </aside>
  </div>
</template>
