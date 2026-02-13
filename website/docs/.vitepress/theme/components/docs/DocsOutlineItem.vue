<script setup lang="ts">
import type { OutlineItem } from '@/composables/useOutline'

defineProps<{
  headers: OutlineItem[]
  root?: boolean
}>()
</script>

<template>
  <ul class="border-l border-border">
    <li v-for="item in headers" :key="item.link">
      <a
        :href="item.link"
        class="block py-1 pl-3 text-[13px] text-muted-foreground hover:text-foreground transition-colors -ml-px border-l border-transparent hover:border-foreground"
      >
        {{ item.title }}
      </a>
      <DocsOutlineItem
        v-if="item.children?.length"
        :headers="item.children"
        :root="false"
        class="ml-3"
      />
    </li>
  </ul>
</template>
