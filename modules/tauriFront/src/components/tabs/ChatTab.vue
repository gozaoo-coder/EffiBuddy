<script setup lang="ts">
/**
 * ChatTab —— chat 页签视图容器
 *
 * 职责：包装现有 ChatWindow，透传全部 props/emit，不做任何内部逻辑改动。
 * ChatWindow 不直接出现在 App.vue，仅通过本组件渲染。
 *
 * 透传约定：
 *   props.backend            → ChatWindow.backend
 *   props.tab.conversationId → ChatWindow.conversation-id
 *   ChatWindow@update:conversation-id → emit('update:conversation-id')
 *   ChatWindow@conversation-changed     → emit('conversation-changed')
 *
 * 状态上报：当 ChatWindow 流式发送中/结束时，向上 emit update:status，
 * 供 TabBar 显示 loading 指示与"录音中不可关"等约束（chat 类型仅用于 loading 态）。
 */
import ChatWindow from '../ChatWindow.vue'
import type { TabItem } from '../../types'

defineOptions({ name: 'ChatTab' })

const props = defineProps<{
  tab: TabItem
  backend?: string
}>()

const emit = defineEmits<{
  (e: 'update:conversation-id', id: string | null): void
  (e: 'conversation-changed'): void
  (e: 'update:status', status: TabItem['status']): void
}>()

// 透传 conversation-id 变更（新建会话 / 切换会话）
function onConvIdUpdate(id: string | null) {
  emit('update:conversation-id', id)
}
function onConvChanged() {
  emit('conversation-changed')
}
</script>

<template>
  <div class="chat-tab">
    <ChatWindow
      :backend="props.backend"
      :conversation-id="props.tab.conversationId ?? null"
      @update:conversation-id="onConvIdUpdate"
      @conversation-changed="onConvChanged"
    />
  </div>
</template>

<style scoped>
.chat-tab {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.chat-tab :deep(.chat-window) {
  flex: 1;
  min-height: 0;
}
</style>
