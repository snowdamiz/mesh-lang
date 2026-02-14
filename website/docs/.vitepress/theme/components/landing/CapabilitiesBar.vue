<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { Zap, Cpu, Shield, Package } from 'lucide-vue-next'
import { useScrollReveal } from '@/composables/useScrollReveal'

const { observe } = useScrollReveal()
const items = ref<HTMLElement[]>([])

const capabilities = [
  { icon: Zap, stat: '100K+ Actors', description: 'Lightweight concurrency' },
  { icon: Cpu, stat: 'LLVM Native', description: 'Compiled binaries' },
  { icon: Shield, stat: 'Type-Safe', description: 'Full inference' },
  { icon: Package, stat: 'Batteries Included', description: 'HTTP, DB, WebSockets' },
]

onMounted(() => {
  items.value.forEach((el) => {
    if (el) observe(el)
  })
})
</script>

<template>
  <section class="border-y border-border bg-muted/30 py-12 md:py-16">
    <div class="mx-auto max-w-5xl px-4">
      <div class="grid grid-cols-2 gap-8 md:grid-cols-4">
        <div
          v-for="(cap, index) in capabilities"
          :key="cap.stat"
          :ref="(el) => { if (el) items[index] = el as HTMLElement }"
          class="reveal flex flex-col items-center text-center"
          :class="`reveal-delay-${index + 1}`"
        >
          <div class="mb-3 flex size-10 items-center justify-center rounded-lg bg-muted text-foreground">
            <component :is="cap.icon" class="size-5" />
          </div>
          <div class="text-lg font-bold text-foreground sm:text-xl">{{ cap.stat }}</div>
          <div class="mt-0.5 text-sm text-muted-foreground">{{ cap.description }}</div>
        </div>
      </div>
    </div>
  </section>
</template>
