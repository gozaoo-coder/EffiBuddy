import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'node:path'

// Multi-entry build: each window gets its own HTML entry.
const entries = {
  dock: resolve(__dirname, 'index.html'),
  'widget-host': resolve(__dirname, 'widget-host.html'),
  'package-store': resolve(__dirname, 'package-store.html'),
  settings: resolve(__dirname, 'settings.html'),
}

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
      '@desktop-suite/plugin-sdk-vue': resolve(
        __dirname,
        '../../sdk/plugin-sdk-vue/src/index.ts',
      ),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**', '**/packages/**/backend/**'],
    },
  },
  build: {
    target: 'es2021',
    minify: 'esbuild',
    sourcemap: false,
    rollupOptions: {
      input: entries,
    },
  },
})
