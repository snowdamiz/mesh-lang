import fs from 'node:fs';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// The language version is the compiler's, as on meshlang.dev.
const MESH_VERSION = fs
  .readFileSync(new URL('../compiler/meshc/Cargo.toml', import.meta.url), 'utf8')
  .match(/^version\s*=\s*"([^"]+)"/m)[1];

export default defineConfig({
  plugins: [sveltekit()],
  define: { __MESH_VERSION__: JSON.stringify(MESH_VERSION) },
  server: { port: 5174 }
});
