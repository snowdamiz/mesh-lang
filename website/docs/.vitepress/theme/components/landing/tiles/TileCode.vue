<script setup lang="ts">
import type { LandingData } from '../landing.data.mjs'

defineProps<{ art: LandingData['art'] }>()
</script>

<template>
  <div class="tile-art tile-art--start">
    <pre class="tile-code code"><code v-html="art.pipes" /></pre>
  </div>
</template>
