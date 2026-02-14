<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { Droplets, Cog, Gauge } from 'lucide-vue-next'
import { useScrollReveal } from '@/composables/useScrollReveal'

const { observe } = useScrollReveal()
const cards = ref<HTMLElement[]>([])

const comparisons = [
  {
    icon: Droplets,
    label: 'vs Elixir',
    description:
      'Mesh shares Elixir\'s actor model and let-it-crash philosophy, but adds static types with full inference. No runtime type errors, no dialyzer setup, and it compiles to native binaries instead of running on the BEAM.',
  },
  {
    icon: Cog,
    label: 'vs Rust',
    description:
      'Mesh provides Rust-level native performance without borrow checking complexity. Mesh uses per-actor garbage collection instead of ownership, making concurrent code dramatically simpler to write.',
  },
  {
    icon: Gauge,
    label: 'vs Go',
    description:
      'Like Go, Mesh compiles to fast native binaries with lightweight concurrency. But Mesh adds pattern matching, algebraic types, type inference, and supervision trees — making it more expressive and fault-tolerant.',
  },
]

onMounted(() => {
  cards.value.forEach((el) => {
    if (el) observe(el)
  })
})
</script>

<template>
  <section class="border-t border-border py-20 md:py-28">
    <div class="mx-auto max-w-5xl px-4">
      <!-- Section header -->
      <div class="text-center">
        <div class="text-sm font-mono uppercase tracking-widest text-muted-foreground">Comparison</div>
        <h2 class="mt-3 text-3xl font-bold tracking-tight text-foreground sm:text-4xl lg:text-5xl">
          Why Mesh?
        </h2>
        <p class="mx-auto mt-4 max-w-lg text-lg text-muted-foreground">
          Mesh sits at a unique intersection in the programming language landscape.
        </p>
      </div>

      <div class="mt-14 grid gap-5 md:grid-cols-3">
        <div
          v-for="(comparison, index) in comparisons"
          :key="comparison.label"
          :ref="(el) => { if (el) cards[index] = el as HTMLElement }"
          class="reveal rounded-xl border border-foreground/10 bg-card p-8 transition-all duration-300 hover:-translate-y-0.5 hover:border-foreground/30 hover:shadow-lg"
          :class="`reveal-delay-${index + 1}`"
        >
          <div class="mb-4 flex size-11 items-center justify-center rounded-xl bg-muted text-foreground">
            <component :is="comparison.icon" class="size-5" />
          </div>
          <div class="inline-flex items-center rounded-md bg-muted px-3 py-1.5 text-sm font-bold text-foreground">
            {{ comparison.label }}
          </div>
          <p class="mt-4 text-base leading-relaxed text-muted-foreground">
            {{ comparison.description }}
          </p>
        </div>
      </div>
    </div>
  </section>
</template>
