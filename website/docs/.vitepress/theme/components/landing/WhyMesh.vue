<script setup lang="ts">
import { ArrowRight } from 'lucide-vue-next'

const comparisons = [
  {
    language: 'Elixir',
    label: 'vs Elixir',
    description:
      'Mesh shares Elixir\'s actor model and let-it-crash philosophy, but adds static types with full inference. No runtime type errors, no dialyzer setup, and it compiles to native binaries instead of running on the BEAM.',
  },
  {
    language: 'Rust',
    label: 'vs Rust',
    description:
      'Mesh provides Rust-level native performance without borrow checking complexity. Mesh uses per-actor garbage collection instead of ownership, making concurrent code dramatically simpler to write.',
  },
  {
    language: 'Go',
    label: 'vs Go',
    description:
      'Like Go, Mesh compiles to fast native binaries with lightweight concurrency. But Mesh adds pattern matching, algebraic types, type inference, and supervision trees -- making it more expressive and fault-tolerant.',
  },
]
</script>

<template>
  <section class="border-t border-border py-20 md:py-28">
    <div class="mx-auto max-w-5xl px-4">
      <div class="text-center">
        <h2 class="text-3xl font-bold tracking-tight text-foreground sm:text-4xl" style="letter-spacing: -0.03em;">
          Why Mesh?
        </h2>
        <p class="mx-auto mt-4 max-w-lg text-muted-foreground">
          Mesh sits at a unique intersection in the programming language landscape.
        </p>
      </div>

      <div class="mt-14 grid gap-5 md:grid-cols-3">
        <div
          v-for="comparison in comparisons"
          :key="comparison.language"
          class="rounded-xl border border-border bg-card p-6 transition-colors hover:border-foreground/20"
        >
          <div class="inline-flex items-center rounded-md bg-muted px-2.5 py-1 text-xs font-medium text-foreground">
            {{ comparison.label }}
          </div>
          <p class="mt-4 text-sm leading-relaxed text-muted-foreground">
            {{ comparison.description }}
          </p>
        </div>
      </div>

      <!-- Footer CTA -->
      <div class="mt-20 text-center">
        <div class="inline-flex flex-col items-center gap-4">
          <p class="text-sm text-muted-foreground">
            Mesh is open source and under active development.
          </p>
          <a
            href="https://github.com/user/mesh"
            class="inline-flex items-center gap-1.5 text-sm font-medium text-foreground transition-colors hover:text-muted-foreground"
          >
            View on GitHub
            <ArrowRight class="size-3.5" />
          </a>
        </div>
      </div>
    </div>
  </section>
</template>
