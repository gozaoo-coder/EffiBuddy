<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import ChatWindow from './components/ChatWindow.vue'
import DevicePanel from './components/DevicePanel.vue'

const agentBackend = ref('')
// 窄屏下设备面板以抽屉形式折叠，默认收起。
const panelOpen = ref(false)

onMounted(async () => {
  try {
    agentBackend.value = await invoke<string>('get_agent_backend')
  } catch {
    agentBackend.value = 'unknown'
  }
})

function togglePanel() {
  panelOpen.value = !panelOpen.value
}
</script>

<template>
  <div class="app-shell">
    <header class="app-header">
      <div class="brand">
        <span class="brand-mark">EffiSuite</span>
        <span v-if="agentBackend" class="agent-badge">backend: {{ agentBackend }}</span>
      </div>
      <button class="panel-toggle" :class="{ active: panelOpen }" @click="togglePanel">
        设备
      </button>
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
  </div>
</template>
