<script setup lang="ts">
import { ref, onMounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import ChatWindow from './components/ChatWindow.vue'
import DevicePanel from './components/DevicePanel.vue'
import SettingsPanel from './components/SettingsPanel.vue'
import ThemeSwitcher from './components/ThemeSwitcher.vue'
import { IconButton, ToastHost, SnackbarHost, useToast } from './components/basic'
import { applyThemeNow } from './composables/useTheme'
import { useLayoutAnimation } from './composables/useLayoutAnimation'

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

// ---------- animejs v4 Layout 动画 ----------
// 主区域 flex 布局切换动画（侧边栏开/关）
const mainLayout = useLayoutAnimation({
  container: '.app-main',
  duration: 300,
  ease: 'outQuad',
  leaveTo: {
    transform: 'scale(0.92)',
    opacity: 0,
    duration: 250,
    ease: 'out(2)',
  },
  enterFrom: {
    transform: 'scale(0.92)',
    opacity: 0,
    duration: 350,
    ease: 'out(3)',
  },
})

// 监听 device panel 切换，触发布局动画
watch(panelOpen, () => {
  mainLayout.update(({ root }) => {
    // 触发 layout 重排以触发 animejs 检测
    const aside = root.querySelector('.device-area') as HTMLElement | null
    if (aside) {
      // 强制重排：toggle class 驱动 animejs 的显示/隐藏检测
      void aside.offsetHeight
    }
  })
})

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

// 设置面板关闭时，触发布局动画（让 BindSheet 关闭后 app-main 有过渡）
function onSettingsClose() {
  settingsOpen.value = false
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

    <main class="app-main layout-container">
      <section class="chat-area layout-item">
        <ChatWindow />
      </section>
      <aside class="device-area layout-item" :class="{ open: panelOpen }">
        <DevicePanel />
      </aside>
    </main>

    <div v-if="panelOpen" class="overlay" @click="panelOpen = false"></div>

    <SettingsPanel
      :open="settingsOpen"
      @close="onSettingsClose"
      @saved="onSettingsSaved"
    />

    <!-- 全局反馈宿主：Toast / Snackbar -->
    <ToastHost />
    <SnackbarHost />
  </div>
</template>