/// Package store window entry. Mounts PackageStoreView.
import { createApp } from 'vue'
import PackageStoreView from '@/views/PackageStoreView.vue'
import { setup } from '@/bootstrap'

setup(createApp(PackageStoreView)).mount('#app')
