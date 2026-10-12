import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// `pnpm dev` proxies the API to a locally running Cratarr server.
export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: { '/api': process.env.CRATARR_URL ?? 'http://127.0.0.1:8080' },
  },
});
