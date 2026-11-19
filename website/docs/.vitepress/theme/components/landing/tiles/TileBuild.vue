<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import type { LandingData } from '../landing.data.mjs'
import { isOnScreen, prefersReducedMotion, whenVisible } from '../../../composables/motion'

const props = defineProps<{ art: LandingData['art'] }>()
const { rows, label } = props.art.binary

// Server render shows the finished session; the client types it once on first view.
const shown = ref(rows.length)
const typed = ref<number | null>(null)

const el = ref<HTMLElement>()
let cancel = () => {}
let alive = true
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

async function play() {
  await sleep(300)
  for (let i = 0; i < rows.length && alive; i++) {
    shown.value = i + 1
    const command = rows[i].command
    if (!command) { await sleep(420); continue }
    for (let k = 0; k <= command.length && alive; k++) { typed.value = k; await sleep(55) }
    await sleep(260)
    typed.value = null
  }
}

onMounted(() => {
  if (prefersReducedMotion() || isOnScreen(el.value!)) return
  shown.value = 0
  cancel = whenVisible(el.value!, play, 0.7)
})
onBeforeUnmount(() => { alive = false; cancel() })
</script>

<template>
  <div class="tile-art">
    <div ref="el" class="term-mini" role="img" :aria-label="label">
      <div v-for="(row, i) in rows" :key="i" class="tm-row" :class="{ 'is-hidden': i >= shown, 'tm-dim': row.dim, 'tm-out': !row.command && !row.dim }" aria-hidden="true">
        <template v-if="row.command"><span class="pr" aria-hidden="true">$ </span><span>{{ i === shown - 1 && typed !== null ? row.command.slice(0, typed) : row.command }}</span><span v-if="i === shown - 1 && typed !== null" class="tm-caret" /></template>
        <template v-else>{{ row.dim ? '  ' : '' }}<span v-if="row.ok" class="ok">{{ row.ok }}</span>{{ row.output }}</template>
      </div>
    </div>
  </div>
</template>
