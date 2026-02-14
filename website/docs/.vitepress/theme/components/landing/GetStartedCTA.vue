<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { Button } from '@/components/ui/button'
import { ArrowRight, Copy, Check } from 'lucide-vue-next'
import { useScrollReveal } from '@/composables/useScrollReveal'

const { observe } = useScrollReveal()
const section = ref<HTMLElement>()
const copied = ref(false)

const installCommand = 'curl -sSf https://mesh-lang.org/install | sh'

async function copyCommand() {
  try {
    await navigator.clipboard.writeText(installCommand)
    copied.value = true
    setTimeout(() => { copied.value = false }, 2000)
  } catch {
    // Clipboard API not available
  }
}

onMounted(() => {
  if (section.value) observe(section.value)
})
</script>

<template>
  <section class="relative border-t border-border py-24 md:py-32">
    <!-- Subtle radial wash -->
    <div class="absolute inset-0 bg-[radial-gradient(ellipse_60%_40%_at_50%_50%,var(--muted),transparent_70%)] opacity-50" />

    <div ref="section" class="reveal relative mx-auto max-w-2xl px-4 text-center">
      <h2 class="text-3xl font-bold tracking-tight text-foreground sm:text-4xl">
        Ready to build with Mesh?
      </h2>
      <p class="mt-4 text-lg text-muted-foreground">
        Get started in seconds.
      </p>

      <!-- Install command -->
      <div class="mx-auto mt-8 flex max-w-xl items-center gap-3 rounded-lg border border-border bg-card px-5 py-4 font-mono text-sm">
        <span class="text-muted-foreground select-none">$</span>
        <code class="flex-1 text-left text-foreground">{{ installCommand }}</code>
        <button
          @click="copyCommand"
          class="shrink-0 rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          :title="copied ? 'Copied!' : 'Copy to clipboard'"
        >
          <Check v-if="copied" class="size-4" />
          <Copy v-else class="size-4" />
        </button>
      </div>

      <!-- CTA buttons -->
      <div class="mt-8 flex items-center justify-center gap-3">
        <Button as="a" href="/docs/getting-started/" size="lg" class="h-12 px-8 rounded-lg text-base font-semibold">
          Get Started
          <ArrowRight class="ml-1.5 size-4" />
        </Button>
        <Button as="a" href="https://github.com/snowdamiz/mesh-lang" variant="outline" size="lg" class="h-12 px-8 rounded-lg text-base font-semibold">
          GitHub
        </Button>
      </div>
    </div>
  </section>
</template>
