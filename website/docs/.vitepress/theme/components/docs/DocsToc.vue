<script setup lang="ts">
import { computed } from 'vue'
import { useData } from 'vitepress'
import { useOutline, type OutlineItem } from '../../composables/useOutline'
import { useActiveHeading } from '../../composables/useActiveHeading'

const { theme } = useData()
const { headers } = useOutline()
const { activeId } = useActiveHeading()

const flatten = (items: OutlineItem[]): OutlineItem[] => items.flatMap((item) => [item, ...flatten(item.children)])
const items = computed(() => flatten(headers.value))

// The router scrolls to the heading; focus follows so the next Tab starts there.
function focusHeading(link: string) {
  document.getElementById(decodeURIComponent(link.slice(1)))?.focus({ preventScroll: true })
}
</script>

<template>
  <aside class="toc" aria-label="On this page">
    <template v-if="items.length">
      <p class="toc-title">{{ theme.outline?.label ?? 'On this page' }}</p>
      <ol>
        <li v-for="item in items" :key="item.link">
          <a :href="item.link" :class="{ sub: item.level > 2, on: activeId === item.link.slice(1) }" @click="focusHeading(item.link)">{{ item.title }}</a>
        </li>
      </ol>
    </template>
  </aside>
</template>
