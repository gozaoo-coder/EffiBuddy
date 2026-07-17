<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import ChatWindow from './components/ChatWindow.vue'
import DevicePanel from './components/DevicePanel.vue'
import SettingsPanel from './components/SettingsPanel.vue'

const agentBackend = ref('')
// 窄屏下设备面板以抽屉形式折叠，默认收起。
const panelOpen = ref(false)
// 设置面板开关
const settingsOpen = ref(false)

async function refreshBackend() {
  try {
    agentBackend.value = await invoke<string>('get_agent_backend')
  } catch {
    agentBackend.value = 'unknown'
  }
}

onMounted(refreshBackend)

function togglePanel() {
  panelOpen.value = !panelOpen.value
}

function openSettings() {
  settingsOpen.value = true
}

function onSettingsSaved(backend: string) {
  // 后端已热替换，刷新顶部 badge
  agentBackend.value = backend
}
</script>

<template>
  <div class="app-shell">
    <header class="app-header">
      <div class="brand">
        <span class="brand-mark">EffiSuite</span>
        <span v-if="agentBackend" class="agent-badge">backend: {{ agentBackend }}</span>
      </div>
      <div class="header-actions">
        <button class="header-btn" @click="openSettings">设置</button>
        <button class="panel-toggle" :class="{ active: panelOpen }" @click="togglePanel">
          设备
        </button>
      </div>
    </header>

    <main class="app-main">
      <section class="chat-area">
        <ChatWindow />
      </section>
      <aside class="device-area" :class="{ open: panelOpen }">
        <DevicePanel />
      </aside>
    </main>

    <div v-if="panelOpen" class="overlay" @click="panelOpen = false"></div>

    <SettingsPanel
      :open="settingsOpen"
      @close="settingsOpen = false"
      @saved="onSettingsSaved"
    />
  </div>
</template>
