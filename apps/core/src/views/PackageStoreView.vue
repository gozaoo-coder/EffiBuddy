<template>
  <div class="store">
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
import { usePackage, type PackageInfo } from '@/composables/usePackage'

const { listPackages, enablePlugin, disablePlugin, uninstallPackage } = usePackage()
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

onMounted(loadLocal)
</script>

<style scoped>
.store {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #1e1e2e;
  color: #cdd6f4;
}
.store-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  border-bottom: 1px solid #313244;
}
.store-header h1 {
  font-size: 16px;
  margin: 0;
}
.refresh {
  background: #89b4fa;
  color: #1e1e2e;
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
  color: #6c7086;
  grid-column: 1 / -1;
  text-align: center;
  padding: 32px;
}
</style>
