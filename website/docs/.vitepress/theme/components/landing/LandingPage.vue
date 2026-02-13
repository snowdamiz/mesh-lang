<script setup lang="ts">
import HeroSection from './HeroSection.vue'
import FeatureShowcase from './FeatureShowcase.vue'
import WhyMesh from './WhyMesh.vue'
</script>

<template>
  <div class="min-h-screen">
    <HeroSection />
    <FeatureShowcase />
    <WhyMesh />
    <!-- Footer -->
    <footer class="border-t border-border py-8">
      <div class="mx-auto max-w-5xl px-4 text-center text-xs text-muted-foreground">
        The Mesh Programming Language
      </div>
    </footer>
  </div>
</template>
