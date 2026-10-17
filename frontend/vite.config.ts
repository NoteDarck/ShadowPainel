import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  server: {
    port: 5173,
    // `npm run dev` encaminha /api para o backend Rust
    proxy: { '/api': 'http://127.0.0.1:4173' }
  }
});
