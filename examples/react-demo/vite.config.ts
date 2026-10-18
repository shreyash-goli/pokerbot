import { defineConfig } from 'vite';

export default defineConfig({
  // Relative asset URLs, so dist/ works from any sub-path of a site.
  base: './',
  // The linked package has no React of its own: always use this app's copy.
  resolve: { dedupe: ['react', 'react-dom'] },
  oxc: { jsx: { runtime: 'automatic' } },
  worker: { format: 'es' },
  // Dev server: local only; allow the linked ../../packages source.
  server: { host: '127.0.0.1', fs: { allow: ['../..'] } },
  preview: { host: '127.0.0.1' },
  build: { target: 'es2022', sourcemap: false },
});
