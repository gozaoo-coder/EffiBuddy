<template>
  <div v-if="open" class="dialog-backdrop" @click.self="$emit('cancel')">
    <div class="dialog">
      <h2>Install {{ pkg?.name }}</h2>
      <p class="id">{{ pkg?.id }} v{{ pkg?.version }}</p>
      <p v-if="pkg?.description">{{ pkg.description }}</p>
      <div v-if="pkg?.permissions?.length" class="perms">
        <strong>Requested permissions:</strong>
        <ul>
          <li v-for="p in pkg.permissions" :key="p">{{ p }}</li>
        </ul>
      </div>
      <div class="actions">
        <button class="btn" @click="$emit('cancel')">Cancel</button>
        <button class="btn primary" @click="$emit('confirm')">Install</button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import type { PackageInfo } from '@/composables/usePackage'

defineProps<{ open: boolean; pkg: PackageInfo | null }>()
defineEmits<{
  (e: 'confirm'): void
  (e: 'cancel'): void
}>()
</script>

<style scoped>
.dialog-backdrop {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}
.dialog {
  background: #1e1e2e;
  color: #cdd6f4;
  border-radius: 12px;
  padding: 20px;
  width: 360px;
  border: 1px solid #313244;
}
.dialog h2 {
  margin: 0 0 4px;
  font-size: 16px;
}
.id {
  font-size: 11px;
  color: #6c7086;
  font-family: monospace;
  margin: 0 0 12px;
}
.perms strong {
  font-size: 12px;
}
.perms ul {
  margin: 4px 0 12px;
  padding-left: 20px;
  font-size: 12px;
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 16px;
}
.btn {
  border: none;
  padding: 6px 14px;
  border-radius: 6px;
  cursor: pointer;
  background: #45475a;
  color: #cdd6f4;
}
.btn.primary {
  background: #89b4fa;
  color: #1e1e2e;
}
</style>
