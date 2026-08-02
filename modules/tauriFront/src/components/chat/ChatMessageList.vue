<script setup lang="ts">
/**
 * ChatMessageList —— 消息列表（消息 + 任务气泡交错渲染）
 *
 * 持有 scroller 元素（自动滚动绑定），遍历 renderList：
 * - kind=message → MessageBubble（普通消息气泡）
 * - kind=task    → TaskBubble（长程任务气泡，内含本组 assistant 输出）
 *
 * 任务气泡在消息流中按其起始消息位置就地插入，早期/中间非任务消息
 * 始终保持为普通气泡，不会被吸纳进任务组件。
 */
import { inject, watch, onUnmounted } from 'vue'
import MessageBubble from './MessageBubble.vue'
import TaskBubble from './TaskBubble.vue'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const { scroller, attachScroller } = store.autoscroll
const { renderList } = store.taskMode
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
    <template v-for="item in renderList" :key="item.kind === 'message' ? item.message.id : item.group.id">
      <MessageBubble
        v-if="item.kind === 'message'"
        :message="item.message"
        :meta="getMeta(item.message.id)"
        :is-streaming="item.message.id === streamingBubbleId"
        :is-dark="isDark"
      />
      <TaskBubble
        v-else
        :group="item.group"
      />
    </template>
  </div>
</template>
