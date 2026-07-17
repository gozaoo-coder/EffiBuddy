<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
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

// 顶部药丸导航中显示的当前模型名称
const modelDisplay = computed(() => {
  if (!agentBackend.value || agentBackend.value === 'unknown') return 'EffiBuddy'
  // 简单处理：截取 provider/model 的最后一段，如 openai:gpt-4o -> gpt-4o
  const idx = agentBackend.value.lastIndexOf(':')
  const name = idx >= 0 ? agentBackend.value.slice(idx + 1) : agentBackend.value
  return name || 'EffiBuddy'
})
</script>

<template>
  <div class="app-shell">
    <!-- Kimi 风格顶部导航：左侧汉堡菜单、中间模型药丸、右侧操作 -->
    <header class="app-header">
      <div class="header-left">
        <IconButton
          icon="☰"
          size="md"
          container
          dot
          title="设备面板"
          @click="togglePanel"
        />
      </div>

      <div class="header-center">
        <div class="model-pill">
          <span class="model-name">{{ modelDisplay }}</span>
          <span class="model-tag">Fast</span>
        </div>
      </div>

      <div class="header-right">
        <ThemeSwitcher />
        <IconButton icon="🔇" size="md" container title="静音" />
        <IconButton icon="⚙" size="md" container title="设置" @click="openSettings" />
      </div>
    </header>

    <main class="app-main">
      <section class="chat-area">
        <ChatWindow :backend="agentBackend" />
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
