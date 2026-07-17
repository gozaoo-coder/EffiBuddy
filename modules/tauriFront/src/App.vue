<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import ChatWindow from './components/ChatWindow.vue'
import DevicePanel from './components/DevicePanel.vue'
import SettingsPanel from './components/SettingsPanel.vue'
import ThemeSwitcher from './components/ThemeSwitcher.vue'
import { IconButton, ToastHost, SnackbarHost, useToast } from './components/basic'
import { applyThemeNow } from './composables/useTheme'

const agentBackend = ref('')
const panelOpen = ref(false)
const settingsOpen = ref(false)
const { toast } = useToast()

async function refreshBackend() {
  try {
    agentBackend.value = await invoke<string>('get_agent_backend')
  } catch {
    agentBackend.value = 'unknown'
  }
}

onMounted(async () => {
  // 启动时立即应用持久化的主题，避免闪烁
  try {
    const config = await invoke<{ theme: 'system' | 'light' | 'dark' }>('get_config')
    applyThemeNow(config.theme)
  } catch {
    // 默认 system
  }
  await refreshBackend()
})

function togglePanel() {
  panelOpen.value = !panelOpen.value
}

function openSettings() {
  settingsOpen.value = true
}

function onSettingsSaved(backend: string) {
  agentBackend.value = backend
  toast({ content: `Agent 已切换：${backend}`, type: 'success' })
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
        <ThemeSwitcher />
        <IconButton icon="⚙" size="sm" container title="设置" @click="openSettings" />
        <IconButton
          :icon="panelOpen ? '✕' : '📱'"
          size="sm"
          container
          :variant="panelOpen ? 'primary' : 'normal'"
          :title="panelOpen ? '关闭设备面板' : '打开设备面板'"
          @click="togglePanel"
        />
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

    <!-- 全局反馈宿主：Toast / Snackbar -->
    <ToastHost />
    <SnackbarHost />
  </div>
</template>
