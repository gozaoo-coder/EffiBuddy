<script setup lang="ts">
/**
 * TabEmpty —— 无页签空状态
 *
 * 展示品牌引导 + 标准版完整输入栏（直接复用 ChatComposer 组件，
 * 含 + 附件 / 思考设置 / 工作区 / 命令会话 / 发送 等全部控件）。
 * 输入后回车 / 点发送即新建聊天页签并立即发送。
 *
 * 此时还没有任何 ChatWindow（会话级 store 不存在），因此：
 *  - 通过 pendingPrompt 把提示词传给新挂载的 ChatWindow 消费（见 pendingPrompt.ts）；
 *  - ChatComposer 依赖的 chat store 在此 provide 一个「最小可渲染适配版」：
 *    发送行为改为「写 pendingPrompt + 新建页签」，不创建真实会话；
 *    activeModelInfo / loadActiveModelInfo 直连后端 get_active_model_info，
 *    保证空态下模型选择 pill 同样可用。
 */
import { ref, provide, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import Icon from './Icon.vue'
import ChatComposer from './chat/ChatComposer.vue'
import { useToast } from './basic'
import { CHAT_STORE_KEY, type ChatStore } from '../composables/chat/store'
import { useTabs, NEW_CHAT_TAB_ID } from '../composables/useTabs'
import { setPendingPrompt } from '../composables/chat/pendingPrompt'
import type { ActiveModelInfo } from '../composables/chat/useChatCore'

// 声明 emits 以消费父级透传的监听器，避免落到根 div 成为无效事件属性
defineEmits<{
  (e: 'update:conversation-id', id: string | null): void
  (e: 'conversation-changed'): void
  (e: 'update:status', status: string): void
}>()

const { openTab } = useTabs()
const { toast } = useToast()

// ---------- 空态适配 store（仅供 ChatComposer 渲染与发送） ----------
const input = ref('')
const sending = ref(false)
const queuedCount = ref(0)
const workingDir = ref<string | null>(null)
const thinking = ref(false)
const reasoningEffort = ref<'low' | 'high' | 'max'>('high')
const workingDirSheetOpen = ref(false)
const toolSheetOpen = ref(false)
const shellBarExpanded = ref(false)
const shellActiveCount = ref(0)
const quoteChips = ref<{ messageId: string; snippet: string }[]>([])
const compressBadgeInfo = ref<null>(null)
const compressSavedInfo = ref<null>(null)
const compressionSheetOpen = ref(false)

/** 当前激活模型信息：空态下直连后端，保证 composer 模型 pill 可显示/切换 */
const activeModelInfo = ref<ActiveModelInfo | null>(null)
async function loadActiveModelInfo() {
  try {
    activeModelInfo.value = await invoke<ActiveModelInfo>('get_active_model_info')
  } catch {
    activeModelInfo.value = null
  }
}
onMounted(() => void loadActiveModelInfo())

function toggleShellBar() {
  shellBarExpanded.value = !shellBarExpanded.value
}

/** 发送：写入待发送提示词 → 新建聊天页签（ChatWindow 挂载后自动消费发送） */
async function send() {
  const text = input.value.trim()
  if (!text) return
  setPendingPrompt(text)
  openTab({
    id: NEW_CHAT_TAB_ID,
    kind: 'chat',
    title: '新对话',
    closable: true,
    instanceKey: '',
  })
}

async function stopGenerating() {}

provide(CHAT_STORE_KEY, {
  core: {
    input,
    sending,
    queuedCount,
    workingDir,
    thinking,
    reasoningEffort,
    workingDirSheetOpen,
    toolSheetOpen,
    shellBarExpanded,
    shellActiveCount,
    toggleShellBar,
    activeModelInfo,
    loadActiveModelInfo,
    toast,
  },
  menu: { quoteChips, scrollToMessage: () => {}, removeQuote: () => {} },
  compression: { compressBadgeInfo, compressSavedInfo, compressionSheetOpen },
  send: { send, stopGenerating },
} as unknown as ChatStore)
</script>

<template>
  <div class="tab-empty">
    <div class="tab-empty-inner">
      <div class="tab-empty-badge">
        <Icon name="chat" :size="44" />
      </div>
      <h2 class="tab-empty-title">开始新的对话</h2>
      <p class="tab-empty-desc">输入问题直接开启新对话，或从左侧历史记录选择会话</p>
    </div>

    <!-- 标准版完整输入栏：+ 附件 / 思考设置 / 工作区 / 命令会话 / 发送 -->
    <ChatComposer />
  </div>
</template>

<style scoped>
.tab-empty {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  padding: var(--space-8);
  overflow-y: auto; /* 极矮容器下允许滚动，不裁切 */
  text-align: center;
  background: var(--bg);
}

/* margin:auto 安全居中：容器够高时垂直水平居中，内容超高时回退为顶部对齐 + 滚动；
   inner 吸收自由空间后，其后的 ChatComposer 自然落到底部（贴底输入栏） */
.tab-empty-inner {
  margin: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
  max-width: 560px;
}

.tab-empty-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 88px;
  height: 88px;
  border-radius: var(--radius-full);
  /* 主题感知的 primary 淡彩底：暗色下蓝灰、亮色下浅灰，避免硬编码 rgba 与主题脱节 */
  background: linear-gradient(
    135deg,
    color-mix(in srgb, var(--primary) 16%, var(--card)),
    color-mix(in srgb, var(--primary) 6%, var(--card))
  );
  color: var(--primary);
  margin-bottom: var(--space-5);
  box-shadow: var(--shadow-sm);
}

.tab-empty-title {
  margin: 0 0 var(--space-2);
  font-size: var(--fs-xl);
  font-weight: 600;
  color: var(--text);
  letter-spacing: 0.3px;
}

.tab-empty-desc {
  margin: 0 0 var(--space-6);
  max-width: 360px;
  font-size: var(--fs-base);
  line-height: 1.6;
  color: var(--muted);
}
</style>
