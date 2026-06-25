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
  background: var(--bg-overlay);
  backdrop-filter: blur(16px);
  border-radius: 16px;
  border: 1px solid var(--border);
  box-shadow: var(--shadow);
}
</style>
