<script setup lang="ts">
import type { SidebarItem } from '@/composables/useSidebar'
import { ScrollArea } from '@/components/ui/scroll-area'
import DocsSidebarGroup from './DocsSidebarGroup.vue'
import DocsSidebarItem from './DocsSidebarItem.vue'

defineProps<{
  items: SidebarItem[]
}>()
</script>

<template>
  <ScrollArea class="h-full py-4 px-3">
    <template v-for="item in items" :key="item.text">
      <DocsSidebarGroup v-if="item.items" :item="item" />
      <DocsSidebarItem v-else :item="item" />
    </template>
  </ScrollArea>
</template>
