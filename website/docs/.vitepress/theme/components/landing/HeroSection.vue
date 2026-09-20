<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useData } from 'vitepress'
import { getHighlighter, highlightCode } from '@/composables/useShiki'

const { theme } = useData()

const highlightedHtml = ref('')

const heroCode = `# work.mpl
@cluster
pub fn add() -> Int do
  1 + 1
end

# api/router.mpl
from Api.Todos import (
  handle_get_todo,
  handle_list_todos
)

pub fn build_router() do
  HTTP.router()
    |> HTTP.on_get("/todos",
         HTTP.clustered(handle_list_todos))
    |> HTTP.on_get("/todos/:id",
         HTTP.clustered(handle_get_todo))
end`

const specs = [
  { key: 'compiled', value: 'LLVM native binaries' },
  { key: 'typed', value: 'Hindley–Milner inference' },
  { key: 'concurrent', value: 'actor-model runtime' },
  { key: 'distributed', value: '@cluster is a primitive' },
]

// Internal hairlines of the 2×2 (mobile) / 1×4 (desktop) instrument panel
const specBorders = [
  '',
  'border-l border-border/70',
  'border-t border-border/70 lg:border-t-0 lg:border-l',
  'border-l border-t border-border/70 lg:border-t-0',
]

onMounted(async () => {
  try {
    const hl = await getHighlighter()
    highlightedHtml.value = highlightCode(hl, heroCode)
  } catch {
    // Highlighting failed -- raw code fallback remains visible
  }
})
</script>

<template>
  <section class="relative overflow-hidden">
    <!-- Atmosphere: soft aurora glows -->
    <div class="l-enter-fade">
      <div
        class="l-aurora l-breathe -top-40 left-[8%] h-[26rem] w-[34rem]"
        style="background: color-mix(in oklab, var(--l-accent) 11%, transparent);"
      />
      <div
        class="l-aurora -top-24 right-[-10%] h-[22rem] w-[30rem] opacity-50"
        style="background: color-mix(in oklab, var(--l-accent-hi) 8%, transparent); animation-delay: 2s;"
      />
    </div>

    <!-- Organic mesh: curved links between drifting nodes -->
    <div class="l-enter-fade pointer-events-none absolute inset-x-0 top-0" style="animation-delay: 0.25s">
    <svg
      class="l-drift h-[36rem] w-full min-w-[1200px] text-[var(--l-accent)]"
      viewBox="0 0 1400 620"
      fill="none"
      aria-hidden="true"
      style="mask-image: linear-gradient(to bottom, black 30%, transparent 95%); -webkit-mask-image: linear-gradient(to bottom, black 30%, transparent 95%);"
    >
      <g stroke="currentColor" stroke-width="1" opacity="0.12">
        <path id="l-mesh-a" d="M120 150 Q 320 40 560 130" />
        <path d="M560 130 Q 800 220 1060 110" />
        <path id="l-mesh-b" d="M1060 110 Q 1240 180 1360 320" />
        <path d="M120 150 Q 200 330 430 380" />
        <path d="M430 380 Q 640 460 900 370" />
        <path id="l-mesh-c" d="M560 130 Q 500 280 430 380" />
        <path d="M900 370 Q 990 220 1060 110" />
        <path d="M900 370 Q 1140 420 1360 320" />
      </g>
      <g fill="currentColor" opacity="0.3">
        <circle cx="120" cy="150" r="4" />
        <circle cx="560" cy="130" r="5" />
        <circle cx="1060" cy="110" r="4" />
        <circle cx="430" cy="380" r="5" />
        <circle cx="900" cy="370" r="4" />
        <circle cx="1360" cy="320" r="4" />
      </g>
      <circle r="3" fill="currentColor" opacity="0.7">
        <animateMotion dur="9s" repeatCount="indefinite">
          <mpath href="#l-mesh-a" />
        </animateMotion>
      </circle>
      <circle r="3" fill="currentColor" opacity="0.55">
        <animateMotion dur="11s" begin="3s" repeatCount="indefinite">
          <mpath href="#l-mesh-c" />
        </animateMotion>
      </circle>
      <circle r="3" fill="currentColor" opacity="0.55">
        <animateMotion dur="10s" begin="5.5s" repeatCount="indefinite">
          <mpath href="#l-mesh-b" />
        </animateMotion>
      </circle>
    </svg>
    </div>

    <div class="relative mx-auto w-full max-w-6xl px-4 pt-16 pb-16 sm:px-6 sm:pt-24 sm:pb-20 lg:pt-28 lg:pb-24">
      <!-- Kicker row -->
      <div class="l-enter flex flex-wrap items-center gap-3">
        <span class="l-chip !text-foreground/75">
          <span class="l-ping" />
          v{{ theme.meshVersion }} — in development
        </span>
        <span class="hidden font-mono text-xs tracking-[0.08em] text-muted-foreground sm:inline">a language for distributed systems</span>
      </div>

      <div class="mt-12 grid items-center gap-14 lg:grid-cols-[1.12fr_1fr] lg:gap-16">
        <!-- Headline block -->
        <div class="min-w-0">
          <h1 class="font-display l-enter text-[clamp(2.9rem,8.5vw,5.4rem)] font-extrabold leading-[1.02] text-foreground" style="animation-delay: 0.08s">
            Write a server.<br />
            Ship a <em class="l-fancy">fleet.</em>
          </h1>

          <p class="l-enter mt-8 max-w-md text-base leading-relaxed text-muted-foreground sm:text-lg" style="text-wrap: pretty; animation-delay: 0.18s">
            Mesh is a compiled language with typed actors and distribution in the programming model. Declare eligible
            work with <span class="font-mono text-foreground">@cluster</span>; the runtime applies the routing,
            continuity, and capacity policy in your manifest.
          </p>

          <div class="l-enter mt-10 flex flex-col gap-3 sm:flex-row sm:items-center" style="animation-delay: 0.28s">
            <a href="/docs/getting-started/" class="l-btn l-btn-primary">
              Get started <span aria-hidden="true">→</span>
            </a>
            <a href="https://github.com/hyperpush-org/mesh-lang" target="_blank" rel="noopener" class="l-btn l-btn-soft">
              View source
            </a>
          </div>
        </div>

        <!-- Code window -->
        <div class="l-enter-zoom relative min-w-0" style="animation-delay: 0.15s">
          <div
            class="l-aurora -bottom-16 -right-10 h-56 w-72 opacity-60"
            style="background: color-mix(in oklab, var(--l-accent) 10%, transparent);"
          />
          <div class="l-window relative">
            <div class="l-window-head">
              <span class="l-window-dots" aria-hidden="true"><span /><span /><span /></span>
              <span class="min-w-0 truncate">work.mpl · api/router.mpl</span>
              <span class="l-chip shrink-0 !px-2.5 !py-0.5 !text-[10px] !text-foreground/70">mesh</span>
            </div>
            <div
              v-if="highlightedHtml"
              v-html="highlightedHtml"
              class="vp-code landing-code w-full max-w-full font-mono"
            />
            <pre
              v-else
              class="max-w-full overflow-x-auto px-6 py-4 font-mono text-[0.8125rem] leading-[1.9] text-foreground"><code>{{ heroCode }}</code></pre>
            <div class="flex flex-wrap items-center justify-between gap-2 border-t border-border px-5 py-2.5 font-mono text-[11px] text-muted-foreground">
              <span>work declaration: <span class="text-foreground">@cluster</span></span>
              <span>runtime policy: <span class="text-foreground">mesh.toml</span></span>
            </div>
          </div>
        </div>
      </div>

      <!-- Spec instrument panel -->
      <div class="l-enter-zoom mt-16 overflow-hidden rounded-xl border border-border bg-card/60 backdrop-blur-sm" style="animation-delay: 0.4s">
        <div class="grid grid-cols-2 lg:grid-cols-4">
          <div
            v-for="(spec, i) in specs"
            :key="spec.key"
            class="group p-5 sm:p-6"
            :class="specBorders[i]"
          >
            <!-- Pictogram -->
            <div class="h-10 text-muted-foreground/60 transition-colors duration-300 group-hover:text-muted-foreground" aria-hidden="true">
              <!-- compiled: source lines feed the binary -->
              <svg v-if="spec.key === 'compiled'" viewBox="0 0 56 40" class="h-full w-auto" fill="none">
                <g stroke="currentColor" stroke-width="1.5" stroke-linecap="round" class="l-compile-lines">
                  <line x1="4" y1="12" x2="18" y2="12" />
                  <line x1="4" y1="20" x2="13" y2="20" style="animation-delay: 0.5s" />
                  <line x1="4" y1="28" x2="16" y2="28" style="animation-delay: 1s" />
                </g>
                <path d="M24 20h7m0 0-3-3m3 3-3 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                <rect x="38" y="9" width="14" height="22" rx="4.5" fill="var(--l-accent)" opacity="0.85" />
                <rect x="42" y="14" width="6" height="1.5" rx="0.75" fill="var(--l-accent-ink)" opacity="0.55" />
                <rect x="42" y="19" width="6" height="1.5" rx="0.75" fill="var(--l-accent-ink)" opacity="0.55" />
                <rect x="42" y="24" width="4" height="1.5" rx="0.75" fill="var(--l-accent-ink)" opacity="0.55" />
              </svg>
              <!-- typed: x inferred to Int -->
              <svg v-else-if="spec.key === 'typed'" viewBox="0 0 56 40" class="h-full w-auto" fill="none">
                <text x="5" y="26" font-family="var(--font-mono)" font-size="14" font-weight="600" fill="currentColor">x</text>
                <path d="M15 20c5-7 10-7 15-1" stroke="currentColor" stroke-width="1.25" stroke-dasharray="2.5 3" stroke-linecap="round" />
                <path d="m30 19 .4-4m-.4 4-3.8-1.4" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" />
                <text x="33" y="26" font-family="var(--font-mono)" font-size="13" font-weight="700" fill="var(--l-accent)">Int</text>
              </svg>
              <!-- concurrent: packets riding actor lanes -->
              <svg v-else-if="spec.key === 'concurrent'" viewBox="0 0 56 40" class="h-full w-auto" fill="none">
                <g stroke="currentColor" stroke-width="1.5" stroke-linecap="round" opacity="0.3">
                  <line x1="4" y1="8" x2="52" y2="8" />
                  <line x1="4" y1="20" x2="52" y2="20" />
                  <line x1="4" y1="32" x2="52" y2="32" />
                </g>
                <circle cx="6" cy="8" r="3" fill="var(--l-accent)" class="l-lane-dot" style="animation-duration: 2.7s" />
                <circle cx="6" cy="20" r="3" fill="var(--l-accent)" class="l-lane-dot" style="animation-duration: 3.6s; animation-delay: 0.9s" />
                <circle cx="6" cy="32" r="3" fill="var(--l-accent)" class="l-lane-dot" style="animation-duration: 3.1s; animation-delay: 1.7s" />
              </svg>
              <!-- distributed: the mesh itself -->
              <svg v-else viewBox="0 0 56 40" class="h-full w-auto" fill="none">
                <g stroke="currentColor" stroke-width="1.25" opacity="0.45">
                  <path d="M28 8Q15 15 10 30" />
                  <path d="M28 8q13 7 18 22" />
                  <path d="M10 30q18 8 36 0" />
                </g>
                <circle cx="28" cy="8" r="7" fill="var(--l-accent)" opacity="0.18" class="l-node-halo" style="transform-box: fill-box; transform-origin: center;" />
                <circle cx="28" cy="8" r="3.5" fill="var(--l-accent)" />
                <circle cx="10" cy="30" r="3" fill="var(--l-accent)" opacity="0.65" />
                <circle cx="46" cy="30" r="3" fill="var(--l-accent)" opacity="0.65" />
              </svg>
            </div>

            <div class="mt-4 font-mono text-xs font-semibold tracking-[0.08em] text-foreground">{{ spec.key }}</div>
            <div class="mt-1 text-[13px] leading-snug text-muted-foreground">{{ spec.value }}</div>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>
