<template>
  <div class="dock-bar">
    <DockItem
      v-for="item in items"
      :key="item.id"
      :label="item.label"
      :icon="item.icon"
      @activate="$emit('activate', item)"
      @hover="onHover(item.id)"
      @leave="onLeave"
    />
  </div>
</template>

<script setup lang="ts">
import DockItem from './DockItem.vue'
import { useMagnify } from './DockMagnify'

interface DockItemData {
  id: string
  label: string
  icon: string
  action: () => void
}

defineProps<{ items: DockItemData[] }>()
defineEmits<{ (e: 'activate', item: DockItemData): void }>()

const { onHover, onLeave } = useMagnify()
</script>

<style scoped>
.dock-bar {
  display: flex;
  align-items: flex-end;
  gap: 8px;
  padding: 6px 12px;
  background: rgba(30, 30, 46, 0.55);
  backdrop-filter: blur(16px);
  border-radius: 16px;
  border: 1px solid rgba(255, 255, 255, 0.08);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.35);
}
</style>
