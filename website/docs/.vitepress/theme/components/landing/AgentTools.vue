<script setup lang="ts">
import type { LandingData } from './landing.data.mjs'
import { vReveal } from '../../composables/motion'

defineProps<{ agents: LandingData['agents'] }>()
</script>

<template>
  <div class="agents">
    <div v-reveal class="codecard on-night agents-code">
      <p class="agents-cmd"><span aria-hidden="true">$ </span>{{ agents.command }}</p>
      <pre class="code"><code v-html="agents.diagnostic" /></pre>
    </div>
    <ul class="agents-list">
      <li v-for="item in agents.items" :key="item.command" v-reveal>
        <code class="agents-item-cmd">{{ item.command }}</code>
        <p>{{ item.line }}</p>
      </li>
    </ul>
  </div>
</template>
