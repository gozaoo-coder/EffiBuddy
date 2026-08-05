<script setup lang="ts">
/**
 * SubAgentMiniCard —— 主 agent 视图中的紧凑子代理卡片
 *
 * 子 agent 的完整过程不再内嵌在主会话气泡中（已改为独立子代理页签查看），
 * 主视图仅保留一张轻量卡片：名称 + 状态 + 任务标题，点击进入子代理聊天视图。
 */
import { inject } from 'vue'
import { Icon } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'
import type { SubAgentRecord } from '../../types'

const props = defineProps<{
  record: SubAgentRecord
}>()

const store = inject(CHAT_STORE_KEY)!

const statusText =
  props.record.status === 'done' ? '完成' : props.record.status === 'error' ? '出错' : '运行中'

function enter() {
  // 进入「主会话内嵌子代理视图」：chat-main 切换为该子代理全流程，顶栏显示
  // `[ 父标题 ] / [ 子代理标题 ]` 面包屑，点击父标题可返回。
  store.core.enterSubAgentView(props.record.session_id, props.record.name)
}

/** 任务标题摘要（单行截断） */
function taskTitle(): string {
  const t = props.record.task.trim()
  if (t) return t.length > 40 ? t.slice(0, 40) + '…' : t
  if (props.record.text.trim()) {
    const s = props.record.text.trim()
    return s.length > 40 ? s.slice(0, 40) + '…' : s
  }
  return '（尚无内容）'
}
</script>

<template>
  <button type="button" class="mini-card" :class="`st-${record.status}`" @click="enter">
    <span class="mini-avatar"><Icon name="robot" :size="15" /></span>
    <span class="mini-title">
      <span class="mini-name">{{ record.name }}</span>
      <span class="mini-task">{{ taskTitle() }}</span>
    </span>
    <span class="mini-status" :class="`st-${record.status}`">
      <span class="mini-status-dot" />{{ statusText }}
    </span>
    <span class="mini-enter"><Icon name="arrow-right" :size="13" /></span>
  </button>
</template>

<style scoped>
.mini-card {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  min-width: 0;
  padding: 9px 12px;
  text-align: left;
  background: var(--bg-subtle, rgba(128, 128, 128, 0.06));
  border: 1px solid var(--border-color, rgba(128, 128, 128, 0.28));
  border-radius: 10px;
  cursor: pointer;
  user-select: none;
  transition: border-color 0.15s, background 0.15s, transform 0.15s;
}

.mini-card:hover {
  border-color: var(--accent, #4f7cff);
  background: var(--bg-subtle, rgba(128, 128, 128, 0.1));
  transform: translateX(2px);
}

.mini-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 26px;
  height: 26px;
  border-radius: 8px;
  color: var(--accent, #4f7cff);
  background: color-mix(in srgb, var(--accent, #4f7cff) 12%, transparent);
}

.mini-title {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1;
}

.mini-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
}

.mini-task {
  font-size: 11px;
  color: var(--text-secondary, rgba(128, 128, 128, 0.85));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mini-status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex-shrink: 0;
  font-size: 11px;
}

.mini-status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
}

.st-running .mini-status-dot {
  background: #ffb020;
  animation: mini-pulse 1s ease-in-out infinite;
}
.st-done .mini-status-dot {
  background: #2ecc71;
}
.st-error .mini-status-dot {
  background: #ff4d4f;
}

@keyframes mini-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.3;
  }
}

.mini-enter {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--text-secondary, rgba(128, 128, 128, 0.7));
  transition: color 0.15s, transform 0.15s;
}

.mini-card:hover .mini-enter {
  color: var(--accent, #4f7cff);
  transform: translateX(2px);
}
</style>