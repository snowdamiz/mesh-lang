<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useScrollReveal } from '@/composables/useScrollReveal'

type Mark = 'yes' | 'partial' | 'no'

const { observe } = useScrollReveal()
const root = ref<HTMLElement>()

const columns = ['Mesh', 'Elixir', 'Go', 'Node.js']

const rows: Array<{ capability: string; marks: Mark[] }> = [
  { capability: 'Static types, fully inferred', marks: ['yes', 'no', 'partial', 'partial'] },
  { capability: 'Compiles to a native binary', marks: ['yes', 'no', 'yes', 'no'] },
  { capability: 'Actor-model concurrency', marks: ['yes', 'yes', 'partial', 'no'] },
  { capability: 'Distribution in the language', marks: ['yes', 'partial', 'no', 'no'] },
  { capability: 'Runtime-owned failover', marks: ['yes', 'partial', 'no', 'no'] },
  { capability: 'Server stdlib — HTTP · DB · WS', marks: ['yes', 'partial', 'partial', 'no'] },
]

const notes = [
  {
    label: 'vs Elixir',
    text: 'Same actor model and let-it-crash philosophy, plus static inference. No Dialyzer setup, no BEAM — native binaries.',
  },
  {
    label: 'vs Go',
    text: 'Goroutines are fast, but distribution still means Redis, queues, or external systems. In Mesh it\'s @cluster — zero infrastructure.',
  },
  {
    label: 'vs Node.js',
    text: 'True multi-core actors and multi-node distribution, with type safety and no build-step toolchain to maintain.',
  },
]

const glyph: Record<Mark, string> = { yes: '✓', partial: '~', no: '—' }
const markClass: Record<Mark, string> = { yes: 'l-mark-yes', partial: 'l-mark-partial', no: 'l-mark-no' }

onMounted(() => {
  root.value?.querySelectorAll('.reveal, .reveal-zoom, .reveal-stagger').forEach((el) => observe(el))
})
</script>

<template>
  <section class="mx-auto max-w-6xl px-4 py-20 sm:px-6 md:py-28">
    <div ref="root">
      <span class="l-eyebrow reveal">the field</span>

      <div class="reveal reveal-d1 mt-6 flex flex-wrap items-end justify-between gap-6">
        <h2 class="font-display text-4xl font-extrabold leading-[1.05] text-foreground sm:text-[2.75rem]">
          Pick your <em class="l-fancy">tradeoffs.</em>
        </h2>
        <p class="max-w-sm text-base leading-relaxed text-muted-foreground">
          Every stack can be distributed eventually. The question is how much of it the language carries for you.
        </p>
      </div>

      <!-- Capability matrix -->
      <div class="reveal-zoom l-card mt-10 overflow-x-auto">
        <table class="w-full min-w-[640px] border-collapse text-left">
          <thead>
            <tr class="border-b border-border">
              <th class="px-6 py-4 font-mono text-xs font-semibold tracking-[0.08em] text-foreground">capability</th>
              <th
                v-for="(col, i) in columns"
                :key="col"
                class="w-[13%] px-4 py-4 text-center font-mono text-xs font-bold tracking-[0.06em]"
                :class="i === 0 ? 'bg-[color-mix(in_oklab,var(--l-accent)_8%,transparent)] text-[var(--l-accent)]' : 'text-muted-foreground'"
              >
                {{ col }}
              </th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in rows" :key="row.capability" class="border-b border-border last:border-b-0">
              <td class="px-6 py-3.5 text-sm text-foreground">{{ row.capability }}</td>
              <td
                v-for="(mark, i) in row.marks"
                :key="i"
                class="px-4 py-3.5 text-center"
                :class="i === 0 ? 'bg-[color-mix(in_oklab,var(--l-accent)_8%,transparent)]' : ''"
              >
                <span class="l-mark" :class="markClass[mark]">{{ glyph[mark] }}</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- Legend -->
      <div class="reveal reveal-d2 mt-5 flex flex-wrap gap-x-6 gap-y-2 font-mono text-[11px] text-muted-foreground">
        <span class="inline-flex items-center gap-2"><span class="l-mark l-mark-yes !size-5 !text-[11px]">✓</span> native to the language</span>
        <span class="inline-flex items-center gap-2"><span class="l-mark l-mark-partial !size-5 !text-[11px]">~</span> via libraries, config, or discipline</span>
        <span class="inline-flex items-center gap-2"><span class="l-mark l-mark-no !size-5 !text-[11px]">—</span> absent</span>
      </div>

      <!-- Head-to-head notes -->
      <div class="reveal-stagger mt-10 grid gap-3 md:grid-cols-3">
        <div v-for="note in notes" :key="note.label" class="rounded-xl border border-border bg-card p-6">
          <div class="font-mono text-xs font-semibold tracking-[0.08em] text-[var(--l-accent)]">{{ note.label }}</div>
          <p class="mt-3 text-sm leading-relaxed text-muted-foreground">{{ note.text }}</p>
        </div>
      </div>
    </div>
  </section>
</template>
