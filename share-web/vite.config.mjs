import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  root: fileURLToPath(new URL('.', import.meta.url)),
  plugins: [svelte({ configFile: false })],
  resolve: { alias: { $lib: fileURLToPath(new URL('../src/lib', import.meta.url)) } },
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false },
});
