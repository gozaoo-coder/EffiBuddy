<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import TitleBar from './components/TitleBar.vue'
import TabBar from './components/TabBar.vue'
import TabContent from './components/TabContent.vue'
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
import { useTabs, NEW_CHAT_TAB_ID } from './composables/useTabs'
import { useAsr } from './composables/useAsr'
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
const { toast } = useToast()

// ============= 多页签状态 =============
const { tabs, openTab, activate, updateTab, findChatByConversationId, getActive } = useTabs()
const activeTab = getActive()

// ASR 事件监听安装句柄（onMounted 调用 install，onUnmounted 卸载）
const { install: installAsrEvents } = useAsr()
let uninstallAsrEvents: (() => void) | null = null

// 当前激活 chat 页签的 conversationId：供 HistoryRail 高亮 + SettingsPanel 上下文
// 非 chat 页签激活时返回 null（历史列表不高亮）
const activeChatConvId = computed<string | null>(() => {
  const t = activeTab.value
  return t?.kind === 'chat' ? (t.conversationId ?? null) : null
})

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

  // 安装 ASR 事件监听（asr-stream-chunk / asr-session-status / asr-upload-progress / asr-record-updated）
  // 全局单例：install 内部有 installed 标记，重复调用安全
  uninstallAsrEvents = await installAsrEvents()

  // 监听 set_title 工具成功更新标题事件：刷新 HistoryRail 列表 + 同步页签标题
  unlistens.push(
    await listen<ConversationTitlePayload>('conversation-title-updated', (e) => {
      historyRailRef.value?.refresh()
      const { conversation_id, title } = e.payload
      if (title) {
        const tab = findChatByConversationId(conversation_id)
        if (tab && tab.title !== title) updateTab(tab.id, { title })
      }
    }),
  )
})

onUnmounted(() => {
  unlistens.forEach((fn) => fn?.())
  unlistens = []
  uninstallAsrEvents?.()
  uninstallAsrEvents = null
})

function onSettingsSaved(backend: string) {
  agentBackend.value = backend
  // 模型切换/保存后刷新顶部模型名（取真实 model_name，而非后端标识）
  void refreshModelDisplay()
  toast({ content: `Agent 已切换：${backend}`, type: 'success' })
}

// HistoryRail 选择会话（null 表示新建聊天，title 仅已有会话携带）
// 多页签语义：已打开则激活，未打开则新建页签；新建聊天复用全局唯一 __new_chat__ 页签
function handleSelectConv(id: string | null, title?: string | null) {
  if (id === null) {
    // 新建聊天：复用已存在的 __new_chat__ 页签，避免多个空对话页签
    const sentinel = tabs.value.find((t) => t.id === NEW_CHAT_TAB_ID)
    if (sentinel) {
      activate(sentinel.id)
      return
    }
    openTab({
      id: NEW_CHAT_TAB_ID,
      kind: 'chat',
      title: '新对话',
      closable: true,
      instanceKey: '',
    })
    return
  }
  // 已有会话：按 conversationId 去重
  const found = findChatByConversationId(id)
  if (found) {
    activate(found.id)
    return
  }
  openTab({
    id,
    kind: 'chat',
    title: title?.trim() || '对话',
    closable: true,
    conversationId: id,
    instanceKey: '',
  })
}

// 页签内容会话变更（新建会话建立 / 流式结束）→ 刷新 HistoryRail 列表
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

// 从 IconRail 打开 ASR 页签（流式录入 / 文件转写 / 历史记录）
// 单例页签：相同 id 已存在则仅激活，避免重复打开
function openAsrTab(kind: 'asr-stream' | 'asr-upload' | 'asr-history') {
  const titleMap = {
    'asr-stream': 'ASR 录入',
    'asr-upload': 'ASR 转写',
    'asr-history': 'ASR 历史',
  } as const
  openTab({
    id: kind,
    kind,
    title: titleMap[kind],
    closable: true,
    instanceKey: '',
  })
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
        @open-asr="openAsrTab"
      />

      <!-- 第二栏：历史记录（新建聊天 / 置顶 / 文件夹分类） -->
      <HistoryRail
        ref="historyRailRef"
        :active-id="activeChatConvId"
        @select-conversation="handleSelectConv"
      />

      <!-- 主内容区：多页签栏 + 页签内容容器 -->
      <section class="chat-area">
        <TabBar />
        <TabContent
          :backend="agentBackend"
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
      :conversation-id="activeChatConvId"
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
