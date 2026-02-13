<script setup lang="ts">
import type { OutlineItem } from '@/composables/useOutline'

defineProps<{
  headers: OutlineItem[]
  root?: boolean
}>()
</script>

<template>
  <ul>
    <li v-for="item in headers" :key="item.link">
      <a
        :href="item.link"
        class="block py-1 text-sm text-muted-foreground hover:text-foreground transition-colors"
      >
        {{ item.title }}
      </a>
      <DocsOutlineItem
        v-if="item.children?.length"
        :headers="item.children"
        :root="false"
        class="pl-3"
      />
    </li>
  </ul>
</template>
