<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useScrollReveal } from '@/composables/useScrollReveal'

const { observe } = useScrollReveal()
const root = ref<HTMLElement>()

const rows = [
  {
    capability: 'Language',
    surface: 'Inference, generics, algebraic types, interfaces, pattern matching, pipes, and Result propagation',
    href: '/docs/language-basics/',
  },
  {
    capability: 'Concurrency',
    surface: 'Typed actors, services, supervisors, jobs, timers, links, monitors, and bounded channels',
    href: '/docs/concurrency/',
  },
  {
    capability: 'Service stack',
    surface: 'HTTP and WebSocket servers and clients, structured JSON, SQLite, PostgreSQL, pools, ORM, and migrations',
    href: '/docs/web/',
  },
  {
    capability: 'Distribution',
    surface: '@cluster work, remote actors, continuity, adaptive routing, capacity drivers, and bounded telemetry',
    href: '/docs/autonomous-clusters/',
  },
  {
    capability: 'Native ecosystem',
    surface: 'Checksum-pinned ABI 1 archives plus official Borsh, Anchor validation, and Solana packages',
    href: '/docs/native-packages/',
  },
  {
    capability: 'Toolchain',
    surface: 'Native compiler, formatter, tests, REPL, migrations, package manager, LSP, editor support, and operator CLI',
    href: '/docs/tooling/',
  },
]

onMounted(() => {
  root.value?.querySelectorAll('.reveal, .reveal-zoom, .reveal-stagger').forEach((el) => observe(el))
})
</script>

<template>
  <section class="mx-auto max-w-6xl px-4 py-20 sm:px-6 md:py-28">
    <div ref="root">
      <span class="l-eyebrow reveal">current surface</span>

      <div class="reveal reveal-d1 mt-6 flex flex-wrap items-end justify-between gap-6">
        <h2 class="font-display text-4xl font-extrabold leading-[1.05] text-foreground sm:text-[2.75rem]">
          One language,<br />the <em class="l-fancy">whole path.</em>
        </h2>
        <p class="max-w-sm text-base leading-relaxed text-muted-foreground">
          Mesh 14 connects the type system, actor runtime, service libraries, native packages, and cluster operations.
          Each row links to the exact public contract.
        </p>
      </div>

      <!-- Capability matrix -->
      <div class="reveal-zoom l-card mt-10 overflow-x-auto">
        <table class="w-full min-w-[600px] border-collapse text-left">
          <thead>
            <tr class="border-b border-border">
              <th class="w-[22%] px-6 py-4 font-mono text-xs font-semibold tracking-[0.08em] text-foreground">area</th>
              <th class="px-6 py-4 font-mono text-xs font-semibold tracking-[0.08em] text-foreground">implemented surface</th>
              <th class="w-[12%] px-6 py-4 text-right font-mono text-xs font-semibold tracking-[0.08em] text-foreground">guide</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in rows" :key="row.capability" class="border-b border-border last:border-b-0">
              <td class="bg-[color-mix(in_oklab,var(--l-accent)_8%,transparent)] px-6 py-4 font-mono text-xs font-bold tracking-[0.04em] text-[var(--l-accent)]">
                {{ row.capability }}
              </td>
              <td class="px-6 py-4 text-sm leading-relaxed text-muted-foreground">{{ row.surface }}</td>
              <td class="px-6 py-4 text-right">
                <a
                  :href="row.href"
                  class="font-mono text-xs font-semibold text-foreground underline decoration-border underline-offset-4 transition-colors hover:text-[var(--l-accent)] hover:decoration-[var(--l-accent)]"
                >read →</a>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>
