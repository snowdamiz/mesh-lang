<script setup lang="ts">
import { ref, nextTick } from 'vue'
import { data } from './landing.data'

const modules = data.modules
const active = ref(0)

// Arrow keys move between tabs, per the WAI-ARIA tabs pattern
function onKey(e: KeyboardEvent) {
  const step = ({ ArrowRight: 1, ArrowLeft: -1 } as Record<string, number>)[e.key]
  const edge = ({ Home: 0, End: modules.length - 1 } as Record<string, number>)[e.key]
  if (step === undefined && edge === undefined) return
  e.preventDefault()
  active.value = edge ?? (active.value + step! + modules.length) % modules.length
  nextTick(() => document.getElementById(`l-tab-${active.value}`)?.focus())
}
</script>

<template>
  <section class="l-section">
    <div class="l-pad grid gap-12 lg:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)] lg:gap-16">
      <div>
        <p class="l-label">Standard library</p>
        <h2 class="l-h2 mt-5">Server primitives, included.</h2>
        <p class="l-lede mt-6 max-w-[27rem]">
          HTTP, WebSockets, Postgres, SQLite, JSON, jobs, and a test runner ship with the toolchain. No framework to
          pick first.
        </p>
        <a href="/docs/stdlib/" class="l-link mt-8 inline-block text-[15px]">Browse the standard library →</a>
      </div>

      <div class="l-panel min-w-0 self-start">
        <div
          role="tablist"
          aria-label="Standard library examples"
          class="flex overflow-x-auto border-b border-[var(--l-line)] bg-[var(--l-panel-head)] [scrollbar-width:none]"
          @keydown="onKey"
        >
          <button
            v-for="(mod, i) in modules"
            :id="`l-tab-${i}`"
            :key="mod.name"
            type="button"
            role="tab"
            :aria-selected="active === i"
            aria-controls="l-tabpanel"
            :tabindex="active === i ? 0 : -1"
            class="relative shrink-0 px-3 py-3 font-mono text-xs transition-colors"
            :class="active === i ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'"
            @click="active = i"
          >
            {{ mod.name }}
            <span
              v-if="active === i"
              class="absolute inset-x-2.5 -bottom-px h-0.5 bg-[var(--l-accent)]"
              aria-hidden="true"
            />
          </button>
        </div>

        <div id="l-tabpanel" role="tabpanel" :aria-labelledby="`l-tab-${active}`" class="min-h-[17rem]">
          <div class="l-code vp-code" v-html="modules[active].html" />
        </div>

        <div class="l-panel-foot">
          <span class="min-w-0 truncate">{{ modules[active].file }}</span>
          <a :href="modules[active].href" class="shrink-0 text-foreground hover:text-[var(--l-accent)]">
            {{ modules[active].name }} guide →
          </a>
        </div>
      </div>
    </div>
  </section>
</template>
