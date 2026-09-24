<script setup lang="ts">
import { ref, watch } from 'vue'
import { Content, useRoute } from 'vitepress'
import { useSidebar } from '@/composables/useSidebar'
import { useMediaQuery } from '@vueuse/core'
import DocsSidebar from './DocsSidebar.vue'
import DocsTableOfContents from './DocsTableOfContents.vue'
import DocsPrevNext from './DocsPrevNext.vue'
import DocsEditLink from './DocsEditLink.vue'
import DocsLastUpdated from './DocsLastUpdated.vue'
import DocsVersionBadge from './DocsVersionBadge.vue'
import MobileSidebar from './MobileSidebar.vue'

const { sidebar, hasSidebar, close } = useSidebar()
// Layout switches in CSS so server and client render the same tree; this only
// closes the mobile sheet if the window grows past the breakpoint.
const isDesktop = useMediaQuery('(min-width: 960px)')
watch(isDesktop, (desktop) => { if (desktop) close() })

const route = useRoute()
const contentKey = ref(0)

watch(() => route.path, () => {
  contentKey.value++
})
</script>

<template>
  <div class="relative mx-auto flex max-w-[90rem]">
    <!-- Desktop sidebar -->
    <aside
      v-if="hasSidebar"
      class="hidden min-[960px]:block sticky top-14 h-[calc(100vh-3.5rem)] w-64 shrink-0 border-r border-border"
    >
      <DocsSidebar :items="sidebar" />
    </aside>

    <!-- Mobile sidebar -->
    <MobileSidebar v-if="hasSidebar" :items="sidebar" />

    <!-- Main content -->
    <main class="min-w-0 flex-1 px-6 py-12 lg:px-12">
      <div class="mx-auto max-w-3xl">
        <div :key="contentKey" class="docs-content prose dark:prose-invert max-w-none animate-fade-in">
          <Content />
        </div>
        <div class="mt-12 flex flex-wrap items-center justify-between gap-4 border-t border-border pt-5 not-prose">
          <DocsEditLink />
          <div class="flex items-center gap-3">
            <DocsVersionBadge />
            <DocsLastUpdated />
          </div>
        </div>
        <DocsPrevNext class="mt-8" />
      </div>
    </main>

    <!-- Right aside: Table of Contents -->
    <aside
      class="hidden xl:block sticky top-14 h-[calc(100vh-3.5rem)] w-56 shrink-0"
    >
      <DocsTableOfContents />
    </aside>
  </div>
</template>
