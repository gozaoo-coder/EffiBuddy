/// Settings window entry. Mounts SettingsView.
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import SettingsView from '@/views/SettingsView.vue'
import '@/assets/styles/global.css'

createApp(SettingsView).use(createPinia()).mount('#app')
