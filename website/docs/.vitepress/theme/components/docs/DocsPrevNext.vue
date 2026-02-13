<script setup lang="ts">
import { usePrevNext } from '@/composables/usePrevNext'
import { ChevronLeft, ChevronRight } from 'lucide-vue-next'

const prevNext = usePrevNext()
</script>

<template>
  <div class="not-prose flex justify-between border-t border-border pt-6">
    <!-- Previous link -->
    <a
      v-if="prevNext.prev?.link"
      :href="prevNext.prev.link"
      class="flex items-center gap-1 text-sm text-primary hover:text-foreground transition-colors"
    >
      <ChevronLeft class="size-4" />
      {{ prevNext.prev.text }}
    </a>
    <span v-else />

    <!-- Next link -->
    <a
      v-if="prevNext.next?.link"
      :href="prevNext.next.link"
      class="ml-auto flex items-center gap-1 text-sm text-primary hover:text-foreground transition-colors"
    >
      {{ prevNext.next.text }}
      <ChevronRight class="size-4" />
    </a>
  </div>
</template>
