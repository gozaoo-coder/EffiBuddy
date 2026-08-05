<script setup lang="ts">
/**
 * ToolCallGroup 工具调用提示组
 *
 * 用于展示 LLM 连续调用多个工具的过程：
 * - 多条工具调用默认折叠为一个组，标题"🔧 使用了 N 个工具"
 * - 展开后每条 tool call 占 54px 高度，显示工具名、参数摘要、状态
 * - 点击某条 tool call → BindSheet 弹出显示该工具的完整参数与返回结果
 *
 * 设计要点：
 * - 连续 tools 折叠显示，避免占用过多纵向空间
 * - 每条 54px 高度限制，超出截断
 * - 点击查看详情使用 BindSheet（side=bottom），与移动端体验一致
 */
import { ref, computed, watch, nextTick } from 'vue'
import { animate } from 'animejs'
import { BindSheet, Icon } from './basic'
import type { ToolCallRecord } from '../types'

const props = withDefaults(
  defineProps<{
    /** 工具调用记录列表（按时间顺序） */
    calls: ToolCallRecord[]
    /** 嵌入模式：去掉卡片外观与自带组标题，由外层(如 ProcessSection)统一控制折叠 */
    embedded?: boolean
    /** 结果预览模式：每条工具调用下方直接显示执行结果摘要（供穿插在思考文字间展示） */
    showResult?: boolean
  }>(),
  {
    embedded: false,
    showResult: false,
  },
)

// 整组是否折叠：多条默认折叠，单条默认展开
const groupCollapsed = ref(props.calls.length > 1)
// 当列表长度变化时，若是首次新增（从 0 → 1），保持展开
watch(
  () => props.calls.length,
  (n, old) => {
    if (old === 0 && n === 1) groupCollapsed.value = false
  },
)

const groupBodyRef = ref<HTMLElement | null>(null)

// 当前选中查看详情的 tool call
const selectedIdx = ref<number | null>(null)
const detailVisible = ref(false)

const selectedCall = computed(() =>
  selectedIdx.value !== null ? props.calls[selectedIdx.value] : null,
)

// 整组展开/折叠切换
function toggleGroup() {
  if (groupCollapsed.value) expandGroup()
  else collapseGroup()
}

function expandGroup() {
  groupCollapsed.value = false
  nextTick(() => {
    const el = groupBodyRef.value
    if (!el) return
    const targetH = el.scrollHeight
    animate(el, {
      maxHeight: ['0px', `${targetH}px`],
      opacity: [0, 1],
      duration: 280,
      ease: 'out(3)',
      onComplete: () => {
        // 释放显式 maxHeight，让后续新增 tool 项可自然撑高并触发动画
        el.style.maxHeight = ''
      },
    })
  })
}

function collapseGroup() {
  const el = groupBodyRef.value
  if (el) {
    animate(el, {
      maxHeight: [`${el.scrollHeight}px`, '0px'],
      opacity: [1, 0],
      duration: 220,
      ease: 'inOut(2)',
      onComplete: () => {
        groupCollapsed.value = true
        el.style.maxHeight = ''
      },
    })
  } else {
    groupCollapsed.value = true
  }
}

// 监听 calls 长度变化：新增 tool 项时，对 group-body 高度变化做 anime.js 动画
// 用 flush:'pre' 在 DOM 更新前捕获旧高度并锁定，nextTick 后测量新自然高度并动画过渡
watch(
  () => props.calls.length,
  (n, old) => {
    if (n <= old) return
    if (groupCollapsed.value) return // 折叠状态下不可见，无需动画
    const el = groupBodyRef.value
    if (!el) return
    // DOM 更新前：当前渲染高度即为旧高度，锁住防止更新时跳变
    const oldHeight = el.offsetHeight
    el.style.maxHeight = oldHeight + 'px'
    nextTick(() => {
      void el.offsetHeight // 强制 reflow，确保 scrollHeight 反映新 DOM
      const newHeight = el.scrollHeight
      if (Math.abs(newHeight - oldHeight) < 1) {
        el.style.maxHeight = ''
        return
      }
      animate(el, {
        maxHeight: [oldHeight + 'px', newHeight + 'px'],
        duration: 220,
        ease: 'out(3)',
        onComplete: () => {
          el.style.maxHeight = ''
        },
      })
    })
  },
  { flush: 'pre' },
)

// 点击单条 tool call → 弹出详情
function openDetail(idx: number) {
  selectedIdx.value = idx
  detailVisible.value = true
}

// 工具图标：根据工具名映射
function toolIcon(name: string): string {
  const map: Record<string, string> = {
    search_history: 'search',
    get_time: 'clock',
    read_file: 'file',
    list_files: 'folder',
    shell: 'settings',
    web_fetch: 'globe',
  }
  return map[name] || 'tool'
}

// 参数摘要：优先提取关键字段(文件路径/URL/命令等)，取前 48 字符
const ARG_KEY_PRIORITY = [
  'file_path',
  'path',
  'url',
  'command',
  'query',
  'pattern',
  'description',
]

function argsSummary(args: string): string {
  if (!args || args === 'null' || args === '{}') return '无参数'
  try {
    const obj = JSON.parse(args)
    if (obj && typeof obj === 'object') {
      for (const k of ARG_KEY_PRIORITY) {
        const v = (obj as Record<string, unknown>)[k]
        if (typeof v === 'string' && v) {
          return v.length > 48 ? v.slice(0, 48) + '…' : v
        }
      }
    }
  } catch {
    /* 非 JSON 时回退原始文本 */
  }
  return args.length > 40 ? args.slice(0, 40) + '…' : args
}

// 美化 JSON 用于详情显示
function prettyJson(s: string): string {
  if (!s) return ''
  try {
    return JSON.stringify(JSON.parse(s), null, 2)
  } catch {
    return s
  }
}

// 状态文案
function statusText(c: ToolCallRecord): string {
  if (c.pending) return '执行中…'
  if (c.is_error) return '失败'
  return '完成'
}

// 完成数量
const doneCount = computed(() => props.calls.filter((c) => !c.pending).length)

// 结果预览摘要：压平空白后截断（单条工具段内的紧凑展示）
function resultSummary(c: ToolCallRecord): string {
  if (c.pending) return '执行中…'
  if (!c.result) return c.is_error ? '执行失败' : '无返回结果'
  const compact = c.result.replace(/\s+/g, ' ').trim()
  return compact.length > 200 ? compact.slice(0, 200) + '…' : compact
}
</script>

<template>
  <div v-if="calls.length > 0" class="tool-group" :class="{ embedded }">
    <!-- 组标题：54px（嵌入模式下由外层提供标题，隐藏） -->
    <div v-if="!embedded" class="group-header" @click="toggleGroup">
      <span class="group-icon"><Icon name="tool" :size="16" /></span>
      <span class="group-title">
        {{ calls.length === 1 ? '使用了工具' : `使用了 ${calls.length} 个工具` }}
      </span>
      <span class="group-progress">{{ doneCount }}/{{ calls.length }}</span>
      <span class="group-arrow"><Icon :name="groupCollapsed ? 'chevron-right' : 'chevron-down'" :size="12" /></span>
    </div>

    <!-- 工具列表：每条 54px（嵌入模式下始终展示，折叠由外层控制） -->
    <div v-show="embedded || !groupCollapsed" ref="groupBodyRef" class="group-body">
      <div
        v-for="(c, idx) in calls"
        :key="c.call_id"
        class="tool-item"
        @click="openDetail(idx)"
      >
        <span class="tool-icon"><Icon :name="toolIcon(c.tool_name)" :size="18" /></span>
        <div class="tool-info">
          <div class="tool-name-row">
            <span class="tool-name">{{ c.tool_name }}</span>
            <span
              class="tool-status"
              :class="{ pending: c.pending, error: c.is_error && !c.pending, ok: !c.pending && !c.is_error }"
            >
              <span v-if="c.pending" class="status-dot"></span>
              {{ statusText(c) }}
            </span>
          </div>
          <div class="tool-args">{{ argsSummary(c.arguments) }}</div>
        </div>
        <span v-if="!embedded" class="tool-arrow"><Icon name="chevron-right" :size="16" /></span>
        <!-- 结果预览：嵌入 + showResult 模式下直接展示执行结果（执行中先隐藏） -->
        <span v-if="showResult && !c.pending" class="tool-result-inline">{{ resultSummary(c) }}</span>
      </div>
    </div>

    <!-- 单条 tool 详情 BindSheet -->
    <BindSheet
      v-model:visible="detailVisible"
      side="bottom"
      :title="selectedCall ? selectedCall.tool_name : '工具详情'"
      :height="'60vh'"
    >
      <div v-if="selectedCall" class="tool-detail">
        <section class="detail-section">
          <h4 class="detail-section-title">工具名</h4>
          <pre class="detail-pre">{{ selectedCall.tool_name }}</pre>
        </section>

        <section class="detail-section">
          <h4 class="detail-section-title">输入参数</h4>
          <pre class="detail-pre">{{ prettyJson(selectedCall.arguments) || '无' }}</pre>
        </section>

        <section class="detail-section">
          <h4 class="detail-section-title">
            返回结果
            <span
              v-if="!selectedCall.pending"
              class="result-status"
              :class="{ error: selectedCall.is_error, ok: !selectedCall.is_error }"
            >{{ selectedCall.is_error ? '失败' : '成功' }}</span>
            <span v-else class="result-status pending">执行中…</span>
          </h4>
          <pre
            v-if="selectedCall.result"
            class="detail-pre"
            :class="{ 'is-error': selectedCall.is_error }"
          >{{ selectedCall.result }}</pre>
          <div v-else class="detail-empty">等待结果返回…</div>
        </section>
      </div>
    </BindSheet>
  </div>
</template>

<style scoped>
.tool-group {
  margin: 6px 0 8px;
  border-radius: var(--radius-lg);
  background: var(--card-2, rgba(0, 0, 0, 0.04));
  border: 1px solid var(--border, rgba(0, 0, 0, 0.08));
  overflow: hidden;
  font-size: 13px;
}

/* 嵌入模式：无外层卡片，每条工具调用为单行文档流样式（无卡片、无边框、无底色） */
.tool-group.embedded {
  margin: 0;
  background: transparent;
  border: none;
  border-radius: 0;
  overflow: visible;
}

.tool-group.embedded .group-body {
  border-top: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

/* 单行布局：icon + 工具名/状态 + 参数摘要 全在一行内，超出省略 */
.tool-group.embedded .tool-item {
  height: auto;
  min-height: 22px;
  max-height: none;
  padding: 0 2px;
  border: none;
  border-radius: 0;
  background: transparent;
  gap: 6px;
  transition: none;
}

.tool-group.embedded .tool-item:hover {
  background: transparent;
  border-color: transparent;
}

.tool-group.embedded .tool-icon {
  width: 16px;
  font-size: 14px;
}

.tool-group.embedded .tool-info {
  flex-direction: row;
  align-items: center;
  gap: 6px;
}

.tool-group.embedded .tool-name-row {
  flex-shrink: 1;
  min-width: 0;
  gap: 5px;
}

.tool-group.embedded .tool-name {
  font-size: 12px;
}

.tool-group.embedded .tool-args {
  flex: 1;
  min-width: 0;
  font-size: 11.5px;
}

.tool-group.embedded .tool-status {
  font-size: 10.5px;
  padding: 0;
  background: transparent;
}

/* 结果预览模式：参数摘要让位给结果，工具行只保留 名称/状态 + 结果 */
.tool-group.embedded.show-result .tool-args {
  display: none;
}

/* 结果预览（showResult 模式）：等宽小字，单行省略，点缀在工具行内 */
.tool-result-inline {
  flex: 1;
  min-width: 0;
  font-size: 11px;
  line-height: 1.5;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  color: var(--muted, #888);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  border-left: 1px solid var(--border, rgba(0, 0, 0, 0.06));
  padding-left: 8px;
}

.tool-group.embedded .tool-item:hover .tool-result-inline {
  color: var(--text, #555);
}

/* 执行中的状态小圆点：呼吸动画 */
.status-dot {
  display: inline-block;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
  animation: status-pulse 1s infinite ease-in-out;
}

@keyframes status-pulse {
  0%, 100% {
    opacity: 0.4;
  }
  50% {
    opacity: 1;
  }
}

/* 组标题：38px */
.group-header {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 38px;
  max-height: 38px;
  padding: 0 12px;
  cursor: pointer;
  user-select: none;
  transition: background var(--duration-fast, 120ms) var(--ease-standard, ease);
}

.group-header:hover {
  background: var(--card, rgba(0, 0, 0, 0.06));
}

.group-icon {
  font-size: 16px;
  line-height: 1;
}

.group-title {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  font-weight: 500;
  color: var(--text, #333);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.group-progress {
  font-size: 11px;
  color: var(--muted, #888);
  padding: 2px 6px;
  background: var(--card, rgba(0, 0, 0, 0.08));
  border-radius: 7px;
}

.group-arrow {
  font-size: 12px;
  color: var(--muted, #888);
}

/* 工具列表 */
.group-body {
  overflow: hidden;
  border-top: 1px solid var(--border, rgba(0, 0, 0, 0.05));
}

/* 每条 tool call：38px 高度 */
.tool-item {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 38px;
  max-height: 38px;
  padding: 0 12px;
  cursor: pointer;
  border-bottom: 1px solid var(--border, rgba(0, 0, 0, 0.04));
  transition: background var(--duration-fast, 120ms) var(--ease-standard, ease);
  overflow: hidden;
}

.tool-item:last-child {
  border-bottom: none;
}

.tool-item:hover {
  background: var(--card, rgba(0, 0, 0, 0.06));
}

.tool-icon {
  font-size: 18px;
  line-height: 1;
  width: 24px;
  text-align: center;
  flex-shrink: 0;
}

.tool-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow: hidden;
}

.tool-name-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.tool-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text, #333);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

.tool-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  padding: 1px 7px;
  border-radius: 7px;
  flex-shrink: 0;
  font-weight: 500;
}

.tool-status.pending {
  color: var(--muted, #888);
  background: var(--card, rgba(0, 0, 0, 0.08));
}

.tool-status.error {
  color: #c62828;
  background: color-mix(in srgb, #e53935 14%, transparent);
}

.tool-status.ok {
  color: #2e7d32;
  background: color-mix(in srgb, #43a047 14%, transparent);
}

.tool-args {
  font-size: 11px;
  color: var(--muted, #888);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.tool-arrow {
  font-size: 18px;
  color: var(--muted, #888);
  flex-shrink: 0;
}

/* BindSheet 详情内容 */
.tool-detail {
  padding: 8px 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.detail-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.detail-section-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text, #333);
  margin: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.result-status {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 7px;
  font-weight: 400;
}

.result-status.pending {
  color: var(--muted, #888);
  background: var(--card, rgba(0, 0, 0, 0.08));
}

.result-status.error {
  color: #fff;
  background: #e53935;
}

.result-status.ok {
  color: #fff;
  background: #43a047;
}

.detail-pre {
  margin: 0;
  padding: 10px 12px;
  font-size: 12px;
  line-height: 1.5;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  background: var(--card-2, rgba(0, 0, 0, 0.04));
  border: 1px solid var(--border, rgba(0, 0, 0, 0.08));
  border-radius: 7px;
  color: var(--text, #333);
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 240px;
  overflow-y: auto;
}

.detail-pre.is-error {
  border-color: #e53935;
  color: #c62828;
}

.detail-empty {
  padding: 12px;
  font-size: 12px;
  color: var(--muted, #888);
  text-align: center;
  background: var(--card-2, rgba(0, 0, 0, 0.04));
  border-radius: 7px;
}
</style>
