/// <reference types="vitest/config" />
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

// `kith-ui --dev` serves the API here; `npm run dev` proxies /api to it.
const api = `http://127.0.0.1:${process.env['KITH_UI_PORT'] ?? '41680'}`;

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': { target: api, changeOrigin: true },
    },
  },
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true,
    // CSP is default-src 'self': fonts and images must be files, never data: URLs.
    assetsInlineLimit: 0,
    modulePreload: { polyfill: false },
    reportCompressedSize: false,
  },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
  },
});
