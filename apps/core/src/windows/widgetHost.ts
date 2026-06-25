/// Widget host window entry. Mounts WidgetHostView.
import { createApp } from 'vue'
import WidgetHostView from '@/views/WidgetHostView.vue'
import { setup } from '@/bootstrap'

setup(createApp(WidgetHostView)).mount('#app')
