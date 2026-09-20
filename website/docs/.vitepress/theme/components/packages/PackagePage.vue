<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'

const REGISTRY_URL = 'https://registry.meshlang.dev'

// Read package name from URL query param: ?name=owner/package-name
function getPackageName(): string {
  if (typeof window === 'undefined') return ''
  const params = new URLSearchParams(window.location.search)
  return params.get('name') ?? ''
}

interface VersionInfo {
  version: string
  sha256: string
  published_at?: string
  size_bytes?: number
  download_count?: number
}

interface PackageData {
  name: string
  description: string
  owner: string
  download_count: number
  latest: { version: string; sha256: string } | null
  readme?: string
  versions?: VersionInfo[]
}

const backLabel = '← All packages'
const packageName = ref(getPackageName())
const pkg = ref<PackageData | null>(null)
const versions = ref<VersionInfo[]>([])
const loading = ref(true)
const error = ref<string | null>(null)
const versionsExpanded = ref(false)
const copySuccess = ref(false)

const installCommand = computed(() => {
  if (!pkg.value?.latest) return ''
  return `meshpkg install ${pkg.value.name}@${pkg.value.latest.version}`
})

async function copyInstallCommand() {
  try {
    await navigator.clipboard.writeText(installCommand.value)
    copySuccess.value = true
    setTimeout(() => { copySuccess.value = false }, 2000)
  } catch {
    // Fallback: select all in a temporary input
  }
}

// Simple markdown renderer for common formatting.
function renderMarkdown(text: string): string {
  if (!text) return '<p class="text-muted-foreground">No README available.</p>'
  // Escape HTML first, then apply basic markdown transforms
  const escaped = text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')

  return escaped
    // Code blocks (``` ... ```)
    .replace(/```[\w]*\n([\s\S]*?)```/g, '<pre class="bg-muted border border-border rounded-lg p-4 overflow-x-auto text-sm my-4"><code>$1</code></pre>')
    // Inline code
    .replace(/`([^`]+)`/g, '<code class="bg-muted border border-border rounded-md px-1.5 py-0.5 text-sm font-mono">$1</code>')
    // Headers
    .replace(/^### (.+)$/gm, '<h3 class="text-lg font-semibold mt-6 mb-2 text-foreground">$1</h3>')
    .replace(/^## (.+)$/gm, '<h2 class="text-xl font-semibold mt-8 mb-3 text-foreground">$1</h2>')
    .replace(/^# (.+)$/gm, '<h1 class="text-2xl font-bold mt-8 mb-4 text-foreground">$1</h1>')
    // Bold
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    // Italic
    .replace(/\*(.+?)\*/g, '<em>$1</em>')
    // Paragraphs (double newlines)
    .replace(/\n\n/g, '</p><p class="mb-4 text-muted-foreground">')
    // Wrap in paragraph
    .replace(/^/, '<p class="mb-4 text-muted-foreground">')
    .replace(/$/, '</p>')
}

function formatDate(iso?: string): string {
  if (!iso) return ''
  try {
    return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric' })
  } catch {
    return iso
  }
}

function formatBytes(bytes?: number): string {
  if (!bytes) return ''
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

async function loadPackage() {
  const name = packageName.value
  if (!name) {
    error.value = 'No package name specified.'
    loading.value = false
    return
  }

  loading.value = true
  error.value = null

  try {
    const resp = await fetch(`${REGISTRY_URL}/api/v1/packages/${encodeURIComponent(name)}`)
    if (resp.status === 404) throw new Error(`Package "${name}" not found.`)
    if (!resp.ok) throw new Error(`Registry returned ${resp.status}`)
    const data = await resp.json()
    pkg.value = data

    // If the API returns versions list, use it; otherwise fetch separately
    if (data.versions) {
      versions.value = data.versions
    }
  } catch (e: any) {
    error.value = e.message ?? 'Failed to load package'
  } finally {
    loading.value = false
  }
}

onMounted(loadPackage)
</script>

<template>
  <div class="mx-auto max-w-4xl px-4 py-12 sm:px-6">
    <!-- Back link -->
    <a href="/packages" class="mb-6 inline-flex items-center gap-1.5 text-sm text-muted-foreground no-underline transition-colors hover:text-brand">
      {{ backLabel }}
    </a>

    <!-- Loading -->
    <div v-if="loading" class="py-16 text-center text-muted-foreground">Loading…</div>

    <!-- Error -->
    <div v-else-if="error" class="py-12 text-center">
      <p class="text-muted-foreground">{{ error }}</p>
      <a
        href="/packages"
        class="mt-4 inline-flex items-center rounded-lg border border-border px-4 py-1.5 text-sm font-medium text-foreground no-underline transition-colors hover:border-[color-mix(in_oklab,var(--brand)_55%,var(--border))] hover:text-brand"
      >
        Browse packages
      </a>
    </div>

    <!-- Package content -->
    <template v-else-if="pkg">
      <!-- Metadata card -->
      <div class="mb-8 rounded-xl border border-border bg-card p-6 shadow-[0_1px_2px_rgba(0,0,0,0.03),0_16px_40px_-24px_rgba(0,0,0,0.15)] sm:p-7">
        <!-- Package name + latest version badge -->
        <div class="mb-4 flex items-start justify-between gap-4">
          <div>
            <h1
              class="break-all font-mono text-2xl font-bold text-foreground"
            >{{ pkg.name }}</h1>
            <p class="mt-1 text-muted-foreground">{{ pkg.description || 'No description.' }}</p>
          </div>
          <div class="shrink-0 text-right">
            <span class="inline-block rounded-md bg-brand/12 px-2.5 py-1 font-mono text-xs font-semibold text-brand">
              v{{ pkg.latest?.version ?? 'unknown' }}
            </span>
            <p class="mt-1.5 text-xs text-muted-foreground">{{ pkg.download_count?.toLocaleString() }} downloads</p>
          </div>
        </div>

        <!-- Install command (prominent, copy-to-clipboard) -->
        <div v-if="installCommand" class="mb-4 flex items-center gap-3 rounded-lg border border-border bg-muted/60 py-2 pl-4 pr-2 font-mono text-sm">
          <span class="select-none font-bold text-brand">$</span>
          <code class="min-w-0 flex-1 truncate text-foreground">{{ installCommand }}</code>
          <button
            @click="copyInstallCommand"
            class="shrink-0 rounded-md px-3 py-1.5 font-mono text-xs font-semibold transition-all"
            :class="copySuccess ? 'bg-brand text-brand-ink' : 'border border-border bg-background text-muted-foreground hover:text-brand hover:border-[color-mix(in_oklab,var(--brand)_55%,var(--border))]'"
          >
            {{ copySuccess ? '✓ copied' : 'copy' }}
          </button>
        </div>

        <!-- Author -->
        <div class="flex items-center gap-4 text-sm text-muted-foreground">
          <span>by <a :href="`https://github.com/${pkg.owner}`" class="text-brand hover:underline" target="_blank">{{ pkg.owner }}</a></span>
        </div>
      </div>

      <!-- Version history (expandable) -->
      <div v-if="versions.length > 0" class="mb-8">
        <button
          @click="versionsExpanded = !versionsExpanded"
          class="mb-3 flex items-center gap-2 text-sm font-semibold text-foreground transition-colors hover:text-brand"
        >
          <span>Version History ({{ versions.length }})</span>
          <span class="text-xs">{{ versionsExpanded ? '▲' : '▼' }}</span>
        </button>

        <div v-show="versionsExpanded" class="overflow-hidden rounded-xl border border-border bg-card">
          <div
            v-for="ver in versions"
            :key="ver.version"
            class="flex items-center gap-4 border-b border-border px-5 py-3.5 transition-colors last:border-b-0 hover:bg-muted/60"
          >
            <span class="w-24 shrink-0 font-mono text-sm font-semibold text-foreground">v{{ ver.version }}</span>
            <span class="flex-1 text-xs text-muted-foreground">{{ formatDate(ver.published_at) }}</span>
            <span v-if="ver.size_bytes" class="text-xs text-muted-foreground">{{ formatBytes(ver.size_bytes) }}</span>
            <code class="hidden font-mono text-xs text-muted-foreground sm:block">
              meshpkg install {{ pkg!.name }}@{{ ver.version }}
            </code>
          </div>
        </div>
      </div>

      <!-- README -->
      <div class="rounded-xl border border-border bg-card p-6 sm:p-7">
        <h2 class="mb-4 font-mono text-xs font-semibold tracking-[0.08em] text-foreground">readme</h2>
        <div class="prose dark:prose-invert max-w-none" v-html="renderMarkdown(pkg.readme ?? '')"></div>
      </div>
    </template>
  </div>
</template>
