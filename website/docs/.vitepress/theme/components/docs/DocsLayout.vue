<script setup lang="ts">
import { computed } from 'vue'
import { useData } from 'vitepress'
import { isActive, useSidebar } from '../../composables/useSidebar'
import { useMenu } from '../../composables/useMenu'
import DocsSidebar from './DocsSidebar.vue'
import DocsToc from './DocsToc.vue'
import DocsPager from './DocsPager.vue'

const { page, theme } = useData()
const { sidebar } = useSidebar()
const { isOpen: menuOpen } = useMenu('docs')

const group = computed(() =>
  sidebar.value.find((g) => g.items?.some((item) => isActive(page.value.relativePath, item.link))),
)
const editLink = computed(() => {
  const { editLink } = theme.value
  if (!editLink?.pattern) return null
  return { url: editLink.pattern.replace(':path', page.value.relativePath), text: editLink.text ?? 'Edit this page' }
})
</script>

<template>
  <div class="docs">
    <DocsSidebar :groups="sidebar" />
    <main id="main" class="article" :inert="menuOpen">
      <div class="article-in">
        <p v-if="group" class="article-kicker">{{ group.text }}</p>
        <div class="prose docs-content"><Content /></div>
        <footer class="article-foot">
          <a v-if="editLink" class="edit-link" :href="editLink.url" target="_blank" rel="noopener">{{ editLink.text }}</a>
          <DocsPager />
        </footer>
      </div>
    </main>
    <DocsToc :inert="menuOpen" />
  </div>
</template>
