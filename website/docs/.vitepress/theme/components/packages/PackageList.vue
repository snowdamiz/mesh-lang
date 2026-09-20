<script setup lang="ts">
interface Package {
  name: string
  version: string
  description: string
  downloadCount?: number
}
defineProps<{ packages: Package[] }>()

function packageUrl(name: string) {
  return `/packages/package?name=${encodeURIComponent(name)}`
}
</script>

<template>
  <div class="overflow-hidden rounded-xl border border-border bg-card">
    <a
      v-for="pkg in packages"
      :key="pkg.name"
      :href="packageUrl(pkg.name)"
      class="group flex items-center gap-4 border-b border-border px-5 py-3.5 no-underline transition-colors last:border-b-0 hover:bg-muted/60"
    >
      <div class="min-w-0 flex-1">
        <span class="break-all font-mono text-sm font-semibold text-foreground transition-colors group-hover:text-brand">
          {{ pkg.name }}
        </span>
        <span class="ml-3 hidden truncate text-sm text-muted-foreground sm:inline">
          {{ pkg.description || 'No description.' }}
        </span>
      </div>
      <span class="shrink-0 rounded-md bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">v{{ pkg.version }}</span>
    </a>
  </div>
</template>
