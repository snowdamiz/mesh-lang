<script setup lang="ts">
import { useData } from 'vitepress'
import NavBar from './components/NavBar.vue'
import LandingPage from './components/landing/LandingPage.vue'
import DocsLayout from './components/docs/DocsLayout.vue'
import { useSidebar } from '@/composables/useSidebar'

const { frontmatter } = useData()
const { hasSidebar } = useSidebar()
</script>

<template>
  <div class="min-h-screen bg-background text-foreground">
    <NavBar />
    <LandingPage v-if="frontmatter.layout === 'home'" />
    <DocsLayout v-else-if="hasSidebar" />
    <main v-else class="mx-auto max-w-4xl px-4 py-8">
      <Content />
    </main>
  </div>
</template>
