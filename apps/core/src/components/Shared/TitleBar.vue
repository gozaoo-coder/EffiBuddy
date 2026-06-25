<template>
  <div class="title-bar" @mousedown="onDrag">
    <span class="title">{{ title }}</span>
    <div class="actions">
      <button class="win-btn" title="Minimize" @click="$emit('minimize')">—</button>
      <button class="win-btn close" title="Close" @click="$emit('close')">✕</button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { useWindow } from '@/composables/useWindow'

const props = defineProps<{ title: string; windowLabel: string }>()
defineEmits<{ (e: 'minimize'): void; (e: 'close'): void }>()

const { startDrag } = useWindow(props.windowLabel)

function onDrag(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('.win-btn')) return
  startDrag()
}
</script>

<style scoped>
.title-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 36px;
  padding: 0 8px 0 16px;
  -webkit-app-region: drag;
  user-select: none;
}
.title {
  font-size: 13px;
  font-weight: 600;
  color: var(--fg);
}
.actions {
  display: flex;
  gap: 4px;
  -webkit-app-region: no-drag;
}
.win-btn {
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--fg);
  cursor: pointer;
  font-size: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.win-btn:hover {
  background: var(--border);
}
.win-btn.close:hover {
  background: var(--danger);
  color: var(--bg);
}
</style>
