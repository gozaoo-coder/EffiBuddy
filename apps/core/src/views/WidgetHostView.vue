<template>
  <div class="widget-host">
    <div v-if="slots.length === 0" class="empty">No widgets enabled</div>
    <WidgetSlot
      v-for="slot in slots"
      :key="slot.key"
      :plugin-id="slot.pluginId"
      :widget-type="slot.widgetType"
    />
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import WidgetSlot from '@/components/Widgets/WidgetSlot.vue'
import { usePackage } from '@/composables/usePackage'

const { listPackages } = usePackage()

interface Slot {
  key: string
  pluginId: string
  widgetType: string
}

const slots = ref<Slot[]>([])

onMounted(async () => {
  const pkgs = await listPackages()
  for (const p of pkgs) {
    if (!p.enabled) continue
    for (const w of p.widgets) {
      slots.value.push({
        key: `${p.id}:${w.type}`,
        pluginId: p.id,
        widgetType: w.type,
      })
    }
  }
})
</script>

<style scoped>
.widget-host {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 8px;
  width: 100vw;
  height: 100vh;
}
.empty {
  color: var(--muted);
  font-size: 12px;
  text-align: center;
  padding: 16px;
}
</style>
