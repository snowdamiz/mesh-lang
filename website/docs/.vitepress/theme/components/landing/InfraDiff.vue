<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { getHighlighter, highlightCode } from '@/composables/useShiki'
import { useScrollReveal } from '@/composables/useScrollReveal'

const { observe } = useScrollReveal()
const root = ref<HTMLElement>()

const removed = [
  'docker-compose.redis.yml',
  'kafka/ (14 config files)',
  'k8s/work-queue-operator.yaml',
  'terraform/queues.tf',
  'retry + backoff glue code',
  'service-discovery sidecar',
]

const routerCode = `from Api.Health import handle_health
from Api.Todos import handle_create_todo,
  handle_get_todo, handle_list_todos

pub fn build_router() do
  HTTP.router()
    |> HTTP.on_get("/health", handle_health)
    |> HTTP.on_get("/todos",
         HTTP.clustered(handle_list_todos))
    |> HTTP.on_get("/todos/:id",
         HTTP.clustered(handle_get_todo))
    |> HTTP.on_post("/todos", handle_create_todo)
end`

const highlightedHtml = ref('')

onMounted(async () => {
  root.value?.querySelectorAll('.reveal, .reveal-zoom, .reveal-stagger').forEach((el) => observe(el))
  try {
    const hl = await getHighlighter()
    highlightedHtml.value = highlightCode(hl, routerCode)
  } catch {
    // Highlighting failed -- raw code fallback remains visible
  }
})
</script>

<template>
  <section class="mx-auto max-w-6xl px-4 py-20 sm:px-6 md:py-28">
    <div ref="root">
      <span class="l-eyebrow reveal">the infrastructure diff</span>

      <div class="reveal-stagger mt-6 grid gap-12 lg:grid-cols-[1fr_1.2fr] lg:gap-16">
        <!-- Copy + diff -->
        <div class="min-w-0">
          <h2 class="font-display text-4xl font-extrabold leading-[1.05] text-foreground sm:text-[2.75rem]">
            Delete the orchestration <em class="l-fancy">layer.</em>
          </h2>
          <p class="mt-5 max-w-md text-base leading-relaxed text-muted-foreground">
            Distributing work usually means adopting a queue, a broker, an operator, and the config to keep them alive.
            In Mesh the annotation <em class="not-italic font-mono text-foreground text-[0.9em]">@cluster</em> is the
            entire contract — the runtime owns placement, retries, and failover.
          </p>

          <div class="l-window mt-8">
            <div class="l-window-head">
              <span>infra.diff</span>
              <span class="shrink-0">−6 systems · +1 annotation</span>
            </div>
            <div class="px-5 py-4 font-mono text-[13px] leading-[2.1]">
              <div
                v-for="line in removed"
                :key="line"
                class="flex items-baseline gap-3 text-muted-foreground"
              >
                <span class="select-none font-bold" :style="{ color: 'var(--err)' }">−</span>
                <span class="truncate line-through decoration-[color:var(--err)]/40">{{ line }}</span>
              </div>
              <div class="mt-1 flex items-baseline gap-3 text-foreground">
                <span class="select-none font-bold" :style="{ color: 'var(--ok)' }">+</span>
                <span
                  class="rounded-full px-2.5 py-0.5 font-bold"
                  style="background: color-mix(in oklab, var(--ok) 14%, transparent);"
                >@cluster</span>
              </div>
            </div>
          </div>
        </div>

        <!-- The code that remains -->
        <div class="min-w-0">
          <div class="l-window flex h-full flex-col">
            <div class="l-window-head">
              <span class="l-window-dots" aria-hidden="true"><span /><span /><span /></span>
              <span class="min-w-0 truncate">api/router.mpl — everything that's left</span>
              <span class="l-chip shrink-0 !px-2.5 !py-0.5 !text-[10px] !text-foreground/70">mesh</span>
            </div>
            <div
              v-if="highlightedHtml"
              v-html="highlightedHtml"
              class="vp-code landing-code w-full max-w-full flex-1 font-mono"
            />
            <pre
              v-else
              class="max-w-full flex-1 overflow-x-auto px-6 py-4 font-mono text-[0.8125rem] leading-[1.9] text-foreground"><code>{{ routerCode }}</code></pre>
            <div class="border-t border-border px-5 py-2.5 font-mono text-[11px] text-muted-foreground">
              wrap a route in <span class="text-foreground">HTTP.clustered(...)</span> — the fleet takes it from there
            </div>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>
