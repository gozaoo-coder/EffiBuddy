/// Widget host window entry. Mounts WidgetHostView.
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import WidgetHostView from '@/views/WidgetHostView.vue'
import '@/assets/styles/global.css'

createApp(WidgetHostView).use(createPinia()).mount('#app')
