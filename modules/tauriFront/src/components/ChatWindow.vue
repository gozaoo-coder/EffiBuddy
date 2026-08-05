<script setup lang="ts">
/**
 * ChatWindow —— 聊天主窗口(编排壳)
 *
 * 职责边界:只做「组装」——
 *  1. 创建各领域 store(useChatCore / useChatStreaming /
 *     useMessageMenu / useChatCompression / useImagePreview / useAutoScroll /
 *     useAskUser / useVersioning)
 *  2. 注入会话级生命周期钩子(resetAll / afterLoad)
 *  3. 注册后端事件订阅与 conversationId 联动(useChatEvents)
 *  4. provide 共享 store,组装原子子组件
 *
 * 具体业务逻辑已下沉到 composables/chat/* 与 components/chat/*,
 * 本文件不再包含任何消息渲染 / 压缩 / 菜单等实现细节。
 *
 * 对外接口保持不变:props backend / conversationId,
 * emits update:conversation-id / conversation-changed(由 ChatTab 透传)。
 */
import { provide, onMounted, onUnmounted } from 'vue'
import ShellSessionBar from './ShellSessionBar.vue'
import ChatContextPanel from './ChatContextPanel.vue'
import { Menu, Dialog } from './basic'
import GradualBlur from './basic/GradualBlur.vue'
import ChatHome from './chat/ChatHome.vue'
import ChatTopBar from './chat/ChatTopBar.vue'
import SubAgentWindow from './SubAgentWindow.vue'
import { NEW_CHAT_TAB_ID } from '../composables/useTabs'
import ChatMessageList from './chat/ChatMessageList.vue'
import ChatComposer from './chat/ChatComposer.vue'
import ChatContextSheet from './chat/ChatContextSheet.vue'
import CompressionSheet from './chat/CompressionSheet.vue'
import ToolSheet from './chat/ToolSheet.vue'
import WorkingDirSheet from './chat/WorkingDirSheet.vue'
import VersionSheet from './chat/VersionSheet.vue'
import ImagePreview from './chat/ImagePreview.vue'
import AskUserDialog from './chat/AskUserDialog.vue'
import { CHAT_STORE_KEY } from '../composables/chat/store'
import { consumePendingPrompt } from '../composables/chat/pendingPrompt'
import { useAutoScroll } from '../composables/chat/useAutoScroll'
import { useChatCore } from '../composables/chat/useChatCore'
import { useChatStreaming } from '../composables/chat/useChatStreaming'
import { useMessageMenu } from '../composables/chat/useMessageMenu'
import { useChatCompression } from '../composables/chat/useChatCompression'
import { useImagePreview } from '../composables/chat/useImagePreview'
  import { useAskUser } from '../composables/chat/useAskUser'
  import { useChatEvents } from '../composables/chat/useChatEvents'
  import { useChatSend } from '../composables/chat/useChatSend'
  import { useVersioning } from '../composables/chat/useVersioning'

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
// 依赖顺序:autoscroll(无依赖)→ core → streaming / menu / compression / preview
// 状态为实例级:每个 ChatWindow(即每个会话页签)一份,KeepAlive 多实例互不污染。
const autoscroll = useAutoScroll()
const core = useChatCore(props, emit, autoscroll)
const streaming = useChatStreaming(core, autoscroll)
const menu = useMessageMenu(core, streaming)
const compression = useChatCompression(core)
  const preview = useImagePreview()
  const askUser = useAskUser(core, streaming)
  const versioning = useVersioning(core)
  const send = useChatSend(core, streaming, menu, autoscroll)

// ---------- 会话级生命周期钩子 ----------
// loadConversation 在会话切换/清空时调用 resetAll,加载成功后调用 afterLoad。
core.setSessionHooks({
    resetAll: () => {
      streaming.resetAll()
      menu.resetAll()
      askUser.resetAll()
      core.backToParent() // 退出子代理内嵌视图
      core.queuedCount.value = 0
      // 注:压缩浮窗状态按原行为不随会话切换重置,
      // compressExistingState 由事件层 watch conversationId → loadExistingCompression 更新。
    },
  afterLoad: async () => {
    // 从历史消息恢复气泡元数据(计费重算依赖 activeModelInfo,已在 loadConversation 内加载)
    streaming.restoreBubbleMetaFromHistory()
    // 历史消息附件回填 base64
    await streaming.loadConversationAttachments()
    // 刷新会话版本列表(切换会话/回溯后保持一致)
    await versioning.loadVersions()
  },
})

// ---------- 事件订阅(后端流式事件 + conversationId watch + 生命周期) ----------
useChatEvents(core, streaming, compression)

// 加载全局压缩设置(自动压缩阈值/开关),供压缩浮窗设置面板展示与编辑
void compression.loadCompressionSettings()

  provide(CHAT_STORE_KEY, {
    core,
    streaming,
    compression,
    menu,
    preview,
    autoscroll,
    askUser,
    versioning,
    send,
  })

// ---------- 空态聊天框待发送提示词 ----------
// TabEmpty 空态输入发送 → openTab 新建页签 → 本实例挂载后消费并自动发送。
// 仅限无会话的新页签（history 打开的会话不消费）。
onMounted(() => {
  if (props.conversationId) return
  const prompt = consumePendingPrompt()
  if (prompt) void send.sendPrompt(prompt)
})

// ---------- 卸载清理 ----------
onUnmounted(() => {
  autoscroll.dispose()
  menu.resetAll() // 清理消息长按 timer 与菜单状态
})
// ---------- 模板绑定(解构 ref,模板自动解包) ----------
  const {
    activeId,
    messages,
    ctxPanelOpen,
    isEmptyHome,
    contextUsedTokens,
    contextMaxTokens,
    activeModelInfo,
    shellBarExpanded,
    shellActiveCount,
    title,
    subAgentId,
    subAgentName,
    editTitle,
    backToParent,
    toggleCtxPanel,
  } = core
const { msgMenuVisible, msgMenuPosition, msgMenuItems, onMsgMenuSelect } = menu
const { confirmState: versionConfirmState, closeConfirm } = versioning
</script>

<template>
  <div class="chat-window">
    <!-- 聊天主区(侧栏已提升为 App 级 SideNav 抽屉) -->
      <section class="chat-main">
        <!-- 顶部渐进模糊层(vue-bits gradual-blur 移植):消息滚入顶部时渐进虚化;
             层级:chat-topbar / composer 之上,普通消息流之下 -->
        <GradualBlur
          v-if="!subAgentId"
          position="top"
          height="70px"
          :strength="2.5"
          :div-count="7"
          :z-index="20"
          curve="bezier"
        />
        <!-- 顶部悬浮顶栏：标题(默认态可点击修改；子代理态显示 `[ 父标题 ] / [ 子代理标题 ]` 面包屑)
             + 收起面板 + 上下文用量 ring(hover 浮出文字) -->
        <ChatTopBar
          :title="title"
          :sub-title="subAgentName"
          :used="contextUsedTokens"
          :max="contextMaxTokens"
          :show-ring="true"
          :show-panel="true"
          :panel-open="ctxPanelOpen"
          :editable="!!activeId && activeId !== NEW_CHAT_TAB_ID"
          @edit-title="editTitle"
          @back-to-parent="backToParent"
          @toggle-panel="toggleCtxPanel"
        />

        <!-- 空状态首页:中央品牌区 + 快捷胶囊(子代理视图下隐藏) -->
        <ChatHome v-if="isEmptyHome && !subAgentId" />
        <!-- 消息列表 -->
        <ChatMessageList v-else-if="!subAgentId" />
        <!-- 子代理内嵌视图:点击子代理卡片后主视图切换为该子代理全流程,
             顶栏面包屑 `[ 父标题 ] / [ 子代理标题 ]` 提示当前位置,点击父标题可返回 -->
        <div v-else class="sub-agent-embed">
          <SubAgentWindow :session-id="subAgentId ?? ''" embedded />
        </div>

        <!-- 底部悬浮层:命令会话便签栏(输入栏上方) + Kimi 风格输入栏 -->
        <div class="chat-bottom">
          <ShellSessionBar
            v-if="!subAgentId"
            :conversation-id="activeId"
            :expanded="shellBarExpanded"
            @update:expanded="(v) => (shellBarExpanded = v)"
            @running-count="(n) => (shellActiveCount = n)"
          />
          <!-- Kimi 风格底部输入栏(子代理视图下隐藏,子代理由后端驱动) -->
          <ChatComposer v-if="!subAgentId" />
        </div>
      </section>

      <ChatContextPanel
        v-if="ctxPanelOpen"
        :conversation-id="activeId"
        :messages="messages"
        :context-used-tokens="contextUsedTokens"
        :context-max-tokens="contextMaxTokens"
        :pricing="activeModelInfo?.pricing ?? null"
      />

    <!-- 消息长按 / 右键菜单 -->
    <Menu
      v-model:visible="msgMenuVisible"
      :items="msgMenuItems"
      :position="msgMenuPosition"
      @select="onMsgMenuSelect"
    />

    <!-- 会话版本管理(分支/临时版本/检查点):顶部列表 + 检出/删除 -->
    <VersionSheet />

    <!-- 版本破坏性操作确认框(回溯/撤回/检出/删除引用) -->
    <Dialog
      v-if="versionConfirmState"
      :visible="versionConfirmState.visible"
      :title="versionConfirmState.title"
      :content="versionConfirmState.content"
      :confirm-text="versionConfirmState.confirmText"
      :cancel-text="'取消'"
      :danger="versionConfirmState.danger"
      @update:visible="closeConfirm"
      @confirm="versionConfirmState.onConfirm()"
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
