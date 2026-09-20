<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useData } from 'vitepress'
import { useScrollReveal } from '@/composables/useScrollReveal'

const { isDark } = useData()
const { observe } = useScrollReveal()
const wordmark = ref<HTMLElement>()
const currentYear = new Date().getUTCFullYear()

onMounted(() => {
  if (wordmark.value) observe(wordmark.value)
})

const columns = [
  {
    title: 'Resources',
    links: [
      { label: 'Getting Started', href: '/docs/getting-started/' },
      { label: 'Language Guide', href: '/docs/language-basics/' },
      { label: 'Complete Reference', href: '/docs/reference/' },
      { label: 'Standard Library', href: '/docs/stdlib/' },
      { label: 'Developer Tools', href: '/docs/tooling/' },
    ],
  },
  {
    title: 'Features',
    links: [
      { label: 'Concurrency', href: '/docs/concurrency/' },
      { label: 'Web & HTTP', href: '/docs/web/' },
      { label: 'Databases', href: '/docs/databases/' },
      { label: 'Native Packages', href: '/docs/native-packages/' },
      { label: 'Packages & Registry', href: '/docs/packages/' },
    ],
  },
  {
    title: 'Community',
    links: [
      { label: 'GitHub', href: 'https://github.com/hyperpush-org/mesh-lang', target: '_blank' },
      { label: 'Discussions', href: 'https://github.com/hyperpush-org/mesh-lang/discussions', target: '_blank' },
      { label: 'Packages', href: 'https://packages.meshlang.dev', target: '_blank' },
    ],
  },
]
</script>

<template>
  <footer class="overflow-hidden">
    <div class="mx-auto max-w-6xl px-4 pt-16 sm:px-6 md:pt-20">
      <div class="grid gap-10 sm:grid-cols-2 md:grid-cols-4">
        <!-- Brand -->
        <div>
          <div class="flex items-center gap-2.5">
            <img :src="isDark ? '/logo-icon-white.svg' : '/logo-icon-black.svg'" alt="Mesh" class="size-7" />
            <span class="font-display text-lg font-extrabold text-foreground">Mesh</span>
          </div>
          <p class="mt-4 max-w-[230px] text-[13px] leading-relaxed text-muted-foreground">
            Expressive, concurrent, type-safe. Compiled to native binaries, distributed by the runtime.
          </p>
        </div>

        <!-- Link columns -->
        <div v-for="col in columns" :key="col.title">
          <h3 class="font-mono text-xs font-semibold tracking-[0.1em] text-foreground">{{ col.title.toLowerCase() }}</h3>
          <ul class="mt-5 space-y-2.5">
            <li v-for="link in col.links" :key="link.href">
              <a
                :href="link.href"
                :target="(link as any).target"
                class="text-sm text-muted-foreground transition-colors hover:text-[var(--l-accent)]"
              >
                {{ link.label }}
              </a>
            </li>
          </ul>
        </div>
      </div>

      <!-- Bottom bar -->
      <div class="relative mt-14 flex flex-col items-center justify-between gap-4 py-6 font-mono text-[11px] text-muted-foreground sm:flex-row">
        <span
          class="absolute inset-x-0 top-0 h-px"
          style="background: linear-gradient(90deg, transparent, var(--border) 20%, var(--border) 80%, transparent);"
          aria-hidden="true"
        />
        <p>© {{ currentYear }} The Mesh Programming Language</p>
        <div class="flex items-center gap-5">
          <a href="https://github.com/hyperpush-org/mesh-lang" class="transition-colors hover:text-[var(--l-accent)]">GitHub</a>
          <a href="https://github.com/hyperpush-org/mesh-lang/blob/main/LICENSE" class="transition-colors hover:text-[var(--l-accent)]">License</a>
        </div>
      </div>
    </div>

    <!-- Giant fading wordmark -->
    <div ref="wordmark" class="reveal pointer-events-none select-none px-2 pb-0" aria-hidden="true">
      <div
        class="l-wordmark mx-auto max-w-6xl translate-y-[0.18em] text-center text-[clamp(6rem,21vw,19rem)] font-extrabold leading-none tracking-tight"
      >
        mesh
      </div>
    </div>
  </footer>
</template>
