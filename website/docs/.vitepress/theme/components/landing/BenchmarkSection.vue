<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useScrollReveal } from '@/composables/useScrollReveal'

const { observe } = useScrollReveal()
const root = ref<HTMLElement>()
const barsVisible = ref(false)

// Real benchmark data from benchmarks/RESULTS.md — dedicated Fly.io machines
const throughput = [
  { lang: 'Rust', value: 46244, mesh: false },
  { lang: 'Go', value: 30306, mesh: false },
  { lang: 'Mesh', value: 29108, mesh: true },
  { lang: 'Elixir', value: 12441, mesh: false },
]

const latency = [
  { lang: 'Rust', p50: 2.06, p99: 4.55, mesh: false },
  { lang: 'Mesh', p50: 2.77, p99: 16.94, mesh: true },
  { lang: 'Go', p50: 2.95, p99: 8.51, mesh: false },
  { lang: 'Elixir', p50: 6.74, p99: 25.14, mesh: false },
]

const memory = [
  { lang: 'Go', value: 1.5, mesh: false },
  { lang: 'Elixir', value: 1.6, mesh: false },
  { lang: 'Rust', value: 3.4, mesh: false },
  { lang: 'Mesh', value: 4.9, mesh: true },
]

const maxThroughput = Math.max(...throughput.map((d) => d.value))
const maxLatency = Math.max(...latency.map((d) => d.p99))
const maxMemory = Math.max(...memory.map((d) => d.value))

function barWidth(ratio: number, i: number) {
  return {
    width: barsVisible.value ? `${ratio * 100}%` : '0%',
    transitionDelay: `${i * 120}ms`,
  }
}

onMounted(() => {
  if (root.value) {
    root.value?.querySelectorAll('.reveal, .reveal-zoom, .reveal-stagger').forEach((el) => observe(el))
    const io = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting) {
          barsVisible.value = true
          io.disconnect()
        }
      },
      { threshold: 0.15 },
    )
    io.observe(root.value)
  }
})
</script>

<template>
  <section class="mx-auto max-w-6xl px-4 py-20 sm:px-6 md:py-28">
    <div ref="root">
      <span class="l-eyebrow reveal">measured, not marketed</span>

      <!-- Headline + the number that matters -->
      <div class="reveal reveal-d1 mt-6 grid gap-10 lg:grid-cols-[1fr_auto] lg:items-end lg:gap-16">
        <div>
          <h2 class="font-display text-4xl font-extrabold leading-[1.05] text-foreground sm:text-[2.75rem]">
            Native speed.<br /><em class="l-fancy">Honest</em> numbers.
          </h2>
          <p class="mt-5 max-w-md text-base leading-relaxed text-muted-foreground">
            Benchmarked on dedicated Fly.io machines — 2 vCPU, 4 GB RAM, same region, private network. The comparison
            shown here is one minimal HTTP/1.1 endpoint, not a general application-performance claim.
          </p>
        </div>
        <div class="flex items-baseline gap-4 lg:flex-col lg:items-end lg:gap-1 lg:text-right">
          <div class="font-display l-grad-text text-6xl font-extrabold tracking-tight sm:text-7xl">2.3×</div>
          <div class="max-w-[16rem] font-mono text-[11.5px] leading-relaxed tracking-[0.06em] text-muted-foreground">
            Elixir's throughput,<br class="hidden lg:block" />
            with 59% lower median latency
          </div>
        </div>
      </div>

      <!-- Three metric cards -->
      <div class="reveal-stagger mt-12 grid gap-4 lg:grid-cols-3">
        <!-- Throughput -->
        <div class="l-card p-6 sm:p-7">
          <div class="flex items-baseline justify-between gap-2">
            <span class="font-mono text-xs font-semibold tracking-[0.08em] text-foreground">throughput</span>
            <span class="font-mono text-[10px] text-muted-foreground">req/s · higher ↑</span>
          </div>
          <div class="mt-6 space-y-4.5">
            <div v-for="(item, i) in throughput" :key="item.lang">
              <div class="mb-1.5 flex items-baseline justify-between font-mono text-xs">
                <span :class="item.mesh ? 'font-bold text-foreground' : 'text-muted-foreground'">{{ item.lang }}</span>
                <span class="tabular-nums" :class="item.mesh ? 'font-bold text-foreground' : 'text-muted-foreground'">
                  {{ item.value.toLocaleString('en-US') }}
                </span>
              </div>
              <div class="l-bar-track">
                <div class="l-bar" :class="item.mesh ? 'l-bar-mesh' : 'l-bar-other'" :style="barWidth(item.value / maxThroughput, i)" />
              </div>
            </div>
          </div>
        </div>

        <!-- Latency -->
        <div class="l-card p-6 sm:p-7">
          <div class="flex items-baseline justify-between gap-2">
            <span class="font-mono text-xs font-semibold tracking-[0.08em] text-foreground">latency</span>
            <span class="font-mono text-[10px] text-muted-foreground">p50 / p99 ms · lower ↓</span>
          </div>
          <div class="mt-6 space-y-4.5">
            <div v-for="(item, i) in latency" :key="item.lang">
              <div class="mb-1.5 flex items-baseline justify-between font-mono text-xs">
                <span :class="item.mesh ? 'font-bold text-foreground' : 'text-muted-foreground'">{{ item.lang }}</span>
                <span class="tabular-nums" :class="item.mesh ? 'font-bold text-foreground' : 'text-muted-foreground'">
                  {{ item.p50.toFixed(2) }} / {{ item.p99.toFixed(2) }}
                </span>
              </div>
              <div class="l-bar-track">
                <!-- p50 bar -->
                <div class="l-bar" :class="item.mesh ? 'l-bar-mesh' : 'l-bar-other'" :style="barWidth(item.p50 / maxLatency, i)" />
                <!-- p99 marker -->
                <div
                  class="absolute top-1/2 size-2 -translate-y-1/2 rounded-full bg-foreground/50"
                  :style="{ left: `calc(${(item.p99 / maxLatency) * 100}% - 4px)` }"
                />
              </div>
            </div>
          </div>
          <p class="mt-4 font-mono text-[10px] text-muted-foreground">bar = p50 · dot = p99</p>
        </div>

        <!-- Memory -->
        <div class="l-card p-6 sm:p-7">
          <div class="flex items-baseline justify-between gap-2">
            <span class="font-mono text-xs font-semibold tracking-[0.08em] text-foreground">memory</span>
            <span class="font-mono text-[10px] text-muted-foreground">MB startup baseline · lower ↓</span>
          </div>
          <div class="mt-6 space-y-4.5">
            <div v-for="(item, i) in memory" :key="item.lang">
              <div class="mb-1.5 flex items-baseline justify-between font-mono text-xs">
                <span :class="item.mesh ? 'font-bold text-foreground' : 'text-muted-foreground'">{{ item.lang }}</span>
                <span class="tabular-nums" :class="item.mesh ? 'font-bold text-foreground' : 'text-muted-foreground'">
                  {{ item.value.toFixed(1) }}
                </span>
              </div>
              <div class="l-bar-track">
                <div class="l-bar" :class="item.mesh ? 'l-bar-mesh' : 'l-bar-other'" :style="barWidth(item.value / maxMemory, i)" />
              </div>
            </div>
          </div>
          <p class="mt-4 font-mono text-[10px] text-muted-foreground">recorded before load</p>
        </div>
      </div>

      <!-- Methodology -->
      <p class="reveal mt-6 font-mono text-[11px] leading-relaxed text-muted-foreground">
        GET /text · Fly.io performance-2x · 100 connections · 30s warmup + 5×30s runs · run 1 excluded ·
        <a
          href="https://github.com/hyperpush-org/mesh-lang/blob/main/benchmarks/METHODOLOGY.md"
          class="text-foreground underline decoration-border underline-offset-4 transition-colors hover:decoration-[var(--l-accent)]"
        >full methodology →</a>
      </p>
    </div>
  </section>
</template>
