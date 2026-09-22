<script setup lang="ts">
import { computed, type Component } from 'vue'
import { useData } from 'vitepress'
import { isActive, type SidebarItem } from '@/composables/useSidebar'
import {
  Blocks, BookOpen, ClipboardList, Code2, Container, Database, FileText, FlaskConical, Globe,
  Library, ListTree, Network, PackageOpen, Repeat, Shapes, ShieldCheck, Workflow, Wrench,
} from 'lucide-vue-next'

// The `icon` names used by the sidebar in config.mts. Importing the whole
// icon set instead put all of Lucide into the theme chunk every page loads.
const icons: Record<string, Component> = {
  Blocks, BookOpen, ClipboardList, Code2, Container, Database, FileText, FlaskConical, Globe,
  Library, ListTree, Network, PackageOpen, Repeat, Shapes, ShieldCheck, Workflow, Wrench,
}

const props = defineProps<{
  item: SidebarItem
}>()

const { page } = useData()

const active = computed(() => isActive(page.value.relativePath, props.item.link))

const iconComponent = computed(() => (props.item.icon ? icons[props.item.icon] ?? null : null))
</script>

<template>
  <div>
    <a
      :href="item.link"
      class="flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-[13px] transition-colors"
      :class="[
        active
          ? 'bg-brand/10 text-brand font-semibold'
          : 'text-muted-foreground hover:text-foreground hover:bg-accent',
      ]"
    >
      <component
        v-if="iconComponent"
        :is="iconComponent"
        class="size-3.5 shrink-0"
      />
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
