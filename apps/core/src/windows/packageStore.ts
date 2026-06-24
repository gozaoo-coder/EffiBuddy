/// Package store window entry. Mounts PackageStoreView.
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import PackageStoreView from '@/views/PackageStoreView.vue'
import '@/assets/styles/global.css'

createApp(PackageStoreView).use(createPinia()).mount('#app')
