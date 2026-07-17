import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// Tauri 期望固定端口（见 tauri.conf.json 的 devUrl），故 strictPort。
// clearScreen:false 让 Tauri 的终端输出不被 Vite 清屏覆盖。
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    target: 'es2021',
    outDir: 'dist',
  },
})
