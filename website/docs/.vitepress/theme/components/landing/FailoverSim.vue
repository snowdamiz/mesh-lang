<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'

interface SimNode {
  id: number
  label: string
  leader: boolean
  status: 'up' | 'down' | 'joining'
  y: number
}

interface LogEntry {
  id: number
  t: string
  level: 'ok' | 'info' | 'warn' | 'fail'
  msg: string
}

const panel = ref<HTMLElement>()

// Nodes stack to the right of the ingress
const NODE_X = 256
const NODE_W = 168
const NODE_H = 60

const nodes = ref<SimNode[]>([
  { id: 0, label: 'node-1', leader: true, status: 'up', y: 22 },
  { id: 1, label: 'node-2', leader: false, status: 'up', y: 110 },
  { id: 2, label: 'node-3', leader: false, status: 'up', y: 198 },
])

const log = ref<LogEntry[]>([])
const busy = ref(false)

const upCount = computed(() => nodes.value.filter((n) => n.status === 'up').length)
// One failure at a time keeps the story (and the log) coherent
const canStop = computed(() => !busy.value && upCount.value === 3)
const state = computed(() => (upCount.value === 3 ? 'healthy' : busy.value ? 'degraded' : 'recovering'))

// Ingress exits at (124, 140) and fans out to each node's left edge
const edges = computed(() =>
  nodes.value.map((n) => {
    const cy = n.y + NODE_H / 2
    return { id: n.id, d: `M 124 140 C 190 140, 190 ${cy}, ${NODE_X} ${cy}`, up: n.status === 'up' }
  }),
)

const tone = {
  ok: 'var(--ok)',
  info: 'var(--muted-foreground)',
  warn: 'var(--warn)',
  fail: 'var(--err)',
} as const

function nodeTone(n: SimNode) {
  return n.status === 'up' ? tone.ok : n.status === 'down' ? tone.fail : tone.warn
}

// ── Simulation plumbing ─────────────────────────────
let logId = 0
let timers: ReturnType<typeof setTimeout>[] = []
let ambient: ReturnType<typeof setInterval> | undefined
let io: IntersectionObserver | undefined
let visible = false
let booted = false
let lastUserAction = 0
let clockStart = 0

function stamp(): string {
  const ms = (14 * 3600 + 2 * 60 + 7) * 1000 + Math.floor(performance.now() - clockStart)
  const pad = (n: number, w = 2) => String(n).padStart(w, '0')
  return `${pad(Math.floor(ms / 3600000) % 24)}:${pad(Math.floor(ms / 60000) % 60)}:${pad(Math.floor(ms / 1000) % 60)}.${pad(ms % 1000, 3)}`
}

function push(level: LogEntry['level'], msg: string) {
  log.value.push({ id: logId++, t: stamp(), level, msg })
  if (log.value.length > 7) log.value.shift()
}

function after(ms: number, fn: () => void) {
  timers.push(setTimeout(fn, ms))
}

function kill(node: SimNode, byUser = true) {
  if (!canStop.value || node.status !== 'up') return
  busy.value = true
  if (byUser) lastUserAction = Date.now()
  node.status = 'down'
  push('fail', `${node.label} stopped responding`)

  const survivors = nodes.value.filter((n) => n.status === 'up')
  after(400, () => push('info', `new requests → ${survivors.map((n) => n.label).join(', ')}`))

  if (node.leader) {
    after(800, () => push('warn', 'leader lost, electing a successor'))
    after(1200, () => {
      node.leader = false
      survivors[0].leader = true
      push('ok', `${survivors[0].label} elected leader`)
    })
  }

  after(node.leader ? 1650 : 1100, () => {
    busy.value = false
    push('ok', 'traffic rebalanced')
  })
  after(3900, () => {
    node.status = 'joining'
    push('info', `${node.label} rejoining`)
  })
  after(5400, () => {
    node.status = 'up'
    push('ok', `${node.label} back, 3/3 nodes up`)
  })
}

function killRandom(byUser = true) {
  const alive = nodes.value.filter((n) => n.status === 'up')
  if (canStop.value) kill(alive[Math.floor(Math.random() * alive.length)], byUser)
}

onMounted(() => {
  clockStart = performance.now()
  io = new IntersectionObserver(
    ([entry]) => {
      visible = entry.isIntersecting
      if (visible && !booted) {
        booted = true
        after(200, () => push('ok', 'node-1 joined as leader'))
        after(450, () => push('ok', 'node-2 joined'))
        after(700, () => push('ok', 'node-3 joined'))
        after(1000, () => push('info', 'cluster ready, 3 nodes'))
      }
    },
    { threshold: 0.3 },
  )
  if (panel.value) io.observe(panel.value)

  // If nobody pulls the trigger, fail a node now and then so the story still plays
  ambient = setInterval(() => {
    if (visible && Date.now() - lastUserAction > 8000) killRandom(false)
  }, 11000)
})

onUnmounted(() => {
  timers.forEach(clearTimeout)
  timers = []
  clearInterval(ambient)
  io?.disconnect()
})
</script>

<template>
  <section class="l-section">
    <div class="l-pad">
      <div class="grid gap-6 lg:grid-cols-2 lg:items-end lg:gap-16">
        <div>
          <p class="l-label">Distribution</p>
          <h2 class="l-h2 mt-5">Keep orchestration out of your handlers.</h2>
        </div>
        <p class="l-lede max-w-[30rem] lg:justify-self-end">
          Handlers declare what may run on the cluster. The runtime owns placement, routing, and continuity, so a
          lost node is its problem, not your code's.
        </p>
      </div>

      <div ref="panel" class="l-panel mt-12">
        <div class="l-panel-head">
          <span class="flex min-w-0 items-center gap-2.5">
            <span
              class="size-2 shrink-0 rounded-full"
              :class="{ 'l-blink': state !== 'healthy' }"
              :style="{ background: state === 'healthy' ? tone.ok : state === 'degraded' ? tone.fail : tone.warn }"
            />
            <span class="truncate">
              <span class="text-foreground">{{ state }}</span> · {{ upCount }}/3 nodes
            </span>
          </span>
          <button type="button" class="l-fault-btn shrink-0" :disabled="!canStop" @click="killRandom()">
            Stop a node
          </button>
        </div>

        <div class="grid lg:grid-cols-[1.2fr_1fr]">
          <div class="border-b border-[var(--l-line)] lg:border-b-0 lg:border-r">
            <svg
              viewBox="0 0 440 280"
              class="mx-auto block w-full max-w-xl p-4 sm:p-6"
              role="group"
              aria-label="Illustrative cluster: an ingress routing requests to three nodes"
            >
              <!-- requests arriving -->
              <line x1="0" y1="140" x2="20" y2="140" stroke="var(--l-line-strong)" />
              <circle r="2.5" fill="var(--l-accent)" class="l-motion">
                <animateMotion dur="1.4s" repeatCount="indefinite" path="M -6 140 L 20 140" />
              </circle>

              <!-- ingress -->
              <rect x="20" y="108" width="104" height="64" rx="8" fill="var(--background)" stroke="var(--l-line-strong)" />
              <text x="72" y="136" text-anchor="middle" font-family="var(--font-mono)" font-size="11" font-weight="600" fill="var(--foreground)">ingress</text>
              <text x="72" y="152" text-anchor="middle" font-family="var(--font-mono)" font-size="9" fill="var(--muted-foreground)">load balancer</text>

              <!-- links and traffic -->
              <g v-for="edge in edges" :key="edge.id" fill="none">
                <path
                  :d="edge.d"
                  :stroke="edge.up ? 'var(--l-line-strong)' : tone.fail"
                  :stroke-dasharray="edge.up ? undefined : '2 5'"
                  :opacity="edge.up ? 1 : 0.5"
                />
                <template v-if="edge.up">
                  <circle r="2.75" fill="var(--l-accent)" class="l-motion">
                    <animateMotion :dur="`${1.7 + edge.id * 0.3}s`" repeatCount="indefinite" :path="edge.d" />
                  </circle>
                  <circle r="2.75" fill="var(--l-accent)" opacity="0.45" class="l-motion">
                    <animateMotion :dur="`${2.1 + edge.id * 0.25}s`" begin="0.9s" repeatCount="indefinite" :path="edge.d" />
                  </circle>
                </template>
              </g>

              <!-- nodes: click or press Enter to stop one -->
              <g
                v-for="node in nodes"
                :key="node.id"
                role="button"
                :tabindex="canStop ? 0 : -1"
                :aria-label="`Stop ${node.label}`"
                :aria-disabled="!canStop"
                class="outline-none [&:focus-visible>rect:first-child]:stroke-[var(--l-accent)]"
                :style="{ cursor: canStop ? 'pointer' : 'default' }"
                @click="kill(node)"
                @keydown.enter.prevent="kill(node)"
                @keydown.space.prevent="kill(node)"
              >
                <rect
                  :x="NODE_X" :y="node.y" :width="NODE_W" :height="NODE_H" rx="8"
                  fill="var(--background)"
                  :stroke="node.status === 'up' ? 'var(--l-line-strong)' : nodeTone(node)"
                  :stroke-dasharray="node.status === 'down' ? '4 4' : undefined"
                  :opacity="node.status === 'down' ? 0.65 : 1"
                />
                <circle
                  :cx="NODE_X + 18" :cy="node.y + 22" r="3.5"
                  :fill="nodeTone(node)"
                  :class="{ 'l-blink': node.status !== 'up' }"
                />
                <text
                  :x="NODE_X + 30" :y="node.y + 26"
                  font-family="var(--font-mono)" font-size="12" font-weight="600"
                  fill="var(--foreground)"
                  :opacity="node.status === 'down' ? 0.55 : 1"
                >{{ node.label }}</text>
                <text
                  :x="NODE_X + NODE_W - 14" :y="node.y + 26" text-anchor="end"
                  font-family="var(--font-mono)" font-size="9.5"
                  :fill="node.status !== 'up' ? nodeTone(node) : 'var(--l-accent)'"
                >{{ node.status === 'down' ? 'offline' : node.status === 'joining' ? 'joining' : node.leader ? 'leader' : '' }}</text>
                <!-- load: survivors absorb the stopped node's share -->
                <rect :x="NODE_X + 18" :y="node.y + 38" :width="NODE_W - 36" height="5" rx="2.5" fill="var(--l-line)" />
                <rect
                  :x="NODE_X + 18" :y="node.y + 38" height="5" rx="2.5"
                  :width="node.status === 'up' ? Math.round((NODE_W - 36) / upCount) : 0"
                  fill="var(--l-accent)"
                  style="transition: width 0.8s cubic-bezier(0.22, 1, 0.36, 1)"
                />
              </g>
            </svg>
          </div>

          <div class="flex min-h-[15rem] flex-col justify-end gap-1 overflow-hidden px-5 py-5 font-mono text-xs leading-[1.9]">
            <div v-for="entry in log" :key="entry.id" class="flex min-w-0 items-baseline gap-3">
              <span class="shrink-0 tabular-nums text-muted-foreground/60">{{ entry.t }}</span>
              <span class="size-1.5 shrink-0 self-center rounded-full" :style="{ background: tone[entry.level] }" />
              <span class="truncate text-foreground/85">{{ entry.msg }}</span>
            </div>
          </div>
        </div>

        <p class="l-panel-foot">
          Illustrative. Real recovery depends on quorum, replicas, and your continuity policy.
        </p>
      </div>
    </div>
  </section>
</template>
