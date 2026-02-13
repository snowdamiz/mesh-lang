<script setup lang="ts">
import { useData } from 'vitepress'
import NavBar from './components/NavBar.vue'

const { frontmatter } = useData()
</script>

<template>
  <div class="min-h-screen bg-background text-foreground">
    <NavBar />
    <main class="mx-auto max-w-4xl px-4 py-8">
      <Content />
    </main>
  </div>
</template>
