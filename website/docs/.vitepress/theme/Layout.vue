<script setup lang="ts">
import { computed, type Component } from 'vue'
import { useData } from 'vitepress'
import SiteNav from './components/SiteNav.vue'
import SiteFooter from './components/SiteFooter.vue'
import NotFoundPage from './components/NotFoundPage.vue'
import PlainPage from './components/PlainPage.vue'
import LandingPage from './components/landing/LandingPage.vue'
import DocsLayout from './components/docs/DocsLayout.vue'
import DocsMenuBar from './components/docs/DocsMenuBar.vue'
import { useSidebar } from './composables/useSidebar'
import { focusMain, useMenuEffects } from './composables/useMenu'

type Kind = 'home' | 'docs' | 'notFound' | 'page'

const LAYOUTS: Record<Kind, { view: Component; nav: 'site' | 'app'; footer: boolean }> = {
  home: { view: LandingPage, nav: 'site', footer: true },
  docs: { view: DocsLayout, nav: 'app', footer: false },
  notFound: { view: NotFoundPage, nav: 'site', footer: true },
  page: { view: PlainPage, nav: 'site', footer: true },
}

const { page, frontmatter } = useData()
const { hasSidebar } = useSidebar()
useMenuEffects()

const kind = computed<Kind>(() => {
  if (page.value.isNotFound) return 'notFound'
  if (frontmatter.value.layout === 'home') return 'home'
  return hasSidebar.value ? 'docs' : 'page'
})
const layout = computed(() => LAYOUTS[kind.value])
</script>

<template>
  <a class="skip" href="#main" @click="focusMain()">Skip to content</a>
  <SiteNav :variant="layout.nav" :home="kind === 'home'">
    <DocsMenuBar v-if="kind === 'docs'" />
  </SiteNav>
  <component :is="layout.view" />
  <SiteFooter v-if="layout.footer" />
</template>
