import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'node:path'

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      '@desktop-suite/plugin-sdk-vue': resolve(
        __dirname,
        '../../../sdk/plugin-sdk-vue/src/index.ts',
      ),
    },
  },
  build: {
    lib: {
      entry: resolve(__dirname, 'index.ts'),
      formats: ['es'],
      fileName: 'index',
    },
    rollupOptions: {
      external: ['vue', '@tauri-apps/api', '@tauri-apps/api/event', '@tauri-apps/api/core'],
    },
  },
})
