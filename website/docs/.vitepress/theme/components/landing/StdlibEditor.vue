<script setup lang="ts">
import { computed, ref } from 'vue'

type Stdlib = {
  file: string
  lines: { html: string; modules: string[]; groups: string[] }[]
  groups: { name: string; modules: string[]; sample?: { line: number; html: string } }[]
  link: { text: string; href: string }
}

const props = defineProps<{ stdlib: Stdlib }>()

const hovered = ref<string | null>(null)
const pinned = ref<string | null>(null)
const active = computed(() => hovered.value ?? pinned.value)

const used = computed(() => new Set(props.stdlib.lines.flatMap((line) => line.groups)))
const moduleCount = computed(() => props.stdlib.groups.reduce((count, group) => count + group.modules.length, 0))
const digest = computed(() => props.stdlib.groups.flatMap((group) => (group.sample ? [group.sample] : [])).sort((a, b) => a.line - b.line))

function pin(name: string) {
  pinned.value = pinned.value === name ? null : name
}
</script>

<template>
  <div class="stdlib">
    <figure class="ed" :class="{ 'has-active': active }">
      <figcaption class="ed-bar">
        <span class="ed-file">{{ stdlib.file }}</span>
        <span class="ed-note">0 imports</span>
      </figcaption>
      <pre class="ed-code code"><span
        v-for="(line, i) in stdlib.lines"
        :key="i"
        class="ed-line"
        :class="{ on: active !== null && line.groups.includes(active) }"
      ><span class="ed-num" aria-hidden="true">{{ i + 1 }}</span><code v-html="line.html || ' '" /><span v-if="line.modules.length" class="ed-hint" aria-hidden="true">{{ line.modules.join(' · ') }}</span></span></pre>
      <div class="ed-digest code">
        <span v-for="use in digest" :key="use.line" class="ed-use"><span class="ed-num" aria-hidden="true">{{ use.line }}</span><code v-html="use.html" /></span>
      </div>
    </figure>

    <div class="legend">
      <ul role="list">
        <li v-for="group in stdlib.groups" :key="group.name">
          <button
            type="button"
            :aria-pressed="pinned === group.name"
            :class="{ on: active === group.name, unused: !used.has(group.name) }"
            @mouseenter="hovered = group.name"
            @mouseleave="hovered = null"
            @focus="hovered = group.name"
            @blur="hovered = null"
            @click="pin(group.name)"
          >
            <span class="lg-name">{{ group.name }}</span>
            <span class="lg-mods">{{ group.modules.join(' ') }}</span>
          </button>
        </li>
      </ul>
      <div class="legend-foot">
        <p class="legend-count">{{ moduleCount }} modules built in.</p>
        <a class="link-arrow" :href="stdlib.link.href">{{ stdlib.link.text }} <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M3 8h10M9 4l4 4-4 4" /></svg></a>
      </div>
    </div>
  </div>
</template>
