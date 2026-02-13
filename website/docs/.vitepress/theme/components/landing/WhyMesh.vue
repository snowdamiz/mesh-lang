<script setup lang="ts">
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
  <section class="border-t border-border py-16 md:py-24">
    <div class="mx-auto max-w-4xl px-4">
      <h2 class="text-center text-3xl font-bold tracking-tight text-foreground">
        Why Mesh?
      </h2>
      <p class="mx-auto mt-4 max-w-2xl text-center text-muted-foreground">
        Mesh sits at a unique intersection in the programming language landscape.
      </p>

      <div class="mt-12 grid gap-6 md:grid-cols-3">
        <div
          v-for="comparison in comparisons"
          :key="comparison.language"
          class="rounded-lg border border-border p-6"
        >
          <h3 class="text-lg font-semibold text-foreground">
            {{ comparison.label }}
          </h3>
          <p class="mt-3 text-sm leading-relaxed text-muted-foreground">
            {{ comparison.description }}
          </p>
        </div>
      </div>

      <p class="mt-12 text-center text-sm text-muted-foreground">
        Mesh is open source and under active development.
        <a
          href="https://github.com/user/mesh"
          class="text-foreground underline underline-offset-4 transition-colors hover:text-muted-foreground"
        >
          View on GitHub
        </a>
      </p>
    </div>
  </section>
</template>
