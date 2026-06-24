/// Dock window entry. Mounts DockView.
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import DockView from '@/views/DockView.vue'
import '@/assets/styles/global.css'

createApp(DockView).use(createPinia()).mount('#app')
