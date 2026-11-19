export interface Link {
  text: string
  href: string
}

export const GITHUB_URL = 'https://github.com/hyperpush-org/mesh-lang'
// Vite fills import.meta.env for the theme; config.mts loads this file in Node, where it is absent.
const env = ((import.meta as { env?: Record<string, string | boolean | undefined> }).env ?? {})

/** The packages site. A dev server links to the local one; VITE_PACKAGES_URL overrides either. */
export const PACKAGES_URL = String(env.VITE_PACKAGES_URL || (env.DEV ? 'http://localhost:5174' : 'https://packages.meshlang.dev')).replace(/\/$/, '')
/** VitePress's own key, so choices made on the old site carry over */
export const THEME_KEY = 'vitepress-theme-appearance'
/** Browser chrome colour per theme, matching --paper in tokens.css */
export const THEME_COLOR = { light: '#f7f2ea', dark: '#141312' } as const

declare const __MESH_VERSION__: string | undefined
/** The compiler's version, injected by config.mts; absent when config.mts itself loads this file. */
export const VERSION = typeof __MESH_VERSION__ === 'string' ? __MESH_VERSION__ : ''
export const STATUS = {
  label: `v${VERSION} · in active development`,
  detail: 'The language, runtime, package format, and cluster protocol can still change before a stable release.',
}

export const NAV_LINKS: (Link & { section?: string })[] = [
  { text: 'Docs', href: '/docs/getting-started/', section: 'docs/' },
  { text: 'Packages', href: PACKAGES_URL },
  { text: 'GitHub', href: GITHUB_URL },
]

export const FOOTER_COLUMNS: { title: string; links: Link[] }[] = [
  {
    title: 'Learn',
    links: [
      { text: 'Getting Started', href: '/docs/getting-started/' },
      { text: 'Language Basics', href: '/docs/language-basics/' },
      { text: 'Concurrency', href: '/docs/concurrency/' },
      { text: 'Autonomous Clusters', href: '/docs/autonomous-clusters/' },
      { text: 'Syntax Cheatsheet', href: '/docs/cheatsheet/' },
    ],
  },
  {
    title: 'Ecosystem',
    links: [
      { text: 'Packages', href: PACKAGES_URL },
      { text: 'Publish a package', href: `${PACKAGES_URL}/publish` },
      { text: 'Developer Tools', href: '/docs/tooling/' },
    ],
  },
  {
    title: 'Project',
    links: [
      { text: 'GitHub', href: GITHUB_URL },
      { text: 'Release Notes', href: '/docs/autonomous-release-notes/' },
    ],
  },
]
