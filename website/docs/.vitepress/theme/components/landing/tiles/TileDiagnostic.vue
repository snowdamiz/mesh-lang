<script setup lang="ts">
import type { LandingData } from '../landing.data.mjs'

defineProps<{ art: LandingData['art'] }>()
</script>

<template>
  <div class="tile-art">
    <figure class="diag-mini" role="img" :aria-label="`Compiler error ${art.pid.code}, ${art.pid.message}: expected Message, found String`">
      <p class="diag-mini-head" aria-hidden="true"><span class="err-code">{{ art.pid.code }}</span>{{ art.pid.message }}</p>
      <p class="diag-mini-src code" aria-hidden="true"><span class="sq" v-html="art.pid.src" /></p>
      <p class="diag-mini-label" aria-hidden="true">{{ art.pid.label }}</p>
    </figure>
  </div>
</template>
