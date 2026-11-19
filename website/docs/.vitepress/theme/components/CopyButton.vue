<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'

const props = defineProps<{ text: string; label?: string; action: string }>()

type Result = 'copied' | 'failed'
const result = ref<Result | null>(null)
const SHOWN: Record<Result, { label: string; said: string }> = {
  copied: { label: 'Copied', said: 'Copied to clipboard' },
  failed: { label: 'Copy failed', said: 'Could not copy to the clipboard' },
}
const shown = computed(() => (result.value ? SHOWN[result.value] : null))
let reset: ReturnType<typeof setTimeout> | undefined
let disposed = false

function copyWithSelection(text: string): boolean {
  const previous = document.activeElement as HTMLElement | null
  const area = document.createElement('textarea')
  area.value = text
  area.setAttribute('readonly', '')
  area.style.cssText = 'position:fixed;top:0;left:0;opacity:0'
  document.body.append(area)
  area.focus({ preventScroll: true })
  area.select()
  try {
    return document.execCommand('copy')
  } catch {
    return false
  } finally {
    area.remove()
    previous?.focus({ preventScroll: true })
  }
}

async function writeClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    return copyWithSelection(text)
  }
}

async function copy() {
  const copied = await writeClipboard(props.text)
  if (disposed) return
  result.value = copied ? 'copied' : 'failed'
  clearTimeout(reset)
  reset = setTimeout(() => (result.value = null), 1600)
}

onBeforeUnmount(() => {
  disposed = true
  clearTimeout(reset)
})
</script>

<template>
  <button type="button" :class="[label ? 'copy' : 'icon-btn', { done: result === 'copied', failed: result === 'failed' }]" :aria-label="shown?.label ?? action" @click="copy">
    <svg v-if="result === 'copied'" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M3 8.5l3.2 3L13 4.5" /></svg>
    <svg v-else-if="result === 'failed'" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
    <svg v-else viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="5.5" y="5.5" width="8" height="8" rx="1.5" /><path d="M10.5 5.5V3.5A1.5 1.5 0 0 0 9 2H3.5A1.5 1.5 0 0 0 2 3.5V9a1.5 1.5 0 0 0 1.5 1.5h2" /></svg>
    <span v-if="label">{{ shown?.label ?? label }}</span>
    <span class="sr-only" aria-live="polite">{{ shown?.said ?? '' }}</span>
  </button>
</template>
