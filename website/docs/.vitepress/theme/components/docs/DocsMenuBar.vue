<script setup lang="ts">
import { useData } from 'vitepress'
import { useMenu } from '../../composables/useMenu'

const { page } = useData()
const { isOpen, panelId, toggle } = useMenu('docs')
</script>

<template>
  <div class="docs-mbar">
    <button type="button" aria-label="Docs menu" :aria-controls="panelId" :aria-expanded="isOpen" @click="toggle">
      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M2 4h12M2 8h12M2 12h8" /></svg>
      Menu
    </button>
    <span>{{ page.title }}</span>
  </div>
</template>
