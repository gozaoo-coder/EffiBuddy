/// Dock window entry. Mounts DockView.
import { createApp } from 'vue'
import DockView from '@/views/DockView.vue'
import { setup } from '@/bootstrap'

setup(createApp(DockView)).mount('#app')
