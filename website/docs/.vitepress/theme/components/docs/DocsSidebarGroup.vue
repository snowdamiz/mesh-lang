<script setup lang="ts">
import { ref } from 'vue'
import type { SidebarItem } from '@/composables/useSidebar'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { ChevronRight } from 'lucide-vue-next'
import DocsSidebarItem from './DocsSidebarItem.vue'

const props = defineProps<{
  item: SidebarItem
}>()

const isOpen = ref(props.item.collapsed !== undefined ? !props.item.collapsed : true)
</script>

<template>
  <!-- Non-collapsible group (collapsed is undefined) -->
  <div v-if="item.collapsed === undefined" class="mb-4">
    <div class="font-semibold text-sm text-foreground mb-1 px-2">
      {{ item.text }}
    </div>
    <ul class="flex flex-col gap-0.5">
      <li v-for="child in item.items" :key="child.text">
        <DocsSidebarItem :item="child" />
      </li>
    </ul>
  </div>

  <!-- Collapsible group (collapsed is boolean) -->
  <Collapsible v-else v-model:open="isOpen" class="mb-4">
    <CollapsibleTrigger class="flex w-full items-center gap-1 px-2 py-1 font-semibold text-sm text-foreground hover:bg-accent/50 rounded-md transition-colors">
      <ChevronRight
        class="size-4 shrink-0 transition-transform duration-200"
        :class="{ 'rotate-90': isOpen }"
      />
      {{ item.text }}
    </CollapsibleTrigger>
    <CollapsibleContent>
      <ul class="flex flex-col gap-0.5 mt-1">
        <li v-for="child in item.items" :key="child.text">
          <DocsSidebarItem :item="child" />
        </li>
      </ul>
    </CollapsibleContent>
  </Collapsible>
</template>
