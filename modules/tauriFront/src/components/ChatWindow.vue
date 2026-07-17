<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { animate } from 'animejs'
import type {
  Message,
  Conversation,
  ConversationMeta,
  StreamTokenPayload,
  StreamErrorPayload,
} from '../types'

// ---------- 状态 ----------
const conversations = ref<ConversationMeta[]>([])
const currentId = ref<string | null>(null)
const messages = ref<Message[]>([])
const input = ref('')
const sending = ref(false)
const scroller = ref<HTMLElement | null>(null)
const streamingBubbleId = ref<string | null>(null) // 当前正在流式填充的气泡 id

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
    console.warn('create_conversation failed', e)
  }
}

async function deleteCurrent() {
  if (!currentId.value || sending.value) return
  const id = currentId.value
  if (!confirm('确定删除该会话？此操作不可撤销。')) return
  try {
    await invoke('delete_conversation', { id })
    if (currentId.value === id) {
      currentId.value = null
      messages.value = []
    }
    await loadConversations()
  } catch (e) {
    console.warn('delete_conversation failed', e)
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
        <span>会话</span>
        <button class="new-btn" :disabled="sending" @click="newConversation">+ 新建</button>
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
        <span v-if="currentMeta">对话 · {{ currentMeta.id.slice(0, 8) }}… · {{ currentMeta.message_count }} 条</span>
        <span v-else>未选择会话</span>
        <button
          v-if="currentId"
          class="del-btn"
          :disabled="sending"
          @click="deleteCurrent"
        >删除</button>
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
        >{{ m.content }}<span v-if="m.id === streamingBubbleId" class="cursor">▍</span></div>
      </div>

      <div class="composer">
        <textarea
          v-model="input"
          :placeholder="sending ? '生成中…' : '输入消息，Enter 发送，Shift+Enter 换行'"
          :disabled="sending || !currentId"
          @keydown="onKeydown"
        ></textarea>
        <button :disabled="sending || !input.trim() || !currentId" @click="send">
          {{ sending ? '生成中' : '发送' }}
        </button>
      </div>
    </section>
  </div>
</template>
