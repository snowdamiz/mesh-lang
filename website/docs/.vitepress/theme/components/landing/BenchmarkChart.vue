<script setup lang="ts">
import { computed, ref, useId } from 'vue'
import type { LandingData } from './landing.data.mjs'
import { useRovingTabs } from '../../composables/useRovingTabs'

const BETTER = { higher: 'Higher is better.', lower: 'Lower is better.' }

const props = defineProps<{ benchmarks: LandingData['benchmarks'] }>()
const id = useId()
const selected = ref(0)
const panel = computed(() => props.benchmarks.panels[selected.value])
const onTabKey = useRovingTabs((i) => { selected.value = i })
</script>

<template>
  <div class="bench">
    <div class="dtabs bench-tabs" role="tablist" aria-label="Workloads" @keydown="onTabKey">
      <button
        v-for="(p, i) in benchmarks.panels"
        :id="`${id}-${p.id}`"
        :key="p.id"
        class="dtab"
        type="button"
        role="tab"
        :aria-selected="i === selected"
        :aria-controls="`${id}-panel`"
        :tabindex="i === selected ? 0 : -1"
        @click="selected = i"
      >{{ p.label }}</button>
    </div>
    <div :id="`${id}-panel`" class="bench-panel" role="tabpanel" tabindex="0" :aria-labelledby="`${id}-${panel.id}`" :aria-describedby="`${id}-cap-${panel.id} ${id}-note-${panel.id}`">
      <div class="bench-stack bench-cap">
        <p v-for="(p, i) in benchmarks.panels" :id="`${id}-cap-${p.id}`" :key="p.id" :class="{ on: i === selected }">{{ p.caption }} <b>{{ BETTER[p.better] }}</b></p>
      </div>
      <ul class="bench-rows">
        <li v-for="row in panel.rows" :key="row.runtime" class="bench-row" :class="{ 'is-highlighted': row.highlighted }">
          <span class="bench-name">{{ row.runtime }}</span>
          <span class="bench-track">
            <span class="bench-bar" aria-hidden="true" :style="{ '--share': row.share }" />
            <span class="bench-fig">{{ row.figure }} <span class="bench-unit">{{ panel.unit }}</span></span>
          </span>
        </li>
      </ul>
      <div class="bench-foot">
        <p>{{ benchmarks.footnote }}</p>
        <a class="link-arrow" :href="benchmarks.link.href">{{ benchmarks.link.text }} <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M3 8h10M9 4l4 4-4 4" /></svg></a>
      </div>
      <div class="bench-stack bench-note">
        <p v-for="(p, i) in benchmarks.panels" :id="`${id}-note-${p.id}`" :key="p.id" :class="{ on: i === selected }">{{ p.note }}</p>
      </div>
    </div>
  </div>
</template>
