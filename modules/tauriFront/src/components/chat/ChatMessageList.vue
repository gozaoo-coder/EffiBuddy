<script setup lang="ts">
/**
 * ChatMessageList —— 消息列表（全部消息以普通气泡渲染）
 *
 * 持有 scroller 元素（自动滚动绑定），遍历 core.messages：
 * - 所有消息 → MessageBubble（普通消息气泡）
 *
 * 长程任务（todo_write）产生的 assistant 消息同样以普通气泡展示。
 */
import { inject, watch, onUnmounted } from 'vue'
import MessageBubble from './MessageBubble.vue'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const { scroller, attachScroller } = store.autoscroll
const { messages } = store.core
const { streamingBubbleId } = store.streaming
const { getMeta } = store.streaming
const { isDark } = store.core

// scroller 是 v-if/v-else 渲染的元素（空状态首页时不挂载），
// watch 中绑定 MutationObserver，确保流式期间 DOM 增长能可靠触发滚动。
watch(scroller, (el, oldEl) => {
  attachScroller(el, oldEl ?? null)
})

onUnmounted(() => {
  attachScroller(null)
})
</script>

<template>
  <div ref="scroller" class="msg-list">
    <MessageBubble
      v-for="m in messages"
      :key="m.id"
      :message="m"
      :meta="getMeta(m.id)"
      :is-streaming="m.id === streamingBubbleId"
      :is-dark="isDark"
    />
  </div>
</template>
