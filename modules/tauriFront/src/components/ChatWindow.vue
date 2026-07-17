<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted, computed, watch, reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { animate } from 'animejs'
import MarkdownRender from 'markstream-vue'
import { useTheme } from '../composables/useTheme'
import { Button, IconButton, BindSheet, Chips, useToast } from './basic'
import ReasoningBox from './ReasoningBox.vue'
import ToolCallGroup from './ToolCallGroup.vue'
import type {
  Message,
  Conversation,
  StreamTokenPayload,
  StreamErrorPayload,
  AgentReasoningPayload,
  AgentToolCallPayload,
  AgentToolResultPayload,
  ToolCallRecord,
  PickedFile,
} from '../types'

// 后端名称（来自 App.vue 顶部模型药丸）+ 当前会话 id（由 App 传入）
const props = defineProps<{
  backend?: string
  conversationId?: string | null
}>()

const emit = defineEmits<{
  (e: 'update:conversation-id', id: string | null): void
  (e: 'conversation-changed'): void
}>()

// 主题：用于把 is-dark 传给 MarkdownRender，确保代码块/深色样式正确
const { resolvedTheme } = useTheme()
const isDark = computed(() => resolvedTheme.value === 'dark')
const { toast } = useToast()

// ---------- 状态 ----------
// activeId：当前正在交互的会话 id（流式事件匹配用）。
// 与 props.conversationId 同步，但在新建会话时可立即赋值，不等 App 回传。
const activeId = ref<string | null>(props.conversationId ?? null)
const messages = ref<Message[]>([])
const input = ref('')
const sending = ref(false)
const scroller = ref<HTMLElement | null>(null)
const streamingBubbleId = ref<string | null>(null) // 当前正在流式填充的气泡 id

// 每个助手气泡的元数据：reasoning / tool calls（流式期间累积，不持久化）
interface BubbleMeta {
  reasoning: string
  isThinking: boolean
  toolCalls: ToolCallRecord[]
}
const bubbleMeta = reactive<Record<string, BubbleMeta>>({})

// 获取某条消息的元数据（若不存在返回 null）
function getMeta(id: string): BubbleMeta | null {
  return bubbleMeta[id] ?? null
}

// 确保某 bubble 的 meta 存在
function ensureMeta(id: string): BubbleMeta {
  if (!bubbleMeta[id]) {
    bubbleMeta[id] = {
      reasoning: '',
      isThinking: false,
      toolCalls: [],
    }
  }
  return bubbleMeta[id]
}

// 底部工具/附件 Sheet
const toolSheetOpen = ref(false)

let unlistens: UnlistenFn[] = []

// ---------- 工具函数 ----------
function newId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

function scrollBottom() {
  const el = scroller.value
  if (el) el.scrollTop = el.scrollHeight
}

// 是否显示空状态首页
const isEmptyHome = computed(() => messages.value.length === 0 && !sending.value)

// ---------- 会话加载 ----------
// conversationId 变化时加载对应会话消息
watch(
  () => props.conversationId,
  (id) => {
    activeId.value = id ?? null
    loadConversation()
  },
)

async function loadConversation() {
  const id = activeId.value
  if (!id) {
    messages.value = []
    // 清空 meta
    Object.keys(bubbleMeta).forEach((k) => delete bubbleMeta[k])
    return
  }
  try {
    const conv = await invoke<Conversation | null>('get_conversation', { id })
    messages.value = conv?.messages ?? []
    // 历史会话不携带 reasoning/tools 元数据，清空
    Object.keys(bubbleMeta).forEach((k) => delete bubbleMeta[k])
    await nextTick()
    scrollBottom()
  } catch (e) {
    console.warn('get_conversation failed', e)
    messages.value = []
    Object.keys(bubbleMeta).forEach((k) => delete bubbleMeta[k])
  }
}

// ---------- 消息渲染 ----------
async function addMessage(msg: Message) {
  messages.value.push(msg)
  await nextTick()
  animate('.msg-bubble:last-child', {
    opacity: [0, 1],
    translateY: [10, 0],
    duration: 400,
    easing: 'easeOutQuad',
  })
  scrollBottom()
}

async function appendStreamToken(token: string) {
  if (!streamingBubbleId.value) {
    streamingBubbleId.value = newId()
    await addMessage({
      id: streamingBubbleId.value,
      role: 'assistant',
      content: token,
      timestamp: Date.now(),
    })
  } else {
    const target = messages.value.find((m) => m.id === streamingBubbleId.value)
    if (target) {
      target.content += token
    }
    await nextTick()
    scrollBottom()
  }
  // 收到文本 token 表示推理阶段已结束
  if (streamingBubbleId.value) {
    const meta = bubbleMeta[streamingBubbleId.value]
    if (meta && meta.isThinking) meta.isThinking = false
  }
}

// ---------- 推理事件 ----------
async function onReasoning(content: string) {
  if (!streamingBubbleId.value) {
    // 没有气泡时先创建一个空的 assistant 气泡
    streamingBubbleId.value = newId()
    await addMessage({
      id: streamingBubbleId.value,
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
    })
  }
  const meta = ensureMeta(streamingBubbleId.value)
  meta.isThinking = true
  meta.reasoning += content
  await nextTick()
  scrollBottom()
}

// ---------- 工具调用事件 ----------
async function onToolCall(call: AgentToolCallPayload) {
  if (!streamingBubbleId.value) {
    streamingBubbleId.value = newId()
    await addMessage({
      id: streamingBubbleId.value,
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
    })
  }
  const meta = ensureMeta(streamingBubbleId.value)
  // 收到 tool call 表示推理阶段结束
  meta.isThinking = false
  meta.toolCalls.push({
    call_id: call.call_id,
    tool_name: call.tool_name,
    arguments: call.arguments,
    result: null,
    is_error: false,
    pending: true,
  })
  await nextTick()
  scrollBottom()
}

// ---------- 工具结果事件 ----------
async function onToolResult(result: AgentToolResultPayload) {
  if (!streamingBubbleId.value) return
  const meta = bubbleMeta[streamingBubbleId.value]
  if (!meta) return
  const target = meta.toolCalls.find((c) => c.call_id === result.call_id)
  if (target) {
    target.result = result.output
    target.is_error = result.is_error
    target.pending = false
  }
  await nextTick()
  scrollBottom()
}

async function finalizeStream(full: string) {
  if (!streamingBubbleId.value) {
    if (full) {
      await addMessage({
        id: newId(),
        role: 'assistant',
        content: full,
        timestamp: Date.now(),
      })
    }
  } else {
    const target = messages.value.find((m) => m.id === streamingBubbleId.value)
    if (target && full) target.content = full
  }
  streamingBubbleId.value = null
  // 流式结束后通知 App 刷新 SideNav 列表（消息数/时间更新）
  emit('conversation-changed')
}

// ---------- 发送（流式） ----------
async function send() {
  const content = input.value.trim()
  if (!content || sending.value) return

  // 没有当前会话时新建一个
  let id = activeId.value
  if (!id) {
    try {
      id = await invoke<string>('create_conversation')
      activeId.value = id
      emit('update:conversation-id', id)
      emit('conversation-changed')
    } catch (e) {
      toast({ content: `新建会话失败：${e}`, type: 'error' })
      return
    }
  }

  sending.value = true
  input.value = ''

  await addMessage({
    id: newId(),
    role: 'user',
    content,
    timestamp: Date.now(),
  })

  try {
    await invoke('send_message_stream', {
      conversationId: id,
      content,
    })
  } catch (e) {
    sending.value = false
    await addMessage({
      id: newId(),
      role: 'system',
      content: `请求失败：${e}`,
      timestamp: Date.now(),
    })
    toast({ content: `请求失败：${e}`, type: 'error' })
  }
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault()
    send()
  }
}

// ---------- 快捷胶囊 ----------
const quickActions = [
  { label: 'PPT', icon: '🖥' },
  { label: '集群', icon: '🌐' },
  { label: '网站', icon: '🌐' },
  { label: '深度研究', icon: '🔬' },
]

function applyQuickAction(label: string) {
  input.value = `帮我做一个${label}相关的方案`
}

// ---------- 底部工具 Sheet ----------
const toolCategories = [
  { label: '拍照', icon: '📷' },
  { label: '照片', icon: '🖼' },
  { label: '本地文件', icon: '📁' },
  { label: '微信文件', icon: '💬' },
]

const pluginItems = [
  { label: '插件', desc: '接入 App 和数据库，帮你自动操作', icon: '🔌' },
  { label: '技能', desc: '复用专业能力，稳定处理特定任务', icon: '🛠' },
]

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

// 工具卡片点击：拍照/照片/本地文件接真实 command
async function onToolClick(label: string) {
  toolSheetOpen.value = false
  try {
    let file: PickedFile | null = null
    if (label === '拍照') {
      file = await invoke<PickedFile>('capture_photo')
    } else if (label === '照片') {
      file = await invoke<PickedFile>('pick_image')
    } else if (label === '本地文件') {
      file = await invoke<PickedFile>('pick_file')
    } else {
      toast({ content: `${label} 功能即将上线`, type: 'info' })
      return
    }
    if (file) {
      toast({
        content: `已选择：${file.name}（${formatFileSize(file.size)}）`,
        type: 'success',
      })
    }
  } catch (e) {
    toast({ content: `${label}失败：${e}`, type: 'error' })
  }
}

// ---------- 事件订阅 ----------
onMounted(async () => {
  // 初始加载当前会话（如果有）
  if (activeId.value) {
    await loadConversation()
  }

  unlistens.push(
    await listen<StreamTokenPayload>('agent-token', async (e) => {
      const p = e.payload
      if (p.done) return
      if (activeId.value && p.conversation_id !== activeId.value) return
      await appendStreamToken(p.content)
    }),
  )

  unlistens.push(
    await listen<AgentReasoningPayload>('agent-reasoning', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await onReasoning(p.content)
    }),
  )

  unlistens.push(
    await listen<AgentToolCallPayload>('agent-tool-call', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await onToolCall(p)
    }),
  )

  unlistens.push(
    await listen<AgentToolResultPayload>('agent-tool-result', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await onToolResult(p)
    }),
  )

  unlistens.push(
    await listen<StreamTokenPayload>('agent-done', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await finalizeStream(p.content)
      sending.value = false
    }),
  )

  unlistens.push(
    await listen<StreamErrorPayload>('agent-stream-error', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await addMessage({
        id: newId(),
        role: 'system',
        content: `流式错误：${p.error}`,
        timestamp: Date.now(),
      })
      toast({ content: `流式错误：${p.error}`, type: 'error' })
      streamingBubbleId.value = null
      sending.value = false
    }),
  )
})

onUnmounted(() => {
  unlistens.forEach((fn) => fn?.())
  unlistens = []
})
</script>

<template>
  <div class="chat-window">
    <!-- 聊天主区（侧栏已提升为 App 级 SideNav 抽屉） -->
    <section class="chat-main">
      <!-- 空状态首页：Kimi 风格中央品牌区 + 快捷胶囊 -->
      <div v-if="isEmptyHome" class="home-empty">
        <div class="home-brand">
          <div class="home-logo">
            <span class="home-logo-text">Effi</span>
            <span class="home-logo-icon">🤖</span>
            <span class="home-logo-text">Buddy</span>
          </div>
          <div class="home-subtitle">
            {{ props.backend && props.backend !== 'unknown' ? props.backend : 'Kimi K3: 2.8万亿参数' }}
          </div>
          <div class="home-subtitle secondary">为你进化</div>
        </div>

        <div class="home-actions">
          <Chips
            v-for="action in quickActions"
            :key="action.label"
            :label="action.label"
            :icon="action.icon"
            size="md"
            @click="applyQuickAction(action.label)"
          />
        </div>
      </div>

      <!-- 消息列表 -->
      <div v-else ref="scroller" class="msg-list">
        <div
          v-for="m in messages"
          :key="m.id"
          class="msg-bubble"
          :class="[`role-${m.role}`, { streaming: m.id === streamingBubbleId }]"
        >
          <template v-if="m.role === 'assistant'">
            <!-- 推理折叠框：仅在存在 reasoning 时渲染 -->
            <ReasoningBox
              v-if="getMeta(m.id)?.reasoning"
              :content="getMeta(m.id)!.reasoning"
              :is-thinking="getMeta(m.id)!.isThinking"
            />
            <!-- 工具调用提示组：仅在存在 tool calls 时渲染 -->
            <ToolCallGroup
              v-if="getMeta(m.id)?.toolCalls.length"
              :calls="getMeta(m.id)!.toolCalls"
            />
            <!-- 正文：仅在内容非空时渲染（思考/工具阶段内容可能为空） -->
            <MarkdownRender
              v-if="m.content"
              mode="chat"
              :content="m.content"
              :final="m.id !== streamingBubbleId"
              :is-dark="isDark"
              :fade="false"
              smooth-streaming="auto"
              :code-block-props="{
                theme: { light: 'vitesse-light', dark: 'vitesse-dark' },
              }"
            />
          </template>
          <template v-else>{{ m.content }}</template>
        </div>
      </div>

      <!-- Kimi 风格底部输入栏 -->
      <div class="composer-kimi">
        <div class="composer-inner">
          <IconButton
            icon="＋"
            size="md"
            container
            title="附件"
            @click="toolSheetOpen = true"
          />
          <textarea
            v-model="input"
            class="composer-input"
            :placeholder="sending ? '生成中…' : '尽管问，带图也行'"
            :disabled="sending"
            rows="1"
            @keydown="onKeydown"
          ></textarea>
          <Button
            v-if="!input.trim()"
            icon-only
            shape="circle"
            size="md"
            variant="normal"
            title="语音输入"
            @click="toast({ content: '语音输入即将上线', type: 'info' })"
          >
            <template #icon>🎙</template>
          </Button>
          <Button
            v-else
            icon-only
            shape="circle"
            size="md"
            variant="primary"
            :disabled="!input.trim()"
            title="发送"
            @click="send"
          >
            <template #icon>↑</template>
          </Button>
        </div>
        <div class="composer-footer">内容由 AI 生成</div>
      </div>
    </section>

    <!-- 底部工具/附件 Sheet -->
    <BindSheet v-model:visible="toolSheetOpen" title="工具" side="bottom" :height="'auto'">
      <div class="tool-sheet">
        <div class="tool-row">
          <div
            v-for="t in toolCategories"
            :key="t.label"
            class="tool-card"
            @click="onToolClick(t.label)"
          >
            <span class="tool-card-icon">{{ t.icon }}</span>
            <span class="tool-card-label">{{ t.label }}</span>
          </div>
        </div>

        <div class="tool-list">
          <div
            v-for="p in pluginItems"
            :key="p.label"
            class="tool-list-item"
            @click="onToolClick(p.label)"
          >
            <span class="tool-list-icon">{{ p.icon }}</span>
            <div class="tool-list-text">
              <div class="tool-list-title">{{ p.label }}</div>
              <div class="tool-list-desc">{{ p.desc }}</div>
            </div>
            <span class="tool-list-arrow">›</span>
          </div>
        </div>

        <div class="tool-list-item" @click="onToolClick('联网搜索')">
          <span class="tool-list-icon">🌐</span>
          <div class="tool-list-text">
            <div class="tool-list-title">联网搜索</div>
          </div>
          <span class="tool-list-status">自动</span>
          <span class="tool-list-arrow">›</span>
        </div>
      </div>
    </BindSheet>
  </div>
</template>

<style scoped>
/* ChatWindow 局部样式：补充基础组件未覆盖的部分 */
.composer-input {
  flex: 1;
  resize: none;
  min-height: 40px;
  max-height: 120px;
  padding: 10px 12px;
  font-family: inherit;
  font-size: 15px;
  color: var(--text);
  background: transparent;
  border: none;
  border-radius: var(--radius-full);
  outline: none;
  line-height: 1.4;
}

.composer-input::placeholder {
  color: var(--muted);
}

.composer-input:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

/* Kimi 风格空状态首页 */
.home-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 24px;
  gap: 48px;
  overflow-y: auto;
}

.home-brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.home-logo {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 42px;
  font-weight: 800;
  letter-spacing: 2px;
  color: var(--text);
}

.home-logo-icon {
  font-size: 0.8em;
}

.home-subtitle {
  font-size: 16px;
  color: var(--text);
  font-weight: 500;
}

.home-subtitle.secondary {
  font-size: 15px;
  color: var(--muted);
  font-weight: 400;
}

.home-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 10px;
}

/* Kimi 风格底部工具 Sheet */
.tool-sheet {
  padding: 8px 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.tool-row {
  display: flex;
  gap: 12px;
  overflow-x: auto;
  padding: 8px 0 16px;
}

.tool-card {
  flex: 0 0 auto;
  width: 88px;
  height: 88px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  background: var(--card-2);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
}

.tool-card:hover {
  background: var(--border);
  transform: translateY(-2px);
}

.tool-card-icon {
  font-size: 28px;
  line-height: 1;
}

.tool-card-label {
  font-size: 13px;
  color: var(--text);
}

.tool-list {
  display: flex;
  flex-direction: column;
}

.tool-list-item {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 0;
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
}

.tool-list-item:last-child {
  border-bottom: none;
}

.tool-list-item:hover {
  background: var(--card-2);
  margin: 0 -20px;
  padding-left: 20px;
  padding-right: 20px;
}

.tool-list-icon {
  font-size: 22px;
  line-height: 1;
  width: 28px;
  text-align: center;
}

.tool-list-text {
  flex: 1;
  min-width: 0;
}

.tool-list-title {
  font-size: 15px;
  font-weight: 500;
  color: var(--text);
}

.tool-list-desc {
  font-size: 13px;
  color: var(--muted);
  margin-top: 2px;
}

.tool-list-status {
  font-size: 13px;
  color: var(--muted);
}

.tool-list-arrow {
  font-size: 18px;
  color: var(--muted);
}
</style>
