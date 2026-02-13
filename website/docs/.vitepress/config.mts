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
