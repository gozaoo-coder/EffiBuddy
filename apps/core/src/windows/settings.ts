/// Settings window entry. Mounts SettingsView.
import { createApp } from 'vue'
import SettingsView from '@/views/SettingsView.vue'
import { setup } from '@/bootstrap'

setup(createApp(SettingsView)).mount('#app')
