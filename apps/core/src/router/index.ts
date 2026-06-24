/**
 * Router placeholder. Multi-window apps don't share a router; each window
 * mounts its own view. This file exists for future single-window shells.
 */
import { createRouter, createWebHashHistory } from 'vue-router'

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', redirect: '/dock' },
    { path: '/dock', component: () => import('@/views/DockView.vue') },
    { path: '/widgets', component: () => import('@/views/WidgetHostView.vue') },
    { path: '/store', component: () => import('@/views/PackageStoreView.vue') },
    { path: '/settings', component: () => import('@/views/SettingsView.vue') },
  ],
})
