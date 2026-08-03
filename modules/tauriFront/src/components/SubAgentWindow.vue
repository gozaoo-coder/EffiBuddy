<script setup lang="ts">
/**
 * SubAgentWindow —— 子 agent 独立会话窗口（main-content 全窗视图）
 *
 * 子 agent 不再仅以卡片内嵌在主会话气泡中：每当主 agent 召唤子 agent，
 * 系统在后台自动打开一个独立会话页签（kind='sub-agent'），本组件作为该页签的
 * 内容视图，实时展示子 agent 的全流程：
 *
 * - 头部：名称 + 模型 + 嵌套深度 + 状态徽标（运行中 / 完成 / 出错）
 * - 任务：主 agent 交给子 agent 的任务原文
 * - 正文：流式回复（实时）、内部工具调用列表、生成的图片
 * - 结果：完成后高亮展示最终回复，可一键复制
 *
 * 数据来源：useSubAgentStore（全局单例），按 session_id 拉取实时记录。
 * 记录在事件推送期间实时累积；即使窗口被关闭后重新打开，历史事件仍可查。
 *
 * 深色/浅色主题自适应（CSS 变量）。图片经 read_attachment 命令转 data URL 渲染。
 */
import { ref, computed, watch, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Icon } from './basic'
import { useSubAgentStore } from '../composables/useSubAgentStore'

const props = defineProps<{
  /** 子 agent 会话 id（来自 sub-agent-event 的 session_id） */
  sessionId: string
}>()

const { getRecord } = useSubAgentStore()

/** 实时记录（响应式；未开始返回 undefined） */
const rec = computed(() => getRecord(props.sessionId))

// 附件图片 data URL 缓存（同 SubAgentCard 逻辑）
const imageUrls = ref<Record<string, string>>({})
watch(
  () => rec.value?.images.length ?? 0,
  async () => {
    const r = rec.value
    if (!r) return
    for (const img of r.images) {
      if (imageUrls.value[img.path]) continue
      try {
        const url = await invoke<string>('read_attachment', { path: img.path })
        imageUrls.value[img.path] = url
      } catch {
        /* 图片读取失败不阻塞窗口 */
      }
    }
  },
  { immediate: true },
)

// 复制结果
const copied = ref(false)
async function copyResult() {
  const r = rec.value
  if (!r) return
  try {
    await navigator.clipboard.writeText(r.text)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)
  } catch {
    /* clipboard 不可用时静默 */
  }
}

// 状态徽标文案与样式
const statusText = computed(() =>
  rec.value?.status === 'done' ? '完成' : rec.value?.status === 'error' ? '出错' : '运行中',
)
const statusClass = computed(() => rec.value?.status ?? 'running')

/** 工具参数摘要（截断单行） */
function argSummary(args: string): string {
  if (!args) return ''
  const s = args.replace(/\s+/g, ' ').trim()
  return s.length > 60 ? s.slice(0, 60) + '…' : s
}
</script>

<template>
  <div class="sub-window" :class="`st-${rec?.status ?? 'running'}`">
    <!-- 头部：名称 / 模型 / 深度 / 状态 -->
    <header class="sw-header">
      <span class="sw-avatar"><Icon name="robot" :size="16" /></span>
      <div class="sw-title">
        <span class="sw-name">{{ rec?.name || '子 agent' }}</span>
        <span class="sw-model">{{ rec?.model || '…' }}</span>
        <span v-if="(rec?.depth ?? 1) > 1" class="sw-depth">深度 {{ rec?.depth }}</span>
      </div>
      <span class="sw-status" :class="statusClass">
        <span class="sw-status-dot" />
        {{ statusText }}
      </span>
    </header>

    <!-- 未开始占位 -->
    <div v-if="!rec" class="sw-empty">
      <Icon name="sparkles" :size="32" />
      <p>等待子 agent 开始…</p>
    </div>

    <!-- 主体 -->
    <div v-else class="sw-body">
      <!-- 任务 -->
      <div v-if="rec.task" class="sw-section">
        <span class="sw-label"><Icon name="file" :size="12" /> 任务</span>
        <div class="sw-task-text">{{ rec.task }}</div>
      </div>

      <!-- 工具调用 -->
      <div v-if="rec.toolCalls.length" class="sw-section">
        <span class="sw-label"><Icon name="tool" :size="12" /> 工具调用（{{ rec.toolCalls.length }}）</span>
        <div class="sw-tools">
          <div
            v-for="(tc, i) in rec.toolCalls"
            :key="tc.call_id || i"
            class="sw-tool-row"
            :class="{ err: tc.is_error }"
          >
            <span class="sw-tool-name">{{ tc.tool_name }}</span>
            <span class="sw-tool-args">{{ argSummary(tc.arguments) }}</span>
            <span class="sw-tool-state">
              <span v-if="tc.pending" class="sw-spin" />
              <Icon v-else-if="tc.is_error" name="close" :size="12" />
              <Icon v-else name="check" :size="12" />
            </span>
          </div>
        </div>
      </div>

      <!-- 生成的图片 -->
      <div v-if="rec.images.length" class="sw-section">
        <span class="sw-label"><Icon name="image" :size="12" /> 生成图片</span>
        <div class="sw-images">
          <img
            v-for="img in rec.images"
            :key="img.path"
            v-show="imageUrls[img.path]"
            :src="imageUrls[img.path]"
            :alt="img.name"
            class="sw-image"
            loading="lazy"
          />
        </div>
      </div>

      <!-- 流式 / 最终回复 -->
      <div v-if="rec.text" class="sw-section">
        <span class="sw-label"><Icon name="chat" :size="12" /> 回复</span>
        <div class="sw-text">{{ rec.text }}</div>
      </div>

      <!-- 错误 -->
      <div v-if="rec.status === 'error'" class="sw-error">
        <Icon name="close" :size="13" /> {{ rec.error }}
      </div>

      <!-- 完成后的结果条 -->
      <div v-if="rec.status === 'done'" class="sw-result">
        <span class="sw-result-icon"><Icon name="check" :size="13" /></span>
        <span class="sw-result-text">{{ rec.text || '（无文本回复）' }}</span>
        <button type="button" class="sw-copy" @click="copyResult">
          <Icon :name="copied ? 'check' : 'copy'" :size="13" />
          {{ copied ? '已复制' : '复制' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sub-window {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: var(--bg);
}

/* 头部 */
.sw-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-2);
  flex-shrink: 0;
  user-select: none;
}

.sw-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border-radius: var(--radius-sm);
  background: rgba(240, 192, 74, 0.14);
  color: var(--warn, #f0c04a);
  flex-shrink: 0;
}

.sw-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1;
}

.sw-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sw-model {
  font-size: 12px;
  color: var(--muted);
  padding: 1px 8px;
  border-radius: var(--radius-full);
  background: var(--card);
  border: 1px solid var(--border);
  flex-shrink: 0;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sw-depth {
  font-size: 11px;
  padding: 1px 8px;
  border-radius: var(--radius-full);
  background: var(--card);
  color: var(--muted);
  flex-shrink: 0;
}

.sw-status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 12px;
  flex-shrink: 0;
}

.sw-status-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
}

.st-running .sw-status-dot {
  background: var(--warn, #f0c04a);
  animation: sw-pulse 1s ease-in-out infinite;
}

.st-done .sw-status-dot {
  background: var(--success, #3ecf8e);
}

.st-error .sw-status-dot {
  background: var(--danger);
}

@keyframes sw-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

/* 主体 */
.sw-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.sw-body::-webkit-scrollbar {
  width: 6px;
}

.sw-body::-webkit-scrollbar-thumb {
  background: var(--border);
  border-radius: 3px;
}

.sw-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--muted);
  font-size: 13px;
}

.sw-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.sw-label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  font-weight: 600;
  color: var(--muted);
  text-transform: none;
}

.sw-task-text {
  font-size: 13px;
  color: var(--text);
  line-height: 1.6;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 10px 12px;
  white-space: pre-wrap;
  word-break: break-word;
}

/* 工具 */
.sw-tools {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.sw-tool-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  padding: 5px 10px;
  border-radius: var(--radius-sm);
  background: var(--bg-2);
  border: 1px solid var(--border);
}

.sw-tool-row.err .sw-tool-name {
  color: var(--danger);
}

.sw-tool-name {
  font-weight: 600;
  flex-shrink: 0;
  font-family: 'SF Mono', 'JetBrains Mono', 'Consolas', monospace;
}

.sw-tool-args {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--muted);
}

.sw-tool-state {
  display: inline-flex;
  color: var(--success, #3ecf8e);
  flex-shrink: 0;
}

.sw-spin {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  border: 2px solid var(--border);
  border-top-color: var(--warn, #f0c04a);
  animation: sw-rotate 0.8s linear infinite;
}

@keyframes sw-rotate {
  to { transform: rotate(360deg); }
}

/* 图片 */
.sw-images {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.sw-image {
  width: 140px;
  height: 140px;
  object-fit: cover;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
}

/* 文本 */
.sw-text {
  font-size: 14px;
  line-height: 1.65;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text);
}

/* 错误 */
.sw-error {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 10px 12px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--danger) 10%, transparent);
  border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent);
  color: var(--danger);
  font-size: 12px;
  line-height: 1.5;
  word-break: break-word;
}

/* 结果 */
.sw-result {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 12px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--success) 8%, transparent);
  border: 1px solid color-mix(in srgb, var(--success) 25%, transparent);
}

.sw-result-icon {
  display: inline-flex;
  color: var(--success, #3ecf8e);
  margin-top: 2px;
  flex-shrink: 0;
}

.sw-result-text {
  flex: 1;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 240px;
  overflow-y: auto;
  color: var(--text);
  font-size: 13px;
}

.sw-copy {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  font-size: 12px;
  padding: 3px 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.sw-copy:hover {
  background: var(--card);
  color: var(--text);
}
</style>
