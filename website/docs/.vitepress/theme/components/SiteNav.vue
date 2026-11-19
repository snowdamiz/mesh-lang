<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useData } from 'vitepress'
import VPNavBarSearch from 'vitepress/dist/client/theme-default/components/VPNavBarSearch.vue'
import lockup from '../assets/mesh-lockup.svg?raw'
import ThemeToggle from './ThemeToggle.vue'
import { NAV_LINKS, STATUS } from '../site'
import { useMenu } from '../composables/useMenu'

defineProps<{ variant: 'site' | 'app'; home: boolean }>()

const { page } = useData()
const { isOpen: menuOpen, panelId, toggle, onLinkClick } = useMenu('site')
const scrolled = ref(false)

const current = (section?: string) => (section && page.value.relativePath.startsWith(section) ? 'page' : undefined)

const onScroll = () => { scrolled.value = window.scrollY > 8 }

onMounted(() => {
  onScroll()
  window.addEventListener('scroll', onScroll, { passive: true })
})
onBeforeUnmount(() => window.removeEventListener('scroll', onScroll))
</script>

<template>
  <div class="site-top" :class="{ 'is-scrolled': scrolled }">
    <header class="nav" :class="{ 'nav--app': variant === 'app', open: menuOpen }">
      <div class="nav-in" :class="{ wrap: variant === 'site' }">
        <a class="brand" href="/" aria-label="Mesh home" v-html="lockup" />
        <nav :id="panelId" class="nav-links" aria-label="Primary">
          <a v-for="link in NAV_LINKS" :key="link.href" :href="link.href" :aria-current="current(link.section)" @click="onLinkClick">{{ link.text }}</a>
          <a class="nav-status" href="/docs/getting-started/" @click="onLinkClick"><i />{{ STATUS.label }}</a>
        </nav>
        <div class="nav-end">
          <VPNavBarSearch v-if="variant === 'app'" />
          <span v-if="variant === 'app'" class="status" :title="STATUS.detail"><i />{{ STATUS.label }}</span>
          <a v-else class="status" href="/docs/getting-started/" :title="STATUS.detail"><i />{{ STATUS.label }}</a>
          <ThemeToggle />
          <a v-if="variant === 'site'" class="btn btn-ink" :href="home ? '#install' : '/#install'">Install</a>
          <button type="button" class="menu-btn" aria-label="Site menu" :aria-controls="panelId" :aria-expanded="menuOpen" @click="toggle">
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M2 5h12M2 11h12" /></svg>
          </button>
        </div>
      </div>
    </header>
    <slot />
  </div>
</template>
