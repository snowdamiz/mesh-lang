<script setup lang="ts">
import { useData } from 'vitepress'
import { useOutline } from '@/composables/useOutline'
import { ScrollArea } from '@/components/ui/scroll-area'
import DocsOutlineItem from './DocsOutlineItem.vue'

const { theme } = useData()
const { headers } = useOutline()

const label = theme.value.outline?.label ?? 'On this page'
</script>

<template>
  <nav v-if="headers.length > 0" class="py-4">
    <ScrollArea class="h-full">
      <div class="font-semibold text-sm mb-3">{{ label }}</div>
      <DocsOutlineItem :headers="headers" />
    </ScrollArea>
  </nav>
</template>
