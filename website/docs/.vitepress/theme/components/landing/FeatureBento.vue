<script setup lang="ts">
import type { Component } from 'vue'
import type { LandingData, TileId } from './landing.data.mjs'
import { vReveal } from '../../composables/motion'
import TileCode from './tiles/TileCode.vue'
import TileSupervisor from './tiles/TileSupervisor.vue'
import TileDiagnostic from './tiles/TileDiagnostic.vue'
import TileBuild from './tiles/TileBuild.vue'
import TilePlaintext from './tiles/TilePlaintext.vue'

defineProps<{ tiles: LandingData['features']['tiles']; art: LandingData['art'] }>()

const ART: Record<TileId, Component> = {
  syntax: TileCode,
  supervisor: TileSupervisor,
  pid: TileDiagnostic,
  binary: TileBuild,
  safety: TilePlaintext,
}
</script>

<template>
  <div class="bento">
    <article v-for="tile in tiles" :key="tile.id" v-reveal class="tile" :class="[`tile--${tile.span}`, { 'tile--dark on-night': tile.dark }]">
      <div class="tile-text">
        <h3 class="tile-title">{{ tile.title }}</h3>
        <p class="tile-line">{{ tile.line }}</p>
      </div>
      <component :is="ART[tile.id]" :art="art" />
    </article>
  </div>
</template>
