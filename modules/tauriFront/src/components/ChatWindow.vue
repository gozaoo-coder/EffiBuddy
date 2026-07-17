<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { animate } from 'animejs'
import type { Message, AgentMessagePayload } from '../types'

const messages = ref<Message[]>([])
const input = ref('')
const sending = ref(false)
const scroller = ref<HTMLElement | null>(null)

let unlisten: UnlistenFn | null = null

function newId(): string {
  // 浏览器环境统一使用 crypto.randomUUID；不可用时回退到时间戳。
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

function scrollBottom() {
  const el = scroller.value
  if (el) el.scrollTop = el.scrollHeight
}

// 追加一条消息并以 anime.js v4 做淡入+上移动画。
// DOM 结构保证 .msg-bubble 共享同一父容器，故 :last-child 恰好命中最新气泡。
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

async function send() {
  const content = input.value.trim()
  if (!content || sending.value) return
  sending.value = true

  await addMessage({
    id: newId(),
    role: 'user',
    content,
    timestamp: Date.now(),
  })
  input.value = ''

  try {
    const reply = await invoke<string>('send_message', { content })
    await addMessage({
      id: newId(),
      role: 'assistant',
      content: reply,
      timestamp: Date.now(),
    })
  } catch (e) {
    await addMessage({
      id: newId(),
      role: 'system',
      content: `请求失败：${e}`,
      timestamp: Date.now(),
    })
  } finally {
    sending.value = false
  }
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault()
    send()
  }
}

onMounted(async () => {
  // 启动欢迎语。
  try {
    const greeting = await invoke<string>('greet', { name: 'EffiSuite' })
    await addMessage({
      id: newId(),
      role: 'assistant',
      content: greeting,
      timestamp: Date.now(),
    })
  } catch {
    // 静默忽略：UI 仍可用
  }

  // 监听后端转发的 agent-message 事件（流式场景）。
  // MockAgent 不会触发该事件，这里为真实流式 agent 预留。
  unlisten = await listen<AgentMessagePayload>('agent-message', async (e) => {
    if (e.payload.done) {
      await addMessage({
        id: newId(),
        role: 'assistant',
        content: e.payload.content,
        timestamp: Date.now(),
      })
    }
  })
})

onUnmounted(() => {
  unlisten?.()
})
</script>

<template>
  <div class="chat-window">
    <div class="chat-title">对话 · EffiSuite Agent</div>
    <div ref="scroller" class="msg-list">
      <div
        v-for="m in messages"
        :key="m.id"
        class="msg-bubble"
        :class="`role-${m.role}`"
      >{{ m.content }}</div>
    </div>
    <div class="composer">
      <textarea
        v-model="input"
        placeholder="输入消息，Enter 发送，Shift+Enter 换行"
        @keydown="onKeydown"
      ></textarea>
      <button :disabled="sending || !input.trim()" @click="send">
        {{ sending ? '发送中' : '发送' }}
      </button>
    </div>
  </div>
</template>
