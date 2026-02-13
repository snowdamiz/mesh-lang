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
