<template>
  <div class="settings">
    <h1>Settings</h1>
    <section class="panel">
      <h2>General</h2>
      <label>
        <input type="checkbox" v-model="autostart" @change="onAutostart" />
        Launch at login
      </label>
    </section>
    <section class="panel">
      <h2>Installed Packages</h2>
      <ul>
        <li v-for="p in packages" :key="p.id">
          <strong>{{ p.name }}</strong> ({{ p.id }}) v{{ p.version }}
          <span :class="['badge', p.enabled ? 'on' : 'off']">
            {{ p.enabled ? 'enabled' : 'disabled' }}
          </span>
        </li>
        <li v-if="packages.length === 0" class="muted">No packages installed.</li>
      </ul>
    </section>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { usePackage, type PackageInfo } from '@/composables/usePackage'

const { listPackages } = usePackage()
const packages = ref<PackageInfo[]>([])
const autostart = ref(false)

async function onAutostart() {
  // Autostart toggle wired via tauri-plugin-autostart commands in P6.
  console.log('autostart', autostart.value)
}

onMounted(async () => {
  packages.value = await listPackages()
})
</script>

<style scoped>
.settings {
  padding: 16px 24px;
  height: 100vh;
  overflow: auto;
  background: #1e1e2e;
  color: #cdd6f4;
}
h1 {
  font-size: 18px;
  margin: 0 0 16px;
}
.panel {
  border: 1px solid #313244;
  border-radius: 8px;
  padding: 12px 16px;
  margin-bottom: 16px;
}
.panel h2 {
  font-size: 14px;
  margin: 0 0 8px;
  color: #89b4fa;
}
ul {
  list-style: none;
  padding: 0;
  margin: 0;
}
li {
  padding: 6px 0;
  border-bottom: 1px solid #313244;
}
.badge {
  font-size: 11px;
  padding: 2px 6px;
  border-radius: 4px;
  margin-left: 8px;
}
.badge.on {
  background: #a6e3a1;
  color: #1e1e2e;
}
.badge.off {
  background: #6c7086;
  color: #1e1e2e;
}
.muted {
  color: #6c7086;
}
</style>
