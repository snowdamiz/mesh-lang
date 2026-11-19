<script setup lang="ts">
import { computed, ref, useId } from 'vue'
import type { LandingData } from './landing.data.mjs'
import { useRovingTabs } from '../../composables/useRovingTabs'

const BETTER = { higher: 'Higher is better', lower: 'Lower is better' }

const props = defineProps<{ benchmarks: LandingData['benchmarks'] }>()
const id = useId()
const selected = ref(0)
const panel = computed(() => props.benchmarks.panels[selected.value])
const onTabKey = useRovingTabs((i) => { selected.value = i })
</script>

<template>
  <div class="bench">
    <div class="bench-tabs" role="tablist" aria-label="Workloads" @keydown="onTabKey">
      <button
        v-for="(p, i) in benchmarks.panels"
        :id="`${id}-${p.id}`"
        :key="p.id"
        type="button"
        role="tab"
        :aria-selected="i === selected"
        :aria-controls="`${id}-panel`"
        :tabindex="i === selected ? 0 : -1"
        @click="selected = i"
      >{{ p.label }}</button>
    </div>
    <div :id="`${id}-panel`" class="bench-panel" role="tabpanel" tabindex="0" :aria-labelledby="`${id}-${panel.id}`">
      <div class="stack">
        <div v-for="(p, i) in benchmarks.panels" :key="p.id" :class="{ on: i === selected }">
          <p class="bench-headline">{{ p.headline }}<span class="bench-headline-unit">{{ p.unit }}</span></p>
          <p class="bench-measure">{{ p.measure }}</p>
          <p class="bench-setup">{{ p.setup }}</p>
          <p v-if="p.note" class="bench-note">{{ p.note }}</p>
          <p class="bench-better" :class="`is-${p.better}`">{{ BETTER[p.better] }}</p>
        </div>
      </div>
      <ul class="bench-rows">
        <li v-for="row in panel.rows" :key="row.runtime" class="bench-row" :class="{ 'is-highlighted': row.highlighted }">
          <span class="bench-name">{{ row.runtime }}</span>
          <span class="bench-track">
            <span class="bench-bar" aria-hidden="true" :style="{ '--share': row.share }" />
            <span class="bench-fig">{{ row.figure }}</span>
          </span>
        </li>
      </ul>
    </div>
    <div class="bench-meta">
      <p>{{ benchmarks.footnote }}</p>
      <a class="link-arrow" :href="benchmarks.link.href">{{ benchmarks.link.text }} <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M3 8h10M9 4l4 4-4 4" /></svg></a>
    </div>
  </div>
</template>
