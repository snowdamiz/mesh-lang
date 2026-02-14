<script setup lang="ts">
import { useData } from 'vitepress'

const { isDark } = useData()

const resources = [
  { label: 'Getting Started', href: '/docs/getting-started/' },
  { label: 'Language Guide', href: '/docs/language-basics/' },
  { label: 'Cheatsheet', href: '/docs/cheatsheet/' },
]

const community = [
  { label: 'GitHub', href: 'https://github.com/snowdamiz/mesh-lang' },
  { label: 'Discussions', href: 'https://github.com/snowdamiz/mesh-lang/discussions' },
]

const more = [
  { label: 'Concurrency', href: '/docs/concurrency/' },
  { label: 'Web & HTTP', href: '/docs/web/' },
]
</script>

<template>
  <footer class="border-t border-border py-12 md:py-16">
    <div class="mx-auto max-w-5xl px-4">
      <div class="grid gap-8 sm:grid-cols-2 md:grid-cols-4">
        <!-- Brand -->
        <div>
          <div class="flex items-center gap-2">
            <img
              :src="isDark ? '/logo-icon-white.svg' : '/logo-icon-black.svg'"
              alt="Mesh"
              class="size-6"
            />
            <span class="text-base font-semibold text-foreground">Mesh</span>
          </div>
          <p class="mt-3 text-sm text-muted-foreground leading-relaxed">
            Expressive, concurrent, type-safe programming language.
          </p>
        </div>

        <!-- Resources -->
        <div>
          <h3 class="text-sm font-semibold text-foreground">Resources</h3>
          <ul class="mt-3 space-y-2">
            <li v-for="link in resources" :key="link.href">
              <a :href="link.href" class="text-sm text-muted-foreground transition-colors hover:text-foreground">
                {{ link.label }}
              </a>
            </li>
          </ul>
        </div>

        <!-- Community -->
        <div>
          <h3 class="text-sm font-semibold text-foreground">Community</h3>
          <ul class="mt-3 space-y-2">
            <li v-for="link in community" :key="link.href">
              <a :href="link.href" class="text-sm text-muted-foreground transition-colors hover:text-foreground">
                {{ link.label }}
              </a>
            </li>
          </ul>
        </div>

        <!-- More -->
        <div>
          <h3 class="text-sm font-semibold text-foreground">More</h3>
          <ul class="mt-3 space-y-2">
            <li v-for="link in more" :key="link.href">
              <a :href="link.href" class="text-sm text-muted-foreground transition-colors hover:text-foreground">
                {{ link.label }}
              </a>
            </li>
          </ul>
        </div>
      </div>

      <!-- Bottom bar -->
      <div class="mt-12 border-t border-border pt-6">
        <p class="text-xs text-muted-foreground">
          &copy; {{ new Date().getFullYear() }} The Mesh Programming Language
        </p>
      </div>
    </div>
  </footer>
</template>
