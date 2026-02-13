import { defineConfig } from 'vitepress'
import tailwindcss from '@tailwindcss/vite'
import path from 'node:path'

export default defineConfig({
  title: 'Mesh',
  description: 'The Mesh Programming Language',

  // Built-in dark mode with FOUC prevention
  appearance: true,

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
