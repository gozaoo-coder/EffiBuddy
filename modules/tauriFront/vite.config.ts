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
    chunkSizeWarningLimit: 1500,
    rollupOptions: {
      output: {
        // 大型第三方依赖独立分包，避免主 bundle 过大、拖慢首屏解析
        manualChunks: {
          // 图表 / 数学公式渲染（体积大，独立缓存）
          mermaid: ['mermaid'],
          katex: ['katex'],
          // markdown 流式渲染
          markstream: ['markstream-vue', 'stream-monaco', 'stream-diffs'],
          // 动画与状态
          vendor: ['vue', 'pinia', 'animejs'],
          icons: ['@hugeicons/core-free-icons'],
        },
      },
    },
  },
})
