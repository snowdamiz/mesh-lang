<script setup lang="ts">
import { computed } from 'vue'
import { useData } from 'vitepress'
import { isActive, type SidebarItem } from '@/composables/useSidebar'

const props = defineProps<{
  item: SidebarItem
}>()

const { page } = useData()

const active = computed(() => isActive(page.value.relativePath, props.item.link))
</script>

<template>
  <div>
    <a
      :href="item.link"
      class="block rounded-md px-2 py-1.5 text-[13px] transition-colors"
      :class="[
        active
          ? 'bg-foreground text-background font-medium'
          : 'text-muted-foreground hover:text-foreground hover:bg-accent',
      ]"
    >
      {{ item.text }}
    </a>
    <!-- Recursive children with left padding -->
    <ul v-if="item.items?.length" class="flex flex-col gap-0.5 pl-3 mt-0.5">
      <li v-for="child in item.items" :key="child.text">
        <DocsSidebarItem :item="child" />
      </li>
    </ul>
  </div>
</template>
