<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { getHighlighter, highlightCode } from '@/composables/useShiki'

interface Feature {
  title: string
  description: string
  code: string
}

const features: Feature[] = [
  {
    title: 'Lightweight Actors',
    description:
      'Spawn millions of lightweight actors with crash isolation and supervision trees. Each actor has its own heap and message queue.',
    code: `actor Counter do
  def init() do
    0
  end

  def handle_cast(:increment, state) do
    state + 1
  end

  def handle_call(:get, _from, state) do
    {:reply, state, state}
  end
end

let pid = spawn(Counter)
cast(pid, :increment)
let count = call(pid, :get)`,
  },
  {
    title: 'Pattern Matching',
    description:
      'First-class pattern matching with exhaustiveness checking. Destructure any value -- structs, tuples, sum types, lists.',
    code: `fn describe(value) do
  match value do
    0 -> "zero"
    n when n > 0 -> "positive"
    n when n < 0 -> "negative"
  end
end

fn process(result) do
  match result do
    Ok(value) -> IO.puts("Got: \${value}")
    Err(msg) -> IO.puts("Error: \${msg}")
  end
end`,
  },
  {
    title: 'Type Inference',
    description:
      'Hindley-Milner type inference means you rarely write type annotations. The compiler catches errors at compile time.',
    code: `# Types are inferred -- no annotations needed
let name = "Mesh"
let numbers = [1, 2, 3, 4, 5]
let doubled = numbers
  |> List.map(fn(n) do n * 2 end)
  |> List.filter(fn(n) do n > 4 end)

# Structs with inferred field types
struct User do
  name :: String
  age :: Int
end

let user = User { name: "Alice", age: 30 }`,
  },
  {
    title: 'Pipe Operator',
    description:
      'Chain transformations naturally with the pipe operator. Data flows left to right, just like you read it.',
    code: `let result = "hello world"
  |> String.split(" ")
  |> List.map(fn(word) do
    String.upcase(word)
  end)
  |> String.join(", ")

# result == "HELLO, WORLD"`,
  },
]

const highlighted = ref<Record<number, string>>({})

onMounted(async () => {
  try {
    const hl = await getHighlighter()
    features.forEach((feature, index) => {
      highlighted.value[index] = highlightCode(hl, feature.code)
    })
  } catch {
    // Highlighting failed -- raw code fallback remains visible
  }
})
</script>

<template>
  <section class="border-t border-border py-20 md:py-28">
    <div class="mx-auto max-w-5xl px-4">
      <div class="text-center">
        <h2 class="text-3xl font-bold tracking-tight text-foreground sm:text-4xl" style="letter-spacing: -0.03em;">
          What makes Mesh special
        </h2>
        <p class="mx-auto mt-4 max-w-lg text-muted-foreground">
          A language designed for building reliable, concurrent systems with minimal boilerplate.
        </p>
      </div>

      <div class="mt-14 grid gap-5 md:grid-cols-2">
        <div
          v-for="(feature, index) in features"
          :key="feature.title"
          class="group rounded-xl border border-border bg-card p-6 transition-colors hover:border-foreground/20"
        >
          <h3 class="text-base font-semibold text-foreground tracking-tight">
            {{ feature.title }}
          </h3>
          <p class="mt-2 text-sm leading-relaxed text-muted-foreground">
            {{ feature.description }}
          </p>
          <div class="mt-4 overflow-hidden rounded-lg border border-border bg-muted/50">
            <div
              v-if="highlighted[index]"
              v-html="highlighted[index]"
              class="vp-code [&_pre]:p-4 [&_pre]:!bg-transparent [&_pre]:text-xs [&_pre]:leading-relaxed"
            />
            <pre
              v-else
              class="overflow-x-auto p-4 text-xs leading-relaxed text-foreground font-mono"
            ><code>{{ feature.code }}</code></pre>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>
