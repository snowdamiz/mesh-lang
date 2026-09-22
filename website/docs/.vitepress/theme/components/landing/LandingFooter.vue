<script setup lang="ts">
const year = new Date().getUTCFullYear()

const columns = [
  {
    title: 'Learn',
    links: [
      { label: 'Getting started', href: '/docs/getting-started/' },
      { label: 'Language basics', href: '/docs/language-basics/' },
      { label: 'Standard library', href: '/docs/stdlib/' },
      { label: 'Reference', href: '/docs/reference/' },
    ],
  },
  {
    title: 'Build',
    links: [
      { label: 'Concurrency', href: '/docs/concurrency/' },
      { label: 'Web and databases', href: '/docs/web/' },
      { label: 'Distributed actors', href: '/docs/distributed/' },
      { label: 'Tooling', href: '/docs/tooling/' },
    ],
  },
  {
    title: 'Project',
    links: [
      { label: 'GitHub', href: 'https://github.com/hyperpush-org/mesh-lang', external: true },
      { label: 'Packages', href: 'https://packages.meshlang.dev', external: true },
      { label: 'Native packages', href: '/docs/native-packages/' },
      { label: 'License', href: 'https://github.com/hyperpush-org/mesh-lang/blob/main/LICENSE', external: true },
    ],
  },
]
</script>

<template>
  <footer class="l-section">
    <div class="grid gap-12 px-5 py-16 sm:px-10 lg:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)] lg:gap-16 lg:px-14">
      <div class="flex flex-col justify-between gap-8">
        <a href="/" class="w-fit" aria-label="Mesh home">
          <img src="/logo-black.svg" alt="" class="h-6 w-auto dark:hidden" />
          <img src="/logo-white.svg" alt="" class="hidden h-6 w-auto dark:block" />
        </a>
        <p class="font-mono text-xs text-muted-foreground">© {{ year }} The Mesh Programming Language</p>
      </div>

      <nav class="grid grid-cols-2 gap-10 sm:grid-cols-3" aria-label="Footer">
        <div v-for="col in columns" :key="col.title">
          <h3 class="font-mono text-xs text-muted-foreground">{{ col.title }}</h3>
          <ul class="mt-4 space-y-2.5">
            <li v-for="link in col.links" :key="link.href">
              <a
                :href="link.href"
                :target="link.external ? '_blank' : undefined"
                :rel="link.external ? 'noopener' : undefined"
                class="text-sm text-foreground/80 transition-colors hover:text-foreground"
              >{{ link.label }}</a>
            </li>
          </ul>
        </div>
      </nav>
    </div>
  </footer>
</template>
