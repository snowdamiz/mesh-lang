<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { Button } from '@/components/ui/button'
import { getHighlighter, highlightCode } from '@/composables/useShiki'
import { ArrowRight } from 'lucide-vue-next'

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
  <section class="relative overflow-hidden">
    <!-- Subtle grid pattern background -->
    <div class="absolute inset-0 bg-[linear-gradient(to_right,var(--border)_1px,transparent_1px),linear-gradient(to_bottom,var(--border)_1px,transparent_1px)] bg-[size:4rem_4rem] [mask-image:radial-gradient(ellipse_60%_50%_at_50%_0%,#000_70%,transparent_100%)]" />

    <div class="relative mx-auto max-w-5xl px-4 pt-20 pb-16 md:pt-32 md:pb-24">
      <div class="text-center">
        <!-- Version badge -->
        <div class="mb-8 inline-flex items-center gap-2 rounded-full border border-border bg-background px-3.5 py-1.5 text-xs font-medium text-muted-foreground">
          <span class="inline-block size-1.5 rounded-full bg-foreground" />
          Now in development &mdash; v0.1.0
        </div>

        <h1 class="text-5xl font-bold tracking-tight text-foreground sm:text-6xl lg:text-7xl" style="letter-spacing: -0.035em; line-height: 1.05;">
          Expressive. Concurrent.<br />Type-safe.
        </h1>

        <p class="mx-auto mt-6 max-w-xl text-base text-muted-foreground sm:text-lg" style="line-height: 1.7;">
          Mesh combines Elixir-style concurrency with static type inference,
          compiling to fast native binaries.
        </p>

        <div class="mt-10 flex items-center justify-center gap-3">
          <Button as="a" href="/docs/getting-started/" size="lg" class="h-11 px-6 text-sm font-medium">
            Get Started
            <ArrowRight class="ml-1 size-4" />
          </Button>
          <Button as="a" href="https://github.com/user/mesh" variant="outline" size="lg" class="h-11 px-6 text-sm font-medium">
            View on GitHub
          </Button>
        </div>
      </div>

      <!-- Hero code block -->
      <div class="mx-auto mt-16 max-w-2xl">
        <div class="overflow-hidden rounded-xl border border-border bg-muted/50 shadow-sm">
          <!-- Code block header -->
          <div class="flex items-center gap-2 border-b border-border px-4 py-3">
            <div class="flex gap-1.5">
              <div class="size-2.5 rounded-full bg-border" />
              <div class="size-2.5 rounded-full bg-border" />
              <div class="size-2.5 rounded-full bg-border" />
            </div>
            <span class="ml-2 text-xs text-muted-foreground font-medium">example.mesh</span>
          </div>
          <!-- Code content -->
          <div v-if="highlightedHtml" v-html="highlightedHtml" class="vp-code [&_pre]:px-5 [&_pre]:py-4 [&_pre]:!bg-transparent" />
          <pre v-else class="overflow-x-auto px-5 py-4 text-sm leading-relaxed text-foreground font-mono"><code>{{ heroCode }}</code></pre>
        </div>
      </div>
    </div>
  </section>
</template>
