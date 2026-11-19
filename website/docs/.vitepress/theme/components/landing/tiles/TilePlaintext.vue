<script setup lang="ts">
import type { LandingData } from '../landing.data.mjs'

const props = defineProps<{ art: LandingData['art'] }>()
const { label, source, blocked, allowed } = props.art.safety
const fan = (y: number, x: number) => `M138 94C170 94 168 ${y} ${x} ${y}`
</script>

<template>
  <div class="tile-art">
    <svg class="fan" viewBox="0 0 320 188" role="img" :aria-label="label">
      <g class="no">
        <template v-for="(name, i) in blocked" :key="name">
          <path :d="fan(14 + 32 * i, 192)" />
          <path class="x" :d="`M197 ${10 + 32 * i}l8 8M205 ${10 + 32 * i}l-8 8`" />
          <text x="216" :y="18 + 32 * i">{{ name }}</text>
        </template>
      </g>
      <g class="yes">
        <template v-for="(name, i) in allowed" :key="name">
          <path :d="fan(142 + 32 * i, 204)" />
          <circle cx="204" :cy="142 + 32 * i" r="3" />
          <text x="216" :y="146 + 32 * i">{{ name }}</text>
        </template>
      </g>
      <g class="src"><rect x="2" y="76" width="136" height="36" rx="18" /><text x="70" y="98.5" text-anchor="middle">{{ source }}</text></g>
    </svg>
  </div>
</template>
