<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import type { LandingData } from './landing.data.mjs'
import { prefersReducedMotion, vReveal } from '../../composables/motion'

const props = defineProps<{ figure: LandingData['distribution']['figure'] }>()
const markup = props.figure.lg + props.figure.sm

// The pulse walks the request's three hops (ingress, execution, replica), then rests.
const HOPS = [900, 1500, 1300]
const REST = 1100
const CYCLE = HOPS.reduce((a, b) => a + b, 0) + REST

const root = ref<HTMLElement>()
let raf = 0
let io: IntersectionObserver | undefined
let inView = false
let elapsed = 0
let last = 0

interface Variant { svg: SVGSVGElement; hops: SVGGeometryElement[]; pulse: SVGCircleElement; lengths?: number[] }
let variants: Variant[] = []

function draw(now: number) {
  raf = 0
  elapsed += Math.min(64, now - last)
  last = now
  let t = elapsed % CYCLE
  let hop = 0
  while (hop < HOPS.length && t > HOPS[hop]) t -= HOPS[hop++]
  const v = variants.find((x) => x.svg.getClientRects().length > 0)
  if (v) {
    v.lengths ??= v.hops.map((p) => p.getTotalLength())
    if (hop >= HOPS.length) v.pulse.setAttribute('opacity', '0')
    else {
      const k = t / HOPS[hop]
      const eased = k < 0.5 ? 2 * k * k : 1 - Math.pow(-2 * k + 2, 2) / 2
      const pt = v.hops[hop].getPointAtLength(eased * v.lengths[hop])
      v.pulse.setAttribute('cx', pt.x.toFixed(2))
      v.pulse.setAttribute('cy', pt.y.toFixed(2))
      v.pulse.setAttribute('opacity', '1')
      v.pulse.classList.toggle('is-replica', hop === 2)
    }
  }
  wake()
}
function wake() {
  if (raf || !inView || document.hidden) return
  last = performance.now()
  raf = requestAnimationFrame(draw)
}

onMounted(() => {
  if (prefersReducedMotion()) return
  variants = [...root.value!.querySelectorAll('svg')].map((svg) => ({
    svg,
    hops: [...svg.querySelectorAll<SVGGeometryElement>('.hop')],
    pulse: svg.querySelector<SVGCircleElement>('.pulse')!,
  }))
  io = new IntersectionObserver((entries) => { inView = entries[0].isIntersecting; wake() }, { threshold: 0.15 })
  io.observe(root.value!)
  document.addEventListener('visibilitychange', wake)
})
onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  io?.disconnect()
  document.removeEventListener('visibilitychange', wake)
})
</script>

<template>
  <div ref="root" v-reveal class="cluster-fig" role="img" :aria-label="figure.label" v-html="markup" />
</template>
