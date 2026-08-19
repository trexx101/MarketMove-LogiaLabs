import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  server: {
    proxy: {
      '/api': 'http://localhost:3000',
    },
  },
  preview: {
    proxy: {
      // `npm run preview` serves frontend/dist; proxy API to the local engine.
      '/api': 'http://localhost:8080',
    },
  },
});
