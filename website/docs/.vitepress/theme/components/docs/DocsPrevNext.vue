<script setup lang="ts">
import { usePrevNext } from '@/composables/usePrevNext'
import { ChevronLeft, ChevronRight } from 'lucide-vue-next'

const prevNext = usePrevNext()
</script>

<template>
  <div class="not-prose flex justify-between gap-4 border-t border-border pt-6">
    <!-- Previous link -->
    <a
      v-if="prevNext.prev?.link"
      :href="prevNext.prev.link"
      class="group flex items-center gap-2 rounded-lg border border-border px-4 py-3 text-sm transition-colors hover:border-foreground/20 hover:bg-accent"
    >
      <ChevronLeft class="size-4 text-muted-foreground group-hover:text-foreground transition-colors" />
      <span class="text-muted-foreground group-hover:text-foreground transition-colors">{{ prevNext.prev.text }}</span>
    </a>
    <span v-else />

    <!-- Next link -->
    <a
      v-if="prevNext.next?.link"
      :href="prevNext.next.link"
      class="group ml-auto flex items-center gap-2 rounded-lg border border-border px-4 py-3 text-sm transition-colors hover:border-foreground/20 hover:bg-accent"
    >
      <span class="text-muted-foreground group-hover:text-foreground transition-colors">{{ prevNext.next.text }}</span>
      <ChevronRight class="size-4 text-muted-foreground group-hover:text-foreground transition-colors" />
    </a>
  </div>
</template>
