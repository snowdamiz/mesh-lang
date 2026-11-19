<script setup lang="ts">
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import type { LandingData } from '../landing.data.mjs'
import { prefersReducedMotion, whenVisible } from '../../../composables/motion'

const props = defineProps<{ art: LandingData['art'] }>()

type WorkerPhase = 'idle' | 'panic' | 'down' | 'restart' | 'up'
// A crash runs panic → down → restart (wire lit) → up → idle.
const NEXT: Record<Exclude<WorkerPhase, 'idle'>, [WorkerPhase, number]> = {
  panic: ['down', 700],
  down: ['restart', 550],
  restart: ['up', 350],
  up: ['idle', 1500],
}
const NODE_CLASS: Record<WorkerPhase, string | undefined> = { idle: undefined, panic: 'is-panic', down: 'is-down', restart: 'is-down', up: 'is-up' }
const WIRES = ['M150 33V75H54V117', 'M150 33V117', 'M150 33V75H246V117']
const X = ['18%', '50%', '82%']

const { root, workers, hint } = props.art.supervisor
const phases = reactive<Record<string, WorkerPhase>>(Object.fromEntries(workers.map((w) => [w, 'idle'])))
const live = ref('')
const timers = new Map<string, ReturnType<typeof setTimeout>>()

function enter(id: string, phase: WorkerPhase) {
  phases[id] = phase
  if (phase === 'panic') live.value = `${id} panicked.`
  if (phase === 'up') live.value = `${root} restarted ${id}. The other workers kept running.`
  if (phase === 'idle') return
  const [next, ms] = NEXT[phase]
  timers.set(id, setTimeout(() => enter(id, next), ms))
}
const crash = (id: string) => { if (phases[id] === 'idle') enter(id, 'panic') }

const el = ref<HTMLElement>()
let cancel = () => {}
onMounted(() => {
  if (prefersReducedMotion()) return
  cancel = whenVisible(el.value!, () => timers.set('auto', setTimeout(() => crash(workers[1]), 500)), 0.7)
})
onBeforeUnmount(() => {
  cancel()
  for (const t of timers.values()) clearTimeout(t)
})
</script>

<template>
  <div class="tile-art">
    <div ref="el" class="sup">
      <svg class="sup-wires" viewBox="0 0 300 150" preserveAspectRatio="none" aria-hidden="true">
        <path v-for="(d, i) in WIRES" :key="d" :d="d" vector-effect="non-scaling-stroke" :class="{ 'is-hot': ['restart', 'up'].includes(phases[workers[i]]) }" />
      </svg>
      <span class="sup-node sup-root" style="--x: 50%; --y: 22%">{{ root }}</span>
      <button
        v-for="(w, i) in workers"
        :key="w"
        type="button"
        class="sup-node"
        :class="NODE_CLASS[phases[w]]"
        :style="{ '--x': X[i], '--y': '78%' }"
        :aria-label="`Crash worker ${w}`"
        @click="crash(w)"
      >{{ w }}</button>
      <p class="sr-only" aria-live="polite">{{ live }}</p>
    </div>
    <p class="tile-hint">{{ hint }}</p>
  </div>
</template>
