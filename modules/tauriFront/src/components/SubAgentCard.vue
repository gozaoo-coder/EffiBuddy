<script setup lang="ts">
/**
 * SubAgentCard 子 agent 过程卡片
 *
 * 展示主 agent 召唤的子 agent 全流程：
 * - 头部：名称 + 模型 + 嵌套深度 + 状态徽标（运行中/完成/出错）+ 折叠
 * - 任务：主 agent 交给子 agent 的任务原文
 * - 正文：子 agent 流式回复（实时）、内部工具调用列表、生成的图片
 * - 结果：完成后高亮展示最终回复，可一键复制
 *
 * 设计要点：
 * - 运行中自动展开并跟随内容；完成后可折叠为单行摘要
 * - 深色/浅色主题自适应（CSS 变量）
 * - 图片经 read_attachment 命令转 data URL 渲染
 */
import { ref, watch, nextTick, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { animate } from 'animejs'
import { Icon } from './basic'
import type { SubAgentRecord } from '../types'

const props = defineProps<{
  /** 子 agent 记录（ChatWindow 按 session_id 聚合） */
  record: SubAgentRecord
}>()

// 折叠状态：运行中强制展开
const collapsed = ref(false)
watch(
  () => props.record.status,
  (s) => {
    if (s === 'running') collapsed.value = false
  },
)

const bodyRef = ref<HTMLElement | null>(null)
let animating = false

function toggle() {
  if (props.record.status === 'running' && collapsed.value) {
    collapsed.value = false
    return
  }
  if (collapsed.value) {
    collapsed.value = false
    nextTick(() => {
      const el = bodyRef.value
      if (!el) return
      const h = el.scrollHeight
      animate(el, {
        maxHeight: ['0px', `${h}px`],
        opacity: [0, 1],
        duration: 260,
        ease: 'out(3)',
        onComplete: () => {
          el.style.maxHeight = ''
          animating = false
        },
      })
    })
  } else {
    const el = bodyRef.value
    if (!el) return
    animate(el, {
      maxHeight: [`${el.scrollHeight}px`, '0px'],
      opacity: [1, 0],
      duration: 200,
      ease: 'inOut(2)',
      onComplete: () => {
        collapsed.value = true
        el.style.maxHeight = ''
        animating = false
      },
    })
  }
}

// 状态徽标文案与样式
const statusText = ref<'运行中' | '完成' | '出错'>('运行中')
const statusClass = ref<'running' | 'done' | 'error'>('running')
watch(
  () => props.record.status,
  (s) => {
    statusText.value = s === 'done' ? '完成' : s === 'error' ? '出错' : '运行中'
    statusClass.value = s
  },
  { immediate: true },
)

// 附件图片 data URL 缓存
const imageUrls = ref<Record<string, string>>({})
watch(
  () => props.record.images.length,
  async () => {
    for (const img of props.record.images) {
      if (imageUrls.value[img.path]) continue
      try {
        const url = await invoke<string>('read_attachment', { path: img.path })
        imageUrls.value[img.path] = url
      } catch {
        /* 图片读取失败不阻塞卡片 */
      }
    }
  },
  { immediate: true },
)

// 复制结果
const copied = ref(false)
async function copyResult() {
  try {
    await navigator.clipboard.writeText(props.record.text)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)
  } catch {
    /* clipboard 不可用时静默 */
  }
}

// 折叠摘要：最终回复前 48 字符
function summary() {
  const t = props.record.text.trim()
  if (t) return t.length > 48 ? t.slice(0, 48) + '…' : t
  return props.record.error || '执行中…'
}

// 工具参数摘要（截断单行）
function argSummary(args: string): string {
  if (!args) return ''
  const s = args.replace(/\s+/g, ' ').trim()
  return s.length > 60 ? s.slice(0, 60) + '…' : s
}
</script>

<template>
  <div class="sub-agent-card" :class="`st-${record.status}`">
    <!-- 头部：名称 / 模型 / 深度 / 状态 / 折叠 -->
    <div class="sa-header" @click="toggle">
      <span class="sa-avatar"><Icon name="robot" :size="15" /></span>
      <span class="sa-name">{{ record.name }}</span>
      <span class="sa-model">{{ record.model }}</span>
      <span v-if="record.depth > 1" class="sa-depth">深度 {{ record.depth }}</span>
      <span class="sa-status" :class="statusClass">
        <span class="sa-status-dot" />
        {{ statusText }}
      </span>
      <span class="sa-toggle"><Icon name="chevron-down" :size="14" /></span>
    </div>

    <!-- 折叠摘要（仅折叠时显示） -->
    <div v-if="collapsed" class="sa-summary" @click="toggle">
      <span class="sa-summary-icon"><Icon name="spark" :size="12" /></span>
      <span class="sa-summary-text">{{ summary() }}</span>
    </div>

    <!-- 展开主体 -->
    <div ref="bodyRef" class="sa-body" :class="{ hidden: collapsed }">
      <!-- 任务 -->
      <div class="sa-task">
        <span class="sa-label">任务</span>
        <span class="sa-task-text">{{ record.task }}</span>
      </div>

      <!-- 子 agent 内部工具调用 -->
      <div v-if="record.toolCalls.length" class="sa-tools">
        <div
          v-for="(tc, i) in record.toolCalls"
          :key="tc.call_id || i"
          class="sa-tool-row"
          :class="{ err: tc.is_error }"
        >
          <span class="sa-tool-name"><Icon name="tool" :size="12" /> {{ tc.tool_name }}</span>
          <span class="sa-tool-args">{{ argSummary(tc.arguments) }}</span>
          <span class="sa-tool-state">
            <span v-if="tc.pending" class="sa-spin" />
            <Icon v-else-if="tc.is_error" name="close" :size="12" />
            <Icon v-else name="check" :size="12" />
          </span>
        </div>
      </div>

      <!-- 子 agent 生成的图片 -->
      <div v-if="record.images.length" class="sa-images">
        <img
          v-for="img in record.images"
          :key="img.path"
          v-show="imageUrls[img.path]"
          :src="imageUrls[img.path]"
          :alt="img.name"
          class="sa-image"
          loading="lazy"
        />
      </div>

      <!-- 流式回复 -->
      <div v-if="record.text" class="sa-text">
        <span class="sa-label">回复</span>
        <div class="sa-text-content">{{ record.text }}</div>
      </div>

      <!-- 错误 -->
      <div v-if="record.status === 'error'" class="sa-error">
        <Icon name="close" :size="13" /> {{ record.error }}
      </div>

      <!-- 完成后的结果条 -->
      <div v-if="record.status === 'done'" class="sa-result">
        <span class="sa-result-icon"><Icon name="check" :size="13" /></span>
        <span class="sa-result-text">{{ record.text || '（无文本回复）' }}</span>
        <button type="button" class="sa-copy" @click.stop="copyResult">
          <Icon :name="copied ? 'check' : 'copy'" :size="13" />
          {{ copied ? '已复制' : '复制' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sub-agent-card {
  margin: 6px 0;
  border: 1px solid var(--border-color, rgba(128, 128, 128, 0.25));
  border-radius: 7px;
  background: var(--bg-subtle, rgba(128, 128, 128, 0.06));
  overflow: hidden;
  font-size: 13px;
}

.sa-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  cursor: pointer;
  user-select: none;
}

.sa-avatar {
  display: flex;
  color: var(--accent, #4f7cff);
}

.sa-name {
  font-weight: 600;
}

.sa-model {
  color: var(--text-secondary, rgba(128, 128, 128, 0.9));
  font-size: 12px;
  padding: 1px 6px;
  border-radius: 5px;
  background: rgba(128, 128, 128, 0.12);
}

.sa-depth {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 5px;
  background: rgba(128, 128, 128, 0.12);
  color: var(--text-secondary, rgba(128, 128, 128, 0.9));
}

.sa-status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-left: auto;
  font-size: 12px;
}

.sa-status-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
}

.st-running .sa-status-dot {
  background: #ffb020;
  animation: sa-pulse 1s ease-in-out infinite;
}

.st-done .sa-status-dot {
  background: #2ecc71;
}

.st-error .sa-status-dot {
  background: #ff4d4f;
}

@keyframes sa-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.3;
  }
}

.sa-toggle {
  display: flex;
  color: var(--text-secondary, rgba(128, 128, 128, 0.8));
  transition: transform 0.2s;
}

.sa-summary {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 12px 8px;
  cursor: pointer;
  color: var(--text-secondary, rgba(128, 128, 128, 0.85));
}

.sa-summary-icon {
  display: flex;
  color: var(--accent, #4f7cff);
}

.sa-summary-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sa-body.hidden {
  display: none;
}

.sa-task {
  display: flex;
  gap: 8px;
  padding: 2px 12px 8px;
  align-items: baseline;
}

.sa-label {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary, rgba(128, 128, 128, 0.85));
  background: rgba(128, 128, 128, 0.12);
  padding: 0 6px;
  border-radius: 5px;
  line-height: 18px;
}

.sa-task-text {
  color: var(--text-secondary, rgba(128, 128, 128, 0.9));
  line-height: 1.5;
  word-break: break-all;
}

.sa-tools {
  padding: 0 12px 8px;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.sa-tool-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  padding: 3px 8px;
  border-radius: 7px;
  background: rgba(128, 128, 128, 0.08);
}

.sa-tool-row.err .sa-tool-name {
  color: #ff4d4f;
}

.sa-tool-name {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-weight: 600;
  flex-shrink: 0;
}

.sa-tool-args {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary, rgba(128, 128, 128, 0.8));
}

.sa-tool-state {
  display: flex;
  color: #2ecc71;
}

.sa-tool-state .err {
  color: #ff4d4f;
}

.sa-spin {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  border: 2px solid rgba(128, 128, 128, 0.3);
  border-top-color: #ffb020;
  animation: sa-rotate 0.8s linear infinite;
}

@keyframes sa-rotate {
  to {
    transform: rotate(360deg);
  }
}

.sa-images {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  padding: 0 12px 8px;
}

.sa-image {
  width: 120px;
  height: 120px;
  object-fit: cover;
  border-radius: 7px;
  border: 1px solid var(--border-color, rgba(128, 128, 128, 0.2));
}

.sa-text {
  display: flex;
  gap: 8px;
  padding: 0 12px 8px;
  align-items: flex-start;
}

.sa-text-content {
  flex: 1;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 260px;
  overflow-y: auto;
}

.sa-error {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 12px 10px;
  padding: 8px 10px;
  border-radius: 7px;
  background: rgba(255, 77, 79, 0.1);
  color: #ff4d4f;
  font-size: 12px;
  line-height: 1.5;
  word-break: break-word;
}

.sa-result {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin: 0 12px 10px;
  padding: 8px 10px;
  border-radius: 7px;
  background: rgba(46, 204, 113, 0.08);
  border: 1px solid rgba(46, 204, 113, 0.2);
}

.sa-result-icon {
  display: flex;
  color: #2ecc71;
  margin-top: 2px;
}

.sa-result-text {
  flex: 1;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 180px;
  overflow-y: auto;
}

.sa-copy {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  font-size: 12px;
  padding: 3px 8px;
  border-radius: 5px;
  border: 1px solid var(--border-color, rgba(128, 128, 128, 0.3));
  background: transparent;
  color: var(--text-secondary, rgba(128, 128, 128, 0.9));
  cursor: pointer;
}

.sa-copy:hover {
  background: rgba(128, 128, 128, 0.1);
}
</style>
