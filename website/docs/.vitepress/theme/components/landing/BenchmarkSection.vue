<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'

// benchmarks/RESULTS.md, "Isolated Peak Throughput", GET /text: every figure is from that one run
const rows = [
  { lang: 'Rust', rps: 46244, p50: 2.06, p99: 4.55 },
  { lang: 'Go', rps: 30306, p50: 2.95, p99: 8.51 },
  { lang: 'Mesh', rps: 29108, p50: 2.77, p99: 16.94 },
  { lang: 'Elixir', rps: 12441, p50: 6.74, p99: 25.14 },
]
const max = Math.max(...rows.map((r) => r.rps))

const chart = ref<HTMLElement>()
const shown = ref(false)
let io: IntersectionObserver | undefined

onMounted(() => {
  io = new IntersectionObserver(([entry]) => {
    if (entry.isIntersecting) {
      shown.value = true
      io?.disconnect()
    }
  }, { threshold: 0.3 })
  if (chart.value) io.observe(chart.value)
})
onUnmounted(() => io?.disconnect())
</script>

<template>
  <section class="l-section">
    <div class="l-pad grid gap-12 lg:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)] lg:gap-16">
      <div>
        <p class="l-label">Performance</p>
        <h2 class="l-h2 mt-5">Native speed, measured.</h2>
        <p class="l-lede mt-6 max-w-[27rem]">
          One minimal HTTP endpoint on dedicated 2&nbsp;vCPU Fly.io machines, 100 connections. A benchmark, not a
          promise about your app.
        </p>
        <a
          href="https://github.com/hyperpush-org/mesh-lang/blob/main/benchmarks/METHODOLOGY.md"
          target="_blank"
          rel="noopener"
          class="l-link mt-8 inline-block text-[15px]"
        >Read the methodology →</a>
      </div>

      <figure ref="chart" class="self-center">
        <table class="w-full border-collapse text-left font-mono text-[13px]">
          <caption class="sr-only">Requests per second and latency for GET /text</caption>
          <thead>
            <tr class="text-xs text-muted-foreground">
              <th scope="col" class="w-16 pb-3 font-normal"><span class="sr-only">Language</span></th>
              <th scope="col" class="pb-3 font-normal">requests / s</th>
              <th scope="col" class="w-14 pb-3 text-right font-normal sm:w-16">p50</th>
              <th scope="col" class="w-14 pb-3 text-right font-normal sm:w-16">p99</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="(row, i) in rows"
              :key="row.lang"
              class="border-t border-[var(--l-line)]"
              :class="row.lang === 'Mesh' ? 'text-foreground' : 'text-muted-foreground'"
            >
              <th scope="row" class="py-3.5 pr-3 font-normal" :class="{ 'font-semibold': row.lang === 'Mesh' }">
                {{ row.lang }}
              </th>
              <td class="py-3.5 pr-3">
                <div class="flex items-center gap-3">
                  <div class="h-2 flex-1">
                    <div
                      class="l-bar"
                      :class="{ 'is-mesh': row.lang === 'Mesh' }"
                      :style="{
                        width: `${(row.rps / max) * 100}%`,
                        transform: shown ? 'none' : 'scaleX(0)',
                        transitionDelay: `${i * 90}ms`,
                      }"
                    />
                  </div>
                  <span class="w-[4.5ch] shrink-0 text-right tabular-nums">{{ (row.rps / 1000).toFixed(1) }}k</span>
                </div>
              </td>
              <td class="py-3.5 text-right tabular-nums">{{ row.p50.toFixed(2) }}</td>
              <td class="py-3.5 text-right tabular-nums">{{ row.p99.toFixed(2) }}</td>
            </tr>
          </tbody>
        </table>
        <figcaption class="mt-4 font-mono text-xs text-muted-foreground">
          GET /text · latency in ms · 5 × 30 s runs after warmup, first run excluded
        </figcaption>
      </figure>
    </div>
  </section>
</template>
