<script setup lang="ts">
import HeroSection from './HeroSection.vue'
import FailoverSim from './FailoverSim.vue'
import StdlibGrid from './StdlibGrid.vue'
import BenchmarkSection from './BenchmarkSection.vue'
import InstallCTA from './InstallCTA.vue'
import LandingFooter from './LandingFooter.vue'
</script>

<template>
  <div class="landing min-h-screen">
    <div class="l-frame">
      <HeroSection />
      <FailoverSim />
      <StdlibGrid />
      <BenchmarkSection />
      <InstallCTA />
      <LandingFooter />
    </div>
  </div>
</template>
