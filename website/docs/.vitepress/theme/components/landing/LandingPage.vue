<script setup lang="ts">
import HeroSection from './HeroSection.vue'
import InfraDiff from './InfraDiff.vue'
import FailoverSim from './FailoverSim.vue'
import StdlibGrid from './StdlibGrid.vue'
import BenchmarkSection from './BenchmarkSection.vue'
import ComparisonTable from './ComparisonTable.vue'
import InstallCTA from './InstallCTA.vue'
import LandingFooter from './LandingFooter.vue'
</script>

<template>
  <div class="landing min-h-screen">
    <HeroSection />
    <InfraDiff />
    <FailoverSim />
    <StdlibGrid />
    <BenchmarkSection />
    <ComparisonTable />
    <InstallCTA />
    <LandingFooter />
  </div>
</template>
