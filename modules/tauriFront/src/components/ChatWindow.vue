<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { animate } from 'animejs'
import MarkdownRender from 'markstream-vue'
import { useTheme } from '../composables/useTheme'
import { Button, IconButton, Dialog, useToast } from './basic'
import type {
  Message,
  Conversation,
  ConversationMeta,
  StreamTokenPayload,
  StreamErrorPayload,
} from '../types'

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

// 当前会话 meta（用于顶部标题）
const currentMeta = computed<ConversationMeta | null>(() => {
  if (!currentId.value) return null
  return conversations.value.find((c) => c.id === currentId.value) ?? null
})

// ---------- 会话管理 ----------
async function loadConversations() {
  try {
    conversations.value = await invoke<ConversationMeta[]>('list_conversations')
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

// 用 Dialog 替代 confirm()：先打开对话框，确认后真正删除
function askDeleteCurrent() {
  if (!currentId.value || sending.value) return
  pendingDeleteId.value = currentId.value
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

// 流式：追加 token 到当前 streaming 气泡；不存在则新建
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
    // 未收到任何 token，但收到了 done 事件（例如纯工具调用）
    if (full) {
      await addMessage({
        id: newId(),
        role: 'assistant',
        content: full,
        timestamp: Date.now(),
      })
    }
  } else {
    // 校正内容（避免增量漏字），并刷新 meta
    const target = messages.value.find((m) => m.id === streamingBubbleId.value)
    if (target && full) target.content = full
  }
  streamingBubbleId.value = null
  // 刷新会话列表的 message_count
  await loadConversations()
}

// ---------- 发送（流式） ----------
async function send() {
  const content = input.value.trim()
  if (!content || sending.value) return
  if (!currentId.value) {
    // 自动新建会话
    await newConversation()
    if (!currentId.value) return
  }

  sending.value = true
  input.value = ''

  // 立即显示用户气泡
  await addMessage({
    id: newId(),
    role: 'user',
    content,
    timestamp: Date.now(),
  })

  try {
    // 触发后端流式 task，token 通过事件异步到达
    await invoke('send_message_stream', {
      conversationId: currentId.value,
      content,
    })
    // 不在此处 resetting sending —— 等流结束事件
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

// ---------- 事件订阅 ----------
onMounted(async () => {
  await loadConversations()
  // 自动选中第一个会话（若有）
  if (conversations.value.length > 0) {
    await selectConversation(conversations.value[0].id)
  } else {
    // 无会话时也自动新建一个，避免空状态
    await newConversation()
  }

  // 流式 token
  unlistens.push(
    await listen<StreamTokenPayload>('agent-token', async (e) => {
      const p = e.payload
      if (p.done) return // 由 agent-done 处理
      if (currentId.value && p.conversation_id !== currentId.value) return
      await appendStreamToken(p.content)
    }),
  )

  // 流结束
  unlistens.push(
    await listen<StreamTokenPayload>('agent-done', async (e) => {
      const p = e.payload
      if (currentId.value && p.conversation_id !== currentId.value) return
      await finalizeStream(p.content)
      sending.value = false
    }),
  )

  // 流错误
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
    <!-- 左侧：会话列表 -->
    <aside class="conv-sidebar">
      <div class="conv-head">
        <span class="conv-head-title">会话</span>
        <Button variant="primary" size="sm" :disabled="sending" @click="newConversation">
          <template #icon>＋</template>
          新建
        </Button>
      </div>
      <div class="conv-list">
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
        </div>
      </div>
    </aside>

    <!-- 右侧：聊天主区 -->
    <section class="chat-main">
      <div class="chat-title">
        <span v-if="currentMeta" class="chat-title-text">
          对话 · {{ currentMeta.id.slice(0, 8) }}… · {{ currentMeta.message_count }} 条
        </span>
        <span v-else class="chat-title-text muted">未选择会话</span>
        <IconButton
          v-if="currentId"
          icon="🗑"
          size="sm"
          container
          variant="danger"
          :disabled="sending"
          title="删除当前会话"
          @click="askDeleteCurrent"
        />
      </div>

      <div ref="scroller" class="msg-list">
        <div v-if="messages.length === 0" class="empty-hint">
          输入消息开始对话。流式响应会逐字显示。
        </div>
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

      <div class="composer">
        <textarea
          v-model="input"
          class="composer-input"
          :placeholder="sending ? '生成中…' : '输入消息，Enter 发送，Shift+Enter 换行'"
          :disabled="sending || !currentId"
          @keydown="onKeydown"
        ></textarea>
        <Button
          variant="primary"
          :loading="sending"
          :disabled="!input.trim() || !currentId"
          @click="send"
        >
          {{ sending ? '生成中' : '发送' }}
        </Button>
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
  min-height: 42px;
  max-height: 120px;
  padding: 10px 12px;
  font-family: inherit;
  font-size: 14px;
  color: var(--text);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.composer-input:focus {
  border-color: var(--primary);
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
</style>
