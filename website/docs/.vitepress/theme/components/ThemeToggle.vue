<script setup lang="ts">
import { onMounted } from 'vue'
import { useData } from 'vitepress'
import { THEME_COLOR, THEME_KEY } from '../site'

// The head script in config.mts already set <html>'s class; sync after hydration.
const { isDark } = useData()
onMounted(() => { isDark.value = document.documentElement.classList.contains('dark') })

function toggle() {
  const dark = !isDark.value
  isDark.value = dark
  document.documentElement.classList.toggle('dark', dark)
  document.querySelector('meta[name="theme-color"]')?.setAttribute('content', THEME_COLOR[dark ? 'dark' : 'light'])
  try { localStorage.setItem(THEME_KEY, dark ? 'dark' : 'light') } catch {}
}
</script>

<template>
  <button type="button" class="icon-btn theme-btn" :aria-label="isDark ? 'Switch to light theme' : 'Switch to dark theme'" @click="toggle">
    <svg class="sun" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><circle cx="8" cy="8" r="3" /><path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.06 1.06M11.54 11.54l1.06 1.06M3.4 12.6l1.06-1.06M11.54 4.46l1.06-1.06" /></svg>
    <svg class="moon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" aria-hidden="true"><path d="M13.5 9.6A5.5 5.5 0 0 1 6.4 2.5a5.5 5.5 0 1 0 7.1 7.1Z" /></svg>
  </button>
</template>
