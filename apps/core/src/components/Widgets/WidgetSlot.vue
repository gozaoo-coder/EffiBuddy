<template>
  <div class="widget-slot">
    <component :is="comp" v-if="comp" />
    <div v-else class="fallback">
      <div class="title">{{ pluginId }} / {{ widgetType }}</div>
      <div class="hint">widget frontend not available</div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { onMounted, shallowRef, type Component } from 'vue'
import { usePluginFrontend } from '@/composables/usePluginFrontend'

const props = defineProps<{ pluginId: string; widgetType: string }>()
const comp = shallowRef<Component | null>(null)
const { loadWidget } = usePluginFrontend()

onMounted(async () => {
  try {
    comp.value = await loadWidget(props.pluginId, props.widgetType)
  } catch (e) {
    console.warn('widget load failed', e)
  }
})
</script>

<style scoped>
.widget-slot {
  background: rgba(30, 30, 46, 0.6);
  backdrop-filter: blur(12px);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 12px;
  padding: 8px;
  min-height: 80px;
}
.fallback {
  color: #cdd6f4;
  font-size: 12px;
}
.title {
  font-weight: 600;
}
.hint {
  color: #6c7086;
}
</style>
