<template>
  <button
    class="dock-item"
    v-tippy="label"
    @mouseenter="$emit('hover')"
    @mouseleave="$emit('leave')"
    @click="$emit('activate')"
  >
    <SvgIcon class="icon" :name="icon" />
  </button>
</template>

<script setup lang="ts">
import SvgIcon from '@/components/Shared/SvgIcon.vue'

defineProps<{ label: string; icon: string }>()
defineEmits<{
  (e: 'activate'): void
  (e: 'hover'): void
  (e: 'leave'): void
}>()
</script>

<style scoped>
.dock-item {
  width: 44px;
  height: 44px;
  border: none;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.08);
  color: var(--fg);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: transform 0.15s ease, background 0.15s ease;
}
.dock-item:hover {
  transform: translateY(-6px) scale(1.15);
  background: color-mix(in srgb, var(--accent) 25%, transparent);
}
.icon {
  font-size: 18px;
  font-weight: 600;
}
</style>
