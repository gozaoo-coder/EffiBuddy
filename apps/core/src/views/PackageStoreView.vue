<template>
  <div class="store">
    <TitleBar title="Package Store" window-label="package-store" @close="onClose" @minimize="onClose" />
    <header class="store-header">
      <h1>Package Store</h1>
      <button class="refresh" @click="loadLocal">Refresh</button>
    </header>
    <section class="grid">
      <PackageCard
        v-for="p in packages"
        :key="p.id"
        :pkg="p"
        @enable="onEnable(p.id)"
        @disable="onDisable(p.id)"
        @uninstall="onUninstall(p.id)"
      />
      <div v-if="packages.length === 0" class="empty">
        No packages installed. Drop a package directory into the packages folder.
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import PackageCard from '@/components/Store/PackageCard.vue'
import TitleBar from '@/components/Shared/TitleBar.vue'
import { usePackage, type PackageInfo } from '@/composables/usePackage'
import { useWindow } from '@/composables/useWindow'

const { listPackages, enablePlugin, disablePlugin, uninstallPackage } = usePackage()
const { hideWindow } = useWindow('package-store')
const packages = ref<PackageInfo[]>([])

async function loadLocal() {
  packages.value = await listPackages()
}
async function onEnable(id: string) {
  await enablePlugin(id)
  await loadLocal()
}
async function onDisable(id: string) {
  await disablePlugin(id)
  await loadLocal()
}
async function onUninstall(id: string) {
  await uninstallPackage(id)
  await loadLocal()
}
function onClose() {
  hideWindow()
}

onMounted(loadLocal)
</script>

<style scoped>
.store {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: transparent;
  color: var(--fg);
}
.store-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
}
.store-header h1 {
  font-size: 16px;
  margin: 0;
}
.refresh {
  background: var(--accent);
  color: var(--bg);
  border: none;
  padding: 6px 12px;
  border-radius: 6px;
  cursor: pointer;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
  padding: 16px;
  overflow: auto;
}
.empty {
  color: var(--muted);
  grid-column: 1 / -1;
  text-align: center;
  padding: 32px;
}
</style>
