/**
 * Packages store. Mirrors backend registry state for cross-window sync.
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { usePackage, type PackageInfo } from '@/composables/usePackage'

export const usePackagesStore = defineStore('packages', () => {
  const items = ref<PackageInfo[]>([])
  const loading = ref(false)

  async function refresh() {
    loading.value = true
    try {
      items.value = await usePackage().listPackages()
    } finally {
      loading.value = false
    }
  }

  return { items, loading, refresh }
})
