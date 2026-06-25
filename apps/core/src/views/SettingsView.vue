<template>
  <div class="settings">
    <h1>Settings</h1>
    <section class="panel">
      <h2>General</h2>
      <label class="row">
        <span>Launch at login</span>
        <input type="checkbox" v-model="autostart" @change="onAutostart" />
      </label>
    </section>
    <section class="panel">
      <h2>Appearance</h2>
      <div class="row">
        <span>Theme</span>
        <div class="seg">
          <button
            v-for="opt in ['system', 'dark', 'light'] as const"
            :key="opt"
            :class="['seg-btn', mode === opt ? 'active' : '']"
            @click="set(opt)"
          >
            {{ opt }}
          </button>
        </div>
      </div>
      <p class="hint">System follows your OS color scheme automatically.</p>
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
import { useTheme, type ThemeMode } from '@/composables/useTheme'

const { listPackages } = usePackage()
const { mode, set } = useTheme()
const packages = ref<PackageInfo[]>([])
const autostart = ref(false)

async function onAutostart() {
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
  background: var(--bg);
  color: var(--fg);
}
h1 {
  font-size: 18px;
  margin: 0 0 16px;
}
.panel {
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 12px 16px;
  margin-bottom: 16px;
  background: var(--bg-elev);
}
.panel h2 {
  font-size: 14px;
  margin: 0 0 12px;
  color: var(--accent);
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 0;
}
.seg {
  display: inline-flex;
  border: 1px solid var(--border);
  border-radius: 8px;
  overflow: hidden;
}
.seg-btn {
  border: none;
  background: transparent;
  color: var(--fg);
  padding: 6px 14px;
  cursor: pointer;
  font-size: 12px;
}
.seg-btn.active {
  background: var(--accent);
  color: var(--bg);
}
.hint {
  font-size: 11px;
  color: var(--muted);
  margin: 6px 0 0;
}
ul {
  list-style: none;
  padding: 0;
  margin: 0;
}
li {
  padding: 6px 0;
  border-bottom: 1px solid var(--border);
}
.badge {
  font-size: 11px;
  padding: 2px 6px;
  border-radius: 4px;
  margin-left: 8px;
}
.badge.on {
  background: var(--success);
  color: var(--bg);
}
.badge.off {
  background: var(--muted);
  color: var(--bg);
}
.muted {
  color: var(--muted);
}
</style>
