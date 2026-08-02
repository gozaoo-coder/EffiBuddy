<script setup lang="ts">
/**
 * ChatWindow —— 聊天主窗口(编排壳)
 *
 * 职责边界:只做「组装」——
 *  1. 创建各领域 store(useChatCore / useChatStreaming / useTaskMode /
 *     useMessageMenu / useChatCompression / useImagePreview / useAutoScroll)
 *  2. 注入会话级生命周期钩子(resetAll / afterLoad)
 *  3. 注册后端事件订阅与 conversationId 联动(useChatEvents)
 *  4. provide 共享 store,组装原子子组件
 *
 * 具体业务逻辑已下沉到 composables/chat/* 与 components/chat/*,
 * 本文件不再包含任何消息渲染 / 压缩 / 任务 / 菜单等实现细节。
 *
 * 对外接口保持不变:props backend / conversationId,
 * emits update:conversation-id / conversation-changed(由 ChatTab 透传)。
 */
import { provide, onUnmounted } from 'vue'
import ShellSessionBar from './ShellSessionBar.vue'
import ChatContextPanel from './ChatContextPanel.vue'
import { Menu } from './basic'
import ChatHome from './chat/ChatHome.vue'
import ChatMessageList from './chat/ChatMessageList.vue'
import ChatComposer from './chat/ChatComposer.vue'
import ChatContextSheet from './chat/ChatContextSheet.vue'
import CompressionSheet from './chat/CompressionSheet.vue'
import ToolSheet from './chat/ToolSheet.vue'
import WorkingDirSheet from './chat/WorkingDirSheet.vue'
import ImagePreview from './chat/ImagePreview.vue'
import AskUserDialog from './chat/AskUserDialog.vue'
import { CHAT_STORE_KEY } from '../composables/chat/store'
import { useAutoScroll } from '../composables/chat/useAutoScroll'
import { useChatCore } from '../composables/chat/useChatCore'
import { useChatStreaming } from '../composables/chat/useChatStreaming'
import { useTaskMode } from '../composables/chat/useTaskMode'
import { useMessageMenu } from '../composables/chat/useMessageMenu'
import { useChatCompression } from '../composables/chat/useChatCompression'
import { useImagePreview } from '../composables/chat/useImagePreview'
import { useAskUser } from '../composables/chat/useAskUser'
import { useChatEvents } from '../composables/chat/useChatEvents'

// 后端名称(来自 App.vue 顶部模型药丸)+ 当前会话 id(由 App 传入)
const props = defineProps<{
  backend?: string
  conversationId?: string | null
}>()

const emit = defineEmits<{
  (e: 'update:conversation-id', id: string | null): void
  (e: 'conversation-changed'): void
}>()

// ---------- 领域 store 创建 ----------
// 依赖顺序:autoscroll(无依赖)→ core → streaming / taskMode / menu / compression / preview
// 状态为实例级:每个 ChatWindow(即每个会话页签)一份,KeepAlive 多实例互不污染。
const autoscroll = useAutoScroll()
const core = useChatCore(props, emit, autoscroll)
const streaming = useChatStreaming(core, autoscroll)
const taskMode = useTaskMode(core)
const menu = useMessageMenu(core, streaming)
const compression = useChatCompression(core)
const preview = useImagePreview()
const askUser = useAskUser(core, streaming)

// ---------- 会话级生命周期钩子 ----------
// loadConversation 在会话切换/清空时调用 resetAll,加载成功后调用 afterLoad。
core.setSessionHooks({
  resetAll: () => {
    streaming.resetAll()
    taskMode.resetAll()
    menu.resetAll()
    askUser.resetAll()
    // 注:压缩浮窗状态按原行为不随会话切换重置,
    // compressExistingState 由事件层 watch conversationId → loadExistingCompression 更新。
  },
  afterLoad: async () => {
    // 从历史消息恢复气泡元数据(计费重算依赖 activeModelInfo,已在 loadConversation 内加载)
    streaming.restoreBubbleMetaFromHistory()
    // 历史消息附件回填 base64
    await streaming.loadConversationAttachments()
    // 任务清单:非空即进入长程任务模式
    await taskMode.loadTodoTree()
    taskMode.syncFromTodo()
  },
})

// ---------- 事件订阅(后端流式事件 + conversationId watch + 生命周期) ----------
useChatEvents(core, streaming, compression, taskMode)

// ---------- 共享 store 下发 ----------
provide(CHAT_STORE_KEY, { core, streaming, compression, taskMode, menu, preview, autoscroll, askUser })

// ---------- 卸载清理 ----------
onUnmounted(() => {
  autoscroll.dispose()
  menu.resetAll() // 清理消息长按 timer 与菜单状态
})

// ---------- 模板绑定(解构 ref,模板自动解包) ----------
const {
  activeId,
  messages,
  sending,
  ctxPanelOpen,
  isEmptyHome,
  contextUsedTokens,
  contextMaxTokens,
  activeModelInfo,
  shellBarExpanded,
  shellActiveCount,
} = core
const { msgMenuVisible, msgMenuPosition, msgMenuItems, onMsgMenuSelect } = menu
</script>

<template>
  <div class="chat-window">
    <!-- 聊天主区(侧栏已提升为 App 级 SideNav 抽屉) -->
    <section class="chat-main">
      <!-- 空状态首页:中央品牌区 + 快捷胶囊 -->
      <ChatHome v-if="isEmptyHome" />
      <!-- 消息列表(含长程任务气泡) -->
      <ChatMessageList v-else />

      <!-- Kimi 风格底部输入栏 -->
      <ChatComposer />

      <!-- main-content 底栏:命令会话便签(可折叠,实时展示 AI 的 shell_session_* 工作状态;
           折叠按钮位于 composer-meta,此处为受控组件) -->
      <ShellSessionBar
        :conversation-id="activeId"
        :expanded="shellBarExpanded"
        @update:expanded="(v) => (shellBarExpanded = v)"
        @running-count="(n) => (shellActiveCount = n)"
      />
    </section>

    <!-- 右栏:上下文面板(todoTree / 上下文窗口 / 用量 / 压缩) -->
    <ChatContextPanel
      v-if="ctxPanelOpen"
      :conversation-id="activeId"
      :messages="messages"
      :context-used-tokens="contextUsedTokens"
      :context-max-tokens="contextMaxTokens"
      :pricing="activeModelInfo?.pricing ?? null"
      :streaming="sending"
    />

    <!-- 消息长按 / 右键菜单 -->
    <Menu
      v-model:visible="msgMenuVisible"
      :items="msgMenuItems"
      :position="msgMenuPosition"
      @select="onMsgMenuSelect"
    />

    <!-- 底部浮窗:上下文管理 / 消息压缩 / 工具 / 工作区 -->
    <ChatContextSheet />
    <CompressionSheet />
    <ToolSheet />
    <WorkingDirSheet />

    <!-- AI 询问用户对话框(ask_user 工具触发) -->
    <AskUserDialog />

    <!-- 图片全屏预览(Teleport 到 body) -->
    <ImagePreview />
  </div>
</template>
