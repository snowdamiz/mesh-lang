<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useData } from 'vitepress'
import { isActive, type SidebarItem } from '../../composables/useSidebar'
import { useMenu } from '../../composables/useMenu'

defineProps<{ groups: SidebarItem[] }>()

const { page } = useData()
const { isOpen, panelId, onLinkClick } = useMenu('docs')
const nav = ref<HTMLElement>()

const current = (link?: string) => (isActive(page.value.relativePath, link) ? 'page' : undefined)

onMounted(() => {
  const el = nav.value
  const link = el?.querySelector<HTMLElement>('[aria-current="page"]')
  if (el && link && link.offsetTop > el.clientHeight * 0.6) el.scrollTop = link.offsetTop - el.clientHeight / 3
})
</script>

<template>
  <nav :id="panelId" ref="nav" class="sidebar" :class="{ open: isOpen }" aria-label="Docs">
    <div v-for="group in groups" :key="group.text" class="sidebar-group">
      <p class="sidebar-title">{{ group.text }}</p>
      <ul>
        <li v-for="item in group.items" :key="item.link">
          <a :href="item.link" :aria-current="current(item.link)" @click="onLinkClick">{{ item.text }}</a>
        </li>
      </ul>
    </div>
  </nav>
</template>
