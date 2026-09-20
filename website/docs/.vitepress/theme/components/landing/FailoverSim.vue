<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useScrollReveal } from '@/composables/useScrollReveal'

interface SimNode {
  id: number
  label: string
  controlRole: 'leader' | 'voter'
  status: 'up' | 'down' | 'joining'
  x: number
  y: number
}

interface LogEntry {
  id: number
  t: string
  level: 'ok' | 'info' | 'warn' | 'fail'
  msg: string
}

const { observe } = useScrollReveal()
const root = ref<HTMLElement>()
const panel = ref<HTMLElement>()

// Server blades, stacked rack-style to the right of the ingress
const BLADE_X = 262
const BLADE_W = 150
const BLADE_H = 64

const nodes = ref<SimNode[]>([
  { id: 0, label: 'node-1', controlRole: 'leader', status: 'up', x: BLADE_X, y: 16 },
  { id: 1, label: 'node-2', controlRole: 'voter', status: 'up', x: BLADE_X, y: 108 },
  { id: 2, label: 'node-3', controlRole: 'voter', status: 'up', x: BLADE_X, y: 200 },
])

const log = ref<LogEntry[]>([])
const clusterState = ref<'HEALTHY' | 'DEGRADED' | 'RECOVERING'>('HEALTHY')
const routed = ref(0)
const busy = ref(false)

const upCount = computed(() => nodes.value.filter((n) => n.status === 'up').length)

// Fan-out from the ingress (exit at 126,140) to each blade's left edge
const edges = computed(() =>
  nodes.value.map((n) => {
    const cy = n.y + BLADE_H / 2
    return {
      id: n.id,
      d: `M 126 140 C 196 140, 196 ${cy}, ${BLADE_X} ${cy}`,
      active: n.status === 'up',
    }
  }),
)

// Each surviving blade absorbs the dead node's share of traffic
function loadWidth(node: SimNode): number {
  if (node.status !== 'up') return 0
  return Math.round(114 / upCount.value)
}

const stateColor = computed(() =>
  clusterState.value === 'HEALTHY' ? 'var(--ok)' : clusterState.value === 'DEGRADED' ? 'var(--err)' : 'var(--warn)',
)

function nodeColor(node: SimNode): string {
  return node.status === 'up' ? 'var(--ok)' : node.status === 'down' ? 'var(--err)' : 'var(--warn)'
}

// ── Simulation plumbing ─────────────────────────────
let logId = 0
let timers: ReturnType<typeof setTimeout>[] = []
let intervals: ReturnType<typeof setInterval>[] = []
let io: IntersectionObserver | null = null
let visible = false
let booted = false
let lastUserAction = 0

const CLOCK_BASE = (14 * 3600 + 2 * 60 + 7) * 1000
let clockStart = 0

function nowStamp(): string {
  const elapsed = typeof performance !== 'undefined' ? performance.now() - clockStart : 0
  const ms = CLOCK_BASE + Math.floor(elapsed)
  const h = Math.floor(ms / 3600000) % 24
  const m = Math.floor(ms / 60000) % 60
  const s = Math.floor(ms / 1000) % 60
  const frac = ms % 1000
  const pad = (n: number, w = 2) => String(n).padStart(w, '0')
  return `${pad(h)}:${pad(m)}:${pad(s)}.${pad(frac, 3)}`
}

function push(level: LogEntry['level'], msg: string) {
  log.value.push({ id: logId++, t: nowStamp(), level, msg })
  if (log.value.length > 9) log.value.shift()
}

function after(ms: number, fn: () => void) {
  timers.push(setTimeout(fn, ms))
}

function rand(n: number): number {
  return Math.floor(Math.random() * n)
}

function kill(node: SimNode) {
  if (busy.value || node.status !== 'up') return
  busy.value = true
  lastUserAction = Date.now()
  clusterState.value = 'DEGRADED'
  node.status = 'down'
  push('fail', `${node.label} heartbeat lost`)

  const wasLeader = node.controlRole === 'leader'
  const survivors = nodes.value.filter((n) => n.status === 'up')

  after(400, () => push('info', `shifting eligible new calls → ${survivors.map((n) => n.label).join(', ')}`))

  if (wasLeader) {
    after(800, () => push('warn', 'controller leader lost — electing successor'))
    after(1200, () => {
      const successor = nodes.value.find((n) => n.status === 'up')
      if (successor) {
        successor.controlRole = 'leader'
        node.controlRole = 'voter'
        push('ok', `${successor.label} elected controller leader`)
      }
    })
  }

  after(wasLeader ? 1650 : 1100, () => {
    push('ok', 'routing shifted to surviving nodes')
    clusterState.value = 'RECOVERING'
  })

  after(3900, () => {
    node.status = 'joining'
    push('info', `${node.label} reconnecting…`)
  })

  after(5400, () => {
    node.status = 'up'
    clusterState.value = 'HEALTHY'
    busy.value = false
    push('ok', 'mesh restored — 3/3 nodes up')
  })
}

function killRandom() {
  const alive = nodes.value.filter((n) => n.status === 'up')
  if (alive.length) kill(alive[rand(alive.length)])
}

function boot() {
  if (booted) return
  booted = true
  after(200, () => push('ok', 'node-1 joined · controller+worker · leader'))
  after(500, () => push('ok', 'node-2 joined · controller+worker · voter'))
  after(750, () => push('ok', 'node-3 joined · controller+worker · voter'))
  after(1100, () => push('info', 'mesh formed — 3 ready worker nodes'))
}

onMounted(() => {
  clockStart = performance.now()
  root.value?.querySelectorAll('.reveal, .reveal-zoom, .reveal-stagger').forEach((el) => observe(el))

  if (panel.value) {
    io = new IntersectionObserver(
      (entries) => {
        visible = entries[0].isIntersecting
        if (visible) boot()
      },
      { threshold: 0.3 },
    )
    io.observe(panel.value)
  }

  // Illustrative new-request counter while at least one ready worker remains.
  intervals.push(
    setInterval(() => {
      if (visible && upCount.value > 0) routed.value += 2 + rand(6)
    }, 160),
  )

  // Ambient chaos if the visitor won't pull the trigger themselves
  intervals.push(
    setInterval(() => {
      if (visible && !busy.value && Date.now() - lastUserAction > 6000) killRandom()
    }, 9500),
  )
})

onUnmounted(() => {
  timers.forEach(clearTimeout)
  intervals.forEach(clearInterval)
  timers = []
  intervals = []
  io?.disconnect()
  io = null
})

const levelColor: Record<LogEntry['level'], string> = {
  ok: 'var(--ok)',
  info: 'var(--muted-foreground)',
  warn: 'var(--warn)',
  fail: 'var(--err)',
}
</script>

<template>
  <section class="relative overflow-hidden">
    <div
      class="l-aurora left-1/2 top-10 h-80 w-[40rem] -translate-x-1/2 opacity-40"
      style="background: color-mix(in oklab, var(--l-accent) 8%, transparent);"
    />
    <div class="relative mx-auto max-w-6xl px-4 py-20 sm:px-6 md:py-28">
      <div ref="root">
        <span class="l-eyebrow reveal">runtime-owned failover</span>

        <div class="reveal reveal-d1 mt-6 flex flex-wrap items-end justify-between gap-6">
          <h2 class="font-display text-4xl font-extrabold leading-[1.05] text-foreground sm:text-[2.75rem]">
            Simulate node loss.<br />Explore <em class="l-fancy">recovery.</em>
          </h2>
          <p class="max-w-sm text-base leading-relaxed text-muted-foreground">
            This interactive panel illustrates topology and rerouting; it is not a live cluster or a zero-loss
            guarantee. Actual recovery depends on quorum, replicas, readiness, continuity policy, and stable operation
            identity. Click any node below to explore the sequence.
          </p>
        </div>

        <!-- Live cluster panel -->
        <div ref="panel" class="reveal-zoom reveal-d1 l-card mt-10 overflow-hidden">
          <div class="flex items-center justify-between gap-4 border-b border-border px-5 py-3.5 sm:px-6">
            <span class="min-w-0 truncate font-mono text-[11.5px] tracking-[0.04em] text-muted-foreground">mesh://cluster — illustrative topology</span>
            <button class="l-fault-btn shrink-0" :disabled="busy" @click="killRandom">
              ⚡ inject fault
            </button>
          </div>

          <div class="grid lg:grid-cols-[1.15fr_1fr]">
            <!-- Topology: one ingress fanning out to a rack of blades -->
            <div class="relative border-b border-border lg:border-b-0 lg:border-r">
              <svg viewBox="0 0 440 280" class="mx-auto block w-full max-w-xl p-4" role="img" aria-label="Illustrative application ingress routing to three controller and worker nodes; click a server to simulate failure">
                <!-- Traffic arriving from the internet -->
                <line x1="0" y1="140" x2="22" y2="140" stroke="currentColor" stroke-width="1.25" class="text-foreground/20" />
                <circle r="2.5" fill="var(--l-accent)" opacity="0.8">
                  <animateMotion dur="1.4s" repeatCount="indefinite" path="M -6 140 L 22 140" />
                </circle>

                <!-- Ingress -->
                <g>
                  <rect x="22" y="104" width="104" height="72" rx="10" class="fill-card" stroke="var(--border)" stroke-width="1" />
                  <!-- globe glyph -->
                  <g stroke="var(--l-accent)" stroke-width="1.1" fill="none" opacity="0.9">
                    <circle cx="74" cy="126" r="8" />
                    <ellipse cx="74" cy="126" rx="3.5" ry="8" />
                    <line x1="66" y1="126" x2="82" y2="126" />
                  </g>
                  <text x="74" y="152" text-anchor="middle" font-family="var(--font-mono)" font-size="10.5" font-weight="700" class="fill-foreground select-none">app ingress</text>
                  <text x="74" y="166" text-anchor="middle" font-family="var(--font-mono)" font-size="8.5" class="fill-current text-muted-foreground select-none">external · load-balanced</text>
                </g>

                <!-- Fan-out edges + packets -->
                <g v-for="edge in edges" :key="edge.id" fill="none">
                  <path
                    :d="edge.d"
                    stroke="currentColor" stroke-width="1.25" stroke-linecap="round"
                    :class="edge.active ? 'text-foreground/20' : 'text-[var(--err)]/40'"
                    :stroke-dasharray="edge.active ? 'none' : '2 6'"
                  />
                  <template v-if="edge.active">
                    <circle r="3" fill="var(--l-accent)" opacity="0.9">
                      <animateMotion :dur="`${1.7 + edge.id * 0.3}s`" repeatCount="indefinite" :path="edge.d" />
                    </circle>
                    <circle r="3" fill="var(--l-accent)" opacity="0.45">
                      <animateMotion :dur="`${2.1 + edge.id * 0.25}s`" repeatCount="indefinite" begin="0.9s" :path="edge.d" />
                    </circle>
                  </template>
                </g>

                <!-- Server blades -->
                <g
                  v-for="node in nodes"
                  :key="node.id"
                  :style="{ cursor: node.status === 'up' && !busy ? 'pointer' : 'default' }"
                  role="button"
                  :aria-label="`Kill ${node.label}`"
                  @click="kill(node)"
                >
                  <rect
                    :x="node.x" :y="node.y" :width="BLADE_W" :height="BLADE_H" rx="10"
                    class="fill-card"
                    :stroke="node.status === 'up' ? 'var(--border)' : nodeColor(node)"
                    stroke-width="1"
                    :stroke-dasharray="node.status === 'down' ? '4 4' : 'none'"
                    :opacity="node.status === 'down' ? 0.6 : 1"
                  />
                  <!-- consensus-leader marker; every illustrated node is also a worker -->
                  <rect
                    v-if="node.controlRole === 'leader' && node.status === 'up'"
                    :x="node.x" :y="node.y + 12" width="3" :height="BLADE_H - 24" rx="1.5"
                    fill="var(--l-accent)"
                  />
                  <!-- status LED -->
                  <circle
                    :cx="node.x + 22" :cy="node.y + 24" r="3.5"
                    :class="{ 'l-blink': node.status !== 'up' }"
                    :fill="nodeColor(node)"
                  />
                  <text
                    :x="node.x + 34" :y="node.y + 28"
                    font-family="var(--font-mono)" font-size="12" font-weight="700"
                    class="fill-foreground select-none"
                    :opacity="node.status === 'down' ? 0.55 : 1"
                  >{{ node.label }}</text>
                  <text
                    :x="node.x + BLADE_W - 14" :y="node.y + 28" text-anchor="end"
                    font-family="var(--font-mono)" font-size="9"
                    class="select-none"
                    :fill="node.status !== 'up' ? nodeColor(node) : node.controlRole === 'leader' ? 'var(--l-accent)' : 'var(--muted-foreground)'"
                  >{{ node.status === 'up' ? `${node.controlRole} · worker` : node.status === 'down' ? 'offline' : 'joining…' }}</text>
                  <!-- load bar: survivors absorb the dead node's share -->
                  <rect
                    :x="node.x + 20" :y="node.y + 40" width="114" height="6" rx="3"
                    fill="currentColor" class="text-foreground/8"
                  />
                  <rect
                    :x="node.x + 20" :y="node.y + 40" :width="loadWidth(node)" height="6" rx="3"
                    fill="var(--l-accent)"
                    :opacity="node.status === 'up' ? 0.9 : 0"
                    style="transition: width 0.8s cubic-bezier(0.22, 1, 0.36, 1), opacity 0.4s ease;"
                  />
                </g>
              </svg>
              <div class="pointer-events-none absolute bottom-3 left-5 font-mono text-[11px] text-muted-foreground/70">
                // click a server to take it down
              </div>
            </div>

            <!-- Event log -->
            <div class="flex min-h-[260px] flex-col justify-end gap-1 overflow-hidden px-5 py-5 font-mono text-[11.5px] leading-[1.9] sm:text-xs lg:min-h-0">
              <div v-for="entry in log" :key="entry.id" class="flex min-w-0 items-baseline gap-2.5 sm:gap-3">
                <span class="shrink-0 tabular-nums text-muted-foreground/60">{{ entry.t }}</span>
                <span
                  class="size-1.5 shrink-0 self-center rounded-full"
                  :style="{ backgroundColor: levelColor[entry.level] }"
                />
                <span class="truncate text-foreground/85">{{ entry.msg }}</span>
              </div>
              <div class="flex gap-3 text-muted-foreground/50">
                <span class="l-caret">▌</span>
              </div>
            </div>
          </div>

          <!-- Status chips -->
          <div class="flex flex-wrap items-center gap-2 border-t border-border px-5 py-3.5 sm:px-6">
            <span class="l-chip">
              <span class="size-1.5 rounded-full" :class="{ 'l-blink': clusterState !== 'HEALTHY' }" :style="{ backgroundColor: stateColor }" />
              cluster <span class="font-bold" :style="{ color: stateColor }">{{ clusterState.toLowerCase() }}</span>
            </span>
            <span class="l-chip">nodes <span class="font-bold text-foreground tabular-nums">{{ upCount }}/3</span></span>
            <span class="l-chip">routed <span class="font-bold text-foreground tabular-nums">{{ routed.toLocaleString('en-US') }}</span></span>
            <span class="l-chip">mode <span class="font-bold text-foreground">illustrative</span></span>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>
