import { defineConfig } from 'vitepress'
import tailwindcss from '@tailwindcss/vite'
import path from 'node:path'
import meshGrammar from '../../../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'
import meshLight from './theme/shiki/mesh-light.json'
import meshDark from './theme/shiki/mesh-dark.json'

export default defineConfig({
  title: 'Mesh',
  description: 'The Mesh Programming Language',

  // Built-in dark mode with FOUC prevention
  appearance: true,

  // Enable git-based last-updated timestamps
  lastUpdated: true,

  // Site-wide SEO defaults
  head: [
    ['meta', { property: 'og:site_name', content: 'Mesh Programming Language' }],
    ['meta', { name: 'twitter:card', content: 'summary' }],
  ],

  // Per-page dynamic SEO meta tags
  transformPageData(pageData) {
    const canonicalUrl = `https://meshlang.org/${pageData.relativePath}`
      .replace(/index\.md$/, '')
      .replace(/\.md$/, '.html')
    pageData.frontmatter.head ??= []
    pageData.frontmatter.head.push(
      ['link', { rel: 'canonical', href: canonicalUrl }],
      ['meta', { property: 'og:title', content: pageData.title + ' | Mesh' }],
      ['meta', { property: 'og:description', content: pageData.description }],
      ['meta', { property: 'og:url', content: canonicalUrl }],
      ['meta', { property: 'og:type', content: 'article' }],
    )
  },

  markdown: {
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
    search: { provider: 'local' },
    editLink: {
      pattern: 'https://github.com/user/mesh/edit/main/website/docs/:path',
      text: 'Edit this page on GitHub',
    },
    meshVersion: '0.1.0',
    sidebar: {
      '/docs/': [
        {
          text: 'Getting Started',
          items: [
            { text: 'Introduction', link: '/docs/getting-started/' },
          ],
        },
        {
          text: 'Language Guide',
          collapsed: false,
          items: [
            { text: 'Language Basics', link: '/docs/language-basics/' },
            { text: 'Type System', link: '/docs/type-system/' },
            { text: 'Concurrency', link: '/docs/concurrency/' },
          ],
        },
        {
          text: 'Web & Networking',
          collapsed: false,
          items: [
            { text: 'Web', link: '/docs/web/' },
          ],
        },
        {
          text: 'Data',
          collapsed: false,
          items: [
            { text: 'Databases', link: '/docs/databases/' },
          ],
        },
        {
          text: 'Distribution',
          collapsed: false,
          items: [
            { text: 'Distributed Actors', link: '/docs/distributed/' },
          ],
        },
        {
          text: 'Tooling',
          collapsed: false,
          items: [
            { text: 'Developer Tools', link: '/docs/tooling/' },
          ],
        },
        {
          text: 'Reference',
          collapsed: false,
          items: [
            { text: 'Syntax Cheatsheet', link: '/docs/cheatsheet/' },
          ],
        },
      ],
    },
    outline: { level: [2, 3], label: 'On this page' },
  },

  vite: {
    plugins: [
      tailwindcss(),
    ],
    resolve: {
      alias: {
        '@': path.resolve(__dirname, './theme'),
      },
    },
  },
})
