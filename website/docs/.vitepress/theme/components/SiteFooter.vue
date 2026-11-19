<script setup lang="ts">
import lockup from '../assets/mesh-lockup.svg?raw'
import { FOOTER_COLUMNS } from '../site'
</script>

<template>
  <footer class="footer">
    <div class="wrap footer-in">
      <div class="footer-brand"><a class="brand" href="/" aria-label="Mesh home" v-html="lockup" /></div>
      <div v-for="column in FOOTER_COLUMNS" :key="column.title">
        <h2>{{ column.title }}</h2>
        <ul>
          <li v-for="link in column.links" :key="link.href"><a :href="link.href">{{ link.text }}</a></li>
        </ul>
      </div>
    </div>
    <div class="wrap footer-base"><span>MIT licensed</span><span>Mesh is in active development.</span></div>
  </footer>
</template>
