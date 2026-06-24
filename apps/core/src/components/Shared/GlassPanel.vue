<template>
  <div class="glass-panel">
    <slot />
  </div>
</template>

<script setup lang="ts">
// Frosted-glass panel shared by overlay windows.
</script>

<style scoped>
.glass-panel {
  background: rgba(30, 30, 46, 0.55);
  backdrop-filter: blur(16px);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 12px;
  color: #cdd6f4;
}
</style>
