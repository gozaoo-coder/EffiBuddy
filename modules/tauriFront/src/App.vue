<script setup lang="ts">
/**
 * EffiSuite 应用根组件
 *
 * 布局（2026-08 重构）：
 * - 顶栏 TitleBar：左侧第一个按钮弹出功能菜单（左栏一重定义），第二个按钮切换左栏二 + 窗口控件
 * - 功能菜单 FeatureMenu：dropdown menu，图标+文字，内置功能 + 插件贡献按钮
 * - 左栏二 SecondRailHost：HistoryRail 展开 / 收起封装，含 交流池/模型配置 三态切换
 * - 主内容区：多页签（聊天 / ASR / 子 agent / 插件页面 / 桌面小组件）
 */
import { ref, onMounted, onUnmounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import TitleBar from './components/TitleBar.vue'
import TabContent from './components/TabContent.vue'
import SecondRailHost from './components/SecondRailHost.vue'
import P2pPanel from './components/P2pPanel.vue'
import SettingsPanel from './components/SettingsPanel.vue'
import SkillPanel from './components/SkillPanel.vue'
import PluginPanel from './components/PluginPanel.vue'
import ClawHubPanel from './components/ClawHubPanel.vue'
import SchedulePanel from './components/SchedulePanel.vue'
import type { ModelSettingsView } from './components/model-settings/ModelSettingsRail.vue'
import ModelSettingsContent from './components/model-settings/ModelSettingsContent.vue'
import { ToastHost, SnackbarHost, BindSheet, useToast } from './components/basic'
import { applyThemeNow } from './composables/useTheme'
import { useTabs, NEW_CHAT_TAB_ID } from './composables/useTabs'
import { useAsr } from './composables/useAsr'
import { useP2p } from './composables/useP2p'
import { useAgentPool } from './composables/useAgentPool'
import { useAnimeTransition } from './composables/useAnimeTransition'
import { usePluginContributions } from './composables/usePluginContributions'
import { useAppActions } from './composables/appActions'
import type { ConversationTitlePayload, RailView } from './types'

const agentBackend = ref('')
// 各功能面板状态（均从 IconRail 触发）
// P2P 设备 composable：pendingCount 驱动角标、refreshAll 在面板打开时刷新数据
const { pendingCount, refreshAll } = useP2p()
// 交流池 composable：activeSessionCount 驱动 IconRail 角标（实时刷新由 composable 内部事件监听负责）
const { activeSessionCount: poolActiveCount } = useAgentPool()
const p2pPanelOpen = ref(false)
const settingsOpen = ref(false)
const scheduledTasksOpen = ref(false)
const skillPanelOpen = ref(false)
const pluginPanelOpen = ref(false)
const clawhubPanelOpen = ref(false)
const { toast } = useToast()

// ============= 模型配置模式 =============
// modelConfigOpen 为 true 时进入模型配置模式：
// - 左2栏从 HistoryRail 切换为 ModelSettingsRail（二级栏目）
// - 主内容区从 TabBar+TabContent 切换为 ModelSettingsContent
// modelSettingsView 记录当前选中的二级子项（'' 表示未选中，显示默认介绍页）
const modelConfigOpen = ref(false)
const modelSettingsView = ref<ModelSettingsView | ''>('')

// ============= 交流池模式 =============
// poolOpen 为 true 时左2栏从 HistoryRail 切换为 AgentPoolRail（展示运行时
// agent 公共会话交流池的全部条目，含状态 / 任务 / 研究报告 / @ 消息）。
// 与 modelConfigOpen 互斥（同一时刻左2栏只能展示一种二级栏目）。
const poolOpen = ref(false)

// ============= 多页签状态 =============
const { tabs, openTab, activate, updateTab, findChatByConversationId, getActive } = useTabs()
const activeTab = getActive()

// ASR 事件监听安装句柄（onMounted 调用 install，onUnmounted 卸载）
const { install: installAsrEvents } = useAsr()
let uninstallAsrEvents: (() => void) | null = null

// 当前激活 chat 页签的 conversationId：供左栏二高亮 + SettingsPanel 上下文
// 非 chat 页签激活时返回 null（历史列表不高亮）
const activeChatConvId = computed<string | null>(() => {
  const t = activeTab.value
  return t?.kind === 'chat' ? (t.conversationId ?? null) : null
})

// SecondRailHost 实例引用：用于在会话变更时调用 refresh()
const secondRailRef = ref<{ refresh: () => void } | null>(null)

// 插件贡献注册：拉取已安装插件的声明式贡献（左栏按钮 / 页面 / 命令）
const { install: installPluginContributions } = usePluginContributions()

// 全局动作中枢：供空态引导卡片 / 插件页面调用 App 级动作
const { register: registerAction } = useAppActions()

// 事件取消订阅句柄集合
let unlistens: UnlistenFn[] = []

// 功能菜单当前高亮视图：有面板打开时高亮对应项，激活 widget 页签时高亮桌面小组件，否则默认聊天
const activeView = computed<RailView | ''>(() => {
  if (modelConfigOpen.value) return 'model-config'
  if (poolOpen.value) return 'pool'
  if (scheduledTasksOpen.value) return 'automation'
  if (skillPanelOpen.value) return 'skills'
  if (pluginPanelOpen.value) return 'plugins'
  if (activeTab.value?.kind === 'widget') return 'widget'
  return 'chat'
})

function closeAllPanels() {
  p2pPanelOpen.value = false
  settingsOpen.value = false
  modelConfigOpen.value = false
  modelSettingsView.value = ''
  poolOpen.value = false
  scheduledTasksOpen.value = false
  skillPanelOpen.value = false
  pluginPanelOpen.value = false
  clawhubPanelOpen.value = false
}

// 功能菜单视图选择：切换视图 / 开关面板
function onRailSelect(view: RailView) {
  // 切换时先关闭其它功能面板，避免叠加
  p2pPanelOpen.value = false
  settingsOpen.value = false
  clawhubPanelOpen.value = false
  switch (view) {
    case 'chat':
      closeAllPanels()
      break
    case 'widget':
      closeAllPanels()
      openWidgetTab()
      break
    case 'pool':
      // 切换交流池模式：再次点击关闭
      if (poolOpen.value) {
        poolOpen.value = false
      } else {
        // 关闭其它二级栏目（互斥）
        modelConfigOpen.value = false
        modelSettingsView.value = ''
        poolOpen.value = true
      }
      break
    case 'model-config':
      // 切换模型配置模式：再次点击关闭
      if (modelConfigOpen.value) {
        modelConfigOpen.value = false
        modelSettingsView.value = ''
      } else {
        // 关闭其它二级栏目（互斥）
        poolOpen.value = false
        modelConfigOpen.value = true
        // 默认进入 AI服务商 子项
        modelSettingsView.value = 'providers'
      }
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

// ModelSettingsRail 子项选择
function onModelSettingsSelect(view: ModelSettingsView) {
  modelSettingsView.value = view
}

// 模型配置面板保存后的回调（model-settings 各子面板的 saved 事件）
function onModelSettingsSaved() {
  void refreshBackend()
}

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

  // 安装 ASR 事件监听（asr-stream-chunk / asr-session-status / asr-upload-progress / asr-record-updated）
  // 全局单例：install 内部有 installed 标记，重复调用安全
  uninstallAsrEvents = await installAsrEvents()

  // 注册全局动作（供空态引导卡片 / 插件页面解耦调用）
  registerAction('new-chat', () => handleSelectConv(null))
  registerAction('open-clawhub', openClawHub)
  registerAction('open-settings', openSettingsPanel)
  registerAction('open-plugin-panel', () => {
    skillPanelOpen.value = false
    pluginPanelOpen.value = true
  })
  registerAction('open-skill-panel', () => {
    pluginPanelOpen.value = false
    skillPanelOpen.value = true
  })
  registerAction('open-asr', () => openAsrTab('asr-stream'))
  registerAction('open-todo', () => openPluginPage('effisuite/user-todo'))
  registerAction('open-automation', () => {
    scheduledTasksOpen.value = true
  })

  // 加载插件声明式贡献（左栏按钮 / 页面 / 命令）
  await installPluginContributions()

  // 监听 set_title 工具成功更新标题事件：刷新左栏二列表 + 同步页签标题
  unlistens.push(
    await listen<ConversationTitlePayload>('conversation-title-updated', (e) => {
      secondRailRef.value?.refresh()
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
  toast({ content: `Agent 已切换：${backend}`, type: 'success' })
}

// 左栏二选择会话（null 表示新建聊天，title 仅已有会话携带）
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

// 页签内容会话变更（新建会话建立 / 流式结束）→ 刷新左栏二列表
function onConversationChanged() {
  secondRailRef.value?.refresh()
}

// 从功能菜单打开桌面小组件页签（单例：__widget_desktop__ 已存在则仅激活）
const WIDGET_TAB_ID = '__widget_desktop__'
function openWidgetTab() {
  const found = tabs.value.find((t) => t.id === WIDGET_TAB_ID)
  if (found) {
    activate(found.id)
    return
  }
  openTab({
    id: WIDGET_TAB_ID,
    kind: 'widget',
    title: '桌面小组件',
    icon: 'layout',
    closable: true,
    instanceKey: '',
  })
}

// 从 IconRail 打开 P2P 设备面板（同时刷新数据）
function openP2pPanel() {
  p2pPanelOpen.value = true
  refreshAll()
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

// 打开插件页面页签（单例：同 id 已存在则仅激活）
function openPluginPage(pageId: string) {
  openTab({
    id: `plugin-page:${pageId}`,
    kind: 'plugin',
    title: '插件页',
    icon: 'puzzle',
    closable: true,
    instanceKey: '',
    pluginPageId: pageId,
  })
  // 打开插件页面时返回聊天主视图（关闭其它面板/二级栏目）
  closeAllPanels()
}

// 插件命令触发：当前版本提示，未来可路由到 agent 技能执行
function handlePluginCommand(commandId: string) {
  toast({ content: `插件命令触发：${commandId}`, type: 'info' })
}

// 主内容区切换动画：TabContent ↔ ModelSettingsContent
const { onEnter: onMainEnter, onLeave: onMainLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateY: [8, 0],
    duration: 260,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    translateY: [0, -6],
    duration: 180,
    ease: 'inOut(2)',
  },
})
</script>

<template>
  <div class="app-shell">
    <!-- 自定义标题栏：左侧功能菜单按钮 + 左栏二切换 + 中间页签栏 + 窗口控件 -->
    <TitleBar
      :model-config-open="modelConfigOpen"
      :active-view="activeView"
      :pending-pair-count="pendingCount"
      :pool-active-count="poolActiveCount"
      @select="onRailSelect"
      @open-plugin-page="openPluginPage"
      @open-plugin-command="handlePluginCommand"
      @open-clawhub="openClawHub"
      @open-p2p="openP2pPanel"
      @open-settings="openSettingsPanel"
      @open-asr="openAsrTab"
    />

    <main class="app-main">
      <!-- 左栏二：SecondRailHost 封装 展开/收起 + 交流池/模型配置 三态切换 -->
      <SecondRailHost
        ref="secondRailRef"
        :pool-open="poolOpen"
        :model-config-open="modelConfigOpen"
        :model-settings-view="modelSettingsView"
        :active-chat-conv-id="activeChatConvId"
        @select-conversation="handleSelectConv"
        @select-model-settings="onModelSettingsSelect"
      />

      <!-- 主内容区：根据模式切换（聊天模式=多页签 / 模型配置模式=模型设置面板） -->
      <Transition :css="false" @enter="onMainEnter" @leave="onMainLeave" mode="out-in">
        <section v-if="!modelConfigOpen" key="chat" class="chat-area">
          <TabContent
            :backend="agentBackend"
            @conversation-changed="onConversationChanged"
          />
        </section>
        <ModelSettingsContent
          v-else
          key="model-settings"
          :view="modelSettingsView"
          @saved="onModelSettingsSaved"
        />
      </Transition>
    </main>

    <!-- P2P 设备面板 -->
    <BindSheet
      v-model:visible="p2pPanelOpen"
      side="right"
      title="P2P 设备"
      width="380px"
    >
      <P2pPanel />
    </BindSheet>

    <SettingsPanel
      :open="settingsOpen"
      :conversation-id="activeChatConvId"
      @close="settingsOpen = false"
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
