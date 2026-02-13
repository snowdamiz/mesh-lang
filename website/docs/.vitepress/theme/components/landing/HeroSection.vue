<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { Button } from '@/components/ui/button'
import { getHighlighter, highlightCode } from '@/composables/useShiki'

const highlightedHtml = ref('')

const heroCode = `# A simple HTTP server with actors
module HttpExample

pub fn main() do
  let router = HTTP.new()
    |> HTTP.on_get("/", fn(req) do
      HTTP.json(200, %{"message": "Hello, Mesh!"})
    end)
    |> HTTP.on_get("/users/:id", fn(req) do
      let id = Request.param(req, "id")
      HTTP.json(200, %{"user": id})
    end)

  HTTP.serve(router, 8080)
  IO.puts("Server running on :8080")
end`

onMounted(async () => {
  try {
    const hl = await getHighlighter()
    highlightedHtml.value = highlightCode(hl, heroCode)
  } catch {
    // Highlighting failed -- raw code fallback remains visible
  }
})
</script>

<template>
  <section class="py-16 md:py-24">
    <div class="mx-auto max-w-4xl px-4 text-center">
      <h1 class="text-4xl font-bold tracking-tight text-foreground md:text-6xl">
        Expressive. Concurrent. Type-safe.
      </h1>
      <p class="mx-auto mt-6 max-w-2xl text-lg text-muted-foreground md:text-xl">
        Mesh combines Elixir-style concurrency with static type inference,
        compiling to fast native binaries.
      </p>

      <div class="mt-8 flex items-center justify-center gap-4">
        <Button as="a" href="/docs/getting-started/" size="lg">
          Get Started
        </Button>
        <a
          href="https://github.com/user/mesh"
          class="text-sm text-muted-foreground transition-colors hover:text-foreground"
        >
          View on GitHub
        </a>
      </div>

      <div class="mx-auto mt-10 max-w-2xl overflow-hidden rounded-lg border border-border bg-muted text-left">
        <div v-if="highlightedHtml" v-html="highlightedHtml" class="vp-code [&_pre]:p-4 [&_pre]:!bg-transparent" />
        <pre v-else class="overflow-x-auto p-4 text-sm leading-relaxed text-foreground"><code>{{ heroCode }}</code></pre>
      </div>
    </div>
  </section>
</template>
