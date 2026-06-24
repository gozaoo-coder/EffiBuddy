<template>
  <button class="icon-btn" :title="title" @click="$emit('click')">
    <slot />
  </button>
</template>

<script setup lang="ts">
defineProps<{ title?: string }>()
defineEmits<{ (e: 'click'): void }>()
</script>

<style scoped>
.icon-btn {
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.08);
  color: #cdd6f4;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}
.icon-btn:hover {
  background: rgba(137, 180, 250, 0.25);
}
</style>
