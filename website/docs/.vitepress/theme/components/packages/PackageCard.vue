<script setup lang="ts">
interface Props {
  name: string
  version: string
  description: string
  downloadCount?: number
  owner?: string
}
const props = defineProps<Props>()

// Navigate to per-package page using query param pattern
function goToPackage() {
  window.location.href = `/packages/package?name=${encodeURIComponent(props.name)}`
}
</script>

<template>
  <div
    class="group cursor-pointer rounded-xl border border-border bg-card p-5 transition-all duration-200 hover:-translate-y-1 hover:border-[color-mix(in_oklab,var(--brand)_45%,var(--border))] hover:shadow-[0_16px_32px_-16px_color-mix(in_oklab,var(--brand)_35%,transparent)]"
    @click="goToPackage"
  >
    <div class="mb-2 flex items-start justify-between gap-3">
      <h3 class="break-all font-mono text-sm font-semibold text-foreground transition-colors group-hover:text-brand">
        {{ name }}
      </h3>
      <span class="shrink-0 rounded-md bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">
        v{{ version }}
      </span>
    </div>
    <p class="mb-3 line-clamp-2 text-sm text-muted-foreground">
      {{ description || 'No description.' }}
    </p>
    <div class="flex items-center gap-3 text-xs text-muted-foreground/80">
      <span v-if="owner">by <a :href="`https://github.com/${owner}`" class="transition-colors hover:text-brand" @click.stop>{{ owner }}</a></span>
      <span v-if="downloadCount !== undefined">{{ downloadCount.toLocaleString() }} downloads</span>
    </div>
  </div>
</template>
