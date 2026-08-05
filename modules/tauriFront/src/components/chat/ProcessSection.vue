<script setup lang="ts">
/**
 * ProcessSection —— 推理过程 + 工具调用的合并展示区块
 *
 * 设计目标：让 agent 回复像文档一样简洁——
 * - 推理与工具调用合并为单行摘要标题（"已思考 8 秒 · 使用了 3 个工具"）
 * - 进行中（思考中 / 工具执行中）自动展开，全部完成后自动折叠
 * - 展开后：推理文本为灰色小字段落，工具列表以嵌入模式(无卡片)呈现
 * - 点击标题行可随时手动展开/折叠
 */
import { ref, computed, watch, nextTick } from 'vue'
import { animate } from 'animejs'
import { Icon } from '../basic'
import ToolCallGroup from '../ToolCallGroup.vue'
import type { ToolCallRecord } from '../../types'

const props = withDefaults(
  defineProps<{
    /** 推理文本 */
    reasoning?: string
    /** 是否仍在思考中 */
    isThinking?: boolean
    /** 工具调用记录列表 */
    toolCalls?: ToolCallRecord[]
  }>(),
  {
    reasoning: '',
    isThinking: false,
    toolCalls: () => [],
  },
)

// 是否忙碌：思考中或仍有工具在执行
const busy = computed(
  () => props.isThinking || props.toolCalls.some((c) => c.pending),
)

// 历史消息（加载时已全部完成）默认折叠；进行中默认展开
const collapsed = ref(!busy.value)
const bodyRef = ref<HTMLElement | null>(null)
const reasoningRef = ref<HTMLElement | null>(null)

// 已思考时长（秒）
const thinkStart = ref<number>(Date.now())
const thinkDuration = ref<number>(0)

watch(
  () => props.isThinking,
  (thinking, was) => {
    if (thinking && !was) {
      thinkStart.value = Date.now()
    } else if (!thinking && was) {
      thinkDuration.value = Math.max(
        1,
        Math.round((Date.now() - thinkStart.value) / 1000),
      )
    }
  },
)

// 进行中 → 展开；全部完成 → 短暂延迟后自动折叠
let collapseTimer: ReturnType<typeof setTimeout> | null = null
watch(
  busy,
  (b, was) => {
    if (collapseTimer) {
      clearTimeout(collapseTimer)
      collapseTimer = null
    }
    if (b && !was) {
      // 进行中 → 直接展开（无动画）
      collapsed.value = false
    } else if (!b && was) {
      collapseTimer = setTimeout(() => {
        collapseNow()
      }, 600)
    }
  },
)

function toggle() {
  if (collapseTimer) {
    clearTimeout(collapseTimer)
    collapseTimer = null
  }
  if (collapsed.value) expandNow()
  else collapseNow()
}

// 展开：直接显示（无动画），并把思考内容定位到底部
function expandNow() {
  collapsed.value = false
  nextTick(() => {
    const el = reasoningRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

// 合并：仅高度变小动画（不做透明度变化）
function collapseNow() {
  const el = bodyRef.value
  if (el) {
    animate(el, {
      maxHeight: [`${el.scrollHeight}px`, '0px'],
      duration: 200,
      ease: 'inOut(2)',
      onComplete: () => {
        collapsed.value = true
        el.style.maxHeight = ''
      },
    })
  } else {
    collapsed.value = true
  }
}

// 思考内容实时滚动到底部（流式增长时跟随）
watch(
  () => props.reasoning,
  () => {
    if (collapsed.value) return
    nextTick(() => {
      const el = reasoningRef.value
      if (el) el.scrollTop = el.scrollHeight
    })
  },
)

// 标题摘要文案：推理 + 工具数量合并
const titleText = computed(() => {
  const parts: string[] = []
  if (props.isThinking) {
    parts.push('思考中')
  } else if (props.reasoning) {
    parts.push(
      thinkDuration.value > 0 ? `已思考 ${thinkDuration.value} 秒` : '推理过程',
    )
  }
  if (props.toolCalls.length) {
    const pending = props.toolCalls.filter((c) => c.pending).length
    parts.push(
      pending > 0
        ? `执行工具中 ${props.toolCalls.length - pending}/${props.toolCalls.length}`
        : props.toolCalls.length === 1
          ? '使用了工具'
          : `使用了 ${props.toolCalls.length} 个工具`,
    )
  }
  return parts.join(' · ')
})
</script>

<template>
  <div class="process-section" :class="{ collapsed }">
    <!-- 合并标题行：点击切换折叠 -->
    <div class="process-header" @click="toggle">
      <span class="process-icon"><Icon name="thinking" :size="14" /></span>
      <span class="process-title">{{ titleText }}</span>
      <span v-if="busy" class="process-dots">
        <span class="dot"></span><span class="dot"></span><span class="dot"></span>
      </span>
      <span class="process-arrow">
        <Icon :name="collapsed ? 'chevron-right' : 'chevron-down'" :size="12" />
      </span>
    </div>

    <!-- 展开内容：推理文本 + 嵌入模式工具列表 -->
    <div v-show="!collapsed" ref="bodyRef" class="process-body">
      <div v-if="reasoning" ref="reasoningRef" class="process-reasoning">{{ reasoning }}</div>
      <ToolCallGroup v-if="toolCalls.length" :calls="toolCalls" embedded />
    </div>
  </div>
</template>

<style scoped>
/* 无卡片外观：以文档流形式融入 assistant 回复；
   左侧缩进与正文形成明确的层级区分 */
.process-section {
  padding-left: 2px;
  font-size: 13px;
  color: var(--muted, #888);
}

.process-header {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 8px;
  margin-left: -8px;
  border-radius: var(--radius-md);
  cursor: pointer;
  user-select: none;
  transition: background var(--duration-fast, 120ms) var(--ease-standard, ease);
}

.process-header:hover {
  background: var(--card-2, rgba(0, 0, 0, 0.04));
}

.process-icon {
  line-height: 1;
  color: var(--muted, #888);
}

.process-title {
  font-size: 12.5px;
  font-weight: 500;
  color: var(--muted, #888);
  white-space: nowrap;
}

/* 进行中跳动的点 */
.process-dots {
  display: inline-flex;
  gap: 3px;
}

.process-dots .dot {
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background: var(--muted, #aaa);
  animation: process-bounce 1.2s infinite ease-in-out;
}

.process-dots .dot:nth-child(2) {
  animation-delay: 0.15s;
}

.process-dots .dot:nth-child(3) {
  animation-delay: 0.3s;
}

@keyframes process-bounce {
  0%, 80%, 100% {
    transform: translateY(0);
    opacity: 0.5;
  }
  40% {
    transform: translateY(-3px);
    opacity: 1;
  }
}

.process-arrow {
  line-height: 1;
  color: var(--muted, #888);
}

/* 展开内容：左侧细线标识层级，缩进与上下 margin 加大，与标题行/正文明确区分；
   overflow:hidden 供合并时的高度缩小动画裁剪内容 */
.process-body {
  margin: 8px 0 6px 8px;
  padding: 2px 0 2px 12px;
  border-left: 2px solid var(--border, rgba(0, 0, 0, 0.08));
  display: flex;
  flex-direction: column;
  gap: 6px;
  overflow: hidden;
}

.process-reasoning {
  max-height: 200px;
  overflow-y: auto;
  padding: 2px 6px 2px 0;
  font-size: 12.5px;
  line-height: 1.6;
  color: var(--muted, #777);
  white-space: pre-wrap;
  word-break: break-word;
}

.process-reasoning::-webkit-scrollbar {
  width: 6px;
}

.process-reasoning::-webkit-scrollbar-thumb {
  background: var(--border, rgba(0, 0, 0, 0.152));
  border-radius: 3px;
}
</style>
