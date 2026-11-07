import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

export default {
  preprocess: vitePreprocess(),
  compilerOptions: {
    runes: true,
    // Short scoping classes (`s1a2b3`, not `svelte-1a2b3c4`; several hundred in the first
    // load), named by the file's path from src/ so every checkout builds the same page.
    cssHash: ({ hash, filename }) => `s${hash(filename.slice(filename.lastIndexOf('src/'))).slice(0, 5)}`,
  },
};
