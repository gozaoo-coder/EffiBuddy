<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import ChatWindow from './components/ChatWindow.vue'
import TitleBar from './components/TitleBar.vue'
import IconRail, { type RailView } from './components/IconRail.vue'
import HistoryRail from './components/HistoryRail.vue'
import DevicePanel from './components/DevicePanel.vue'
import SettingsPanel from './components/SettingsPanel.vue'
import ModelConfigPanel from './components/ModelConfigPanel.vue'
import SkillPanel from './components/SkillPanel.vue'
import PluginPanel from './components/PluginPanel.vue'
import ClawHubPanel from './components/ClawHubPanel.vue'
import SchedulePanel from './components/SchedulePanel.vue'
import { ToastHost, SnackbarHost, BindSheet, useToast } from './components/basic'
import { applyThemeNow } from './composables/useTheme'
import type { ConversationTitlePayload } from './types'

const agentBackend = ref('')
// 当前激活模型名称（真实 model_name，来自 get_active_model_info）
const activeModelName = ref('')
// 各功能面板状态（均从 IconRail 触发）
const devicePanelOpen = ref(false)
const settingsOpen = ref(false)
const modelConfigOpen = ref(false)
const scheduledTasksOpen = ref(false)
const skillPanelOpen = ref(false)
const pluginPanelOpen = ref(false)
const clawhubPanelOpen = ref(false)
// 当前选中的会话 id（由 HistoryRail 选择或 ChatWindow 新建时更新）
const currentConversationId = ref<string | null>(null)
const { toast } = useToast()

// HistoryRail 实例引用：用于在会话变更时调用 refresh()
const historyRailRef = ref<{ refresh: () => void } | null>(null)

// 事件取消订阅句柄集合
let unlistens: UnlistenFn[] = []

// IconRail 当前高亮视图：有面板打开时高亮对应图标，否则默认聊天
const activeView = computed<RailView | ''>(() => {
  if (modelConfigOpen.value) return 'model-config'
  if (scheduledTasksOpen.value) return 'automation'
  if (skillPanelOpen.value) return 'skills'
  if (pluginPanelOpen.value) return 'plugins'
  return 'chat'
})

function closeAllPanels() {
  devicePanelOpen.value = false
  settingsOpen.value = false
  modelConfigOpen.value = false
  scheduledTasksOpen.value = false
  skillPanelOpen.value = false
  pluginPanelOpen.value = false
  clawhubPanelOpen.value = false
}

// IconRail 主栏点击：切换视图 / 开关面板
function onRailSelect(view: RailView) {
  // 切换时先关闭其它功能面板，避免叠加
  devicePanelOpen.value = false
  settingsOpen.value = false
  clawhubPanelOpen.value = false
  switch (view) {
    case 'chat':
      closeAllPanels()
      break
    case 'model-config':
      modelConfigOpen.value = !modelConfigOpen.value
      break
    case 'automation':
      scheduledTasksOpen.value = !scheduledTasksOpen.value
      break
    case 'skills':
      skillPanelOpen.value = !skillPanelOpen.value
      break
    case 'plugins':
      pluginPanelOpen.value = !pluginPanelOpen.value
      break
  }
}

async function refreshBackend() {
  try {
    agentBackend.value = await invoke<string>('get_agent_backend')
  } catch {
    agentBackend.value = 'unknown'
  }
}

// 刷新顶部显示的当前模型名称（真实 model_name）
async function refreshModelDisplay() {
  try {
    const info = await invoke<{ name: string }>('get_active_model_info')
    activeModelName.value = info.name || ''
  } catch {
    activeModelName.value = ''
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
  await refreshModelDisplay()

  // 监听 set_title 工具成功更新标题事件：立即刷新 HistoryRail 列表
  unlistens.push(
    await listen<ConversationTitlePayload>('conversation-title-updated', () => {
      historyRailRef.value?.refresh()
    }),
  )
})

onUnmounted(() => {
  unlistens.forEach((fn) => fn?.())
  unlistens = []
})

function onSettingsSaved(backend: string) {
  agentBackend.value = backend
  // 模型切换/保存后刷新顶部模型名（取真实 model_name，而非后端标识）
  void refreshModelDisplay()
  toast({ content: `Agent 已切换：${backend}`, type: 'success' })
}

// HistoryRail 选择会话（null 表示新建聊天）
function handleSelectConv(id: string | null) {
  currentConversationId.value = id
}

// ChatWindow 会话变更（流式结束 / 新建会话）→ 刷新 HistoryRail 列表
function onConversationChanged() {
  historyRailRef.value?.refresh()
}

// 从「更多」打开各面板
function openDevicePanel() {
  devicePanelOpen.value = true
}

function openSettingsPanel() {
  settingsOpen.value = true
}

function openClawHub() {
  skillPanelOpen.value = false
  pluginPanelOpen.value = false
  clawhubPanelOpen.value = true
}

// 标题栏中间显示的当前模型名称（真实 model_name）
const modelDisplay = computed(() => activeModelName.value || 'EffiBuddy')
</script>

<template>
  <div class="app-shell">
    <!-- 自定义标题栏：左侧品牌、中间模型、右上角窗口控件 -->
    <TitleBar :model-name="modelDisplay" />

    <main class="app-main">
      <!-- 第一栏：router（纯图标 + hover 提示） -->
      <IconRail
        :active="activeView"
        @select="onRailSelect"
        @open-clawhub="openClawHub"
        @open-device="openDevicePanel"
        @open-settings="openSettingsPanel"
      />

      <!-- 第二栏：历史记录（新建聊天 / 置顶 / 文件夹分类） -->
      <HistoryRail
        ref="historyRailRef"
        :active-id="currentConversationId"
        @select-conversation="handleSelectConv"
      />

      <!-- 主聊天区 -->
      <section class="chat-area">
        <ChatWindow
          :backend="agentBackend"
          :conversation-id="currentConversationId"
          @update:conversation-id="currentConversationId = $event"
          @conversation-changed="onConversationChanged"
        />
      </section>
    </main>

    <!-- 设备管理面板 -->
    <BindSheet
      v-model:visible="devicePanelOpen"
      side="right"
      title="设备管理"
      width="380px"
    >
      <DevicePanel />
    </BindSheet>

    <SettingsPanel
      :open="settingsOpen"
      :conversation-id="currentConversationId"
      @close="settingsOpen = false"
    />

    <ModelConfigPanel
      :open="modelConfigOpen"
      @close="modelConfigOpen = false"
      @saved="onSettingsSaved"
    />

    <SkillPanel
      :open="skillPanelOpen"
      @close="skillPanelOpen = false"
      @open-clawhub="openClawHub"
    />

    <PluginPanel
      :open="pluginPanelOpen"
      @close="pluginPanelOpen = false"
      @open-clawhub="openClawHub"
    />

    <ClawHubPanel
      :open="clawhubPanelOpen"
      @close="clawhubPanelOpen = false"
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
