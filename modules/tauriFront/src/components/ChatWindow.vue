<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { animate, stagger } from 'animejs'
import MarkdownRender from 'markstream-vue'
import { useTheme } from '../composables/useTheme'
import { useLayout } from '../composables/useLayout'
import { Button, IconButton, Dialog, BindSheet, Chips, useToast } from './basic'
import type {
  Message,
  Conversation,
  ConversationMeta,
  StreamTokenPayload,
  StreamErrorPayload,
} from '../types'

// 后端名称（来自 App.vue 顶部模型药丸）
const props = defineProps<{
  backend?: string
}>()

// 主题：用于把 is-dark 传给 MarkdownRender，确保代码块/深色样式正确
const { resolvedTheme } = useTheme()
const isDark = computed(() => resolvedTheme.value === 'dark')
const { toast } = useToast()

// ---------- 状态 ----------
const conversations = ref<ConversationMeta[]>([])
const currentId = ref<string | null>(null)
const messages = ref<Message[]>([])
const input = ref('')
const sending = ref(false)
const scroller = ref<HTMLElement | null>(null)
const streamingBubbleId = ref<string | null>(null) // 当前正在流式填充的气泡 id

// 删除确认对话框
const deleteDialogVisible = ref(false)
const pendingDeleteId = ref<string | null>(null)

// 底部工具/附件 Sheet
const toolSheetOpen = ref(false)

// 会话列表容器 ref：用于 anime.js v4 Layout 动画
const convListEl = ref<HTMLElement | null>(null)

// anime.js v4 Layout 实例：会话列表项进出场 + 位置重排动画
const { update: updateConvLayout } = useLayout(convListEl, {
  children: '.conv-item',
  duration: 320,
  ease: 'out(3)',
  enterFrom: {
    opacity: 0,
    transform: 'translateX(-24px) scale(.92)',
    duration: 360,
    ease: 'out(3)',
  },
  leaveTo: {
    opacity: 0,
    transform: 'scale(.85)',
    duration: 240,
    ease: 'inOut(2)',
  },
  swapAt: { opacity: 1 },
})

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

function formatTime(ts: number): string {
  try {
    const d = new Date(ts)
    return d.toLocaleString()
  } catch {
    return String(ts)
  }
}

// 当前会话 meta
const currentMeta = computed<ConversationMeta | null>(() => {
  if (!currentId.value) return null
  return conversations.value.find((c) => c.id === currentId.value) ?? null
})

// 是否显示空状态首页
const isEmptyHome = computed(() => messages.value.length === 0 && !sending.value)

// ---------- 会话管理 ----------
async function loadConversations() {
  try {
    conversations.value = await invoke<ConversationMeta[]>('list_conversations')
    await nextTick()
    updateConvLayout(() => {
      /* Vue 已更新 DOM，layout 自动记录差分 */
    })
  } catch (e) {
    console.warn('list_conversations failed', e)
  }
}

async function selectConversation(id: string) {
  if (sending.value) return
  currentId.value = id
  try {
    const conv = await invoke<Conversation | null>('get_conversation', { id })
    messages.value = conv?.messages ?? []
    await nextTick()
    scrollBottom()
  } catch (e) {
    console.warn('get_conversation failed', e)
    messages.value = []
  }
}

async function newConversation() {
  if (sending.value) return
  try {
    const id = await invoke<string>('create_conversation')
    await loadConversations()
    await selectConversation(id)
  } catch (e) {
    toast({ content: `新建会话失败：${e}`, type: 'error' })
  }
}

function askDeleteCurrent() {
  if (!currentId.value || sending.value) return
  pendingDeleteId.value = currentId.value
  deleteDialogVisible.value = true
}

function askDeleteById(id: string) {
  if (sending.value) return
  pendingDeleteId.value = id
  deleteDialogVisible.value = true
}

async function confirmDelete() {
  const id = pendingDeleteId.value
  if (!id) return
  try {
    await invoke('delete_conversation', { id })
    if (currentId.value === id) {
      currentId.value = null
      messages.value = []
    }
    await loadConversations()
    toast({ content: '会话已删除', type: 'success' })
  } catch (e) {
    toast({ content: `删除会话失败：${e}`, type: 'error' })
  } finally {
    pendingDeleteId.value = null
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
  await loadConversations()
}

// ---------- 发送（流式） ----------
async function send() {
  const content = input.value.trim()
  if (!content || sending.value) return
  if (!currentId.value) {
    await newConversation()
    if (!currentId.value) return
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
      conversationId: currentId.value,
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

function onToolClick(label: string) {
  toolSheetOpen.value = false
  toast({ content: `${label} 功能即将上线`, type: 'info' })
}

// ---------- 事件订阅 ----------
onMounted(async () => {
  await loadConversations()
  if (conversations.value.length > 0) {
    await selectConversation(conversations.value[0].id)
  } else {
    await newConversation()
  }

  unlistens.push(
    await listen<StreamTokenPayload>('agent-token', async (e) => {
      const p = e.payload
      if (p.done) return
      if (currentId.value && p.conversation_id !== currentId.value) return
      await appendStreamToken(p.content)
    }),
  )

  unlistens.push(
    await listen<StreamTokenPayload>('agent-done', async (e) => {
      const p = e.payload
      if (currentId.value && p.conversation_id !== currentId.value) return
      await finalizeStream(p.content)
      sending.value = false
    }),
  )

  unlistens.push(
    await listen<StreamErrorPayload>('agent-stream-error', async (e) => {
      const p = e.payload
      if (currentId.value && p.conversation_id !== currentId.value) return
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
    <!-- 左侧：会话列表（桌面端常驻，窄屏折叠） -->
    <aside class="conv-sidebar">
      <div class="conv-head">
        <span class="conv-head-title">会话</span>
        <Button variant="primary" size="sm" :disabled="sending" @click="newConversation">
          <template #icon>＋</template>
          新建
        </Button>
      </div>
      <div ref="convListEl" class="conv-list">
        <div v-if="conversations.length === 0" class="empty-hint">
          暂无会话，点击「新建」开始
        </div>
        <div
          v-for="c in conversations"
          :key="c.id"
          class="conv-item"
          :class="{ active: c.id === currentId }"
          @click="selectConversation(c.id)"
        >
          <div class="conv-item-title">{{ c.id.slice(0, 8) }}…</div>
          <div class="conv-item-meta">{{ c.message_count }} 条 · {{ formatTime(c.created_at) }}</div>
          <IconButton
            class="conv-delete"
            icon="🗑"
            size="sm"
            variant="danger"
            title="删除会话"
            @click.stop="askDeleteById(c.id)"
          />
        </div>
      </div>
    </aside>

    <!-- 右侧：聊天主区 -->
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
            <MarkdownRender
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
            :disabled="sending || !currentId"
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
            :disabled="!input.trim() || !currentId"
            title="发送"
            @click="send"
          >
            <template #icon>↑</template>
          </Button>
        </div>
        <div class="composer-footer">内容由 AI 生成</div>
      </div>
    </section>

    <!-- 删除会话确认对话框 -->
    <Dialog
      v-model:visible="deleteDialogVisible"
      title="删除会话"
      danger
      confirm-text="删除"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmDelete"
    >
      <div class="dialog-delete-content">
        确定删除该会话？此操作不可撤销。
        <div v-if="pendingDeleteId" class="dialog-delete-id">
          ID：{{ pendingDeleteId.slice(0, 8) }}…
        </div>
      </div>
    </Dialog>

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
.conv-head-title {
  font-size: 13px;
  color: var(--muted);
  font-weight: 500;
}

.chat-title-text {
  font-size: 13px;
  color: var(--muted);
}

.chat-title-text.muted {
  color: var(--muted);
  opacity: 0.7;
}

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

.dialog-delete-content {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}

.dialog-delete-id {
  margin-top: 8px;
  font-size: 12px;
  color: var(--muted);
  font-family: 'SFMono-Regular', Consolas, monospace;
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
