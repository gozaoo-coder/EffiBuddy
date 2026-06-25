<template>
  <div class="package-card">
    <div class="head">
      <div class="name">{{ pkg.name }}</div>
      <div class="ver">v{{ pkg.version }}</div>
    </div>
    <div class="id">{{ pkg.id }}</div>
    <p class="desc">{{ pkg.description ?? 'No description' }}</p>
    <div class="perms" v-if="pkg.permissions.length">
      <span v-for="p in pkg.permissions" :key="p" class="perm">{{ p }}</span>
    </div>
    <div class="actions">
      <button v-if="!pkg.enabled" class="btn primary" @click="$emit('enable')">Enable</button>
      <button v-else class="btn" @click="$emit('disable')">Disable</button>
      <button class="btn danger" @click="$emit('uninstall')">Uninstall</button>
    </div>
  </div>
</template>

<script setup lang="ts">
import type { PackageInfo } from '@/composables/usePackage'

defineProps<{ pkg: PackageInfo }>()
defineEmits<{
  (e: 'enable'): void
  (e: 'disable'): void
  (e: 'uninstall'): void
}>()
</script>

<style scoped>
.package-card {
  background: var(--bg-elev);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
}
.name {
  font-weight: 600;
  color: var(--fg);
}
.ver {
  font-size: 11px;
  color: var(--muted);
}
.id {
  font-size: 11px;
  color: var(--muted);
  font-family: monospace;
}
.desc {
  font-size: 12px;
  color: var(--fg);
  opacity: 0.85;
  margin: 0;
  min-height: 32px;
}
.perms {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.perm {
  font-size: 10px;
  background: var(--border);
  color: var(--fg);
  padding: 2px 6px;
  border-radius: 4px;
}
.actions {
  display: flex;
  gap: 6px;
  margin-top: 4px;
}
.btn {
  flex: 1;
  border: none;
  padding: 6px 8px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 12px;
  background: var(--bg);
  color: var(--fg);
}
.btn.primary {
  background: var(--accent);
  color: var(--bg);
}
.btn.danger {
  background: var(--danger);
  color: var(--bg);
}
</style>
