<script setup lang="ts">
import { useData } from 'vitepress'
import { data } from './landing.data'
import InstallCommand from './InstallCommand.vue'

const { theme } = useData()

const specs = [
  { term: 'Native', desc: 'LLVM-compiled binaries' },
  { term: 'Typed', desc: 'Hindley–Milner inference' },
  { term: 'Concurrent', desc: 'Actors and supervisors' },
  { term: 'Distributed', desc: 'Clustering in the runtime' },
]

// "Ship a fleet.": a small mesh in the space beside the headline, echoing the
// logo's hub and spokes. Coordinates are hero pixels from its top-right corner
// (viewBox x 656..1216 at the widest frame); nodes sit on the 24px dot grid.
const hub = [936, 192]
const nodes = [[792, 72], [1056, 48], [1152, 216], [1032, 336], [792, 312]]
const links = [
  ...nodes.map(([x, y]) => `M${hub[0]} ${hub[1]} L${x} ${y}`),
  'M792 72 L1056 48', 'M1056 48 L1152 216', 'M1152 216 L1032 336', 'M1032 336 L792 312',
]
const packets = [
  { path: 'M792 72 L936 192 L1032 336', dur: 5 },
  { path: 'M1056 48 L936 192 L792 312', dur: 6, begin: 1.5 },
  { path: 'M1152 216 L1032 336 L792 312', dur: 5.5, begin: 3 },
  { path: 'M792 72 L1056 48 L1152 216', dur: 6.5, begin: 4 },
]
</script>

<template>
  <section class="relative overflow-hidden px-5 pb-20 pt-14 sm:px-10 sm:pt-20 lg:px-14 lg:pb-24 lg:pt-24">
    <svg
      class="pointer-events-none absolute right-0 top-0 hidden h-[440px] w-[560px] text-[var(--l-accent)] xl:block"
      viewBox="656 0 560 440"
      fill="none"
      aria-hidden="true"
      style="mask-image: radial-gradient(ellipse 60% 70% at 62% 45%, black 45%, transparent); -webkit-mask-image: radial-gradient(ellipse 60% 70% at 62% 45%, black 45%, transparent);"
    >
      <defs>
        <pattern id="l-dots" width="24" height="24" patternUnits="userSpaceOnUse">
          <circle cx="0" cy="0" r="1.1" fill="var(--l-dot)" />
          <circle cx="24" cy="0" r="1.1" fill="var(--l-dot)" />
          <circle cx="0" cy="24" r="1.1" fill="var(--l-dot)" />
          <circle cx="24" cy="24" r="1.1" fill="var(--l-dot)" />
        </pattern>
      </defs>
      <rect x="656" width="560" height="440" fill="url(#l-dots)" />
      <g stroke="currentColor" stroke-width="1" opacity="0.35">
        <path v-for="d in links" :key="d" :d="d" />
      </g>
      <g v-for="[x, y] in nodes" :key="`${x},${y}`">
        <circle :cx="x" :cy="y" r="8" fill="currentColor" opacity="0.14" />
        <circle :cx="x" :cy="y" r="3.5" fill="currentColor" />
      </g>
      <circle :cx="hub[0]" :cy="hub[1]" r="14" fill="currentColor" opacity="0.12" />
      <circle :cx="hub[0]" :cy="hub[1]" r="6" fill="currentColor" />
      <circle v-for="p in packets" :key="p.path" r="2.5" fill="currentColor" class="l-motion">
        <animateMotion :dur="`${p.dur}s`" :begin="`${p.begin ?? 0}s`" repeatCount="indefinite" :path="p.path" />
      </circle>
    </svg>

    <a
      :href="`https://github.com/hyperpush-org/mesh-lang/releases/tag/v${theme.meshVersion}`"
      target="_blank"
      rel="noopener"
      class="l-announce l-enter relative"
    >
      <span class="l-announce-tag">v{{ theme.meshVersion }}</span>
      Release notes
      <span aria-hidden="true">→</span>
    </a>

    <h1 class="l-display l-enter relative mt-7" style="animation-delay: 60ms">
      <span class="text-muted-foreground">Write a server.</span><br />
      Ship a fleet.
    </h1>

    <div class="relative mt-12 grid items-start gap-14 lg:mt-14 lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)] lg:gap-14">
      <div class="l-enter min-w-0" style="animation-delay: 120ms">
        <p class="l-lede max-w-[30rem]">
          Mesh is a compiled, statically typed language with actors built in and clustering in the runtime.
          Mark the work that can run anywhere; Mesh handles placement, routing, and failover.
        </p>

        <div class="mt-8 flex flex-col gap-3 sm:flex-row sm:flex-wrap sm:items-center">
          <a href="/docs/getting-started/" class="l-btn l-btn-primary">
            Get started <span class="l-arrow" aria-hidden="true">→</span>
          </a>
          <InstallCommand command="curl -sSf https://meshlang.dev/install.sh | sh" />
        </div>

        <dl class="mt-12 grid grid-cols-2 gap-x-6 gap-y-6 border-t border-[var(--l-line)] pt-8">
          <div v-for="spec in specs" :key="spec.term">
            <dt class="font-mono text-xs text-[var(--l-accent)]">{{ spec.term }}</dt>
            <dd class="mt-1 text-[15px] text-foreground">{{ spec.desc }}</dd>
          </div>
        </dl>
      </div>

      <div class="l-enter relative min-w-0" style="animation-delay: 180ms">
        <div class="l-panel relative">
          <div class="l-panel-head">
            <span class="text-foreground">{{ data.hero.file }}</span>
            <span>mesh</span>
          </div>
          <div class="l-code vp-code" v-html="data.hero.html" />
        </div>
      </div>
    </div>
  </section>
</template>
