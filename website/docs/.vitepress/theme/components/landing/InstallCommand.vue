<script setup lang="ts">
import { ref } from 'vue'
import { Check, Copy } from 'lucide-vue-next'

const props = withDefaults(defineProps<{ command: string; prompt?: string }>(), { prompt: '$' })
const copied = ref(false)
let reset: ReturnType<typeof setTimeout> | undefined

async function copy() {
  try {
    await navigator.clipboard.writeText(props.command)
  } catch {
    return // no clipboard access; the command stays selectable
  }
  copied.value = true
  clearTimeout(reset)
  reset = setTimeout(() => (copied.value = false), 1600)
}
</script>

<template>
  <div class="l-install">
    <span class="shrink-0 select-none text-muted-foreground" aria-hidden="true">{{ prompt }}</span>
    <code class="min-w-0 flex-1 truncate">{{ command }}</code>
    <button type="button" class="l-install-copy" :aria-label="`Copy ${command}`" @click="copy">
      <Check v-if="copied" class="size-4 text-[var(--l-accent)]" />
      <Copy v-else class="size-4" />
    </button>
    <span class="sr-only" aria-live="polite">{{ copied ? 'Copied to clipboard' : '' }}</span>
  </div>
</template>
