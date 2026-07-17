import { createApp } from 'vue'
import App from './App.vue'
import './styles/main.css'
import 'markstream-vue/index.css'

// --- markstream-vue 插件：Mermaid 流程图 / KaTeX 数学公式 ---
import { enableMermaid, enableKatex } from 'markstream-vue'
import 'katex/dist/katex.min.css'

enableMermaid()
enableKatex()

createApp(App).mount('#app')
