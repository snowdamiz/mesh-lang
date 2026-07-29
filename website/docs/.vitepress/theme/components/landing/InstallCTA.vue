<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useScrollReveal } from '@/composables/useScrollReveal'

const { observe } = useScrollReveal()
const root = ref<HTMLElement>()
const copied = ref(false)

const installCommand = 'curl -sSf https://meshlang.dev/install.sh | sh'

async function copyCommand() {
  try {
    await navigator.clipboard.writeText(installCommand)
    copied.value = true
    setTimeout(() => {
      copied.value = false
    }, 2000)
  } catch {
    // Clipboard API not available
  }
}

onMounted(() => {
  if (root.value) observe(root.value)
})
</script>

<template>
  <section class="mx-auto max-w-6xl px-4 py-12 sm:px-6 md:py-16">
    <!-- Deep-green capsule — the one dark moment on the page -->
    <div
      ref="root"
      class="reveal-zoom relative overflow-hidden rounded-[1.25rem] px-6 py-16 text-center sm:px-12 md:px-16 md:py-20"
      style="background: linear-gradient(165deg, oklch(0.24 0.035 185), oklch(0.15 0.025 200) 75%);"
    >
      <!-- Inner aurora -->
      <div
        class="l-aurora l-breathe -top-24 left-1/2 h-72 w-[36rem] -translate-x-1/2"
        style="background: oklch(0.8 0.15 163 / 0.22);"
      />
      <div
        class="l-aurora -bottom-32 right-[-8%] h-64 w-96 opacity-60"
        style="background: oklch(0.7 0.12 200 / 0.2);"
      />

      <div class="relative">
        <span
          class="inline-flex items-center gap-2.5 rounded-lg border px-3.5 py-1.5 font-mono text-[11px] tracking-[0.1em]"
          style="border-color: oklch(1 0 0 / 0.16); color: oklch(0.87 0.03 170);"
        >
          <span class="l-ping" />
          install the toolchain
        </span>

        <h2 class="font-display mx-auto mt-7 max-w-3xl text-[clamp(2.5rem,6.5vw,4.25rem)] font-extrabold leading-[1.04]" style="color: oklch(0.97 0.008 170);">
          One <em class="l-fancy" style="color: oklch(0.85 0.15 163);">command</em> to get started.
        </h2>
        <p class="mx-auto mt-5 max-w-md text-base leading-relaxed" style="color: oklch(0.75 0.02 180);">
          Use the command below on macOS or Linux. The getting-started guide includes the Windows PowerShell installer.
        </p>

        <!-- Install pill -->
        <div
          class="mx-auto mt-10 flex w-fit max-w-full items-center gap-3 rounded-xl border py-2.5 pl-5 pr-2.5 font-mono text-[13px] sm:text-sm"
          style="border-color: oklch(1 0 0 / 0.16); background: oklch(0 0 0 / 0.3); color: oklch(0.94 0.01 170);"
        >
          <span class="select-none font-bold" style="color: oklch(0.8 0.15 163);">$</span>
          <code class="install-cmd min-w-0 truncate">{{ installCommand }}</code>
          <button
            class="shrink-0 rounded-lg px-3.5 py-2 font-mono text-[11px] font-bold tracking-[0.08em] transition-all hover:-translate-y-px"
            :style="{
              background: copied ? 'oklch(0.8 0.15 163)' : 'oklch(1 0 0 / 0.12)',
              color: copied ? 'oklch(0.17 0.035 175)' : 'oklch(0.95 0.01 170)',
            }"
            :aria-label="copied ? 'Copied' : 'Copy install command'"
            @click="copyCommand"
          >
            {{ copied ? '✓ copied' : 'copy' }}
          </button>
        </div>

        <!-- CTAs -->
        <div class="mx-auto mt-8 flex flex-col items-center justify-center gap-3 sm:flex-row">
          <a
            href="/docs/getting-started/"
            class="l-btn w-full sm:w-auto"
            style="background: oklch(0.8 0.15 163); color: oklch(0.17 0.035 175); box-shadow: 0 12px 32px -10px oklch(0.8 0.15 163 / 0.5);"
          >
            Get started <span aria-hidden="true">→</span>
          </a>
          <a
            href="/docs/distributed/"
            class="l-btn w-full sm:w-auto"
            style="border: 1px solid oklch(1 0 0 / 0.2); color: oklch(0.95 0.01 170);"
          >
            Distributed docs
          </a>
        </div>
      </div>
    </div>
  </section>
</template>
