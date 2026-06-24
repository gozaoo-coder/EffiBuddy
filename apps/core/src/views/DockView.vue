<template>
  <div class="dock-root" @mousedown="onDragStart">
    <DockBar :items="dockItems" @activate="onActivate" />
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import DockBar from '@/components/Dock/DockBar.vue'
import { useWindow } from '@/composables/useWindow'
import { usePackage } from '@/composables/usePackage'

const { startDrag, showWindow } = useWindow('dock')
const { listPackages } = usePackage()

interface DockItem {
  id: string
  label: string
  icon: string
  action: () => void
}

const dockItems = ref<DockItem[]>([
  { id: 'store', label: 'Package Store', icon: 'store', action: () => showWindow('package-store') },
  { id: 'settings', label: 'Settings', icon: 'settings', action: () => showWindow('settings') },
  { id: 'widgets', label: 'Widgets', icon: 'widgets', action: () => showWindow('widget-host') },
])

function onActivate(item: DockItem) {
  item.action()
}

function onDragStart(e: MouseEvent) {
  // Only drag when pressing on empty dock area, not on items.
  if ((e.target as HTMLElement).classList.contains('dock-root')) {
    startDrag()
  }
}

onMounted(async () => {
  const pkgs = await listPackages()
  for (const p of pkgs) {
    if (p.has_frontend) {
      dockItems.value.push({
        id: p.id,
        label: p.name,
        icon: 'plugin',
        action: () => showWindow('widget-host'),
      })
    }
  }
})
</script>

<style scoped>
.dock-root {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100vw;
  height: 100vh;
  -webkit-app-region: drag;
}
</style>
