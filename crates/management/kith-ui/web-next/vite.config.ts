import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// `kith-ui --dev --fixture …` serves the API; the page proxies /api to it.
const api = `http://127.0.0.1:${process.env['KITH_UI_PORT'] ?? '41680'}`;

export default defineConfig({
  plugins: [tailwindcss(), react()],
  resolve: { alias: { '@kith/types': new URL('../web/src/lib/types.gen.ts', import.meta.url).pathname } },
  server: { port: 5173, strictPort: true, proxy: { '/api': { target: api, changeOrigin: true } } },
  // Matches web/: kith-ui embeds dist/ and serves it under a CSP of default-src 'self'.
  build: {
    target: 'es2022',
    outDir: '../web/dist',
    emptyOutDir: true,
    assetsInlineLimit: 0,
    modulePreload: { polyfill: false },
    reportCompressedSize: false,
    rollupOptions: {
      output: {
        // UI SDK's small shared helpers go in one file; alone, a stub is too small for brotli to shrink.
        codeSplitting: { groups: [{ name: 'ui-foundation', test: /@ui-sdk[\\/]core[\\/]dist[\\/]foundation/ }] },
        entryFileNames: 'assets/[name]-[hash:6].js',
        chunkFileNames: 'assets/[name]-[hash:6].js',
        assetFileNames: 'assets/[name]-[hash:6][extname]',
      },
    },
  },
});
