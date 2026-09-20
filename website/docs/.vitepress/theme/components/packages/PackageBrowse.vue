<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { Search } from 'lucide-vue-next'
import PackageCard from './PackageCard.vue'
import PackageList from './PackageList.vue'

const REGISTRY_URL = 'https://registry.meshlang.dev'
const FEATURED_COUNT = 6

interface PackageItem {
  name: string
  version: string
  description: string
  download_count?: number
  owner?: string
}

const pageTitle = 'Packages'

const allPackages = ref<PackageItem[]>([])
const featured = ref<PackageItem[]>([])
const rest = ref<PackageItem[]>([])
const searchQuery = ref('')
const loading = ref(true)
const error = ref<string | null>(null)

async function fetchPackages() {
  loading.value = true
  error.value = null
  try {
    const q = searchQuery.value.trim()
    const url = q
      ? `${REGISTRY_URL}/api/v1/packages?search=${encodeURIComponent(q)}`
      : `${REGISTRY_URL}/api/v1/packages`

    const resp = await fetch(url)
    if (!resp.ok) throw new Error(`Registry returned ${resp.status}`)
    const data: PackageItem[] = await resp.json()

    if (q) {
      // In search mode, show flat list (no featured split)
      allPackages.value = data
      featured.value = []
      rest.value = data
    } else {
      // Browse mode: top 6 by download_count as featured cards
      allPackages.value = data
      featured.value = data.slice(0, FEATURED_COUNT)
      rest.value = data.slice(FEATURED_COUNT)
    }
  } catch (e: any) {
    error.value = e.message ?? 'Failed to load packages'
  } finally {
    loading.value = false
  }
}

// Debounce search to avoid hammering the API on every keystroke
let searchTimer: ReturnType<typeof setTimeout>
function onSearchInput() {
  clearTimeout(searchTimer)
  searchTimer = setTimeout(fetchPackages, 300)
}

onMounted(fetchPackages)
</script>

<template>
  <div class="mx-auto max-w-5xl px-4 py-12 sm:px-6">
    <div class="mb-8">
      <span class="inline-flex items-center gap-2.5 font-mono text-xs tracking-[0.1em] text-muted-foreground">
        <span class="size-1.5 rounded-full bg-brand" aria-hidden="true" />
        the registry
      </span>
      <h1
        class="mt-3 text-4xl font-bold tracking-tight text-foreground"
        style="font-family: 'Bricolage Grotesque', var(--font-sans); font-optical-sizing: auto; letter-spacing: -0.03em;"
      >
        {{ pageTitle }}
      </h1>
      <p class="mt-2 text-muted-foreground">Browse and install Mesh packages.</p>
    </div>

    <!-- Search pill -->
    <div class="mb-10">
      <div class="relative">
        <Search class="pointer-events-none absolute left-5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <input
          v-model="searchQuery"
          @input="onSearchInput"
          type="text"
          placeholder="Search packages by name or description…"
          class="w-full rounded-xl border border-border bg-card py-3 pl-12 pr-5 text-sm text-foreground placeholder-muted-foreground/70 shadow-[0_1px_2px_rgba(0,0,0,0.03)] transition-colors focus:border-[color-mix(in_oklab,var(--brand)_55%,var(--border))] focus:outline-none focus:ring-2 focus:ring-brand/25"
        />
      </div>
    </div>

    <!-- Loading state -->
    <div v-if="loading" class="flex items-center justify-center py-16 text-muted-foreground">
      <span>Loading packages…</span>
    </div>

    <!-- Error state -->
    <div v-else-if="error" class="py-12 text-center text-muted-foreground">
      <p class="text-sm">{{ error }}</p>
      <button
        @click="fetchPackages"
        class="mt-4 inline-flex items-center rounded-lg border border-border px-4 py-1.5 text-sm font-medium text-foreground transition-colors hover:border-[color-mix(in_oklab,var(--brand)_55%,var(--border))] hover:text-brand"
      >
        Retry
      </button>
    </div>

    <!-- Empty state -->
    <div v-else-if="allPackages.length === 0" class="py-12 text-center text-muted-foreground">
      <p class="text-sm">{{ searchQuery ? `No packages found for "${searchQuery}".` : 'No packages published yet.' }}</p>
    </div>

    <!-- Browse mode: featured cards + list -->
    <template v-else>
      <!-- Featured section (only shown when not searching) -->
      <div v-if="!searchQuery && featured.length > 0" class="mb-12">
        <h2 class="mb-4 font-mono text-xs font-semibold tracking-[0.08em] text-foreground">featured</h2>
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          <PackageCard
            v-for="pkg in featured"
            :key="pkg.name"
            :name="pkg.name"
            :version="pkg.version"
            :description="pkg.description"
            :download-count="pkg.download_count"
            :owner="pkg.owner"
          />
        </div>
      </div>

      <!-- All packages list (or search results) -->
      <div>
        <h2 v-if="!searchQuery && rest.length > 0" class="mb-3 font-mono text-xs font-semibold tracking-[0.08em] text-foreground">all packages</h2>
        <h2 v-else-if="searchQuery" class="mb-3 font-mono text-xs font-semibold tracking-[0.08em] text-foreground">
          {{ allPackages.length }} result{{ allPackages.length !== 1 ? 's' : '' }} for "{{ searchQuery }}"
        </h2>
        <PackageList :packages="searchQuery ? allPackages : rest" />
      </div>
    </template>
  </div>
</template>
