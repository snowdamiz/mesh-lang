import fs from 'node:fs'
import { defineConfig, type HeadConfig, type PageData } from 'vitepress'
import meshGrammar from '../../../tools/editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'
import meshLight from './theme/shiki/mesh-light.json'
import meshDark from './theme/shiki/mesh-dark.json'
import { THEME_COLOR, THEME_KEY } from './theme/site'

const SITE_URL = 'https://meshlang.dev'
const SITE_NAME = 'Mesh Programming Language'
const DEFAULT_DESCRIPTION = 'Mesh is a statically typed, actor-based programming language for native services and distributed systems.'
const SOCIAL_IMAGE_PATH = '/og-image-v3.png'
const SOCIAL_IMAGE_URL = `${SITE_URL}${SOCIAL_IMAGE_PATH}`
const SOCIAL_IMAGE_ALT = 'Mesh social card reading “Typed actors. Native binaries.” A statically typed, actor-based language for native services and distributed systems.'
const SITE_LOGO_URL = `${SITE_URL}/icon-512.png`
// The language version is the compiler's; the release workflow bumps it before deploying the site.
const MESH_VERSION = fs
  .readFileSync(new URL('../../../compiler/meshc/Cargo.toml', import.meta.url), 'utf8')
  .match(/^version\s*=\s*"([^"]+)"/m)![1]
const INDEX_ROBOTS ='index,follow,max-image-preview:large,max-snippet:-1,max-video-preview:-1'

// Runs before first paint: light unless the reader chose dark. 'auto' is what
// VitePress stored for readers who never chose, and they keep following the OS.
const THEME_SCRIPT = `;(() => {
  let choice = null
  try { choice = localStorage.getItem('${THEME_KEY}') } catch {}
  const dark = choice === 'dark' || (choice === 'auto' && matchMedia('(prefers-color-scheme: dark)').matches)
  document.documentElement.classList.toggle('dark', dark)
  const meta = document.createElement('meta')
  meta.name = 'theme-color'
  meta.content = dark ? '${THEME_COLOR.dark}' : '${THEME_COLOR.light}'
  document.head.append(meta)
})()`

function toCanonicalUrl(relativePath: string): string {
  if (!relativePath || relativePath === 'index.md') {
    return `${SITE_URL}/`
  }

  const cleanPath = relativePath
    .replace(/index\.md$/, '')
    .replace(/\.md$/, '')

  return `${SITE_URL}/${cleanPath}`
}

function pageTitle(pageData: PageData): string {
  if (pageData.relativePath === 'index.md') {
    return SITE_NAME
  }

  return pageData.title ? `${pageData.title} | Mesh` : SITE_NAME
}

function pageDescription(pageData: PageData): string {
  if (pageData.description) {
    return pageData.description
  }

  if (pageData.relativePath === 'index.md') {
    return DEFAULT_DESCRIPTION
  }

  return 'Documentation for Mesh language syntax, typed concurrency, native services, packages, databases, and distributed systems.'
}

function pageOgType(relativePath: string): 'website' | 'article' {
  return relativePath.startsWith('docs/') ? 'article' : 'website'
}


function pageStructuredData(pageData: PageData, title: string, description: string, canonicalUrl: string): string {
  const relativePath = pageData.relativePath
  const isHome = relativePath === 'index.md'
  const isDocsPage = relativePath.startsWith('docs/')
  const pageType = isDocsPage ? 'TechArticle' : 'WebPage'

  const pageNode: Record<string, unknown> = {
    '@type': pageType,
    '@id': `${canonicalUrl}#page`,
    url: canonicalUrl,
    name: title,
    description,
    inLanguage: 'en-US',
    isPartOf: { '@id': `${SITE_URL}/#website` },
    publisher: { '@id': `${SITE_URL}/#organization` },
    image: { '@id': `${SOCIAL_IMAGE_URL}#image` },
    primaryImageOfPage: { '@id': `${SOCIAL_IMAGE_URL}#image` },
  }

  if (isDocsPage) {
    pageNode.headline = title
    pageNode.mainEntityOfPage = canonicalUrl
  }

  if (isHome) {
    pageNode.about = [
      'distributed systems',
      'programming language',
      'actors',
      'fault tolerance',
      'native compilation',
      'static type systems',
      'native interoperability',
    ]
  }

  return JSON.stringify({
    '@context': 'https://schema.org',
    '@graph': [
      {
        '@type': 'Organization',
        '@id': `${SITE_URL}/#organization`,
        name: SITE_NAME,
        url: SITE_URL,
        logo: {
          '@type': 'ImageObject',
          url: SITE_LOGO_URL,
        },
        sameAs: ['https://github.com/hyperpush-org/mesh-lang'],
      },
      {
        '@type': 'WebSite',
        '@id': `${SITE_URL}/#website`,
        url: SITE_URL,
        name: SITE_NAME,
        description: DEFAULT_DESCRIPTION,
        inLanguage: 'en-US',
        publisher: { '@id': `${SITE_URL}/#organization` },
        image: { '@id': `${SOCIAL_IMAGE_URL}#image` },
      },
      {
        '@type': 'ImageObject',
        '@id': `${SOCIAL_IMAGE_URL}#image`,
        url: SOCIAL_IMAGE_URL,
        width: 1200,
        height: 630,
        caption: SOCIAL_IMAGE_ALT,
      },
      pageNode,
    ],
  })
}

export default defineConfig({
  title: 'Mesh',
  titleTemplate: ':title | Mesh',
  description: DEFAULT_DESCRIPTION,

  // The theme owns appearance (THEME_SCRIPT and ThemeToggle.vue), so VitePress
  // adds no script of its own and isDark is a plain ref.
  appearance: false,

  vite: { define: { __MESH_VERSION__: JSON.stringify(MESH_VERSION) } },

  // Anchor jumps land below the sticky header (and the docs menu bar on narrow screens)
  scrollOffset: { selector: '.site-top', padding: 16 },

  // Enable clean URLs
  cleanUrls: true,

  // Enable git-based last-updated timestamps
  lastUpdated: true,

  // Generate sitemap
  sitemap: {
    hostname: SITE_URL,
  },

  // Site-wide SEO defaults
  head: [
    ['link', { rel: 'icon', type: 'image/png', sizes: '64x64', href: '/favicon-64.png' }],
    ['link', { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/favicon-32.png' }],
    ['link', { rel: 'icon', type: 'image/png', sizes: '16x16', href: '/favicon-16.png' }],
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/favicon.svg' }],
    ['link', { rel: 'apple-touch-icon', sizes: '180x180', href: '/icon-180.png' }],
    ['link', { rel: 'image_src', href: SOCIAL_IMAGE_URL }],
    ['script', { id: 'check-dark-mode' }, THEME_SCRIPT],
    ['meta', { property: 'og:site_name', content: SITE_NAME }],
    ['meta', { property: 'og:locale', content: 'en_US' }],
    ['meta', { property: 'og:image', content: SOCIAL_IMAGE_URL }],
    ['meta', { property: 'og:image:url', content: SOCIAL_IMAGE_URL }],
    ['meta', { property: 'og:image:secure_url', content: SOCIAL_IMAGE_URL }],
    ['meta', { property: 'og:image:type', content: 'image/png' }],
    ['meta', { property: 'og:image:width', content: '1200' }],
    ['meta', { property: 'og:image:height', content: '630' }],
    ['meta', { property: 'og:image:alt', content: SOCIAL_IMAGE_ALT }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
    ['meta', { name: 'twitter:image', content: SOCIAL_IMAGE_URL }],
    ['meta', { name: 'twitter:image:src', content: SOCIAL_IMAGE_URL }],
    ['meta', { name: 'twitter:image:alt', content: SOCIAL_IMAGE_ALT }],
    ['meta', { name: 'twitter:site', content: '@meshlang' }],
  ],

  // Per-page dynamic SEO meta tags
  transformPageData(pageData) {
    const canonicalUrl = toCanonicalUrl(pageData.relativePath)
    const title = pageTitle(pageData)
    const description = pageDescription(pageData)
    const pageHead: HeadConfig[] = [
      ['link', { rel: 'canonical', href: canonicalUrl }],
      ['meta', { name: 'description', content: description }],
      ['meta', { name: 'robots', content: INDEX_ROBOTS }],
      ['meta', { name: 'googlebot', content: INDEX_ROBOTS }],
      ['meta', { property: 'og:title', content: title }],
      ['meta', { property: 'og:description', content: description }],
      ['meta', { property: 'og:url', content: canonicalUrl }],
      ['meta', { property: 'og:type', content: pageOgType(pageData.relativePath) }],
      ['meta', { name: 'twitter:title', content: title }],
      ['meta', { name: 'twitter:description', content: description }],
      ['script', { type: 'application/ld+json' }, pageStructuredData(pageData, title, description, canonicalUrl)],
    ]

    pageData.frontmatter.head ??= []
    pageData.frontmatter.head.push(...pageHead)
  },

  markdown: {
    config(md) {
      md.renderer.rules.table_open = () => '<div class="table-wrap"><table>\n'
      md.renderer.rules.table_close = () => '</table></div>\n'
      // The page description, as a dek under the page's H1.
      md.core.ruler.push('mesh_dek', (state) => {
        const description: string | undefined = state.env.frontmatter?.description
        const h1 = state.tokens.findIndex((t) => t.type === 'heading_close' && t.tag === 'h1')
        if (!description || h1 < 0) return
        const dek = new state.Token('html_block', '', 0)
        const sentence = /[.!?]$/.test(description) ? description : `${description}.`
        dek.content = `<p class="dek">${md.utils.escapeHtml(sentence)}</p>\n`
        state.tokens.splice(h1 + 1, 0, dek)
      })
    },
    languages: [
      {
        ...(meshGrammar as any),
        name: 'mesh',
      },
    ],
    theme: {
      light: meshLight as any,
      dark: meshDark as any,
    },
  },

  themeConfig: {
    search: {
      provider: 'local',
      options: { translations: { button: { buttonText: 'Search docs', buttonAriaLabel: 'Search docs' } } },
    },
    editLink: {
      pattern: 'https://github.com/hyperpush-org/mesh-lang/edit/main/website/docs/:path',
      text: 'Edit this page on GitHub',
    },
    sidebar: {
      '/docs/': [
        {
          text: 'Getting Started',
          items: [
            { text: 'Introduction', link: '/docs/getting-started/' },
            { text: 'Clustered Example', link: '/docs/getting-started/clustered-example/' },
          ],
        },
        {
          text: 'Language Guide',
          items: [
            { text: 'Language Basics', link: '/docs/language-basics/' },
            { text: 'Type System', link: '/docs/type-system/' },
            { text: 'Iterators', link: '/docs/iterators/' },
            { text: 'Concurrency', link: '/docs/concurrency/' },
          ],
        },
        {
          text: 'Web & Networking',
          items: [
            { text: 'Web', link: '/docs/web/' },
          ],
        },
        {
          text: 'Data',
          items: [
            { text: 'Databases', link: '/docs/databases/' },
          ],
        },
        {
          text: 'Interop & Ecosystem',
          items: [
            { text: 'Native Packages', link: '/docs/native-packages/' },
            { text: 'Library Builds', link: '/docs/library-builds/' },
            { text: 'Packages & Registry', link: '/docs/packages/' },
          ],
        },
        {
          text: 'Distribution',
          items: [
            { text: 'Distributed Actors', link: '/docs/distributed/' },
            { text: 'Autonomous Clusters', link: '/docs/autonomous-clusters/' },
            { text: 'Cluster Operations', link: '/docs/cluster-operations/' },
            { text: 'Capacity Drivers', link: '/docs/capacity-drivers/' },
            { text: 'Cluster Migration', link: '/docs/cluster-migration/' },
            { text: 'Release Notes', link: '/docs/autonomous-release-notes/' },
          ],
        },
        {
          text: 'Tooling',
          items: [
            { text: 'Developer Tools', link: '/docs/tooling/' },
            { text: 'Error Index', link: '/docs/errors/' },
          ],
        },
        {
          text: 'Standard Library',
          items: [
            { text: 'Standard Library', link: '/docs/stdlib/' },
            { text: 'Testing', link: '/docs/testing/' },
          ],
        },
        {
          text: 'Reference',
          items: [
            { text: 'Syntax Cheatsheet', link: '/docs/cheatsheet/' },
            { text: 'Complete Reference', link: '/docs/reference/' },
            { text: 'Environment Variables', link: '/docs/environment-variables/' },
          ],
        },
        {
          text: 'Proof Surfaces',
          items: [
            // Proof surfaces sit outside the reading order of the pager.
            { text: 'Distributed Proof', link: '/docs/distributed-proof/', includeInFooter: false } as any,
          ],
        },
      ],
    },
    outline: { level: [2, 3], label: 'On this page' },
  },
})
