<script setup lang="ts">
/**
 * CompressionActionCard —— 单条压缩决策卡片
 *
 * 三种数据源共用(streaming 实时解析 / done 最终决策 / existing 历史结果):
 *  - streaming 阶段不可展开(interactive=false),仅展示 method/reason/新内容
 *  - done / existing 阶段可点击展开,查看被压缩消息原文
 * 展开态由外部(expandedActions Set)驱动,通过 props 传入。
 */
import { inject } from 'vue'
import { Icon } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'
import type { CompressionAction } from '../../types'

const props = defineProps<{
  action: CompressionAction
  /** 展开状态 key(如 `done-0` / `stream-0` / `existing-0`),用于区分数据源 */
  expandKey: string
  expanded: boolean
  /** 头部是否可点击展开(streaming 阶段为 false) */
  interactive: boolean
}>()

const emit = defineEmits<{
  (e: 'toggle', key: string): void
}>()

const store = inject(CHAT_STORE_KEY)!
const { findMessagesByIds } = store.compression

const methodLabel = (m: CompressionAction['method']) =>
  m === 'keep' ? '保持' : m === 'hide' ? '隐藏' : '替换'
</script>

<template>
  <div class="compress-action-item" :class="`method-${action.method}`">
    <div
      class="compress-action-head"
      :class="{ 'is-button': interactive }"
      :role="interactive ? 'button' : undefined"
      :tabindex="interactive ? 0 : undefined"
      @click="interactive && emit('toggle', expandKey)"
      @keydown.enter.prevent="interactive && emit('toggle', expandKey)"
    >
      <span class="compress-action-method">{{ methodLabel(action.method) }}</span>
      <span class="compress-action-ids">{{ action.message_ids.length }} 条消息</span>
      <span
        v-if="interactive"
        class="compress-action-toggle"
        :class="{ 'is-open': expanded }"
      >
        <Icon name="chevron-down" :size="14" />
      </span>
    </div>
    <div class="compress-action-reason">{{ action.reason }}</div>
    <div v-if="action.method === 'replace' && action.new_content" class="compress-action-new">
      <span class="compress-action-new-label">替换为：</span>
      <div class="compress-action-new-content">{{ action.new_content }}</div>
    </div>
    <!-- 展开内容:列出被压缩的原消息 -->
    <div v-if="interactive && expanded" class="compress-action-expand">
      <div
        v-for="msg in findMessagesByIds(action.message_ids)"
        :key="msg.id"
        class="compress-action-msg"
      >
        <div class="compress-action-msg-head">
          <span class="compress-action-msg-role">{{ msg.role }}</span>
          <span class="compress-action-msg-id">[id:{{ msg.id.slice(0, 8) }}]</span>
        </div>
        <div class="compress-action-msg-content">{{ msg.content }}</div>
      </div>
      <div v-if="findMessagesByIds(action.message_ids).length === 0" class="compress-action-empty">
        消息不在当前会话中（可能已被删除）
      </div>
    </div>
  </div>
</template>

<style scoped>
.compress-action-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 12px;
  background: var(--card);
  border: 1px solid var(--border);
  border-left-width: 3px;
  border-radius: var(--radius-lg);
}

.compress-action-item.method-keep { border-left-color: var(--success, #10a37f); }
.compress-action-item.method-hide { border-left-color: var(--muted); }
.compress-action-item.method-replace { border-left-color: var(--warn, #d97757); }

/* 头部:method / ids / 展开箭头 */
.compress-action-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 2px 0;
}

/* 可点击的头部:cursor + hover 反馈 */
.compress-action-head.is-button {
  cursor: pointer;
  border-radius: var(--radius-sm);
}

.compress-action-head.is-button:hover {
  background: var(--bg-2);
}

.compress-action-head.is-button:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--primary) 60%, transparent);
  outline-offset: 1px;
}

/* 展开切换图标:默认朝下,展开后旋转 180° */
.compress-action-toggle {
  flex: 0 0 auto;
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  color: var(--muted);
  transition: transform 0.2s ease;
}

.compress-action-toggle.is-open {
  transform: rotate(180deg);
}

.compress-action-method {
  font-size: 12px;
  font-weight: 600;
}

.compress-action-item.method-keep .compress-action-method { color: var(--success, #10a37f); }
.compress-action-item.method-hide .compress-action-method { color: var(--muted); }
.compress-action-item.method-replace .compress-action-method { color: var(--warn, #d97757); }

.compress-action-ids {
  font-size: 12px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}

.compress-action-reason {
  font-size: 12px;
  color: var(--muted);
  line-height: 1.5;
}

.compress-action-new {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 12px;
}

.compress-action-new-label {
  color: var(--muted);
}

.compress-action-new-content {
  color: var(--text);
  background: var(--bg-2);
  border: 1px dashed var(--border);
  border-radius: var(--radius-sm);
  padding: 6px 8px;
  white-space: pre-wrap;
  word-break: break-word;
}

/* 展开内容:被压缩消息原文列表 */
.compress-action-expand {
  margin-top: 4px;
  padding: 8px 10px;
  background: var(--card);
  border-radius: var(--radius-md);
  border: 1px dashed var(--border);
  display: flex;
  flex-direction: column;
  gap: 8px;
  animation: compress-expand-fade 0.2s ease;
}

@keyframes compress-expand-fade {
  from { opacity: 0; max-height: 0; }
  to { opacity: 1; max-height: 400px; }
}

.compress-action-msg {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px 0;
}

.compress-action-msg:last-child {
  padding-bottom: 0;
}

.compress-action-msg-head {
  display: flex;
  align-items: center;
  gap: 6px;
}

.compress-action-msg-role {
  font-size: 11px;
  font-weight: 600;
  color: var(--muted);
}

.compress-action-msg-id {
  font-size: 11px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}

.compress-action-msg-content {
  font-size: 12px;
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-word;
  line-height: 1.5;
}

.compress-action-empty {
  font-size: 12px;
  color: var(--muted);
  padding: 4px 0;
}
</style>
