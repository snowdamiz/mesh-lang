<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, triggerRef, useId } from 'vue'
import { cloneFrame, compileStep, demoReducer, type DemoEvent, type DemoState, type Frame, type LineRef } from './demo'
import type { LandingData } from './landing.data.mjs'
import { isOnScreen, prefersReducedMotion } from '../../composables/motion'
import { useRovingTabs } from '../../composables/useRovingTabs'

const props = defineProps<{ demo: LandingData['demo'] }>()
const steps = props.demo.steps
const timelines = steps.map((s) => compileStep(s, props.demo.files[s.file]))
const timing = timelines.map(({ still, end, duration }) => ({ still, end, duration }))
const id = useId()

// The clock advances `state` every animation frame; only step and phase are
// reactive, so the template re-renders when the frame actually changes. The
// server renders the finished first step; the client rewinds it only when it
// can play it unseen (see onMounted).
const first = timelines[0]
let state: DemoState = { step: 0, t: first.still, phase: 'showing' }
const step = ref(0)
const phase = ref<DemoState['phase']>('showing')
const frame = shallowRef<Frame>(cloneFrame(first.init))
const scroll = ref(0)
let applied = 0
let framed = { step: 0, t: 0 }
let reduced = false
// Screen readers hear the caption of a step the reader picked or paused on, not every auto-advance.
const announced = ref('')

const editor = ref<HTMLElement>()
const term = ref<HTMLElement>()
const progress = ref<HTMLElement[]>([])

// The editor above 980px (landing.css: the 560px pane less its 36px tabs and 28px status bar). The server
// and the first client render scroll for it, so the first paint shows the caret and hydration matches.
const WIDE_EDITOR = { height: 496, lh: 21 }

function follow(f: Frame) {
  if (!f.focus) { scroll.value = 0; return }
  const el = editor.value
  const { height, lh } = el ? { height: el.clientHeight, lh: parseFloat(getComputedStyle(el).lineHeight) || 21 } : WIDE_EDITOR
  const content = f.lines.length * lh + 54
  const { line, mode } = f.focus
  let target = scroll.value
  if (mode === 'center') target = line * lh - height / 2 + lh
  else {
    const caretY = line * lh + 14
    if (caretY + lh * 3 > scroll.value + height) target = caretY + lh * 3 - height
    if (caretY < scroll.value + lh) target = caretY - lh
  }
  const snapped = 6 + Math.round((target - 6) / lh) * lh
  scroll.value = Math.max(0, Math.min(snapped, Math.max(0, content - height)))
}

function render() {
  const tl = timelines[state.step]
  // Under reduced motion only still frames show; state.t just times the hold.
  const t = reduced ? tl.still : state.t
  let changed = false
  if (state.step !== framed.step || t < framed.t) {
    frame.value = cloneFrame(tl.init)
    applied = 0
    changed = true
  }
  while (applied < tl.events.length && tl.events[applied].at <= t) {
    tl.events[applied++].apply(frame.value)
    changed = true
  }
  framed = { step: state.step, t }
  step.value = state.step
  phase.value = state.phase
  const bar = progress.value[state.step]
  if (bar) bar.style.transform = `scaleX(${reduced || state.phase === 'paused' ? 0 : Math.min(1, state.t / timing[state.step].duration)})`
  if (!changed) return
  triggerRef(frame)
  follow(frame.value)
  nextTick(() => { if (term.value) term.value.scrollTop = term.value.scrollHeight })
}
render()

function dispatch(e: DemoEvent) {
  const before = state.step
  state = demoReducer(state, e, timing, reduced)
  if (state.step !== before) for (const bar of progress.value) bar.style.transform = 'scaleX(0)'
  if (e.type === 'select' || (e.type === 'toggle' && state.phase === 'paused')) announce(steps[state.step].caption)
  render()
  wake()
}

// Clearing first changes the region even when the caption repeats, so a screen reader reads it again.
function announce(caption: string) {
  announced.value = ''
  nextTick(() => { announced.value = caption })
}

let raf = 0
let last = 0
let inView = false
const running = () => inView && !document.hidden && state.phase !== 'paused'
function loop(now: number) {
  raf = 0
  const dt = Math.min(64, now - last)
  last = now
  dispatch({ type: 'tick', dt })
}
function wake() {
  if (raf || !running()) return
  last = performance.now()
  raf = requestAnimationFrame(loop)
}

const select = (i: number) => dispatch({ type: 'select', step: i })
const onTabKey = useRovingTabs(select)

let io: IntersectionObserver | undefined
const onVisibility = () => wake()
const root = ref<HTMLElement>()
onMounted(() => {
  reduced = prefersReducedMotion()
  if (reduced) state = { step: 0, t: first.still, phase: 'paused' }
  else if (!isOnScreen(root.value!)) state = { step: 0, t: 0, phase: 'typing' }
  render()
  follow(frame.value)
  io = new IntersectionObserver((entries) => { inView = entries[0].isIntersecting; wake() }, { threshold: 0.2 })
  io.observe(root.value!)
  document.addEventListener('visibilitychange', onVisibility)
})
onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  io?.disconnect()
  document.removeEventListener('visibilitychange', onVisibility)
})

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

function lineHtml(ref: LineRef, i: number, f: Frame): string {
  const tokens = props.demo.tokens[ref.key] ?? [[ref.key, 'inherit']]
  const marks = [f.sq && { ...f.sq, cls: 'sq' }, f.sel && { ...f.sel, cls: 'sel' }].filter((m) => m && m.line === i) as { from: number; to: number; cls: string }[]
  const caret = f.caret?.line === i ? f.caret.col : -1
  const starts: number[] = []
  const cuts = new Set([0, ref.len, caret])
  let pos = 0
  for (const [text] of tokens) { starts.push(pos); cuts.add(pos); pos += text.length }
  for (const m of marks) { cuts.add(m.from); cuts.add(m.to) }
  const points = [...cuts].filter((c) => c >= 0 && c <= ref.len).sort((a, b) => a - b)
  let out = ''
  for (let k = 0; k < points.length; k++) {
    if (points[k] === caret) out += '<span class="caret"></span>'
    if (k === points.length - 1) break
    const [a, b] = [points[k], points[k + 1]]
    let t = starts.length - 1
    while (t > 0 && starts[t] > a) t--
    const [, color, italic] = tokens[t]
    let seg = `<span style="color:${color}${italic ? ';font-style:italic' : ''}">${esc(ref.key.slice(a, b))}</span>`
    for (const m of marks) if (a >= m.from && b <= m.to) seg = `<span class="${m.cls}">${seg}</span>`
    out += seg
  }
  return out
}

const lines = computed(() => {
  const f = frame.value
  return f.lines.map((ref, i) => ({ html: lineHtml(ref, i, f), hl: f.hl.includes(i), err: f.err === i }))
})
const position = computed(() => {
  const f = frame.value
  if (f.caret) return `Ln ${f.caret.line + 1}, Col ${f.caret.col + 1}`
  return `Ln ${(f.hl[0] ?? 0) + 1}, Col 1`
})
const cwd = computed(() => steps[step.value].cwd)
const problems = computed(() => `${frame.value.problems} ${frame.value.problems === 1 ? 'problem' : 'problems'}`)
</script>

<template>
  <section id="demo" class="demo-sec" aria-label="Mesh in four steps">
    <div ref="root" class="wrap">
      <div class="demo-bar">
        <div class="dtabs" role="tablist" aria-label="Walkthrough" @keydown="onTabKey">
          <button
            v-for="(s, i) in steps"
            :id="`${id}-${s.id}`"
            :key="s.id"
            type="button"
            class="dtab"
            role="tab"
            :aria-selected="i === step"
            :aria-controls="`${id}-panel`"
            :tabindex="i === step ? 0 : -1"
            @click="select(i)"
          >{{ s.label }}<span ref="progress" class="prog" /></button>
          <span class="dsep" aria-hidden="true" />
          <button
            type="button"
            class="dpause"
            :aria-pressed="phase === 'paused'"
            :aria-label="phase === 'paused' ? 'Play the walkthrough' : 'Pause the walkthrough'"
            @click="dispatch({ type: 'toggle' })"
          >
            <svg v-if="phase === 'paused'" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M4.5 3v10l8-5z" /></svg>
            <svg v-else viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><rect x="4" y="3" width="2.6" height="10" rx=".6" /><rect x="9.4" y="3" width="2.6" height="10" rx=".6" /></svg>
            <span>{{ phase === 'paused' ? 'Play' : 'Pause' }}</span>
          </button>
        </div>
        <p class="dcap" v-html="steps[step].caption" />
        <p class="sr-only" aria-live="polite" v-html="announced" />
      </div>

      <div :id="`${id}-panel`" class="app win" role="tabpanel" :aria-labelledby="`${id}-${steps[step].id}`" tabindex="0">
        <div class="win-top">
          <div class="lights" aria-hidden="true"><i /><i /><i /></div>
          <div class="win-title">{{ cwd }}</div>
          <div />
        </div>
        <div class="app-body">
          <div class="pane">
            <div class="pane-tabs" aria-hidden="true"><span class="on"><i class="dot" />main.mpl</span><span>mesh.toml</span></div>
            <div ref="editor" class="editor">
              <div class="editor-in" :style="{ transform: `translateY(${-scroll}px)` }">
                <div v-for="(line, i) in lines" :key="i" class="ln" :class="{ hl: line.hl, err: line.err }"><span class="g" aria-hidden="true">{{ i + 1 }}</span><span class="tx" v-html="line.html" /></div>
              </div>
            </div>
            <div class="statusbar"><span>{{ position }} · Mesh</span><span class="probs" :class="{ bad: frame.problems > 0 }"><i /> {{ problems }}</span></div>
          </div>
          <div class="pane">
            <div class="pane-tabs" aria-hidden="true"><span class="term-title">{{ cwd.replace('~/', '') }} · zsh</span></div>
            <div ref="term" class="term">
              <div v-for="(row, i) in frame.rows" :key="i" class="row" :class="{ fade: row.fade, diag: row.diag }"><span v-for="(seg, j) in row.segs" :key="j" :class="seg.cls">{{ seg.text }}</span><span v-if="frame.termCaret && i === frame.rows.length - 1" class="caret" /></div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>
