<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import ChatWindow from './components/ChatWindow.vue'
import SideNav from './components/SideNav.vue'
import DevicePanel from './components/DevicePanel.vue'
import SettingsPanel from './components/SettingsPanel.vue'
import ModelConfigPanel from './components/ModelConfigPanel.vue'
import SkillPanel from './components/SkillPanel.vue'
import SchedulePanel from './components/SchedulePanel.vue'
import { IconButton, Icon, ToastHost, SnackbarHost, BindSheet, useToast } from './components/basic'
import { applyThemeNow } from './composables/useTheme'

const agentBackend = ref('')
// 侧栏抽屉（Kimi 风格左侧抽屉）
const sideNavOpen = ref(false)
// 各功能面板状态（均从 SideNav 触发）
const devicePanelOpen = ref(false)
const settingsOpen = ref(false)
const modelConfigOpen = ref(false)
const scheduledTasksOpen = ref(false)
const skillPanelOpen = ref(false)
// 当前选中的会话 id（由 SideNav 选择或 ChatWindow 新建时更新）
const currentConversationId = ref<string | null>(null)
const { toast } = useToast()

// SideNav 实例引用：用于在会话变更时调用 refresh()
const sideNavRef = ref<{ refresh: () => void } | null>(null)

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

function toggleSideNav() {
  sideNavOpen.value = !sideNavOpen.value
}

function onSettingsSaved(backend: string) {
  agentBackend.value = backend
  toast({ content: `Agent 已切换：${backend}`, type: 'success' })
}

// SideNav 选择会话（null 表示新建聊天）
function handleSelectConv(id: string | null) {
  currentConversationId.value = id
}

// ChatWindow 会话变更（流式结束 / 新建会话）→ 刷新 SideNav 列表
function onConversationChanged() {
  sideNavRef.value?.refresh()
}

// 从 SideNav 打开各面板时自动收起抽屉
function openDevicePanel() {
  sideNavOpen.value = false
  devicePanelOpen.value = true
}

function openSettingsPanel() {
  sideNavOpen.value = false
  settingsOpen.value = true
}

function openModelConfig() {
  sideNavOpen.value = false
  // ModelConfigPanel 为独立面板，直接打开
  modelConfigOpen.value = true
}

function openScheduledTasks() {
  sideNavOpen.value = false
  scheduledTasksOpen.value = true
}

function openSkills() {
  sideNavOpen.value = false
  skillPanelOpen.value = true
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
          size="md"
          container
          dot
          title="侧栏"
          @click="toggleSideNav"
        >
          <Icon name="menu" :size="20" />
        </IconButton>
      </div>

      <div class="header-center">
        <div class="model-pill">
          <span class="model-name">{{ modelDisplay }}</span>
          <span class="model-tag">Fast</span>
        </div>
      </div>

      <div class="header-right">
        <IconButton size="md" container title="静音">
          <Icon name="mute" :size="20" />
        </IconButton>
      </div>
    </header>

    <main class="app-main">
      <section class="chat-area">
        <ChatWindow
          :backend="agentBackend"
          :conversation-id="currentConversationId"
          @update:conversation-id="currentConversationId = $event"
          @conversation-changed="onConversationChanged"
        />
      </section>
    </main>

    <!-- 设备管理面板：按需弹出，非常驻 -->
    <BindSheet
      v-model:visible="devicePanelOpen"
      side="right"
      title="设备管理"
      width="380px"
    >
      <DevicePanel />
    </BindSheet>

    <!-- 左侧抽屉导航 -->
    <SideNav
      ref="sideNavRef"
      v-model:open="sideNavOpen"
      :active-id="currentConversationId"
      @open-settings="openSettingsPanel"
      @open-device="openDevicePanel"
      @open-model-config="openModelConfig"
      @open-scheduled-tasks="openScheduledTasks"
      @open-skills="openSkills"
      @select-conversation="handleSelectConv"
    />

    <SettingsPanel
      :open="settingsOpen"
      @close="settingsOpen = false"
    />

    <ModelConfigPanel
      :open="modelConfigOpen"
      @close="modelConfigOpen = false"
      @saved="onSettingsSaved"
    />

    <SkillPanel
      :open="skillPanelOpen"
      :conversation-id="currentConversationId"
      @close="skillPanelOpen = false"
    />

    <SchedulePanel
      :open="scheduledTasksOpen"
      @close="scheduledTasksOpen = false"
    />

    <!-- 全局反馈宿主：Toast / Snackbar -->
    <ToastHost />
    <SnackbarHost />
  </div>
</template>
